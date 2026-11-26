use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::LiveItem;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, MapCoord, RequestId, RiskTier, SessionId,
    WorkBudget,
};
use dfmcp_world::{EvidenceCoverage, WorldGraph, WorldSnapshot};

const RECIPES: [WorkOrderRecipe; 4] = [
    WorkOrderRecipe::WoodenBed,
    WorkOrderRecipe::WoodenDoor,
    WorkOrderRecipe::WoodenTable,
    WorkOrderRecipe::WoodenChair,
];

fn raw(tick: u64, counts: [u32; 4]) -> LiveOperationsObservation {
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
        items: ["BED", "DOOR", "TABLE", "CHAIR"]
            .into_iter()
            .zip(counts)
            .enumerate()
            .filter(|(_, (_, count))| *count > 0)
            .map(|(index, (kind, stack_size))| LiveItem {
                native_id: index as u32 + 1,
                item_type: index as i32 + 5,
                type_key: kind.into(),
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

fn context(raw: &LiveOperationsObservation, epoch: u64, sequence: u64) -> Result<OperationContext> {
    let snapshot = WorldSnapshot::new(
        raw.jobs.fortress_id()?,
        raw.jobs.tick(),
        ObservationCursor { epoch, sequence },
        true,
        WorldGraph::default(),
    );
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 256 * 1024 * 1024,
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
    })
}

fn projection(
    raw: LiveOperationsObservation,
    epoch: u64,
    sequence: u64,
) -> Result<(InventoryBaselineProjection, OperationContext)> {
    let context = context(&raw, epoch, sequence)?;
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

fn assessments(
    projection: &InventoryBaselineProjection,
    context: &OperationContext,
    recipes: &[WorkOrderRecipe],
) -> Result<Vec<StockTargetAssessment>> {
    recipes
        .iter()
        .map(|recipe| projection.stock_target(*recipe, 10, 100, accepted(projection), context))
        .collect()
}

fn initial(
    stable: u32,
    deadline: u64,
) -> Result<(
    StockTargetsWatch,
    Vec<StockTargetAssessment>,
    OperationContext,
)> {
    let (projection, context) = projection(raw(120, [20; 4]), 2, 7)?;
    let originals = assessments(&projection, &context, &[RECIPES[0], RECIPES[2]])?;
    Ok((
        StockTargetsWatch::new(originals.clone(), GameTick(deadline), 10, stable, &context)?,
        originals,
        context,
    ))
}

fn sample(
    watch: &mut StockTargetsWatch,
    tick: u64,
    sequence: u64,
    counts: [u32; 4],
) -> Result<StockTargetsWatchProgress> {
    let (projection, context) = projection(raw(tick, counts), 2, sequence)?;
    watch.sample(&projection, accepted(&projection), &context)
}

#[test]
fn portfolio_watch_requires_a_complete_same_census_satisfied_original() -> Result<()> {
    let (_, originals, context) = initial(2, 200)?;
    for invalid in [
        Vec::new(),
        vec![originals[0].clone(); 5],
        vec![originals[0].clone(), originals[0].clone()],
    ] {
        assert!(StockTargetsWatch::new(invalid, GameTick(200), 10, 2, &context).is_err());
    }
    for cause in 0..3 {
        let mut mismatched = originals.clone();
        let (projection, current) = match cause {
            0 => projection(raw(120, [20; 4]), 2, 7)?,
            1 => projection(raw(120, [20, 20, 21, 20]), 2, 7)?,
            _ => projection(raw(120, [20; 4]), 2, 8)?,
        };
        mismatched[1] = projection.stock_target(
            RECIPES[2],
            if cause == 0 { 21 } else { 10 },
            100,
            accepted(&projection),
            &current,
        )?;
        assert!(StockTargetsWatch::new(mismatched, GameTick(200), 10, 2, &context).is_err());
    }
    Ok(())
}

#[test]
fn portfolio_checkpoint_recovery_restarts_one_complete_joint_streak() -> Result<()> {
    let (mut watch, originals, context) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    let bytes = watch.checkpoint_bytes();
    let mut recovered = StockTargetsWatch::recover(&bytes, &context)?;
    assert_eq!(recovered.originals(), originals);
    assert_eq!(recovered.witness(), watch.witness());
    assert_eq!(recovered.checkpoint_bytes(), bytes);
    assert!(recovered.latest().is_none());
    assert!(!recovered.current_goal_proven());
    assert_eq!(
        sample(&mut recovered, 130, 9, [20; 4])?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut recovered, 140, 10, [20; 4])?.stable_samples_observed,
        1
    );
    assert_eq!(
        sample(&mut recovered, 150, 11, [5, 20, 20, 20])?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut recovered, 160, 12, [20; 4])?.stable_samples_observed,
        1
    );
    assert!(sample(&mut recovered, 170, 13, [20; 4])?.current_goal_proven);
    StockTargetsWatch::verify_checkpoint_continuity(&bytes, &recovered.checkpoint_bytes())?;
    Ok(())
}

