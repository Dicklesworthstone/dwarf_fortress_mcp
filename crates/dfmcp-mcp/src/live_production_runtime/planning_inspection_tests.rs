use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::LiveItem;
use dfmcp_core::{EntityId, MapCoord};
use dfmcp_world::{WorldGraph, WorldSnapshot};

fn fixture() -> Result<(
    LiveOperationsObservation,
    orders::policy::SourceAcceptance,
    OperationContext,
)> {
    let raw = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: 0,
            year_tick: 120,
            paused: true,
            site_id: 4,
            world_folder: "region1".into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 200,
        next_item_id: 300,
        buildings: Vec::new(),
        items: vec![LiveItem {
            native_id: 9,
            item_type: 5,
            type_key: "BED".into(),
            subtype: -1,
            material_type: 420,
            material_index: 0,
            stack_size: 3,
            raw_position: MapCoord::new(1, 2, 3),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        }],
        attachments: Vec::new(),
    };
    let scope = orders::policy::SourceAcceptance {
        kind: orders::policy::ACCEPTANCE_KIND.into(),
        world_folder: "region1".into(),
        site_id: 4,
        df_version: "0.51.11".into(),
        dfhack_version: "0.51.11-r1".into(),
        citizen_bridge_version: "dfmcp-bridge/1".into(),
        citizen_generation: 101,
        work_order_generation: 201,
        progress_generation: 301,
        operations_generation: 601,
    };
    let snapshot = WorldSnapshot::new(
        raw.jobs.fortress_id()?,
        raw.jobs.tick(),
        ObservationCursor {
            epoch: 2,
            sequence: 7,
        },
        true,
        WorldGraph::default(),
    );
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 256 * MIB,
            max_entities: 8192,
            max_actions: 1,
            max_game_ticks: 0,
            max_output_tokens: 32768,
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    };
    Ok((raw, scope, context))
}

fn publish(
    owner: &mut PlanningInspection,
    raw: LiveOperationsObservation,
    scope: &orders::policy::SourceAcceptance,
    context: &OperationContext,
) -> Result<()> {
    owner.note_observation(&raw, scope)?;
    owner.publish(raw, scope, context)
}

