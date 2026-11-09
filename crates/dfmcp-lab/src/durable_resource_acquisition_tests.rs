//! Original resource requests and residual pursuits retain distinct durable custody.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-resource-continuation-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        false,
        WorldGraph::default(),
    )
}

#[test]
fn resource_acquisition_source_tag_preserves_distinct_legacy_bytes() -> Result<()> {
    let digest = Digest32::of_bytes(b"resource source codec");
    let hex = digest.to_hex();
    let sources = [
        (
            "resource_acquisition_continuation",
            DurablePlanSource::ResourceAcquisitionContinuation {
                summary: "resume".to_owned(),
                raw: "{}".to_owned(),
            },
        ),
        (
            "blueprint",
            DurablePlanSource::Blueprint {
                summary: "resume".to_owned(),
                raw: "{}".to_owned(),
            },
        ),
    ];
    for (kind, source) in sources {
        assert_eq!(
            plan_payload(FortressId::new(7), digest, digest, 1, &source),
            format!("7 {hex} {hex} 00000000000000000000000000000001 {kind} 726573756d65 7b7d"),
        );
        let decoded = parse_plan(
            "7",
            &hex,
            &hex,
            "00000000000000000000000000000001",
            kind,
            "726573756d65",
            "7b7d",
        )?;
        assert_eq!(decoded.source, source);
    }
    assert!(
        parse_plan(
            "7",
            &hex,
            &hex,
            "00000000000000000000000000000001",
            "resource_acquisition_continuation_v2",
            "726573756d65",
            "7b7d",
        )
        .is_err_and(|error| error.code == ErrorCode::CorruptLedger)
    );
    Ok(())
}

#[test]
fn resource_acquisition_source_bounds_refuse_without_journal_changes() -> Result<()> {
    let dir = TempDir::new("bounds");
    let mut store = DurableLabStore::open(&dir.0)?;
    let sealed = snapshot(1);
    let digest = Digest32::of_bytes(b"bounded acquisition");
    let hex = digest.to_hex();
    let exact_bound = DurablePlanSource::ResourceAcquisitionContinuation {
        summary: "s".repeat(MAX_PLAN_SUMMARY_BYTES),
        raw: "x".repeat(MAX_PLAN_REQUEST_BYTES),
    };
    exact_bound.validate_bound()?;
    let (kind, summary, raw) = exact_bound.fields();
    assert_eq!(
        parse_plan(
            "7",
            &hex,
            &hex,
            "00000000000000000000000000000001",
            kind,
            &hex_text(summary),
            &hex_text(raw),
        )?
        .source,
        exact_bound,
    );
    for malformed in [
        "f".to_owned(),
        "ff".to_owned(),
        "zz".to_owned(),
        "78".repeat(MAX_PLAN_REQUEST_BYTES + 1),
    ] {
        assert!(
            parse_plan(
                "7",
                &hex,
                &hex,
                "00000000000000000000000000000001",
                kind,
                "726573756d65",
                &malformed,
            )
            .is_err_and(|error| error.code == ErrorCode::CorruptLedger)
        );
    }
    let before = fs::read(dir.0.join("journal")).map_err(|error| io("read journal", &error))?;
    for (summary, raw) in [
        ("s".repeat(MAX_PLAN_SUMMARY_BYTES + 1), "{}".to_owned()),
        ("resume".to_owned(), "x".repeat(MAX_PLAN_REQUEST_BYTES + 1)),
    ] {
        assert!(
            parse_plan(
                "7",
                &hex,
                &hex,
                "00000000000000000000000000000001",
                kind,
                &hex_text(&summary),
                &hex_text(&raw),
            )
            .is_err_and(|error| error.code == ErrorCode::CorruptLedger)
        );
        assert!(
            store
                .persist_objective_commit(
                    &sealed,
                    digest,
                    1,
                    DurablePlanSource::ResourceAcquisitionContinuation { summary, raw },
                    SessionId::new(8),
                    &[],
                )
                .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
        );
        assert_eq!(
            fs::read(dir.0.join("journal")).map_err(|error| io("read journal", &error))?,
            before,
        );
        assert!(store.objective(sealed.fortress_id, digest).is_none());
        assert!(store.commit(sealed.fortress_id, digest).is_none());
    }
    Ok(())
}

#[test]
fn resource_acquisition_lineage_survives_retirement_reopen_and_compaction() -> Result<()> {
    let dir = TempDir::new("lineage");
    let root = Digest32::of_bytes(b"original acquisition");
    let child = Digest32::of_bytes(b"remaining acquisition");
    let request = r#"{"template":"resource_acquisition","planner":"resource_acquisition_v1","quotas":[{"resource":"WOOD","minimum":25},{"resource":"PLANT","minimum":45}],"assign_labor":false}"#;
    let original = DurablePlanSource::Blueprint {
        summary: "original stock goals".to_owned(),
        raw: request.to_owned(),
    };
    let residual = DurablePlanSource::ResourceAcquisitionContinuation {
        summary: "finish the original stock goals\nwith retained lineage".to_owned(),
        raw: format!(
            "{{\"schema\":\"dfmcp.resource-acquisition-continuation/1\",\"planner\":\"resource_acquisition_v1\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"resource_acquisition\":{request}}}",
            root.to_hex(),
            root.to_hex(),
        ),
    };
    let sealed_root = snapshot(1);
    let sealed_child = snapshot(2);
    let records = [
        (root, 1, &sealed_root, &original),
        (child, 2, &sealed_child, &residual),
    ];
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        for (digest, intent, sealed, source) in records {
            store.persist_objective_commit(
                sealed,
                digest,
                intent,
                source.clone(),
                SessionId::new(8),
                &[],
            )?;
            store.retire_commit(sealed.fortress_id, digest)?;
        }
        store.compact()?;
    }
    for _ in 0..2 {
        let mut store = DurableLabStore::open(&dir.0)?;
        for (digest, _, sealed, source) in records {
            let objective = store
                .objective(sealed.fortress_id, digest)
                .ok_or_else(|| corrupt("resource acquisition objective lost"))?;
            assert_eq!(&objective.source, source);
            assert_eq!(objective.owner_session_id, SessionId::new(8));
            assert_eq!(objective.first_satisfied_anchor, None);
            assert_eq!(objective.restore_abandoned_anchor, None);
            assert!(store.commit(sealed.fortress_id, digest).is_none());
            assert_eq!(&store.load_snapshot(objective.sealed_state_hash)?, sealed);
        }
        store.compact()?;
    }
    Ok(())
}
