//! Existing-session negotiation must never discard custody to obtain more room.

use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
    fortress: FortressId,
    shared: bool,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if self.shared {
            release_shared_world(self.fortress);
        } else if let Ok(id) = parse_session_id_arg(&self.id) {
            sessions().remove(&id);
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(scenario: &str, tokens: u32, shared: bool) -> TestResult<Owner> {
    let fortress = FortressId::new(81_702);
    let output = parsed(&open_session_in_scenario(
        Some(true),
        Some(fortress.to_string()),
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("configure_production".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
            ("checkpoint".into(), "guarded".into()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(tokens),
        Some(64),
        Some(scenario.into()),
        Some(shared),
        Some(false),
    ))?;
    assert_eq!(output["ok"], true, "{output}");
    Ok(Owner {
        id: output["session_id"].as_str().ok_or("session ID")?.into(),
        fortress,
        shared,
    })
}

fn state(owner: &Owner) -> TestResult<Value> {
    let session = lookup_session(parse_session_id_arg(&owner.id)?)?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    let context = read_context(&guard);
    Ok(json!({
        "budget": budget_json(guard.budget),
        "request_id": guard.next_request_id.to_string(),
        "session_id": guard.session_id.to_string(),
        "adapter": format!("{:?}", guard.adapter),
        "grants": format!("{:?}", guard.grants),
        "negotiation": guard.negotiation.to_json(),
        "pending": guard.pending.as_ref().map(|pending| json!({
            "digest": pending.digest,
            "plan": format!("{:?}", pending.plan),
            "source": format!("{:?}", pending.source),
        })),
        "actions": format!("{:?}", (guard.last_action, &guard.last_plan_actions, &guard.open_actions)),
        "receipts": guard.commit_receipts,
        "commit_authority": format!("{:?}", guard.commit_authority),
        "leases": format!("{:?}", guard.leases),
        "shared_members": guard.shared_members,
        "history": format!("{:?}", guard.history),
        "objectives": format!("{:?}", guard.objectives),
        "objective_restore": format!("{:?}", guard.objective_restore),
        "durable_plans": format!("{:?}", guard.durable_plans),
        "durable_restore": format!("{:?}", guard.durable_restore),
        "durable_scenario": guard.durable_scenario,
        "durability_fault": guard.durability_fault,
        "carried": format!("{:?}", guard.carried),
        "civilian_recovery": guard.civilian_recovery.projection(),
        "replay": crate::replay::bundle_json(&guard.replay),
        "baselines": format!("{:?}", temporal_query::retained_baseline_work(&context)),
        "watches": format!("{:?}", temporal_query::retained_watch_work(&context)),
    }))
}

fn query(owner: &Owner, query: Value) -> TestResult<Value> {
    let output = parsed(&fortress_query(
        Some(owner.id.clone()),
        Some(
            json!({"mode":"temporal", "request":{
                "schema":"dfmcp.query/1", "query":query,
            }})
            .to_string(),
        ),
    ))?;
    assert_eq!(output["ok"], true, "{output}");
    Ok(output)
}

fn refused_unchanged(owner: &Owner, tokens: u32, code: ErrorCode) -> TestResult {
    let before = state(owner)?;
    let result = renegotiate(owner.id.clone(), tokens, Instant::now());
    assert!(
        matches!(result, Err(ref error) if error.code == code),
        "{result:?}"
    );
    assert_eq!(state(owner)?, before);
    Ok(())
}

#[test]
fn renegotiation_preserves_live_work_pending_seals_query_handles_and_unknown_recovery() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open("starter_fortress", 16_384, false)?;
    let plan = parsed(&plan_with_actions(
        Some(owner.id.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"create_work_order",
            "name":"retained brewing", "job_token":"BREW_DRINK", "amount":2}}])
            .to_string(),
        ),
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let committed = parsed(&fortress_commit(
        Some(owner.id.clone()),
        plan["plan_digest"].as_str().ok_or("plan digest")?.into(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let pending = parsed(&fortress_plan(Some(owner.id.clone()), None, Some(false)))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let baseline = query(
        &owner,
        json!({"kind":"capture", "key":"roster",
        "max_game_ticks":100, "select":{"kind":"entities", "kinds":["unit"],
        "fields":["alive"]}}),
    )?;
    let watch = query(
        &owner,
        json!({"kind":"watch", "key":"drink-recovery",
        "condition":{"op":"field", "entity_id":crate::lab_world::starter::STOCK_LEDGER.to_string(),
            "generation":1, "field":dfmcp_intent::effects::STOCK_DRINK_FIELD,
            "comparison":"ge", "value":{"type":"u64", "value":100}},
        "deadline_tick":100, "poll_interval_ticks":1, "stable_observations":2}),
    )?;
    let (anchor, expected_work, expected_objectives, carried) = {
        let session = resolve_session(Some(owner.id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let anchor = guard.adapter.snapshot().anchor();
        let carried = CarriedStep {
            plan_digest: Digest32::of_bytes(b"output-negotiation-retained-work"),
            step: Some(dfmcp_core::StepId::new(0)),
            kind: "recovered_work",
            proof: Predicate::True,
            failure: None,
            deadline: Some(GameTick(200)),
            monitor: None,
            recorded_state: "verified".into(),
            proof_anchor: Some(anchor),
            observation_error: None,
            failure_reason: None,
            work_step: None,
            work_state: EffectWorkState::Unknown {
                entity_id: None,
                reason: "physical work has not been reobserved".into(),
            },
            work_anchor: None,
            state: "verified".into(),
        };
        let retained = carried.to_json();
        guard.carried.push(carried);
        let projection = temporal_handler::projection(&guard, &read_context(&guard))?;
        (
            anchor,
            projection.active_work,
            projection.objectives,
            retained,
        )
    };
    let mut expected = state(&owner)?;
    let original_request = expected["request_id"]
        .as_str()
        .ok_or("request ID")?
        .parse::<u128>()?;
    let raw = renegotiate(owner.id.clone(), 32_768, Instant::now())?;
    let result = parsed(&raw)?;
    assert_eq!(result["ok"], true);
    assert_eq!(result["schema"], "dfmcp.session-output-budget/1");
    assert_eq!(result["session_id"], owner.id);
    assert_eq!(result["anchor"], anchor_json(&anchor));
    assert_eq!(result["previous_budget"], expected["budget"]);
    assert_eq!(result["game_effect"], "none");
    assert_eq!(result["changed"], true);
    assert!(raw.len() <= 32_768 * 4);
    assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(
        result["agent_turn"]["continuity"]["basis"],
        anchor_json(&anchor)
    );
    let work = &result["agent_turn"]["active_work"];
    for name in ["pending_plans", "actions", "obligations", "untracked_work"] {
        assert_eq!(work[name], expected_work[name]);
    }
    assert!(
        !work["obligations"]
            .as_array()
            .ok_or("obligations")?
            .is_empty()
    );
    assert_eq!(
        work["pending_plans"][0]["plan_digest"],
        pending["plan_digest"]
    );
    assert_eq!(
        work["query_baselines"][0]["baseline"],
        baseline["captured"]["baseline"]
    );
    assert_eq!(work["query_watches"][0]["watch"], watch["record"]["watch"]);
    assert_eq!(
        work["condition_watches"][0]["watch"],
        watch["record"]["watch"]
    );
    assert_eq!(work["carried_obligations"], json!([carried]));
    assert_eq!(
        result["agent_turn"]["briefing"]["objectives"],
        expected_objectives
    );
    expected["budget"]["max_output_tokens"] = json!(32_768);
    expected["request_id"] = json!((original_request + 1).to_string());
    assert_eq!(state(&owner)?, expected);
    Ok(())
}

#[test]
fn invalid_ceiling_and_unpublishable_narrowing_leave_the_original_budget_and_plan() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 4_096, false)?;
    let pending = parsed(&fortress_plan(Some(owner.id.clone()), None, Some(false)))?;
    assert_eq!(pending["ok"], true, "{pending}");
    refused_unchanged(&owner, 0, ErrorCode::InvalidRequest)?;
    refused_unchanged(
        &owner,
        MAX_LAB_BUDGET.max_output_tokens + 1,
        ErrorCode::BudgetExceeded,
    )?;
    refused_unchanged(&owner, 1, ErrorCode::BudgetExceeded)?;
    Ok(())
}

#[test]
fn increased_output_cannot_bypass_original_source_records_or_source_bytes() -> TestResult {
    let _serial = crate::test_serial();
    for records in [true, false] {
        let owner = open("starter_fortress", 4_096, false)?;
        {
            let session = resolve_session(Some(owner.id.clone()))?;
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            if records {
                guard.budget.max_entities = 1;
            } else {
                guard.budget.max_bytes = 64;
            }
        }
        refused_unchanged(&owner, 65_536, ErrorCode::BudgetExceeded)?;
    }
    Ok(())
}

#[test]
fn individually_fitting_world_and_retained_review_share_one_read_allowance() -> TestResult {
    let _serial = crate::test_serial();
    for records in [true, false] {
        let owner = open("starter_fortress", 4_096, false)?;
        let plan = parsed(&fortress_plan(
            Some(owner.id.clone()),
            Some("retain this original request".into()),
            Some(false),
        ))?;
        assert_eq!(plan["ok"], true, "{plan}");
        {
            let session = resolve_session(Some(owner.id.clone()))?;
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            let current = read_context(&guard);
            let world = query_budget::admit_sources(&[guard.adapter.snapshot()], &current)?;
            if records {
                // Each input domain fits alone. The pending record plus its
                // step must still be charged alongside the complete world.
                guard.budget.max_entities = world["records"].as_u64().ok_or("records")? as u32;
                assert!(guard.budget.max_entities > 2);
            } else {
                let bytes = world["canonical_bytes"].as_u64().ok_or("bytes")?;
                guard.budget.max_bytes = bytes + 1;
                let pending = guard.pending.as_ref().ok_or("pending")?;
                assert!(original_source_bytes(&pending.source) as u64 <= guard.budget.max_bytes);
            }
        }
        refused_unchanged(&owner, 65_536, ErrorCode::BudgetExceeded)?;
    }
    Ok(())
}

#[test]
fn escaped_work_projection_bytes_share_the_canonical_source_allowance() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 4_096, false)?;
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let bytes = guard.adapter.snapshot().canonical_bytes().len() as u64;
        // The original world fits, but the complete current-work packet cannot
        // fit the single remaining byte. Raising tokens cannot fund this work.
        guard.budget.max_bytes = bytes + 1;
    }
    refused_unchanged(&owner, 65_536, ErrorCode::BudgetExceeded)?;
    Ok(())
}