#[test]
fn portfolio_checkpoint_retains_joint_achievement_and_withdraws_current_proof() -> Result<()> {
    let (mut watch, _, context) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    sample(&mut watch, 140, 9, [20; 4])?;
    let bytes = watch.checkpoint_bytes();
    let mut recovered = StockTargetsWatch::recover(&bytes, &context)?;
    assert_eq!(recovered.achieved_at(), Some(GameTick(140)));
    assert!(recovered.latest().is_none());
    assert!(!recovered.current_goal_proven());
    assert!(!sample(&mut recovered, 150, 10, [20, 20, 5, 20])?.current_goal_proven);
    assert!(sample(&mut recovered, 210, 11, [20; 4])?.current_goal_proven);
    StockTargetsWatch::verify_checkpoint_continuity(&bytes, &recovered.checkpoint_bytes())?;
    recovered.achieved_at = None;
    assert!(
        StockTargetsWatch::verify_checkpoint_continuity(&bytes, &recovered.checkpoint_bytes())
            .is_err()
    );
    Ok(())
}

#[test]
fn portfolio_checkpoint_refuses_scalar_substitution_partial_vectors_and_noncanonical_bytes()
-> Result<()> {
    let (mut watch, _, context) = initial(2, 200)?;
    let bytes = watch.checkpoint_bytes();
    for end in 0..bytes.len() {
        assert!(
            StockTargetsWatch::recover(&bytes[..end], &context).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(StockTargetsWatch::recover(&trailing, &context).is_err());
    assert!(StockTargetsWatch::recover(&vec![0; 2049], &context).is_err());
    assert!(StockTargetWatch::recover(&bytes, &context).is_err());
    assert!(StockTargetsWatch::recover(&watch.source_guard.checkpoint_bytes(), &context).is_err());
    watch.originals.reverse();
    assert!(StockTargetsWatch::recover(&watch.checkpoint_bytes(), &context).is_err());
    watch.originals.reverse();
    watch.originals.pop();
    assert!(StockTargetsWatch::recover(&watch.checkpoint_bytes(), &context).is_err());
    Ok(())
}

#[test]
fn portfolio_checkpoint_retains_fences_and_fixed_expiry() -> Result<()> {
    let (mut watch, _, context) = initial(2, 140)?;
    sample(&mut watch, 135, 8, [20; 4])?;
    let first = watch.checkpoint_bytes();
    assert!(sample(&mut watch, 140, 9, [20; 4])?.expired);
    let expired = watch.checkpoint_bytes();
    let mut recovered = StockTargetsWatch::recover(&expired, &context)?;
    assert!(sample(&mut recovered, 150, 10, [20; 4])?.expired);
    assert!(!recovered.current_goal_proven());
    StockTargetsWatch::verify_checkpoint_continuity(&first, &recovered.checkpoint_bytes())?;
    assert!(StockTargetsWatch::verify_checkpoint_continuity(&expired, &first).is_err());
    let mut wrong_generation = raw(160, [20; 4]);
    wrong_generation.jobs.bridge_generation += 1;
    assert!(recovered.note_capture(&wrong_generation).is_err());
    let fenced = recovered.checkpoint_bytes();
    assert!(StockTargetsWatch::recover(&fenced, &context)?.fenced());
    assert!(StockTargetsWatch::verify_checkpoint_continuity(&fenced, &expired).is_err());
    Ok(())
}

#[test]
fn portfolio_watch_normalizes_all_four_targets_without_granting_effect_authority() -> Result<()> {
    let (projection, context) = projection(raw(120, [20; 4]), 2, 7)?;
    let originals = assessments(&projection, &context, &RECIPES)?;
    let mut reversed = originals.clone();
    reversed.reverse();
    let mut watch = StockTargetsWatch::new(originals, GameTick(200), 10, 1, &context)?;
    let same = StockTargetsWatch::new(reversed, GameTick(200), 10, 1, &context)?;
    assert_eq!(watch.originals(), same.originals());
    assert_eq!(watch.witness(), same.witness());
    assert_eq!(
        watch
            .originals()
            .iter()
            .map(|target| target.recipe().as_str())
            .collect::<Vec<_>>(),
        ["wooden_bed", "wooden_chair", "wooden_door", "wooden_table"]
    );
    assert!(watch.latest().is_none());
    assert!(!watch.current_goal_proven());
    let complete = sample(&mut watch, 130, 8, [10; 4])?;
    assert!(complete.current_goal_proven);
    assert_eq!(complete.targets.len(), 4);
    assert!(complete.targets.iter().all(|target| {
        target.current_truth == PredicateTruth::True && target.observed_stack_units == Some(10)
    }));
    assert_eq!(
        complete.anchor,
        watch
            .latest()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "missing sample"))?
            .anchor
    );
    assert!(
        context
            .grants
            .iter()
            .all(|grant| matches!(grant.capability, Capability::Query | Capability::Observe))
    );
    assert_eq!(context.budget.max_game_ticks, 0);
    Ok(())
}