#[test]
fn planning_inspection_publishes_complete_stock_without_plan_or_write_authority() -> Result<()> {
    let (raw, scope, context) = fixture()?;
    let mut owner = PlanningInspection::default();
    publish(&mut owner, raw.clone(), &scope, &context)?;
    let state = owner
        .current(context.anchor, context.anchor.tick)
        .expect("published source");
    let snapshot = state.snapshot().expect("canonical operations");
    assert_ne!(snapshot.anchor(), context.anchor);
    assert_eq!(snapshot.tick, context.anchor.tick);
    assert_eq!(state.source_digest()?, raw.source_digest()?);
    assert_eq!(owner.source_digest, Some(raw.source_digest()?));
    assert_eq!(state.observation(), Some(&raw));
    let item = &snapshot.graph.entities[&EntityId::new((2 << 40) + 9)];
    assert_eq!(item.generation, 1);
    assert_eq!(
        item.fields["stack_size"].known_value(),
        Some(&dfmcp_world::Value::U64(3))
    );
    assert!(
        context
            .authorize(Capability::Plan, RiskTier::Guarded, &[], None)
            .is_err()
    );
    assert!(
        context
            .authorize(
                Capability::ConfigureProduction,
                RiskTier::Guarded,
                &[],
                None
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn planning_inspection_withdrawal_preserves_aba_generations_and_source_heartbeats() -> Result<()> {
    let (raw, scope, mut context) = fixture()?;
    let mut owner = PlanningInspection::default();
    publish(&mut owner, raw.clone(), &scope, &context)?;
    let anchor = owner.state.snapshot().expect("source").anchor();
    owner.withdraw();
    assert!(owner.current(context.anchor, context.anchor.tick).is_none());
    context.anchor.cursor.sequence += 1;
    publish(&mut owner, raw.clone(), &scope, &context)?;
    assert_eq!(owner.state.snapshot().expect("heartbeat").anchor(), anchor);
    let mut absent = raw.clone();
    absent.items.clear();
    absent.jobs.year_tick += 1;
    context.anchor.tick = absent.jobs.tick();
    publish(&mut owner, absent, &scope, &context)?;
    owner.withdraw();
    let mut reappeared = raw;
    reappeared.jobs.year_tick += 2;
    context.anchor.tick = reappeared.jobs.tick();
    publish(&mut owner, reappeared, &scope, &context)?;
    let item =
        &owner.state.snapshot().expect("reappeared").graph.entities[&EntityId::new((2 << 40) + 9)];
    assert_eq!(
        item.generation, 2,
        "withdrawal must not reset source-local entity identity"
    );
    Ok(())
}

#[test]
fn planning_inspection_refuses_each_source_pin_or_tick_mismatch_without_publication() -> Result<()>
{
    let (raw, scope, context) = fixture()?;
    for case in 0..8 {
        let mut owner = PlanningInspection::default();
        publish(&mut owner, raw.clone(), &scope, &context)?;
        let mut changed = raw.clone();
        match case {
            0 => changed.jobs.bridge_generation += 1,
            1 => changed.jobs.world_folder = "another-world".into(),
            2 => changed.jobs.site_id += 1,
            3 => changed.jobs.df_version = "different".into(),
            4 => changed.jobs.dfhack_version = "different".into(),
            5 => changed.jobs.year_tick += 1,
            6 => changed.jobs.paused = false,
            _ => changed.jobs.year_tick -= 1,
        }
        assert!(
            publish(&mut owner, changed, &scope, &context).is_err(),
            "case {case}"
        );
        assert!(
            owner.current(context.anchor, context.anchor.tick).is_none(),
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn planning_inspection_retains_rejected_newer_counter_floors_and_fences_regression() -> Result<()> {
    let (raw, scope, context) = fixture()?;
    for counter in 0..3 {
        let mut owner = PlanningInspection::default();
        publish(&mut owner, raw.clone(), &scope, &context)?;
        let mut newer = raw.clone();
        newer.jobs.year_tick += 5;
        match counter {
            0 => newer.jobs.next_job_id += 1,
            1 => newer.next_building_id += 1,
            _ => newer.next_item_id += 1,
        }
        // Same accepted source, wrong canonical citizen tick: no publication,
        // but consumed allocation and time must survive the failed attempt.
        assert!(publish(&mut owner, newer.clone(), &scope, &context).is_err());
        assert_eq!(owner.frontier.expect("consumed floor").tick, GameTick(125));
        let mut current = context.clone();
        current.anchor.tick = GameTick(125);
        let mut regressed = raw.clone();
        regressed.jobs.year_tick = 125;
        assert!(publish(&mut owner, regressed, &scope, &current).is_err());
        assert!(owner.fenced);
        assert!(publish(&mut owner, newer, &scope, &current).is_err());
        assert!(owner.current(current.anchor, current.anchor.tick).is_none());
    }
    Ok(())
}

#[test]
fn planning_inspection_can_reobserve_a_rejected_newer_tick_without_rebasing_citizens() -> Result<()>
{
    let (mut raw, scope, context) = fixture()?;
    let mut owner = PlanningInspection::default();
    raw.jobs.year_tick = 125;
    assert!(publish(&mut owner, raw.clone(), &scope, &context).is_err());
    assert!(owner.state.snapshot().is_none());
    let mut current = context.clone();
    current.anchor.tick = GameTick(125);
    current.anchor.cursor.sequence += 1;
    publish(&mut owner, raw, &scope, &current)?;
    assert!(owner.current(context.anchor, GameTick(125)).is_none());
    assert!(owner.current(current.anchor, GameTick(125)).is_some());
    assert!(owner.current(current.anchor, GameTick(126)).is_none());
    Ok(())
}

#[test]
fn planning_inspection_refuses_revoked_expired_cancelled_and_bounded_reads() -> Result<()> {
    let (raw, scope, context) = fixture()?;
    for case in 0..7 {
        let mut owner = PlanningInspection::default();
        let mut limited = context.clone();
        match case {
            0 => limited
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            1 => limited
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            2 => limited.cancellation_requested = true,
            3 => limited.budget.max_bytes = PROJECTION_BYTES - 1,
            4 => limited.budget.max_entities = 2,
            5 => limited.grants[0].expires_at_tick = Some(GameTick(119)),
            _ => limited.grants[1].expires_at_tick = Some(GameTick(119)),
        }
        assert!(
            publish(&mut owner, raw.clone(), &scope, &limited).is_err(),
            "case {case}"
        );
        assert!(owner.state.snapshot().is_none(), "case {case}");
        assert!(owner.current(context.anchor, context.anchor.tick).is_none());
    }
    Ok(())
}

#[test]
fn planning_inspection_empty_capture_is_distinct_from_absent_or_withdrawn_source() -> Result<()> {
    let (mut raw, scope, context) = fixture()?;
    raw.items.clear();
    let mut owner = PlanningInspection::default();
    assert!(owner.current(context.anchor, context.anchor.tick).is_none());
    publish(&mut owner, raw, &scope, &context)?;
    assert!(
        owner
            .current(context.anchor, context.anchor.tick)
            .expect("complete empty census")
            .observation()
            .expect("source")
            .items
            .is_empty()
    );
    owner.withdraw();
    assert!(owner.current(context.anchor, context.anchor.tick).is_none());
    Ok(())
}

#[test]
fn planning_inspection_selector_is_explicit_closed_and_preserves_workforce_identity() -> Result<()>
{
    let default = ObserveRequest::parse("{}")?;
    let request = ObserveRequest::parse(r#"{"include_production":true}"#)?;
    assert!(!default.include_production());
    assert!(request.include_production());
    assert_eq!(default.selection_json(), request.selection_json());
    assert!(request.is_default(), "workforce projection is unchanged");
    assert_ne!(
        default, request,
        "post-plan Observe must reject the new selector"
    );
    for raw in [
        r#"{"include_production":1}"#,
        r#"{"include_production":null}"#,
        r#"{"include_production":true,"include_production":false}"#,
        r#"{"include_production":true,"operations_generation":601}"#,
    ] {
        assert!(ObserveRequest::parse(raw).is_err(), "{raw}");
    }
    Ok(())
}