#[test]
fn negotiation_requires_current_unlimited_whole_fortress_observe_and_query() -> TestResult {
    let _serial = crate::test_serial();
    for case in 0..5 {
        let owner = open("empty", 4_096, false)?;
        {
            let session = resolve_session(Some(owner.id.clone()))?;
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            match case {
                0 => guard
                    .grants
                    .retain(|grant| grant.capability != Capability::Observe),
                1 => guard
                    .grants
                    .retain(|grant| grant.capability != Capability::Query),
                2 => {
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Query {
                            grant.scope.entity_ids = BTreeSet::from([EntityId::new(1)]);
                        }
                    }
                }
                3 => {
                    let mut source = guard.adapter.snapshot().clone();
                    source.tick = GameTick(2);
                    source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
                    source.refresh_hash();
                    guard.adapter.inject_snapshot(source)?;
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Observe {
                            grant.expires_at_tick = Some(GameTick(1));
                        }
                    }
                }
                _ => {
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Query {
                            grant.remaining_uses = Some(1);
                        }
                    }
                }
            }
        }
        refused_unchanged(&owner, 8_192, ErrorCode::CapabilityDenied)?;
    }
    Ok(())
}

#[test]
fn deadline_and_request_identity_exhaustion_are_atomic() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 4_096, false)?;
    let before = state(&owner)?;
    let started = Instant::now()
        .checked_sub(Duration::from_secs(61))
        .ok_or("test instant")?;
    assert!(matches!(renegotiate(owner.id.clone(), 8_192, started),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(state(&owner)?, before);
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .next_request_id = u128::MAX;
    }
    refused_unchanged(&owner, 8_192, ErrorCode::InternalInvariantViolation)?;
    Ok(())
}

