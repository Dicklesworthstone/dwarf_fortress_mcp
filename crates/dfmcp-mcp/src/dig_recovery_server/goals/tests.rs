use super::*;
use dfmcp_adapter::dig_designation::{DigObservation, rpc::DigManifest};
use dfmcp_core::{RequestId, SessionId, WorkBudget};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fixtures() -> Result<Value> {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/excavation_inventory.json"
    )))
    .map_err(|_| denied())
}
fn bytes(all: &Value, case: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for index in case["frames"].as_array().ok_or_else(denied)? {
        let index = usize::try_from(index.as_u64().ok_or_else(denied)?).map_err(|_| denied())?;
        out.extend_from_slice(all["frames"][index].as_str().ok_or_else(denied)?.as_bytes());
    }
    Ok(out)
}
fn named(name: &str) -> Result<Vec<u8>> {
    let all = fixtures()?;
    let case = all["accepted"]
        .as_array()
        .ok_or_else(denied)?
        .iter()
        .find(|c| c["name"] == name)
        .ok_or_else(denied)?;
    bytes(&all, case)
}
fn spec(path: PathBuf, raw: &[u8]) -> Result<FileSpec> {
    Ok(FileSpec {
        label: "tunnel".into(),
        goal_id: Archive::decode(raw, || Ok(()))?.id.to_string(),
        journal: path,
    })
}
fn packet() -> Result<String> {
    let raw = super::super::presentation::packet(
        "fortress.query",
        json!({"ok":true}),
        None,
        None,
        None,
        None,
    );
    let mut v: Value = serde_json::from_str(&raw).map_err(|_| denied())?;
    v["agent_turn"]["active_work"]["indeterminate_effects"] =
        json!([{"idempotency_key":"native-unknown"}]);
    serde_json::to_string(&v).map_err(|_| denied())
}
fn binding() -> Result<DigBinding> {
    // Independent native incarnations deliberately differ: map=7, dig=41.
    let mut raw = b"DFMDG016".to_vec();
    for n in [41u64, 0, 0] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    for n in [1u32, 64, 64, 8, 15, 15, 2, 2, 2] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    raw.push(1);
    raw.extend_from_slice(&7u16.to_be_bytes());
    raw.extend_from_slice(b"region1");
    raw.extend_from_slice(&48u16.to_be_bytes());
    raw.extend_from_slice(&[0; 48]);
    DigBinding::new(
        "127.0.0.1:5000".parse().map_err(|_| denied())?,
        DigManifest {
            generation: 41,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        },
        &DigObservation::decode(&raw)?,
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
    )
}
fn context(b: &DigBinding) -> OperationContext {
    super::super::context(
        SessionId::new(1),
        RequestId::new(1),
        b.fortress_id(),
        b.scope(),
        0,
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 8192,
            max_entities: 1000,
            max_actions: 1,
            max_game_ticks: 0,
        },
    )
}

#[test]
fn python_canonical_strings_include_controls_unicode_and_surrogate_pairs() -> TestResult {
    for example in fixtures()?["canonical_strings"]
        .as_array()
        .ok_or_else(denied)?
    {
        assert_eq!(
            archive::canonical(&example["text"])?,
            example["ascii_json"]
                .as_str()
                .ok_or_else(denied)?
                .as_bytes()
        );
    }
    assert!(archive::canonical(&json!(1.0)).is_err());
    let v = json!({"z":0,"a":1,"\u{10000}":2,"\u{ffff}":3});
    assert_eq!(
        String::from_utf8(archive::canonical(&v)?)?,
        "{\"a\":1,\"z\":0,\"\\uffff\":3,\"\\ud800\\udc00\":2}"
    );
    Ok(())
}