#[test]
fn portfolio_watch_requires_feasible_fixed_timing_and_complete_prepaid_work() -> Result<()> {
    let (_, originals, mut context) = initial(2, 200)?;
    for (deadline, interval, stable) in [
        (120, 10, 1),
        (130, 10, 2),
        (200, 0, 2),
        (200, 10, 0),
        (2000, 10, 65),
        (403_321, 10, 2),
        (u64::MAX, u64::MAX, 64),
    ] {
        assert!(
            StockTargetsWatch::new(
                originals.clone(),
                GameTick(deadline),
                interval,
                stable,
                &context
            )
            .is_err()
        );
    }
    context.budget.max_bytes = STOCK_TARGETS_WATCH_BYTES - 1;
    assert!(StockTargetsWatch::new(originals, GameTick(200), 10, 2, &context).is_err());
    Ok(())
}

#[test]
fn portfolio_watch_initial_census_replays_and_repeated_ticks_cannot_manufacture_stability()
-> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    assert_eq!(
        sample(&mut watch, 120, 8, [20; 4])?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 129, 9, [20; 4])?.stable_samples_observed,
        0
    );
    let first = sample(&mut watch, 130, 10, [20; 4])?;
    assert_eq!(first.stable_samples_observed, 1);
    assert_eq!(first.achieved_at, None);
    assert_eq!(sample(&mut watch, 130, 10, [20; 4])?, first);
    assert_eq!(
        sample(&mut watch, 130, 11, [20; 4])?.stable_samples_observed,
        1
    );
    let complete = sample(&mut watch, 140, 12, [20; 4])?;
    assert!(complete.current_goal_proven);
    assert_eq!(complete.achieved_at, Some(GameTick(140)));
    assert_eq!(complete.stable_samples_observed, 2);
    assert_eq!(watch.latest(), Some(&complete));
    Ok(())
}

