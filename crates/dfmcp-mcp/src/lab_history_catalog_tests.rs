//! Retained catalog, range and continuation regressions.
//! Owning bead: df-lab-structured-query-f7q.4.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, WorldGraph};

fn world(tick: u64, sequence: u64, epoch: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(94),
        GameTick(tick),
        ObservationCursor { epoch, sequence },
        false,
        WorldGraph::default(),
    )
}

fn context(world: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(941),
        request_id: RequestId::new(1),
        anchor: world.anchor(),
        budget: WorkBudget {
            max_entities: 10_000,
            max_bytes: 4 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(world.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

fn retained(sources: &[WorldSnapshot]) -> VersionRetention {
    let mut history = VersionRetention::new(MAX_CATALOG_VERSIONS, 2);
    for source in sources {
        history.record(source);
    }
    history
}

fn request(value: Value) -> Result<Request> {
    parse(&value.to_string())?.ok_or_else(|| invalid("missing history test request"))
}

fn discover(history: &VersionRetention, current: &WorldSnapshot, value: Value) -> Result<Value> {
    execute(history, current, &context(current), &request(value)?)
}

fn records(value: &Value) -> Result<&Vec<Value>> {
    value["records"].as_array().ok_or_else(|| invariant("test records missing"))
}

fn windows(value: &Value) -> Result<&Vec<Value>> {
    value["replay_windows"].as_array().ok_or_else(|| invariant("test windows missing"))
}

#[test]
fn history_closed_request_preserves_other_query_grammars_and_rejects_ambiguous_fields() -> Result<()> {
    assert!(parse("summary")?.is_none());
    assert!(parse(r#"{"mode":"condition_replay"}"#)?.is_none());
    assert!(parse(r#"{"mode":"history"}"#)?.is_some());
    for raw in [
        r#"{"mode":"history","mode":"history"}"#,
        r#"{"mode":"history","limit":1,"limit":2}"#,
        r#"{"mode":"history","order":"newest_first","order":"oldest_first"}"#,
        r#"{"mode":"history","at":"arbitrary"}"#,
        r#"{"mode":"history","pin":true}"#,
        r#"{"mode":"history","limit":0}"#,
        r#"{"mode":"history","limit":33}"#,
        r#"{"mode":"history","limit":1.5}"#,
        r#"{"mode":"history","order":"tick"}"#,
        r#"{"mode":"history","continuation":{}}"#,
        r#"{"mode":"history","continuation":"lh1:0:no"}"#,
    ] {
        assert_eq!(parse(raw).err().map(|error| error.code), Some(ErrorCode::InvalidRequest), "{raw}");
    }
    let too_long = format!("{{\"mode\":\"history\",\"padding\":\"{}\"}}", "x".repeat(MAX_REQUEST_BYTES));
    assert_eq!(parse(&too_long).err().map(|error| error.code), Some(ErrorCode::BudgetExceeded));
    assert!(parse(&format!("{{\"mode\":\"structured\",\"payload\":\"{}\"}}", "x".repeat(MAX_REQUEST_BYTES)))?.is_none());
    Ok(())
}

#[test]
fn history_pages_recover_exact_anchors_in_both_orders_without_mutating_retention() -> Result<()> {
    let sources: Vec<_> = (1..=7).map(|index| world(index * 10, index * 3, 1)).collect();
    let history = retained(&sources);
    let current = &sources[6];
    let before = format!("{history:?}");
    for (order, expected) in [
        ("oldest_first", (1..=7).collect::<Vec<u64>>()),
        ("newest_first", (1..=7).rev().collect::<Vec<u64>>()),
    ] {
        let mut input = json!({"mode":"history","order":order,"limit":2});
        let mut found = Vec::new();
        let mut generation = None;
        loop {
            let page = discover(&history, current, input.clone())?;
            assert_eq!(page["current_anchor_retained"], true);
            assert_eq!(page["retained_versions"], 7);
            assert_eq!(page["history_pinned"], false);
            assert_eq!(page["world_contents_validated"], false);
            assert_eq!(page["source_work"]["world_contents_scanned"], false);
            if let Some(generation) = &generation {
                assert_eq!(&page["catalog_generation"], generation);
            } else {
                generation = Some(page["catalog_generation"].clone());
            }
            for row in records(&page)? {
                let sequence = row["recording_sequence"].as_u64()
                    .ok_or_else(|| invariant("test sequence missing"))?;
                found.push(sequence);
                assert_eq!(row["anchor"], super::super::anchor_json(&sources[sequence as usize - 1].anchor()));
            }
            if page["continuation"].is_null() {
                assert_eq!(page["truncated"], false);
                break;
            }
            let token = page["continuation"].as_str().ok_or_else(|| invariant("test cursor missing"))?;
            assert_eq!(token.len(), CURSOR_BYTES);
            input = page["next_query"].clone();
            input["limit"] = json!(3);
        }
        assert_eq!(found, expected);
    }
    assert_eq!(format!("{history:?}"), before);
    Ok(())
}

#[test]
fn history_cursor_binds_owner_current_anchor_catalog_membership_and_order() -> Result<()> {
    let sources: Vec<_> = (1..=4).map(|index| world(index, index, 1)).collect();
    let history = retained(&sources);
    let current = &sources[3];
    let page = discover(&history, current, json!({"mode":"history","limit":1}))?;
    let input = page["next_query"].clone();
    let request = request(input.clone())?;
    let mut foreign = context(current);
    foreign.session_id = SessionId::new(942);
    assert_eq!(execute(&history, current, &foreign, &request).unwrap_err().code, ErrorCode::StaleAnchor);
    let newer = world(5, 5, 1);
    assert_eq!(execute(&history, &newer, &context(&newer), &request).unwrap_err().code, ErrorCode::StaleAnchor);
    let mut changed = retained(&sources);
    changed.record(&newer);
    assert_eq!(execute(&changed, &newer, &context(&newer), &request).unwrap_err().code, ErrorCode::StaleAnchor);
    let mut collected = VersionRetention::new(1, 0);
    for source in &sources {
        collected.record(source);
    }
    // Same current anchor, different retained membership.
    collected.collect(&BTreeSet::new());
    assert_eq!(execute(&collected, current, &context(current), &request).unwrap_err().code, ErrorCode::StaleAnchor);
    let mut reversed = input.clone();
    reversed["order"] = json!("oldest_first");
    assert_eq!(discover(&history, current, reversed).unwrap_err().code, ErrorCode::StaleAnchor);
    let mut altered = input;
    let token = altered["continuation"].as_str().ok_or_else(|| invariant("cursor missing"))?.to_owned();
    let last = if token.ends_with('0') { "1" } else { "0" };
    altered["continuation"] = json!(format!("{}{last}", &token[..token.len() - 1]));
    assert_eq!(discover(&history, current, altered).unwrap_err().code, ErrorCode::StaleAnchor);
    Ok(())
}

#[test]
fn recording_holes_split_replay_windows_but_cursor_gaps_preserve_reset_evidence() -> Result<()> {
    let sources = vec![
        world(1, 100, 1), world(2, 101, 1), world(3, 150, 1),
        world(4, 151, 1), world(5, 180, 1),
    ];
    let mut history = VersionRetention::new(2, 2);
    for source in &sources {
        history.record(source);
    }
    history.collect(&[sources[0].state_hash, sources[2].state_hash].into_iter().collect());
    let page = discover(&history, &sources[4], json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(records(&page)?.len(), 4);
    assert_eq!(records(&page)?[1]["recording_sequence"], 3);
    assert_eq!(records(&page)?[1]["recording_gap_before"], json!({"first":2,"last":2,"count":1}));
    assert_eq!(records(&page)?[1]["observation_gap_before"]["count"], 49);
    assert_eq!(windows(&page)?.len(), 2);
    assert_eq!(windows(&page)?[0]["versions"], 1);
    assert_eq!(windows(&page)?[1]["versions"], 3);
    assert_eq!(windows(&page)?[1]["observation_cursor_gaps"], 1);
    assert_eq!(windows(&page)?[1]["stability_resets_at_cursor_gaps"], true);
    for window in windows(&page)? {
        let from = history.find_hex(window["from"].as_str().ok_or_else(|| invariant("from"))?)
            .ok_or_else(|| invariant("from hash"))?;
        let to = history.find_hex(window["to"].as_str().ok_or_else(|| invariant("to"))?)
            .ok_or_else(|| invariant("to hash"))?;
        assert_eq!(history.consecutive_range(&from, &to, 32)?.len() as u64,
            window["versions"].as_u64().ok_or_else(|| invariant("versions"))?);
    }
    assert_eq!(history.consecutive_range(&sources[0].state_hash, &sources[4].state_hash, 32).unwrap_err().code,
        ErrorCode::StaleAnchor);
    Ok(())
}

#[test]
fn collected_prefix_is_explicit_and_old_epochs_are_discoverable_without_replay_authority() -> Result<()> {
    let sources = vec![world(50, 8, 1), world(0, 0, 2), world(1, 1, 2)];
    let history = retained(&sources);
    let page = discover(&history, &sources[2], json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(records(&page)?[0]["condition_replay_eligibility"], "different_epoch");
    assert_eq!(records(&page)?[1]["observation_continuity_from_previous"], "epoch_changed");
    assert_eq!(page["current_epoch_candidate_versions"], 2);
    assert_eq!(windows(&page)?.len(), 1);
    assert_eq!(windows(&page)?[0]["from"], sources[1].state_hash.to_string());
    assert_eq!(windows(&page)?[0]["versions"], 2);
    let mut collected = VersionRetention::new(1, 0);
    for source in &sources {
        collected.record(source);
    }
    collected.collect(&BTreeSet::new());
    let page = discover(&collected, &sources[2], json!({"mode":"history"}))?;
    assert_eq!(records(&page)?[0]["recording_gap_before"], json!({"first":1,"last":2,"count":2}));
    assert_eq!(page["observation_coverage"]["history_complete"], false);
    Ok(())
}

#[test]
fn same_cursor_forks_and_re_recorded_older_worlds_do_not_form_one_replay_interval() -> Result<()> {
    let sources = vec![world(1, 1, 1), world(2, 1, 1), world(3, 3, 1)];
    let history = retained(&sources);
    let page = discover(&history, &sources[2], json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(records(&page)?[1]["observation_continuity_from_previous"], "non_monotonic");
    assert_eq!(windows(&page)?.len(), 2);
    let mut reordered = retained(&sources);
    reordered.record(&sources[0]);
    let page = discover(&reordered, &sources[2], json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(records(&page)?[2]["recording_sequence"], 4);
    assert_eq!(records(&page)?[2]["observation_continuity_from_previous"], "non_monotonic");
    assert_eq!(windows(&page)?.len(), 2);
    Ok(())
}

#[test]
fn history_windows_are_page_relevant_and_never_exceed_actual_replay_bound() -> Result<()> {
    let sources: Vec<_> = (1..=70).map(|index| world(index, index, 1)).collect();
    let history = retained(&sources);
    let current = &sources[69];
    let first = discover(&history, current, json!({"mode":"history","limit":2}))?;
    assert_eq!(windows(&first)?.len(), 1);
    assert_eq!(windows(&first)?[0]["versions"], 32);
    assert_eq!(windows(&first)?[0]["from"], sources[38].state_hash.to_string());
    assert_eq!(windows(&first)?[0]["to"], sources[69].state_hash.to_string());
    let next = discover(&history, current, first["next_query"].clone())?;
    assert_eq!(windows(&next)?[0]["to"], sources[67].state_hash.to_string());
    assert_eq!(windows(&next)?[0]["versions"], 32);
    Ok(())
}

#[test]
fn metadata_budgets_precede_allocation_and_content_size_does_not_become_a_world_scan() -> Result<()> {
    let mut source = world(1, 1, 1);
    let id = EntityId::new(1);
    source.graph.entities.insert(id, EntityRecord {
        id, generation:1, revision:1, kind:EntityKind::Unit,
        label:"x".repeat(128 * 1024), fields:BTreeMap::new(),
    });
    source.refresh_hash();
    let history = retained(&[source.clone()]);
    let request = request(json!({"mode":"history"}))?;
    let mut small = context(&source);
    small.budget.max_bytes = 16_384;
    let value = execute(&history, &source, &small, &request)?;
    assert_eq!(value["source_work"]["retained_metadata_records"], 1);
    assert_eq!(value["source_work"]["world_contents_scanned"], false);
    assert!(value.to_string().len() < 16_384);
    let metadata = value["source_work"]["canonical_metadata_bytes"].as_u64()
        .ok_or_else(|| invariant("metadata bytes"))?;
    small.budget.max_bytes = metadata - 1;
    assert_eq!(execute(&history, &source, &small, &request).unwrap_err().code, ErrorCode::BudgetExceeded);
    let two = [source.clone(), world(2, 2, 1)];
    let mut small = context(&two[1]);
    small.budget.max_entities = 1;
    assert_eq!(execute(&retained(&two), &two[1], &small, &request).unwrap_err().code, ErrorCode::BudgetExceeded);
    let mut small = context(&source);
    small.budget.max_output_tokens = 32;
    assert_eq!(execute(&history, &source, &small, &request).unwrap_err().code, ErrorCode::BudgetExceeded);
    Ok(())
}

#[test]
fn history_authority_cancellation_and_wall_deadline_precede_catalog_disclosure() -> Result<()> {
    let source = world(100, 100, 1);
    let history = retained(&[source.clone()]);
    let request = request(json!({"mode":"history"}))?;
    for case in 0..5 {
        let mut denied = context(&source);
        match case {
            0 => denied.grants.clear(),
            1 => denied.grants[0].expires_at_tick = Some(GameTick(99)),
            2 => denied.grants[0].scope.entity_ids = [EntityId::new(1)].into_iter().collect(),
            3 => denied.grants[0].remaining_uses = Some(0),
            _ => denied.grants[0].scope.fortress_id = Some(FortressId::new(1)),
        }
        assert_eq!(execute(&history, &source, &denied, &request).unwrap_err().code, ErrorCode::CapabilityDenied);
    }
    let mut cancelled = context(&source);
    cancelled.cancellation_requested = true;
    assert_eq!(execute(&history, &source, &cancelled, &request).unwrap_err().code, ErrorCode::CancellationRequested);
    let mut expired = context(&source);
    expired.budget.max_wall_millis = 1;
    assert_eq!(check(&expired, Instant::now() - Duration::from_millis(2)).unwrap_err().code, ErrorCode::BudgetExceeded);
    Ok(())
}

#[test]
fn unrecorded_shared_frontier_is_reported_without_implicitly_recording_it() -> Result<()> {
    let earlier = world(1, 1, 1);
    let current = world(2, 2, 1);
    let history = retained(&[earlier.clone()]);
    let before = format!("{history:?}");
    let page = discover(&history, &current, json!({"mode":"history"}))?;
    assert_eq!(page["current_anchor_retained"], false);
    assert_eq!(page["retained_versions"], 1);
    assert_eq!(records(&page)?[0]["anchor"], super::super::anchor_json(&earlier.anchor()));
    assert_eq!(page["anchor"], super::super::anchor_json(&current.anchor()));
    assert_eq!(format!("{history:?}"), before);
    let empty = discover(&VersionRetention::new(1, 0), &current, json!({"mode":"history"}))?;
    assert_eq!(empty["retained_versions"], 0);
    assert_eq!(empty["returned"], 0);
    assert_eq!(empty["truncated"], false);
    assert!(empty["continuation"].is_null());
    assert!(windows(&empty)?.is_empty());
    Ok(())
}

#[test]
fn a_foreign_fortress_in_retention_is_refused_instead_of_leaking_its_metadata() -> Result<()> {
    let current = world(2, 2, 1);
    let mut foreign = world(1, 1, 1);
    foreign.fortress_id = FortressId::new(95);
    foreign.refresh_hash();
    assert_eq!(discover(&retained(&[foreign, current.clone()]), &current,
        json!({"mode":"history"})).unwrap_err().code, ErrorCode::StaleAnchor);
    Ok(())
}

#[test]
fn payload_fitting_keeps_whole_rows_and_cursor_at_the_actual_returned_prefix() -> Result<()> {
    let sources: Vec<_> = (1..=12).map(|index| world(index, index, 1)).collect();
    let history = retained(&sources);
    let current = &sources[11];
    let full = discover(&history, current, json!({"mode":"history","limit":12}))?;
    let mut narrow = context(current);
    narrow.budget.max_output_tokens = (full.to_string().len() / 8) as u32;
    let page = execute(&history, current, &narrow, &request(json!({"mode":"history","limit":12}))?)?;
    assert!(records(&page)?.len() < records(&full)?.len());
    assert!(!records(&page)?.is_empty());
    assert_eq!(page["truncated"], true);
    let next = discover(&history, current, page["next_query"].clone())?;
    let last = records(&page)?.last().ok_or_else(|| invariant("last"))?["recording_sequence"]
        .as_u64().ok_or_else(|| invariant("last sequence"))?;
    assert_eq!(records(&next)?[0]["recording_sequence"], last - 1);
    for (actual, expected) in records(&page)?.iter().zip(records(&full)?) {
        assert_eq!(actual, expected);
    }
    Ok(())
}