#[test]
fn complete_python_journals_recompute_expected_goal_states() -> TestResult {
    let all = fixtures()?;
    for case in all["accepted"].as_array().ok_or_else(denied)? {
        let a = Archive::decode(&bytes(&all, case)?, || Ok(()))?;
        assert_eq!(a.id.to_string(), case["id"].as_str().ok_or_else(denied)?);
        assert_eq!(
            a.head.to_string(),
            case["head"].as_str().ok_or_else(denied)?
        );
        assert_eq!(a.pending_read, case["pending_read"] == true);
        assert_eq!(
            if a.pending_read {
                "unknown"
            } else {
                a.progress.status().as_str()
            },
            case["status"].as_str().ok_or_else(denied)?
        );
        assert_eq!(
            u64::from(if a.pending_read {
                0
            } else {
                a.progress.streak()
            }),
            case["streak"].as_u64().ok_or_else(denied)?
        );
        assert_eq!(
            a.progress.latest().tick().get(),
            case["sample_tick"].as_u64().ok_or_else(denied)?
        );
    }
    Ok(())
}

#[test]
fn rehashed_but_semantically_invalid_journal_transitions_are_rejected() -> TestResult {
    let all = fixtures()?;
    for case in all["rejected"].as_array().ok_or_else(denied)? {
        assert!(
            Archive::decode(&bytes(&all, case)?, || Ok(())).is_err(),
            "{}",
            case["name"]
        );
    }
    Ok(())
}

#[test]
fn all_partial_prefixes_and_byte_corruptions_fail_except_complete_frame_boundaries() -> TestResult {
    let raw = named("satisfied")?;
    for n in 0..raw.len() {
        assert_eq!(
            Archive::decode(&raw[..n], || Ok(())).is_ok(),
            n > 0 && raw[n - 1] == b'\n',
            "prefix {n}"
        );
    }
    for n in 0..raw.len() {
        let mut changed = raw.clone();
        changed[n] ^= 1;
        assert!(Archive::decode(&changed, || Ok(())).is_err(), "byte {n}");
    }
    Ok(())
}

#[test]
fn duplicate_fields_whitespace_and_raw_unicode_cannot_bypass_exact_canonical_frames() -> TestResult
{
    let raw = String::from_utf8(named("unicode_identity")?)?;
    for changed in [
        raw.replacen("\"event\":", "\"event\":null,\"event\":", 1),
        raw.replacen("{", "{ ", 1),
        raw.replace("\\u00e9", "é"),
        raw.clone() + "\n",
    ] {
        assert!(Archive::decode(changed.as_bytes(), || Ok(())).is_err());
    }
    Ok(())
}