#[test]
fn response_allowance_inspection_needs_no_world_grant_and_has_no_postlude() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 4_096, false)?;
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .grants
            .clear();
    }
    let before = state(&owner)?;
    assert_eq!(current_budget(&owner.id)?.max_output_tokens, 4_096);
    assert_eq!(state(&owner)?, before);
    refused_unchanged(&owner, 8_192, ErrorCode::CapabilityDenied)?;
    Ok(())
}

#[test]
fn explicit_narrowing_and_repeated_values_keep_the_same_session() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 8_192, false)?;
    let narrowed = parsed(&renegotiate(owner.id.clone(), 4_096, Instant::now())?)?;
    assert_eq!(narrowed["previous_budget"]["max_output_tokens"], 8_192);
    assert_eq!(narrowed["budget"]["max_output_tokens"], 4_096);
    assert_eq!(narrowed["changed"], true);
    let repeated = parsed(&renegotiate(owner.id.clone(), 4_096, Instant::now())?)?;
    assert_eq!(repeated["changed"], false);
    assert_eq!(repeated["session_id"], owner.id);
    assert_eq!(repeated["anchor"], narrowed["anchor"]);
    Ok(())
}

#[test]
fn shared_world_members_keep_independent_output_allowances_and_world_custody() -> TestResult {
    let _serial = crate::test_serial();
    let first = open("empty", 4_096, true)?;
    let second = open("empty", 8_192, true)?;
    let untouched = state(&second)?;
    let world = resolve_session(Some(first.id.clone()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .shared
        .clone()
        .ok_or("shared world")?;
    let before = {
        let guard = world.lock().map_err(|_| "world poisoned")?;
        format!("{:?}:{:?}:{:?}", guard.adapter, guard.leases, guard.members)
    };
    let updated = parsed(&renegotiate(first.id.clone(), 16_384, Instant::now())?)?;
    assert_eq!(updated["session_id"], first.id);
    assert_eq!(state(&second)?, untouched);
    let guard = world.lock().map_err(|_| "world poisoned")?;
    assert_eq!(
        format!("{:?}:{:?}:{:?}", guard.adapter, guard.leases, guard.members),
        before
    );
    Ok(())
}

#[test]
fn superseded_durable_owner_cannot_renegotiate_a_retained_session() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", 4_096, false)?;
    let session = resolve_session(Some(owner.id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        // Exercise the exact owner fence without creating a persistent store.
        // This request must refuse before any persistence or source inspection.
        guard.durable_scenario = Some("empty".into());
        durable_lab()
            .owners
            .insert(owner.fortress, SessionId::new(u128::MAX));
    }
    let result = refused_unchanged(&owner, 8_192, ErrorCode::Conflict);
    durable_lab().owners.remove(&owner.fortress);
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .durable_scenario = None;
    result
}
