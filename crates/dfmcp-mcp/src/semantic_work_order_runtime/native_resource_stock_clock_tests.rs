//! Fixed parent slot uses the real native inventory-clock initializer and
//! prospective header codec. No native connection or token is needed here.
use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId};

fn fixture_config(directory: &std::path::Path, endpoint: SocketAddr, configured: bool) -> Value {
    let mut value = json!({
        "schema":"dfmcp.semantic-work-order-development/1",
        "directory":directory,"endpoint":endpoint.to_string(),"initialize":true,
        "sources":{
            "kind":"operator-development-source-acceptance/1",
            "world_folder":"region1","site_id":7,"df_version":"fixture-df",
            "dfhack_version":"fixture-dfhack","citizen_bridge_version":"citizens",
            "citizen_generation":10,"work_order_generation":20,
            "progress_generation":30,"operations_generation":40,
        },
    });
    if configured { value["inventory_clock"] = json!({"generation":60}); }
    value
}
fn fixture_context(fortress: FortressId) -> OperationContext {
    OperationContext {
        session_id:SessionId::new(1701),request_id:RequestId::new(1702),
        anchor:StateAnchor {
            fortress_id:fortress,cursor:ObservationCursor::ORIGIN,
            tick:GameTick(100),state_hash:Digest32::of_bytes(b"stock fixture"),
        },
        budget:WorkBudget {
            max_bytes:512*MIB,max_wall_millis:60_000,max_entities:4096,
            max_actions:1,max_output_tokens:32768,max_game_ticks:1200,
        },
        grants:[Capability::Query,Capability::Plan,Capability::ControlClock]
            .into_iter().map(|capability| CapabilityGrant {
                capability,scope:CapabilityScope {fortress_id:Some(fortress),..CapabilityScope::default()},
                max_risk:RiskTier::Guarded,expires_at_tick:None,remaining_uses:None,
            }).collect(),
        cancellation_requested:false,
    }
}