#[test]
fn replay_is_bounded_and_checks_runtime_cancellation_between_frames() -> TestResult {
    let raw = named("satisfied")?;
    let mut checks = 0;
    assert!(
        Archive::decode(&raw, || {
            checks += 1;
            if checks == 2 {
                Err(budget_error())
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    assert_eq!(checks, 2);
    assert!(Archive::decode(&vec![b'\n'; MAX_BYTES + 1], || Ok(())).is_err());
    assert!(Archive::decode(&vec![b' '; archive::MAX_FRAME + 1], || Ok(())).is_err());
    Ok(())
}

#[test]
fn configuration_is_closed_bounded_deterministic_and_operator_only() -> TestResult {
    let valid = json!([{"label":"tunnel","goal_id":"01".repeat(32),"journal":"/private/tunnel"}]);
    assert_eq!(Files::parse(&valid.to_string())?.0.len(), 1);
    for changed in [
        json!([{"label":"tunnel","goal_id":"01".repeat(32),"journal":"../tunnel"}]),
        json!([{"label":"tunnel","goal_id":"0".repeat(64),"journal":"/private/tunnel"}]),
        json!([{"label":"tunnel","goal_id":"01".repeat(32),"journal":"/private/tunnel","mode":"sample"}]),
        json!([valid[0], valid[0]]),
        json!([valid[0], valid[0], valid[0], valid[0], valid[0]]),
    ] {
        assert!(Files::parse(&changed.to_string()).is_err());
    }
    for path in ["/", "/private//a", "/private/./a", "/private/../a"] {
        let mut changed = valid.clone();
        changed[0]["journal"] = json!(path);
        assert!(Files::parse(&changed.to_string()).is_err());
    }
    let a = json!({"label":"a","goal_id":"01".repeat(32),"journal":"/private/a"});
    let b = json!({"label":"b","goal_id":"02".repeat(32),"journal":"/private/b"});
    assert_eq!(
        Files::parse(&json!([a, b]).to_string())?,
        Files::parse(&json!([b, a]).to_string())?
    );
    Ok(())
}

#[test]
fn satisfied_floor_evidence_cannot_clear_native_unknown_or_grant_retry() -> TestResult {
    let raw = named("satisfied")?;
    let file = spec(PathBuf::from("/private/progress"), &raw)?;
    let row = summary(&file, &Archive::decode(&raw, || Ok(()))?)?;
    assert_eq!(row["floor_goal_satisfied_at_sample"], true);
    assert_eq!(row["mining_action_completed_proven"], false);
    let value: Value = serde_json::from_str(&attach(packet()?, vec![row], 32768)?)?;
    assert_eq!(
        value["agent_turn"]["active_work"]["indeterminate_effects"][0]["idempotency_key"],
        "native-unknown"
    );
    let goals = &value["agent_turn"]["active_work"]["excavation_goals"];
    assert_eq!(goals["pending_count"], 0);
    assert_eq!(goals["map_and_dig_generations_joined"], false);
    assert_eq!(goals["game_effect_obligations_changed"], false);
    Ok(())
}

#[test]
fn unfinished_reads_and_unavailable_prior_success_are_still_visible_as_unknown() -> TestResult {
    let raw = named("read_unfinished")?;
    let file = spec(PathBuf::from("/private/progress"), &raw)?;
    let row = summary(&file, &Archive::decode(&raw, || Ok(()))?)?;
    assert_eq!(row["goal_status"], "unknown");
    assert_eq!(row["matching_samples"], 0);
    let pin = Pin {
        identity: (1, 2, 3, 4),
        length: 1,
        digest: Digest32::ZERO,
        summary: json!({"goal_status":"satisfied","terminal":true}),
    };
    let absent = unavailable(&file, Some(&pin));
    assert_eq!(absent["goal_status"], "unknown");
    assert_eq!(absent["terminal"], false);
    let out: Value = serde_json::from_str(&attach(packet()?, vec![absent], 32768)?)?;
    let goals = &out["agent_turn"]["active_work"]["excavation_goals"];
    assert_eq!(goals["inventory_verified"], false);
    assert_eq!(goals["pending_absence_proven"], false);
    assert_eq!(goals["pending_count"], 1);
    Ok(())
}

#[test]
fn four_complete_goal_summaries_fit_and_overflow_preserves_native_and_goal_work() -> TestResult {
    let raw = named("satisfied")?;
    let a = Archive::decode(&raw, || Ok(()))?;
    let mut rows = Vec::new();
    for n in 1..=4 {
        let file = FileSpec {
            label: format!("{n}{}", "x".repeat(47)),
            goal_id: format!("{n:064x}"),
            journal: "/private/evidence".into(),
        };
        let pin = Pin {
            identity: (1, 2, 3, 4),
            length: raw.len(),
            digest: Digest32::ZERO,
            summary: summary(&file, &a)?,
        };
        rows.push(unavailable(&file, Some(&pin)));
    }
    assert!(serde_json::to_vec(&rows)?.len() <= OUTPUT_RESERVE);
    let mut base: Value = serde_json::from_str(&packet()?)?;
    base["result"]["optional_details"] = json!("x".repeat(32768));
    let rendered = attach(base.to_string(), rows, 32768)?;
    assert!(rendered.len() <= 32768);
    let v: Value = serde_json::from_str(&rendered)?;
    assert_eq!(v["result"]["ok"], false);
    assert_eq!(
        v["agent_turn"]["active_work"]["excavation_goals"]["goals"]
            .as_array()
            .ok_or_else(denied)?
            .len(),
        4
    );
    assert_eq!(
        v["agent_turn"]["active_work"]["indeterminate_effects"][0]["idempotency_key"],
        "native-unknown"
    );
    Ok(())
}

#[test]
fn absent_configuration_preserves_packet_and_reservations_do_not_widen_budgets() -> TestResult {
    let raw = packet()?;
    assert_eq!(attach(raw.clone(), vec![], 32768)?, raw);
    let b = binding()?;
    let mut c = context(&b);
    let file = spec("/private/progress".into(), &named("floor_baseline")?)?;
    let inventory = Inventory::new(Files(vec![file]));
    assert_eq!(
        inventory.reserve(&c)?.budget.max_bytes,
        c.budget.max_bytes - WORK_PER_GOAL
    );
    c.budget.max_bytes = 1;
    assert!(inventory.reserve(&c).is_err());
    c = context(&b);
    c.budget.max_bytes += 1;
    assert!(super::super::Work::new(&c, Instant::now()).is_err());
    Ok(())
}

#[test]
fn release_marks_independent_goals_unverified_and_does_no_io() -> TestResult {
    let files = Files(vec![spec(
        "/this/path/does/not/exist".into(),
        &named("floor_baseline")?,
    )?]);
    let inventory = Inventory::new(files);
    let v: Value = serde_json::from_str(&inventory.release(packet()?, 32768)?)?;
    assert_eq!(
        v["agent_turn"]["active_work"]["excavation_goals"]["goals"][0]["verification"],
        "unavailable"
    );
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> std::io::Result<Self> {
            let dir = std::env::temp_dir().join(format!(
                "dfmcp-floor-inventory-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir)?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            Ok(Self(dir))
        }
        fn put(&self, name: &str, raw: &[u8]) -> std::io::Result<PathBuf> {
            let path = self.0.join(name);
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            f.write_all(raw)?;
            Ok(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn show(inventory: &mut Inventory, b: &DigBinding) -> Result<Value> {
        let c = inventory.reserve(&context(b))?;
        let read = inventory.load(&c, b, Instant::now(), || Ok(()))?;
        serde_json::from_str(&read.finish(inventory, packet()?, 32768, || Ok(()))?)
            .map_err(|_| denied())
    }
    fn row(v: &Value) -> &Value {
        &v["agent_turn"]["active_work"]["excavation_goals"]["goals"][0]
    }

    #[test]
    fn existing_python_history_is_visible_without_creating_or_changing_files() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let mut inventory = Inventory::new(Files(vec![spec(path.clone(), &raw)?]));
        let v = show(&mut inventory, &binding()?)?;
        assert_eq!(row(&v)["goal_status"], "satisfied");
        assert_eq!(row(&v)["map_generation"], 7);
        assert_eq!(inventory.high_tick, 20);
        assert_eq!(fs::read(path)?, raw);
        assert_eq!(fs::read_dir(&temp.0)?.count(), 1);
        Ok(())
    }

    #[test]
    fn append_progress_is_accepted_but_truncation_cannot_restore_a_previous_state() -> TestResult {
        let temp = Temp::new()?;
        let first = named("floor_baseline")?;
        let last = named("satisfied")?;
        assert!(last.starts_with(&first));
        let path = temp.put("goal", &first)?;
        let mut inventory = Inventory::new(Files(vec![spec(path.clone(), &first)?]));
        let b = binding()?;
        assert_eq!(
            row(&show(&mut inventory, &b)?)["goal_status"],
            "stabilizing"
        );
        OpenOptions::new()
            .append(true)
            .open(&path)?
            .write_all(&last[first.len()..])?;
        assert_eq!(row(&show(&mut inventory, &b)?)["goal_status"], "satisfied");
        fs::write(&path, &first)?;
        let v = show(&mut inventory, &b)?;
        assert_eq!(row(&v)["verification"], "unavailable");
        assert_eq!(row(&v)["historical_prior"]["goal_status"], "satisfied");
        assert_eq!(inventory.high_tick, 20);
        Ok(())
    }

    #[test]
    fn missing_busy_replaced_or_corrupt_files_are_unknown_not_absent() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let mut inventory = Inventory::new(Files(vec![spec(path.clone(), &raw)?]));
        let b = binding()?;
        show(&mut inventory, &b)?;
        let locked = OpenOptions::new().read(true).open(&path)?;
        locked.try_lock()?;
        assert_eq!(
            row(&show(&mut inventory, &b)?)["verification"],
            "unavailable"
        );
        drop(locked);
        fs::rename(&path, temp.0.join("old"))?;
        assert_eq!(
            row(&show(&mut inventory, &b)?)["verification"],
            "unavailable"
        );
        temp.put("goal", &raw)?;
        assert_eq!(
            row(&show(&mut inventory, &b)?)["verification"],
            "unavailable"
        );
        fs::write(&path, b"corrupt\n")?;
        assert_eq!(row(&show(&mut inventory, &b)?)["goal_status"], "unknown");
        Ok(())
    }

    #[test]
    fn final_file_change_withdraws_verification_and_does_not_publish_a_pin() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let mut inventory = Inventory::new(Files(vec![spec(path.clone(), &raw)?]));
        let b = binding()?;
        let c = context(&b);
        let read = inventory.load(&c, &b, Instant::now(), || Ok(()))?;
        fs::write(&path, b"corrupt\n")?;
        let v: Value =
            serde_json::from_str(&read.finish(&mut inventory, packet()?, 32768, || Ok(()))?)?;
        assert_eq!(row(&v)["verification"], "unavailable");
        assert!(inventory.pins[0].is_none());
        assert_eq!(
            v["agent_turn"]["active_work"]["indeterminate_effects"][0]["idempotency_key"],
            "native-unknown"
        );
        Ok(())
    }

    #[test]
    fn wrong_pinned_goal_or_fortress_is_not_imported_as_authority() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let mut file = spec(path, &raw)?;
        file.goal_id = "01".repeat(32);
        let mut inventory = Inventory::new(Files(vec![file]));
        assert_eq!(
            row(&show(&mut inventory, &binding()?)?)["verification"],
            "unavailable"
        );
        let unicode = named("unicode_identity")?;
        let path = temp.put("other-fort", &unicode)?;
        let mut inventory = Inventory::new(Files(vec![spec(path, &unicode)?]));
        assert_eq!(
            row(&show(&mut inventory, &binding()?)?)["verification"],
            "unavailable"
        );
        assert_eq!(inventory.high_tick, 0);
        Ok(())
    }

    #[test]
    fn current_query_authority_is_checked_before_io_and_at_observed_tick() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let mut inventory = Inventory::new(Files(vec![spec(path, &raw)?]));
        let b = binding()?;
        let mut c = context(&b);
        c.grants.clear();
        assert!(inventory.load(&c, &b, Instant::now(), || Ok(())).is_err());
        c = context(&b);
        c.grants[0].expires_at_tick = Some(GameTick(15));
        assert!(inventory.load(&c, &b, Instant::now(), || Ok(())).is_err());
        assert_eq!(inventory.high_tick, 20);
        let mut lowered = context(&b);
        inventory.narrow(&mut lowered);
        assert_eq!(lowered.anchor.tick, GameTick(20));
        Ok(())
    }

    #[test]
    fn nofollow_private_modes_single_link_and_final_identity_are_required() -> TestResult {
        let temp = Temp::new()?;
        let raw = named("satisfied")?;
        let path = temp.put("goal", &raw)?;
        let link = temp.0.join("link");
        symlink(&path, &link)?;
        assert!(Snapshot::open(&link, &mut || Ok(())).is_err());
        fs::remove_file(&link)?;
        fs::hard_link(&path, &link)?;
        assert!(Snapshot::open(&path, &mut || Ok(())).is_err());
        fs::remove_file(&link)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        assert!(Snapshot::open(&path, &mut || Ok(())).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let mut snapshot = Snapshot::open(&path, &mut || Ok(()))?;
        fs::rename(&path, temp.0.join("original"))?;
        temp.put("goal", &raw)?;
        assert!(snapshot.verify(&mut || Ok(())).is_err());
        Ok(())
    }
}