#[test]
fn portfolio_watch_disjoint_member_achievements_never_prove_a_joint_goal() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    // BED qualifies twice, but TABLE does not. Later TABLE qualifies while BED
    // is absent. Neither per-member history is a complete simultaneous cohort.
    for (tick, sequence, counts) in [
        (130, 8, [20, 20, 0, 20]),
        (140, 9, [20, 20, 0, 20]),
        (150, 10, [0, 20, 20, 20]),
    ] {
        let progress = sample(&mut watch, tick, sequence, counts)?;
        assert_eq!(progress.current_truth, PredicateTruth::False);
        assert_eq!(progress.stable_samples_observed, 0);
        assert!(!progress.current_goal_proven);
        assert_eq!(progress.achieved_at, None);
    }
    let together = sample(&mut watch, 160, 11, [20; 4])?;
    assert_eq!(together.stable_samples_observed, 1);
    assert!(!together.current_goal_proven);
    let complete = sample(&mut watch, 170, 12, [20; 4])?;
    assert!(complete.current_goal_proven);
    assert_eq!(complete.achieved_at, Some(GameTick(170)));
    Ok(())
}

#[test]
fn portfolio_watch_off_cadence_member_loss_breaks_the_complete_streak() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    let lost = sample(&mut watch, 135, 9, [20, 20, 0, 20])?;
    assert_eq!(lost.stable_samples_observed, 0);
    assert_eq!(lost.targets[1].observed_stack_units, Some(0));
    assert_eq!(
        sample(&mut watch, 136, 10, [20; 4])?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 140, 11, [20; 4])?.stable_samples_observed,
        1
    );
    assert!(sample(&mut watch, 150, 12, [20; 4])?.current_goal_proven);
    Ok(())
}

#[test]
fn portfolio_watch_unknown_census_is_not_zero_and_withdraws_joint_progress() -> Result<()> {
    for missing_domain in [true, false] {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, [20; 4])?;
        let (projection, context) = projection(raw(140, [20; 4]), 2, 9)?;
        let mut policy = accepted(&projection);
        if missing_domain {
            policy.entity_kinds.clear();
        } else {
            policy.sources.clear();
        }
        let unknown = watch.sample(&projection, policy, &context)?;
        assert_eq!(unknown.current_truth, PredicateTruth::Unknown);
        assert_eq!(unknown.stable_samples_observed, 0);
        assert!(
            unknown
                .targets
                .iter()
                .all(|target| target.observed_stack_units.is_none())
        );
        assert!(!unknown.current_goal_proven);
        assert_eq!(
            sample(&mut watch, 150, 10, [20; 4])?.stable_samples_observed,
            1
        );
        assert!(sample(&mut watch, 160, 11, [20; 4])?.current_goal_proven);
    }
    Ok(())
}

#[test]
fn portfolio_watch_gaps_reset_all_targets_and_exact_replay_preserves_the_gap_label() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    let gap = sample(&mut watch, 140, 20, [20; 4])?;
    assert!(!gap.continuous);
    assert_eq!(gap.stable_samples_observed, 1);
    assert!(!gap.current_goal_proven);
    assert_eq!(sample(&mut watch, 140, 20, [20; 4])?, gap);
    assert!(sample(&mut watch, 150, 21, [20; 4])?.current_goal_proven);
    Ok(())
}

#[test]
fn portfolio_watch_failed_qualification_is_atomic_and_keeps_decoded_frontiers() -> Result<()> {
    for cause in 0..7 {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, [20; 4])?;
        let (projection, mut context) = projection(raw(140, [20; 4]), 2, 9)?;
        let mut policy = accepted(&projection);
        match cause {
            0 => context
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            1 => context
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            2 => context.cancellation_requested = true,
            3 => context.budget.max_bytes = STOCK_TARGETS_WATCH_BYTES - 1,
            4 => context.budget.max_entities = 1,
            5 => policy.all_entities = EvidenceCoverage::Complete,
            _ => {
                for grant in &mut context.grants {
                    grant.expires_at_tick = Some(GameTick(139));
                }
            }
        }
        assert!(watch.sample(&projection, policy, &context).is_err());
        assert!(watch.latest().is_none());
        assert!(!watch.current_goal_proven());
        assert!(!watch.fenced());
        assert_eq!(watch.high_tick(), GameTick(140));
        assert_eq!(watch.high_cursor().sequence, 9);
        assert_eq!(
            sample(&mut watch, 140, 9, [20; 4])?.stable_samples_observed,
            1
        );
        assert!(sample(&mut watch, 150, 10, [20; 4])?.current_goal_proven);
    }
    Ok(())
}

