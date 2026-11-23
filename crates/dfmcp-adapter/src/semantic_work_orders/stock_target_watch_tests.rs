use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::LiveItem;
use crate::work_orders::WorkOrderRecipe;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, MapCoord, RequestId, RiskTier, SessionId,
    WorkBudget,
};
use dfmcp_world::{EvidenceCoverage, WorldGraph, WorldSnapshot};

fn raw(tick: u64, beds: Option<u32>) -> LiveOperationsObservation {
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
        items: beds
            .map(|stack_size| LiveItem {
                native_id: 1,
                item_type: 5,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size,
                raw_position: MapCoord::new(1, 1, 1),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .into_iter()
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

fn initial(
    stable: u32,
    deadline: u64,
) -> Result<(StockTargetWatch, StockTargetAssessment, OperationContext)> {
    let (projection, context) = projection(raw(120, Some(20)), 2, 7)?;
    let assessment = projection.stock_target(
        WorkOrderRecipe::WoodenBed,
        10,
        1,
        accepted(&projection),
        &context,
    )?;
    Ok((
        StockTargetWatch::new(assessment.clone(), GameTick(deadline), 10, stable, &context)?,
        assessment,
        context,
    ))
}

fn sample(
    watch: &mut StockTargetWatch,
    tick: u64,
    sequence: u64,
    beds: Option<u32>,
) -> Result<StockTargetWatchProgress> {
    let (projection, context) = projection(raw(tick, beds), 2, sequence)?;
    watch.sample(&projection, accepted(&projection), &context)
}

#[test]
fn no_work_watch_stability_requires_fresh_distinct_cadenced_samples() -> Result<()> {
    let (mut watch, assessment, _) = initial(2, 200)?;
    assert_eq!(watch.original(), &assessment);
    assert!(watch.latest().is_none());
    assert!(!watch.current_goal_proven());
    assert_eq!(watch.high_tick(), GameTick(120));
    assert_eq!(
        sample(&mut watch, 120, 8, Some(20))?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 129, 9, Some(20))?.stable_samples_observed,
        0
    );
    let first = sample(&mut watch, 130, 10, Some(20))?;
    assert_eq!(first.stable_samples_observed, 1);
    assert_eq!(first.achieved_at, None);
    assert!(!first.current_goal_proven);
    assert_eq!(
        sample(&mut watch, 130, 11, Some(20))?.stable_samples_observed,
        1
    );
    let complete = sample(&mut watch, 140, 12, Some(20))?;
    assert_eq!(complete.stable_samples_observed, 2);
    assert_eq!(complete.achieved_at, Some(GameTick(140)));
    assert!(complete.current_goal_proven);
    assert!(watch.current_goal_proven());
    assert_eq!(watch.latest(), Some(&complete));
    assert_eq!(watch.high_cursor().sequence, 12);
    Ok(())
}

#[test]
fn no_work_watch_refuses_positive_work_and_infeasible_timing() -> Result<()> {
    let (_, assessment, context) = initial(2, 200)?;
    for (deadline, interval, stable) in [
        (119, 10, 2),
        (120, 10, 2),
        (130, 10, 2),
        (200, 0, 2),
        (200, 10, 0),
        (2000, 10, 65),
        (403_321, 10, 2),
        (u64::MAX, u64::MAX, 64),
    ] {
        assert!(
            StockTargetWatch::new(
                assessment.clone(),
                GameTick(deadline),
                interval,
                stable,
                &context,
            )
            .is_err()
        );
    }
    let (projection, context) = projection(raw(120, Some(20)), 2, 7)?;
    let work = projection.stock_target(
        WorkOrderRecipe::WoodenBed,
        21,
        1,
        accepted(&projection),
        &context,
    )?;
    assert!(StockTargetWatch::new(work, GameTick(200), 10, 2, &context).is_err());
    Ok(())
}

#[test]
fn no_work_watch_counts_drops_without_sizing_orders_or_excluding_producers() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    assert_eq!(
        sample(&mut watch, 130, 8, Some(20))?.stable_samples_observed,
        1
    );
    let mut capture = raw(140, None);
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
    let (projection, context) = projection(capture, 2, 9)?;
    // The deficit is ten, above the original order limit of one, and has an
    // unknown producer. Neither observation blocks a read-only minimum check.
    let progress = watch.sample(&projection, accepted(&projection), &context)?;
    assert_eq!(progress.observed_stack_units, Some(0));
    assert_eq!(progress.current_truth, PredicateTruth::False);
    assert_eq!(progress.stable_samples_observed, 0);
    assert!(!progress.current_goal_proven);
    assert!(!watch.fenced());
    Ok(())
}

#[test]
fn no_work_watch_unknown_is_not_zero_and_interrupts_stability() -> Result<()> {
    for missing_domain in [true, false] {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, Some(20))?;
        let (projection, context) = projection(raw(140, Some(20)), 2, 9)?;
        let policy = if missing_domain {
            EvidencePolicy::at(projection.snapshot().anchor())
        } else {
            let mut policy = accepted(&projection);
            policy.sources.clear();
            policy
        };
        let progress = watch.sample(&projection, policy, &context)?;
        assert_eq!(progress.observed_stack_units, None);
        assert_eq!(progress.current_truth, PredicateTruth::Unknown);
        assert_eq!(progress.stable_samples_observed, 0);
        assert!(!progress.current_goal_proven);
        assert_eq!(
            sample(&mut watch, 150, 10, Some(20))?.stable_samples_observed,
            1
        );
        assert!(sample(&mut watch, 160, 11, Some(20))?.current_goal_proven);
    }
    Ok(())
}