#[test]
#[cfg(all(target_os="linux",any(target_arch="x86_64",target_arch="aarch64")))]
fn fixed_stock_clock_header_is_acknowledgeable_before_exclusive_creation_and_offline_reopen() -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "RESOURCE_STOCK_CLOCK_FACTORY_CASE";
    const PARENT: &str = "RESOURCE_STOCK_CLOCK_FACTORY_PARENT";
    const CHECKED: &str = "resource-stock-clock-factory-checked";
    let io_error = |_| invalid("stock clock factory fixture I/O");
    if let Ok(case) = std::env::var(CHILD) {
        let configured = case != "unconfigured";
        let config = Config::from_combined_clocks_environment(
            Arc::new(|| Ok(())), || Ok(()), None,
            configured.then_some((60, None)),
        )?;
        let c = fixture_context(config.sources.fortress());
        let native_directory = config.directory.clone();
        let shell = NativeShell::from_combined_config(config)?;
        let parent = PathBuf::from(std::env::var(PARENT).map_err(|_| invalid("fixture parent"))?);
        let path = parent.join("stock-watch-clock.journal");
        assert!(!path.exists());
        assert!(std::env::var(INVENTORY_CLOCK_TOKEN).is_err());
        if !configured {
            assert!(shell.prospective_resource_stock_clock_journal(&parent, &c).is_err());
            assert!(shell.open_resource_stock_clock(
                &parent,Mode::Control,true,Digest32::of_bytes(b"unconfigured"),&c,
            ).is_err());
            assert!(!path.exists());
            println!("{CHECKED}");
            return Ok(());
        }
        let mut query = c.clone();
        query.grants.retain(|grant| grant.capability == Capability::Query);
        let expected = shell.prospective_resource_stock_clock_journal(&parent, &query)?;
        assert!(!path.exists(), "prospective identity created a private journal");
        let (fixed_path,binding) = shell.resource_stock_clock_binding(&parent,&query)?;
        assert_eq!(fixed_path,path);
        assert_eq!(expected,binding.initial_journal_id(
            c.session_id,portfolio_clock_request_id(&path,c.request_id,true),
        ));
        assert_ne!(expected,binding.initial_journal_id(c.session_id,c.request_id));
        for invalid_directory in [
            native_directory.clone(),native_directory.join("nested"),
            parent.join("..").join("project"),
            std::path::Path::new("relative-project").to_path_buf(),
        ] {
            assert!(shell.prospective_resource_stock_clock_journal(&invalid_directory,&query).is_err());
        }
        let mut other = query.clone();
        other.anchor.fortress_id = FortressId::NIL;
        assert!(shell.prospective_resource_stock_clock_journal(&parent,&other).is_err());
        for mode in [Mode::Offline,Mode::Reconcile] {
            assert!(shell.open_resource_stock_clock(&parent,mode,true,expected,&c).is_err());
        }
        assert!(shell.open_resource_stock_clock(&parent,Mode::Control,true,expected,&query).is_err());
        assert!(shell.open_resource_stock_clock(
            &parent,Mode::Control,true,Digest32::of_bytes(b"foreign header"),&c,
        ).is_err());
        assert!(!path.exists(), "refused creation touched private custody");
        if case == "revoked" {
            assert!(shell.open_resource_stock_clock(&parent,Mode::Control,true,expected,&c).is_err());
            assert!(!path.exists());
            println!("{CHECKED}");
            return Ok(());
        }
        let mut journal = shell.open_resource_stock_clock(&parent,Mode::Control,true,expected,&c)?;
        assert_eq!(journal.id(),expected);
        assert_eq!(journal.head(),expected);
        let native = shell.inventory_clock_context(&c,false)?;
        assert!(journal.records(&native)?.is_empty());
        drop(journal);
        let initial = fs::read(&path).map_err(io_error)?;
        // An acknowledged marker is never permission to adopt a collision.
        assert!(shell.open_resource_stock_clock(&parent,Mode::Control,true,expected,&c).is_err());
        assert_eq!(fs::read(&path).map_err(io_error)?,initial);
        assert!(shell.open_resource_stock_clock(
            &parent,Mode::Offline,false,Digest32::of_bytes(b"wrong recovery"),&query,
        ).is_err());
        assert_eq!(fs::read(&path).map_err(io_error)?,initial);
        let mut offline = shell.open_resource_stock_clock(&parent,Mode::Offline,false,expected,&query)?;
        assert_eq!(offline.mode(),dfmcp_adapter::bounded_run::journal::RunMode::Offline);
        assert!(offline.records(&native)?.is_empty());
        drop(offline);
        assert_eq!(fs::read(&path).map_err(io_error)?,initial);
        assert!(std::env::var(INVENTORY_CLOCK_TOKEN).is_err());

        let missing = parent.with_extension("missing");
        fs::create_dir(&missing).map_err(io_error)?;
        fs::set_permissions(&missing,fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        assert_ne!(shell.prospective_resource_stock_clock_journal(&missing,&query)?,expected);
        assert!(shell.open_resource_stock_clock(&missing,Mode::Offline,false,expected,&query).is_err());
        assert!(!missing.join("stock-watch-clock.journal").exists());
        println!("{CHECKED}");
        return Ok(());
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let endpoint = listener.local_addr().map_err(io_error)?;
    let root = std::env::temp_dir().join(format!(
        "dfmcp-stock-clock-factory-{}-{}",std::process::id(),endpoint.port(),
    ));
    fs::create_dir(&root).map_err(io_error)?;
    fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    let full_name = format!(
        "{}::fixed_stock_clock_header_is_acknowledgeable_before_exclusive_creation_and_offline_reopen",
        module_path!(),
    );
    let (_,test_name) = full_name.split_once("::").ok_or_else(|| invalid("fixture test name"))?;
    for case in ["configured","unconfigured","revoked"] {
        let parent = root.join(case);
        fs::create_dir(&parent).map_err(io_error)?;
        fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        let raw = fixture_config(&root.join("native-orders"),endpoint,case!="unconfigured");
        let mut command = std::process::Command::new(std::env::current_exe().map_err(io_error)?);
        command.args(["--exact",test_name,"--nocapture"]).env_clear()
            .env(OPT_IN,"1").env(CONFIG,raw.to_string()).env(CHILD,case).env(PARENT,&parent);
        if case == "configured" { command.env(INVENTORY_CLOCK_AUTHORITY,"1"); }
        let output = command.output().map_err(io_error)?;
        assert!(output.status.success(),"stock-clock factory {case}: {}{}",
            String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains(CHECKED));
        assert!(matches!(listener.accept(),Err(cause) if cause.kind()==std::io::ErrorKind::WouldBlock));
    }
    Ok(())
}