#[test]
fn portfolio_watch_budget_refusal_cannot_roll_back_a_same_tick_canonical_cursor() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    let (projection, mut context) = projection(raw(130, [20; 4]), 2, 10)?;
    context.budget.max_bytes = STOCK_TARGETS_WATCH_BYTES - 1;
    assert!(
        watch
            .sample(&projection, accepted(&projection), &context)
            .is_err()
    );
    assert_eq!(watch.high_cursor().sequence, 10);
    assert!(sample(&mut watch, 130, 9, [20; 4]).is_err());
    assert!(watch.fenced());
    assert!(watch.latest().is_none());
    Ok(())
}

#[test]
fn portfolio_watch_failed_acquisition_or_publication_withdraws_every_member() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    watch.invalidate_current_sample();
    assert!(watch.latest().is_none());
    assert!(!watch.current_goal_proven());
    let restarted = sample(&mut watch, 140, 9, [20; 4])?;
    assert!(!restarted.continuous);
    assert_eq!(restarted.stable_samples_observed, 1);
    assert!(sample(&mut watch, 150, 10, [20; 4])?.current_goal_proven);
    watch.invalidate_current_sample();
    assert!(!watch.current_goal_proven());
    assert!(watch.latest().is_none());
    let current = sample(&mut watch, 160, 11, [20; 4])?;
    assert!(current.current_goal_proven);
    assert_eq!(current.achieved_at, Some(GameTick(150)));
    Ok(())
}

#[test]
fn portfolio_watch_native_source_regression_fences_the_entire_original() -> Result<()> {
    for cause in 0..6 {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, [20; 4])?;
        let mut capture = raw(140, [20; 4]);
        match cause {
            0 => capture.jobs.bridge_generation += 1,
            1 => capture.jobs.next_job_id -= 1,
            2 => capture.next_building_id -= 1,
            3 => capture.next_item_id -= 1,
            4 => capture.jobs.world_folder = "region2".into(),
            _ => capture.jobs.year_tick = 129,
        }
        assert!(watch.note_capture(&capture).is_err());
        assert!(watch.fenced());
        assert!(watch.latest().is_none());
        assert!(sample(&mut watch, 150, 9, [20; 4]).is_err());
    }
    Ok(())
}

#[test]
fn portfolio_watch_retains_rejected_tick_and_allocator_floors_before_projection() -> Result<()> {
    for stale_tick in [true, false] {
        let (mut watch, _, _) = initial(2, 220)?;
        sample(&mut watch, 130, 8, [20; 4])?;
        let mut decoded = raw(180, [20; 4]);
        decoded.next_item_id = 400;
        watch.note_capture(&decoded)?;
        watch.invalidate_current_sample();
        assert_eq!(watch.high_tick(), GameTick(180));
        assert_eq!(watch.high_cursor().sequence, 8);
        let mut later = raw(if stale_tick { 170 } else { 190 }, [20; 4]);
        later.next_item_id = if stale_tick { 400 } else { 399 };
        assert!(watch.note_capture(&later).is_err());
        assert!(watch.fenced());
        assert!(watch.latest().is_none());
    }
    Ok(())
}