#[test]
fn no_work_watch_off_cadence_contradictions_reset_but_never_accelerate_samples() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, Some(20))?;
    assert_eq!(
        sample(&mut watch, 135, 9, Some(5))?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 136, 10, Some(20))?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 140, 11, Some(20))?.stable_samples_observed,
        1
    );
    assert!(sample(&mut watch, 150, 12, Some(20))?.current_goal_proven);
    Ok(())
}

#[test]
fn no_work_watch_exact_replay_and_same_tick_changed_counts_do_not_add_stability() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    let first = sample(&mut watch, 130, 8, Some(20))?;
    let replay = sample(&mut watch, 130, 8, Some(20))?;
    assert_eq!(replay, first);
    assert_eq!(
        sample(&mut watch, 130, 8, Some(5))?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 130, 8, Some(20))?.stable_samples_observed,
        0
    );
    assert_eq!(
        sample(&mut watch, 140, 9, Some(20))?.stable_samples_observed,
        1
    );
    assert!(sample(&mut watch, 150, 10, Some(20))?.current_goal_proven);
    Ok(())
}

#[test]
fn no_work_watch_source_gaps_break_unfinished_stability() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, Some(20))?;
    let after_gap = sample(&mut watch, 140, 20, Some(20))?;
    assert!(!after_gap.continuous);
    assert_eq!(after_gap.stable_samples_observed, 1);
    assert!(!after_gap.current_goal_proven);
    let replay = sample(&mut watch, 140, 20, Some(20))?;
    assert_eq!(replay, after_gap);
    assert!(sample(&mut watch, 150, 21, Some(20))?.current_goal_proven);
    Ok(())
}

#[test]
fn no_work_watch_deadline_is_fixed_and_missed_completion_cannot_be_resurrected() -> Result<()> {
    let (mut watch, _, _) = initial(2, 140)?;
    assert_eq!(
        sample(&mut watch, 135, 8, Some(20))?.stable_samples_observed,
        1
    );
    let deadline = sample(&mut watch, 140, 9, Some(20))?;
    assert!(deadline.deadline_reached);
    assert!(deadline.expired);
    assert!(!deadline.current_goal_proven);
    assert!(!sample(&mut watch, 140, 10, Some(20))?.current_goal_proven);
    assert!(!sample(&mut watch, 150, 11, Some(20))?.current_goal_proven);
    let (mut timely, _, _) = initial(2, 140)?;
    sample(&mut timely, 130, 8, Some(20))?;
    let terminal = sample(&mut timely, 140, 9, Some(20))?;
    assert!(terminal.current_goal_proven);
    assert!(!terminal.expired);
    assert_eq!(terminal.achieved_at, Some(GameTick(140)));
    Ok(())
}

#[test]
fn no_work_watch_preserves_achievement_separately_from_current_stock() -> Result<()> {
    let (mut watch, original, context) = initial(2, 200)?;
    sample(&mut watch, 130, 8, Some(20))?;
    sample(&mut watch, 140, 9, Some(20))?;
    let lost = sample(&mut watch, 150, 10, Some(5))?;
    assert_eq!(lost.achieved_at, Some(GameTick(140)));
    assert_eq!(lost.current_truth, PredicateTruth::False);
    assert!(!lost.current_goal_proven);
    watch.invalidate_current_sample();
    assert!(watch.latest().is_none());
    assert!(!watch.current_goal_proven());
    let regained = sample(&mut watch, 210, 11, Some(20))?;
    assert!(regained.current_goal_proven);
    assert_eq!(regained.achieved_at, Some(GameTick(140)));
    let mut reopened = StockTargetWatch::new(original, GameTick(200), 10, 2, &context)?;
    let fresh = sample(&mut reopened, 210, 12, Some(20))?;
    assert_eq!(fresh.achieved_at, None);
    assert!(fresh.expired);
    assert!(!fresh.current_goal_proven);
    Ok(())
}

