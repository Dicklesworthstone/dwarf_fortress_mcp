//! Exact checkpoint/parent admission tests use the real independently qualified
//! inventory projection and adapter monitors. No retained count supplies a sample.
use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::semantic_work_orders::InventoryBaselineProjection;
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::MapCoord;
use dfmcp_world::{EntityKind, EvidenceCoverage, WorldGraph, WorldSnapshot};

fn operations(tick: u64, beds: u32, doors: u32) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: (tick / 403_200) as u32,
            year_tick: (tick % 403_200) as u32,
            paused: true,
            site_id: 4,
            world_folder: "region1".into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 200,
        next_item_id: 300,
        buildings: Vec::new(),
        items: [(1, "BED", beds), (2, "DOOR", doors)]
            .into_iter()
            .map(|(native_id, type_key, stack_size)| LiveItem {
                native_id,
                item_type: 5,
                type_key: type_key.into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size,
                raw_position: MapCoord::new(1, 1, 1),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
        attachments: Vec::new(),
    }
}

fn projection(
    tick: u64,
    sequence: u64,
    beds: u32,
    doors: u32,
) -> Result<(InventoryBaselineProjection, OperationContext)> {
    let raw = operations(tick, beds, doors);
    let snapshot = WorldSnapshot::new(
        raw.jobs.fortress_id()?,
        raw.jobs.tick(),
        ObservationCursor { epoch: 2, sequence },
        true,
        WorldGraph::default(),
    );
    let context = OperationContext {
        session_id: SessionId::new(4001),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 256 * MIB,
            max_output_tokens: 65536,
            max_entities: 4096,
            max_actions: 1,
            max_game_ticks: 0,
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
    Ok((
        InventoryBaselineProjection::new(context.anchor, raw, &context)?,
        context,
    ))
}

fn accepted(projection: &InventoryBaselineProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(projection.snapshot().anchor());
    policy.paused = true;
    policy
        .entity_kinds
        .insert(EntityKind::Item, EvidenceCoverage::Complete);
    policy.sources = projection.observed_sources();
    policy
}

fn fixture(
    portfolio: bool,
    ticks: &[u64],
    cancelled: bool,
) -> Result<(Checkpoint, ProjectStockGoal, OperationContext)> {
    let (source, context) = projection(120, 7, 20, 20)?;
    let common = json!({
        "expected_observation": Digest32::of_bytes(b"original selected citizens").to_hex(),
        "units": ["43", "252"], "labor": "CARPENTER", "assign_labor": true,
        "deadline_tick": 200, "poll_interval_ticks": 10, "stable_samples": 2,
    });
    let mut request = common;
    let mut goal = json!({
        "deadline_tick": 200, "poll_interval_ticks": 10, "stable_samples": 2,
        "assign_labor": true,
    });
    let checkpoint = if portfolio {
        request["kind"] = json!("stock_targets");
        let targets = json!([
            {"recipe":"wooden_bed", "minimum_stack_units":2, "max_order_amount":5},
            {"recipe":"wooden_door", "minimum_stack_units":3, "max_order_amount":7},
        ]);
        request["targets"] = targets.clone();
        goal["targets"] = targets;
        let request = StockTargetsRequest::parse(&request.to_string())?;
        let assessments = [
            (WorkOrderRecipe::WoodenBed, 2, 5),
            (WorkOrderRecipe::WoodenDoor, 3, 7),
        ]
        .into_iter()
        .map(|(recipe, minimum, maximum)| {
            source.stock_target(recipe, minimum, maximum, accepted(&source), &context)
        })
        .collect::<Result<Vec<_>>>()?;
        let binding = stock_targets_binding::StockTargetsBinding::from_assessments_with_assignment(
            &assessments,
        )?;
        let mut owner = StockTargetsNoWork::new(assessments, binding, request, &context)?;
        for (index, tick) in ticks.iter().enumerate() {
            let (sample, c) = projection(*tick, 8 + index as u64, 20, 20)?;
            owner.monitor.sample(&sample, accepted(&sample), &c)?;
        }
        owner.cancelled = cancelled;
        Checkpoint::portfolio(&owner)
    } else {
        request["kind"] = json!("stock_target");
        for (field, value) in [
            ("recipe", json!("wooden_bed")),
            ("minimum_stack_units", json!(2)),
            ("max_order_amount", json!(5)),
        ] {
            request[field] = value.clone();
            goal[field] = value;
        }
        let request = StockTargetRequest::parse(&request.to_string())?;
        let assessment = source.stock_target(
            WorkOrderRecipe::WoodenBed,
            2,
            5,
            accepted(&source),
            &context,
        )?;
        let mut owner = StockTargetNoWork::new(assessment, request, &context)?;
        for (index, tick) in ticks.iter().enumerate() {
            let (sample, c) = projection(*tick, 8 + index as u64, 20, 20)?;
            owner.monitor.sample(&sample, accepted(&sample), &c)?;
        }
        owner.cancelled = cancelled;
        Checkpoint::scalar(&owner)
    };
    let goal: ProjectStockGoal =
        serde_json::from_value(goal).map_err(|_| invalid("invalid test original goal"))?;
    Ok((checkpoint, goal, context))
}

/// Shared by actual journal/ACK replay regressions; these bytes originate in
/// the real qualified projection rather than a hand-written checkpoint codec.
pub(crate) fn reference_fixture(
    portfolio: bool,
    ticks: &[u64],
    cancelled: bool,
) -> Result<(Value, Value, OperationContext)> {
    let (checkpoint, goal, context) = fixture(portfolio, ticks, cancelled)?;
    let reference = json!({
        "schema": "dfmcp.project-production-recovery/1", "recovery": null,
        "native_custody": {
            "workforce_journal_id": Digest32::of_bytes(b"original workforce custody").to_hex(),
            "order_journal_id": Digest32::of_bytes(b"original order custody").to_hex(),
        },
        "observation_tick_floor": checkpoint.terms()?.high_tick.get(),
        "clock":null, "inventory_clock":null,
        "workshop_prerequisite":null, "manager_prerequisite":null, "input_supply":null,
        "original_goal_proven":false, "preparation_restored":false,
        "stock_watch":checkpoint.to_json(),
    });
    Ok((reference, json!(goal), context))
}

#[test]
fn project_watch_roundtrip_preserves_complete_scalar_and_portfolio_requests() -> Result<()> {
    for portfolio in [false, true] {
        let (checkpoint, goal, context) = fixture(portfolio, &[130], false)?;
        let recovered = Checkpoint::from_json(&checkpoint.to_json())?;
        assert_eq!(recovered, checkpoint);
        recovered.validate_goal(&goal)?;
        assert_eq!(recovered.terms()?.high_tick, GameTick(130));
        let bytes = decode(&recovered.checkpoint_hex)?;
        let mut c = context.clone();
        c.session_id = SessionId::new(4002);
        c.grants
            .retain(|grant| grant.capability == Capability::Query);
        if portfolio {
            let owner = StockTargetsWatch::recover(&bytes, &c)?;
            assert_eq!(owner.witness(), recovered.terms()?.goal_witness);
            assert!(owner.latest().is_none());
            assert!(!owner.current_goal_proven());
            assert_eq!(owner.high_cursor().sequence, 8);
        } else {
            let owner = StockTargetWatch::recover(&bytes, &c)?;
            assert_eq!(owner.witness(), recovered.terms()?.goal_witness);
            assert!(owner.latest().is_none());
            assert!(!owner.current_goal_proven());
            assert_eq!(owner.high_cursor().sequence, 8);
        }
    }
    Ok(())
}

#[test]
fn restarted_project_watches_require_a_new_complete_original_stability_interval() -> Result<()> {
    for portfolio in [false, true] {
        let (checkpoint, _, context) = fixture(portfolio, &[130], false)?;
        let bytes = decode(&checkpoint.checkpoint_hex)?;
        let (first, c1) = projection(140, 9, 20, 20)?;
        let (second, c2) = projection(150, 10, 20, 20)?;
        if portfolio {
            let mut watch = StockTargetsWatch::recover(&bytes, &context)?;
            let first = watch.sample(&first, accepted(&first), &c1)?;
            assert_eq!(first.stable_samples_observed, 1);
            assert!(!first.current_goal_proven);
            assert!(
                watch
                    .sample(&second, accepted(&second), &c2)?
                    .current_goal_proven
            );
        } else {
            let mut watch = StockTargetWatch::recover(&bytes, &context)?;
            let first = watch.sample(&first, accepted(&first), &c1)?;
            assert_eq!(first.stable_samples_observed, 1);
            assert!(!first.current_goal_proven);
            assert!(
                watch
                    .sample(&second, accepted(&second), &c2)?
                    .current_goal_proven
            );
        }
    }
    Ok(())
}

#[test]
fn historical_achievement_survives_restart_without_current_truth_or_maintenance() -> Result<()> {
    for portfolio in [false, true] {
        let (checkpoint, _, context) = fixture(portfolio, &[130, 140], false)?;
        let bytes = decode(&checkpoint.checkpoint_hex)?;
        let (loss, c) = projection(150, 10, 0, 0)?;
        if portfolio {
            let mut watch = StockTargetsWatch::recover(&bytes, &context)?;
            assert_eq!(watch.achieved_at(), Some(GameTick(140)));
            assert!(watch.latest().is_none());
            assert!(
                !watch
                    .sample(&loss, accepted(&loss), &c)?
                    .current_goal_proven
            );
            assert_eq!(watch.achieved_at(), Some(GameTick(140)));
        } else {
            let mut watch = StockTargetWatch::recover(&bytes, &context)?;
            assert_eq!(watch.achieved_at(), Some(GameTick(140)));
            assert!(watch.latest().is_none());
            assert!(
                !watch
                    .sample(&loss, accepted(&loss), &c)?
                    .current_goal_proven
            );
            assert_eq!(watch.achieved_at(), Some(GameTick(140)));
        }
        let (unfinished, _, _) = fixture(portfolio, &[130], false)?;
        assert!(checkpoint.successor(&unfinished).is_err());
    }
    Ok(())
}

#[test]
fn parent_watch_history_cannot_erase_cancellation_or_replace_original_bindings() -> Result<()> {
    for portfolio in [false, true] {
        let (before, _, _) = reference_fixture(portfolio, &[130], false)?;
        let (cancelled, _, _) = reference_fixture(portfolio, &[130], true)?;
        verify_reference_continuity(&before, &cancelled)?;
        assert!(verify_reference_continuity(&cancelled, &before).is_err());
        for (field, value) in [
            ("units", json!(["43"])),
            ("assign_labor", json!(false)),
            (
                "expected_observation",
                json!(Digest32::of_bytes(b"another selector").to_hex()),
            ),
            ("labor", json!("MINE")),
        ] {
            let mut changed = before.clone();
            changed["stock_watch"]["request"][field] = value;
            assert!(
                verify_reference_continuity(&before, &changed).is_err(),
                "{field}"
            );
        }
        let (older, _, _) = reference_fixture(portfolio, &[], false)?;
        assert!(verify_reference_continuity(&before, &older).is_err());
        let mut missing = before.clone();
        missing
            .as_object_mut()
            .ok_or_else(|| invalid("test reference"))?
            .remove("stock_watch");
        assert!(verify_reference_continuity(&before, &missing).is_err());
        let mut original = before.clone();
        original["recovery"] = json!({"historical_original":true});
        verify_reference_continuity(&before, &original)?;
        assert!(verify_reference_continuity(&missing, &original).is_err());
    }
    Ok(())
}

#[test]
fn watch_checkpoint_requires_closed_canonical_bytes_and_exact_original_timing() -> Result<()> {
    for portfolio in [false, true] {
        let (checkpoint, _, _) = fixture(portfolio, &[], false)?;
        let base = checkpoint.to_json();
        for field in ["deadline_tick", "poll_interval_ticks", "stable_samples"] {
            let mut changed = base.clone();
            changed["request"][field] = json!(199);
            assert!(Checkpoint::from_json(&changed).is_err(), "{field}");
        }
        for field in ["minimum_stack_units", "max_order_amount", "recipe"] {
            let mut changed = base.clone();
            let target = if portfolio {
                &mut changed["request"]["targets"][0]
            } else {
                &mut changed["request"]
            };
            target[field] = if field == "recipe" {
                json!("wooden_chair")
            } else {
                json!(4)
            };
            assert!(Checkpoint::from_json(&changed).is_err(), "{field}");
        }
        let mut changed = base.clone();
        changed["current_goal_proven"] = json!(true);
        assert!(Checkpoint::from_json(&changed).is_err());
        let mut missing = base.clone();
        missing["request"]
            .as_object_mut()
            .ok_or_else(|| invalid("test request"))?
            .remove("assign_labor");
        assert!(Checkpoint::from_json(&missing).is_err());
        for hex in [
            checkpoint.checkpoint_hex.to_uppercase(),
            "0".into(),
            "ab".repeat(MAX_WATCH_BYTES + 1),
            format!("{}00", checkpoint.checkpoint_hex),
        ] {
            let mut changed = base.clone();
            changed["checkpoint_hex"] = json!(hex);
            assert!(Checkpoint::from_json(&changed).is_err());
        }
    }
    Ok(())
}

#[test]
fn parent_admission_checks_goal_fortress_and_consumed_horizon_before_ack() -> Result<()> {
    for portfolio in [false, true] {
        let (reference, goal, context) = reference_fixture(portfolio, &[130], false)?;
        let goal: ProjectStockGoal =
            serde_json::from_value(goal).map_err(|_| invalid("test goal"))?;
        let fortress = context.anchor.fortress_id.get().to_string();
        validate_parent_reference(&reference, &goal, &fortress, 100, 130)?;
        assert!(validate_parent_reference(&reference, &goal, "91", 100, 130).is_err());
        assert!(validate_parent_reference(&reference, &goal, &fortress, 121, 130).is_err());
        assert!(validate_parent_reference(&reference, &goal, &fortress, 100, 129).is_err());
        let mut regressed = reference.clone();
        regressed["observation_tick_floor"] = json!(129);
        assert!(from_reference(&regressed).is_err());
        let mut null = reference.clone();
        null["stock_watch"] = Value::Null;
        assert!(from_reference(&null).is_err());
        let (_, other_goal, _) = fixture(!portfolio, &[], false)?;
        assert!(validate_parent_reference(&reference, &other_goal, &fortress, 100, 130).is_err());
    }
    Ok(())
}

#[test]
fn pending_intent_retains_the_newest_comparable_watch_history_without_renewing_terms() -> Result<()>
{
    for portfolio in [false, true] {
        let (older, _, _) = reference_fixture(portfolio, &[], false)?;
        let (newer, _, _) = reference_fixture(portfolio, &[130], false)?;
        for (intended, checkpoint) in [(&older, &newer), (&newer, &older)] {
            let mut intent = intended.clone();
            intent["recovery"] = json!({"prospective_original": true});
            // The enclosing intent codec validates the real original; this
            // helper compares only its independently retained watch custody.
            intent["observation_tick_floor"] = json!(140);
            verify_intent_watch(&intent, checkpoint)?;
            join_intent_watch(&mut intent, checkpoint)?;
            assert_eq!(intent["stock_watch"], newer["stock_watch"]);
            assert_eq!(
                intent["stock_watch"]["request"],
                older["stock_watch"]["request"]
            );
            let mut published = newer.clone();
            published["recovery"] = json!({"prospective_original": true});
            verify_intent_watch(&intent, &published)?;
            published["stock_watch"] = older["stock_watch"].clone();
            assert!(verify_intent_watch(&intent, &published).is_err());
        }
        let mut missing = newer.clone();
        missing
            .as_object_mut()
            .ok_or_else(|| invalid("test reference"))?
            .remove("stock_watch");
        assert!(verify_intent_watch(&newer, &missing).is_err());
        let mut destination = newer.clone();
        assert!(join_intent_watch(&mut destination, &missing).is_err());
        assert_eq!(destination, newer);
    }
    Ok(())
}

#[test]
fn pending_intent_cannot_join_incomparable_watch_branches_or_terminal_predecessors() -> Result<()> {
    for portfolio in [false, true] {
        // Both histories are independently well-formed. One consumed a later
        // tick while the other consumed a later cursor; neither descends from
        // the other. A higher global floor cannot make them comparable.
        let (left, _, _) = reference_fixture(portfolio, &[140], false)?;
        let (right, _, _) = reference_fixture(portfolio, &[130, 135], false)?;
        for (intended, checkpoint) in [(&left, &right), (&right, &left)] {
            let mut intent = intended.clone();
            intent["recovery"] = json!({"prospective_original": true});
            intent["observation_tick_floor"] = json!(160);
            assert!(verify_intent_watch(&intent, checkpoint).is_err());
            let before = intent.clone();
            assert!(join_intent_watch(&mut intent, checkpoint).is_err());
            assert_eq!(intent, before);
        }
        fixture(portfolio, &[130], false)?.0.require_unfinished()?;
        assert!(
            fixture(portfolio, &[130, 140], false)?
                .0
                .require_unfinished()
                .is_err()
        );
        assert!(
            fixture(portfolio, &[130], true)?
                .0
                .require_unfinished()
                .is_err()
        );
    }
    Ok(())
}