#[test]
fn portfolio_watch_canonical_epoch_cursor_and_forks_fence_all_members() -> Result<()> {
    for (epoch, sequence) in [(3, 8), (2, 6), (2, 7)] {
        let (mut watch, _, _) = initial(2, 200)?;
        let (projection, context) = projection(raw(130, [20; 4]), epoch, sequence)?;
        assert!(
            watch
                .sample(&projection, accepted(&projection), &context)
                .is_err()
        );
        assert!(watch.fenced());
    }
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    let raw = raw(130, [20; 4]);
    let mut context = context(&raw, 2, 8)?;
    context.anchor.state_hash = Digest32::of_bytes(b"different canonical citizen world");
    let projection = InventoryBaselineProjection::new(context.anchor, raw, &context)?;
    assert!(
        watch
            .sample(&projection, accepted(&projection), &context)
            .is_err()
    );
    assert!(watch.fenced());
    Ok(())
}

#[test]
fn portfolio_watch_fixed_deadline_requires_a_timely_complete_cohort() -> Result<()> {
    let (mut missed, _, _) = initial(2, 140)?;
    assert_eq!(
        sample(&mut missed, 135, 8, [20; 4])?.stable_samples_observed,
        1
    );
    let deadline = sample(&mut missed, 140, 9, [20; 4])?;
    assert!(deadline.deadline_reached);
    assert!(deadline.expired);
    assert_eq!(deadline.achieved_at, None);
    assert!(!sample(&mut missed, 150, 10, [20; 4])?.current_goal_proven);
    let (mut timely, _, _) = initial(2, 140)?;
    sample(&mut timely, 130, 8, [20; 4])?;
    let complete = sample(&mut timely, 140, 9, [20; 4])?;
    assert!(complete.current_goal_proven);
    assert_eq!(complete.achieved_at, Some(GameTick(140)));
    assert!(!complete.expired);
    Ok(())
}

#[test]
fn portfolio_watch_historical_achievement_is_not_current_stock_or_recovered_proof() -> Result<()> {
    let (mut watch, originals, context) = initial(2, 200)?;
    sample(&mut watch, 130, 8, [20; 4])?;
    sample(&mut watch, 140, 9, [20; 4])?;
    let consumed = sample(&mut watch, 150, 10, [20, 20, 0, 20])?;
    assert_eq!(consumed.achieved_at, Some(GameTick(140)));
    assert_eq!(consumed.current_truth, PredicateTruth::False);
    assert!(!consumed.current_goal_proven);
    assert!(!watch.current_goal_proven());
    watch.invalidate_current_sample();
    let restored = sample(&mut watch, 210, 11, [20; 4])?;
    assert_eq!(restored.achieved_at, Some(GameTick(140)));
    assert!(restored.current_goal_proven);
    let mut reopened = StockTargetsWatch::new(originals, GameTick(200), 10, 2, &context)?;
    assert!(reopened.latest().is_none());
    let fresh = sample(&mut reopened, 210, 12, [20; 4])?;
    assert_eq!(fresh.achieved_at, None);
    assert!(fresh.expired);
    assert!(!fresh.current_goal_proven);
    Ok(())
}

#[test]
fn portfolio_watch_changed_inventory_does_not_size_work_or_credit_production() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    let mut capture = raw(130, [0; 4]);
    capture.jobs.jobs.push(LiveJob {
        native_id: 7,
        job_type: 13,
        type_key: "CustomReaction".into(),
        reaction: "UNKNOWN_OUTPUT".into(),
        suspended: true,
        repeating: true,
        position: MapCoord::new(1, 1, 1),
        worker_native_id: None,
        holder_native_id: None,
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 0,
    });
    let (projection, context) = projection(capture, 2, 8)?;
    let current = watch.sample(&projection, accepted(&projection), &context)?;
    assert_eq!(current.current_truth, PredicateTruth::False);
    assert!(
        current
            .targets
            .iter()
            .all(|target| target.observed_stack_units == Some(0))
    );
    assert!(!watch.fenced());
    assert!(!current.current_goal_proven);
    Ok(())
}