#[test]
fn no_work_watch_failed_qualification_withdraws_and_preserves_decoded_floors() -> Result<()> {
    for cause in 0..6 {
        let (mut watch, _, _) = initial(2, 200)?;
        sample(&mut watch, 130, 8, Some(20))?;
        let (projection, mut context) = projection(raw(140, Some(20)), 2, 9)?;
        let mut policy = accepted(&projection);
        match cause {
            0 => context
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            1 => context.cancellation_requested = true,
            2 => context.budget.max_bytes = STOCK_TARGET_WATCH_BYTES - 1,
            3 => context.budget.max_entities = 1,
            4 => policy.all_entities = EvidenceCoverage::Complete,
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
            sample(&mut watch, 140, 9, Some(20))?.stable_samples_observed,
            1
        );
        assert!(sample(&mut watch, 150, 10, Some(20))?.current_goal_proven);
    }
    Ok(())
}

#[test]
fn no_work_watch_capture_before_policy_consumes_tick_and_allocator_floors() -> Result<()> {
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, Some(20))?;
    let mut newer = raw(180, Some(20));
    newer.next_item_id = 400;
    watch.note_capture(&newer)?;
    assert_eq!(watch.high_tick(), GameTick(180));
    assert_eq!(watch.high_cursor().sequence, 8);
    assert!(watch.latest().is_none());
    // Projection/policy failure after this decode cannot make tick 170 or the
    // previous item allocator usable again.
    watch.invalidate_current_sample();
    assert!(sample(&mut watch, 170, 9, Some(20)).is_err());
    assert!(watch.fenced());
    assert_eq!(watch.high_tick(), GameTick(180));
    Ok(())
}

#[test]
fn no_work_watch_native_lineage_regressions_permanently_fence() -> Result<()> {
    for cause in 0..6 {
        let (mut watch, _, _) = initial(2, 200)?;
        let mut capture = raw(140, Some(20));
        match cause {
            0 => capture.jobs.bridge_generation += 1,
            1 => capture.jobs.next_job_id -= 1,
            2 => capture.next_building_id -= 1,
            3 => capture.next_item_id -= 1,
            4 => capture.jobs.world_folder = "region2".into(),
            _ => capture.jobs.year_tick = 119,
        }
        assert!(watch.note_capture(&capture).is_err());
        assert!(watch.fenced());
        assert!(sample(&mut watch, 150, 8, Some(20)).is_err());
        assert!(watch.latest().is_none());
    }
    Ok(())
}

#[test]
fn no_work_watch_canonical_epoch_sequence_and_cursor_forks_permanently_fence() -> Result<()> {
    for (epoch, sequence) in [(3, 8), (2, 6), (2, 7)] {
        let (mut watch, _, _) = initial(2, 200)?;
        let (projection, context) = projection(raw(130, Some(20)), epoch, sequence)?;
        assert!(
            watch
                .sample(&projection, accepted(&projection), &context)
                .is_err()
        );
        assert!(watch.fenced());
    }
    let (mut watch, _, _) = initial(2, 200)?;
    sample(&mut watch, 130, 8, Some(20))?;
    let capture = raw(130, Some(20));
    let mut context = context(&capture, 2, 8)?;
    context.anchor.state_hash = Digest32::of_bytes(b"forked citizen anchor");
    let projection = InventoryBaselineProjection::new(context.anchor, capture, &context)?;
    assert!(
        watch
            .sample(&projection, accepted(&projection), &context)
            .is_err()
    );
    assert!(watch.fenced());
    Ok(())
}

#[test]
fn no_work_watch_sample_and_original_witnesses_are_deterministic_and_sensitive() -> Result<()> {
    let (mut first, original, context) = initial(2, 200)?;
    let mut same = StockTargetWatch::new(original.clone(), GameTick(200), 10, 2, &context)?;
    let different = StockTargetWatch::new(original, GameTick(210), 10, 2, &context)?;
    assert_eq!(first.witness(), same.witness());
    assert_ne!(first.witness(), different.witness());
    let a = sample(&mut first, 130, 8, Some(20))?;
    let b = sample(&mut same, 130, 8, Some(20))?;
    assert_eq!(a, b);
    let changed = sample(&mut same, 130, 8, Some(21))?;
    assert_ne!(b.source_digest, changed.source_digest);
    assert_ne!(b.witness, changed.witness);
    Ok(())
}
