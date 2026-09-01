#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MISSING_TICKET: AtomicU64 = AtomicU64::new(1);

fn server_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dwarf-fortress-mcp"))
}

#[test]
fn direct_serve_live_without_ticket_fails_closed() -> Result<(), Box<dyn Error>> {
    let output = server_command()
        .arg("serve-live")
        .env_remove("DFMCP_ADMISSION_TICKET")
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("DFMCP_ADMISSION_TICKET is required"));
    assert!(stderr.contains("scripts/serve_admitted_live.py"));
    Ok(())
}

#[test]
fn nonexistent_ticket_path_fails_before_live_server_startup() -> Result<(), Box<dyn Error>> {
    let ordinal = NEXT_MISSING_TICKET.fetch_add(1, Ordering::Relaxed);
    let missing = env::temp_dir().join(format!(
        "dfmcp-missing-admission-ticket-{}-{ordinal}.json",
        std::process::id()
    ));
    let _ignored = std::fs::remove_file(&missing);
    let output = server_command()
        .arg("serve-live")
        .env("DFMCP_ADMISSION_TICKET", &missing)
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("cannot inspect admission ticket"));
    assert!(!stderr.contains("dwarf-fortress-mcp-live"));
    Ok(())
}