#[test]
fn portfolio_watch_witnesses_are_canonical_and_bind_every_member_and_timing() -> Result<()> {
    let (mut watch, originals, context) = initial(2, 200)?;
    let mut same = StockTargetsWatch::new(originals.clone(), GameTick(200), 10, 2, &context)?;
    let changed_deadline =
        StockTargetsWatch::new(originals.clone(), GameTick(210), 10, 2, &context)?;
    let changed_count = StockTargetsWatch::new(originals.clone(), GameTick(200), 10, 3, &context)?;
    let changed_interval = StockTargetsWatch::new(originals, GameTick(200), 11, 2, &context)?;
    assert_eq!(watch.witness(), same.witness());
    assert_ne!(watch.witness(), changed_deadline.witness());
    assert_ne!(watch.witness(), changed_count.witness());
    assert_ne!(watch.witness(), changed_interval.witness());
    let a = sample(&mut watch, 130, 8, [20; 4])?;
    let b = sample(&mut same, 130, 8, [20; 4])?;
    assert_eq!(a, b);
    // A changed later member at the same citizen anchor changes the complete
    // native source commitment and portfolio proof, even while all minima hold.
    let changed = sample(&mut same, 130, 8, [20, 20, 21, 20])?;
    assert_ne!(b.source_digest, changed.source_digest);
    assert_ne!(b.witness, changed.witness);
    assert_eq!(changed.stable_samples_observed, 1);
    Ok(())
}

#[test]
fn portfolio_watch_withheld_for_reviewed_time_preserves_joint_streak_without_current_proof()
-> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    let first = sample(&mut watch, 130, 8, [20; 4])?;
    assert_eq!(first.stable_samples_observed, 1);
    let witness = watch.witness();
    watch.withhold_current_sample();
    assert!(watch.latest().is_none());
    assert!(!watch.current_goal_proven());
    assert_eq!(watch.achieved_at(), None);
    assert_eq!(watch.witness(), witness);
    assert_eq!(watch.high_tick(), GameTick(130));
    assert_eq!(sample(&mut watch, 130, 8, [20; 4])?, first);
    watch.withhold_current_sample();
    let repeated_tick = sample(&mut watch, 130, 9, [20; 4])?;
    assert_eq!(repeated_tick.stable_samples_observed, 1);
    assert_eq!(repeated_tick.last_counted_tick, Some(GameTick(130)));
    watch.withhold_current_sample();
    let complete = sample(&mut watch, 140, 10, [20; 4])?;
    assert_eq!(complete.stable_samples_observed, 2);
    assert!(
        complete
            .targets
            .iter()
            .all(|row| row.current_truth == PredicateTruth::True)
    );
    assert!(complete.continuous);
    assert!(complete.current_goal_proven);
    assert_eq!(complete.achieved_at, Some(GameTick(140)));
    watch.withhold_current_sample();
    assert!(!watch.current_goal_proven());
    assert!(watch.latest().is_none());
    assert_eq!(watch.achieved_at(), Some(GameTick(140)));
    Ok(())
}

#[test]
fn portfolio_watch_withholding_never_rejoins_failed_gapped_or_lost_member_stability() -> Result<()>
{
    for cause in 0..3 {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, [20; 4])?;
        match cause {
            0 => watch.invalidate_current_sample(),
            1 => {}
            _ => {
                let lost = sample(&mut watch, 135, 9, [20, 20, 0, 20])?;
                assert_eq!(lost.current_truth, PredicateTruth::False);
                assert_eq!(lost.stable_samples_observed, 0);
            }
        }
        watch.withhold_current_sample();
        let sequence = match cause {
            0 => 9,
            1 => 20,
            _ => 10,
        };
        let restarted = sample(&mut watch, 140, sequence, [20; 4])?;
        assert_eq!(restarted.stable_samples_observed, 1);
        assert!(!restarted.current_goal_proven);
        if cause < 2 {
            assert!(!restarted.continuous);
        }
        watch.withhold_current_sample();
        let complete = sample(&mut watch, 150, sequence + 1, [20; 4])?;
        assert!(complete.current_goal_proven);
        assert_eq!(complete.achieved_at, Some(GameTick(150)));
    }
    Ok(())
}
