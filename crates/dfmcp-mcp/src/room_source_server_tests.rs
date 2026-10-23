//! Original provisioned-room sources survive server persistence and re-review.

use super::*;
use dfmcp_intent::effects;
use dfmcp_lab::durable::DurablePlanSource;
use serde_json::Value as Json;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn room_request() -> Json {
    json!({
        "template":"provisioned_room",
        "layout":{
            "template":"bedroom_cluster", "origin":[2,6,10],
            "rooms":2, "room_size":[3,3],
        },
        "prerequisites":{
            "assign_labor":true,
            "workshops":[{"building":"workshop:Carpenters","location":[1,0,10]}],
        },
    })
}

fn canonical_source() -> Result<PlanSource> {
    let request =
        room_provisioning::ProvisionedRoomRequest::parse_current(&room_request().to_string())?;
    Ok(PlanSource::Blueprint {
        summary: "Two furnished bedrooms".to_owned(),
        raw: request.canonical_json(),
    })
}

fn prepare(snapshot: &WorldSnapshot, intent: &Intent) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        intent,
        &OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(77),
            anchor: snapshot.anchor(),
            budget: MAX_LAB_BUDGET,
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        },
    )
}

#[test]
fn durable_room_source_replays_exact_future_items_and_original_goal() -> TestResult {
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(963_101), false)?;
    let original = canonical_source()?;
    let (intent, analysis) = original.compile(IntentId::new(77), &snapshot)?;
    let prepared = prepare(&snapshot, &intent)?;
    let durable = original.durable();
    let DurablePlanSource::Blueprint { summary, raw } = &durable else {
        return Err("room source lost its durable blueprint tag".into());
    };
    assert_eq!(summary, "Two furnished bedrooms");
    let retained: Json = serde_json::from_str(raw)?;
    assert_eq!(retained["planner"], "provisioned_room_v1");
    assert_eq!(retained["layout"], room_request()["layout"]);
    assert_eq!(retained["prerequisites"]["assign_labor"], true);
    assert_eq!(
        retained["prerequisites"]["workshops"][0]["building"],
        "workshop:Carpenters"
    );
    assert_eq!(
        retained["prerequisites"]["workshops"][0]["location"],
        json!([1, 0, 10])
    );

    let reopened = PlanSource::from_durable(&durable);
    let (replayed, replay_analysis) = reopened.compile(IntentId::new(77), &snapshot)?;
    assert_eq!(replayed, intent);
    assert_eq!(replay_analysis, analysis);
    assert_eq!(prepare(&snapshot, &replayed)?, prepared);
    let DurablePlanSource::Blueprint {
        raw: reopened_raw, ..
    } = reopened.durable()
    else {
        return Err("reopened room source lost its durable blueprint tag".into());
    };
    assert_eq!(reopened_raw, *raw);

    let mut produced = BTreeSet::new();
    let mut placed = BTreeSet::new();
    for step in &prepared.steps {
        match &step.action {
            Action::CreateWorkOrder {
                job_token, amount, ..
            } => {
                assert_eq!(effects::work_order_output_item(job_token), Some("BED"));
                let order = effects::created_entity_id(&step.idempotency_key, 0);
                for ordinal in 1..=*amount {
                    produced.insert(effects::produced_item_id(order, ordinal));
                }
            }
            Action::PlaceFurniture { item, .. } => {
                assert!(placed.insert(*item), "each room receives a distinct item");
                assert!(!snapshot.graph.entities.contains_key(item));
            }
            _ => {}
        }
    }
    assert_eq!(produced.len(), 2);
    assert_eq!(
        placed, produced,
        "placement uses the sealed production identities"
    );
    Ok(())
}

#[test]
fn archived_room_marker_is_required_without_changing_legacy_blueprints() -> TestResult {
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(963_102), false)?;
    for marker in [None, Some("unrecognized_room_compiler")] {
        let mut request = room_request();
        if let Some(marker) = marker {
            request["planner"] = json!(marker);
        }
        let archived = DurablePlanSource::Blueprint {
            summary: "Archived room".to_owned(),
            raw: request.to_string(),
        };
        assert!(
            PlanSource::from_durable(&archived)
                .compile(IntentId::new(78), &snapshot)
                .is_err(),
            "archive replay must not silently adopt a new compiler generation"
        );
    }

    let old = DurablePlanSource::Blueprint {
        summary: "Legacy bedroom".to_owned(),
        raw: json!({
            "template":"bedroom_cluster", "origin":[2,6,10],
            "rooms":1, "room_size":[3,3],
        })
        .to_string(),
    };
    let (legacy, analysis) =
        PlanSource::from_durable(&old).compile(IntentId::new(78), &snapshot)?;
    assert!(analysis.is_none());
    let prepared = prepare(&snapshot, &legacy)?;
    assert!(
        prepared
            .steps
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
    );
    assert!(
        prepared
            .steps
            .iter()
            .all(|step| !matches!(step.action, Action::PlaceFurniture { .. }))
    );

    let PlanSource::Blueprint { summary, raw } = canonical_source()? else {
        return Err("expected canonical blueprint source".into());
    };
    let wrong_tag = DurablePlanSource::Production { summary, raw };
    assert!(
        PlanSource::from_durable(&wrong_tag)
            .compile(IntentId::new(78), &snapshot)
            .is_err(),
        "changing the durable source tag cannot bypass the room grammar"
    );
    Ok(())
}

#[test]
fn stale_room_placement_requires_reviewed_source_replay() -> TestResult {
    let opened: Json = serde_json::from_str(&open_session_in_scenario(
        Some(false),
        Some("963103".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("starter_fortress".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let owner = resolve_session(Some(
        opened["session_id"]
            .as_str()
            .ok_or("session missing")?
            .to_owned(),
    ))?;
    let mut session = owner.lock().map_err(|_| "session poisoned")?;
    let source = canonical_source()?;
    let intent = source.intent(IntentId::new(79), session.adapter.snapshot())?;
    let plan = prepare(session.adapter.snapshot(), &intent)?;
    assert!(
        plan.steps
            .iter()
            .any(|step| matches!(step.action, Action::PlaceFurniture { .. }))
    );
    let stale = PendingPlan {
        digest: plan.digest.to_string(),
        plan,
        source,
    };
    session.adapter.advance_ticks(1)?;
    let before = session.adapter.snapshot().clone();
    let request_id = session.next_request_id;
    assert_ne!(stale.plan.anchor, before.anchor());
    let Err(refused) = rebase_by_witness(&mut session, &stale) else {
        return Err("item-backed placement was silently witness-rebased".into());
    };
    assert_eq!(refused["accepted"], false);
    assert!(
        refused["reason"]
            .as_str()
            .ok_or("reason missing")?
            .contains("newly sealed intent replay"),
        "{refused}"
    );
    assert_eq!(session.adapter.snapshot(), &before);
    assert_eq!(session.next_request_id, request_id);
    assert!(session.pending.is_none());
    Ok(())
}
