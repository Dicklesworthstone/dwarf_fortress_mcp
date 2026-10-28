#![forbid(unsafe_code)]
//! Public-API integration of the original semantic Build plan, fresh spatial
//! authority, paired review and native furniture journals. Native I/O and disk
//! faults are injected; no DFHack or compatibility admission is claimed.

#[path = "semantic_furniture/fixtures.rs"]
mod fixtures;

use dfmcp_adapter::build_placement::BuildPhase;
use dfmcp_adapter::build_placement::journal::{BuildJournal, BuildMode, BuildState};
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::semantic_furniture::store::AssociationStore;
use dfmcp_adapter::semantic_furniture::{FurnitureEvidenceOwner, NativeFurniturePlacement};
use dfmcp_core::{Capability, Digest32, GameTick, Result};
use dfmcp_world::Predicate;
use fixtures::*;

#[test]
fn original_build_review_places_once_and_retains_unproven_original_construction_goal() -> Result<()>
{
    let mut fixture = Fixture::new()?;
    let original = fixture.original.clone();
    let review = fixture.observe()?;
    assert_eq!(review.original_plan(), &original);
    assert_eq!(
        review.native_plan().key(),
        original.steps[0].idempotency_key
    );
    assert_eq!(review.native_plan().before(), &fixture.capture);
    assert_ne!(review.original_object_digest(), Digest32::ZERO);
    assert_eq!(fixture.native.borrow().prepares, 0);
    assert_eq!(fixture.native.borrow().commits, 0);
    let prepared = fixture.session.prepare(
        review.seal(),
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    assert_eq!(prepared.native_record().state(), BuildState::Prepared);
    assert_eq!(prepared.placement(), &NativeFurniturePlacement::Unverified);
    assert_eq!(
        prepared.pending_obligation(),
        original.steps[0].obligation.as_ref()
    );
    assert!(!prepared.original_goal_proven());
    let before_duplicate = (
        fixture.native.borrow().prepares,
        fixture.evidence_state.borrow().reads,
        fixture.native_disk.syncs(),
    );
    let duplicate = fixture.session.prepare(
        review.seal(),
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    assert_eq!(duplicate.native_record(), prepared.native_record());
    assert_eq!(
        before_duplicate,
        (
            fixture.native.borrow().prepares,
            fixture.evidence_state.borrow().reads,
            fixture.native_disk.syncs()
        )
    );
    assert!(
        fixture
            .session
            .commit(
                review.seal(),
                false,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().commits, 0);
    let result = fixture.session.commit(
        review.seal(),
        true,
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    assert_eq!(result.review().original_plan(), &original);
    assert_eq!(result.review().seal(), review.seal());
    assert_eq!(
        result.pending_obligation(),
        original.steps[0].obligation.as_ref()
    );
    assert!(!result.original_goal_proven());
    assert_eq!(result.native_record().state(), BuildState::Terminal);
    assert!(matches!(
        result.placement(),
        NativeFurniturePlacement::Placed {
            native_item_id: 42,
            ..
        }
    ));
    let native = result
        .native_record()
        .native()
        .ok_or_else(|| invalid("placement receipt"))?;
    assert_eq!(native.phase(), BuildPhase::Placed);
    assert_eq!(native.plan().key(), original.steps[0].idempotency_key);
    let insertion = native
        .insertion()
        .ok_or_else(|| invalid("placement insertion"))?;
    assert_eq!(insertion.building_id(), fixture.capture.next_building_id());
    assert_eq!(insertion.job_id(), fixture.capture.next_job_id());
    assert_eq!(fixture.guard.dispatches.get(), 1);
    let repeat = fixture.session.commit(
        review.seal(),
        true,
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    assert_eq!(repeat.native_record(), result.native_record());
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 1);
    let retained = fixture
        .session
        .inspect(review.seal(), &query_context(&fixture.context))?;
    assert_eq!(retained.review().original_plan(), &original);
    assert!(!retained.original_goal_proven());
    Ok(())
}

#[test]
fn lost_placement_reply_reopens_offline_then_queries_original_key_without_redispatch() -> Result<()>
{
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    fixture.native.borrow_mut().lost_commit_reply = true;
    assert!(
        fixture
            .session
            .commit(
                review.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    let durable = durable_inventory(&fixture.native_disk, &fixture.binding, &fixture.context)?;
    assert_eq!(
        durable
            .entry(review.native_plan().key())
            .ok_or_else(|| invalid("durable dispatch"))?
            .state(),
        BuildState::DispatchStarted
    );
    assert_eq!(fixture.native.borrow().commits, 1);
    let query = query_context(&fixture.context);
    let syncs = (fixture.native_disk.syncs(), fixture.associations.syncs());
    let calls = (
        fixture.native.borrow().connections,
        fixture.native.borrow().queries,
        fixture.evidence_state.borrow().reads,
    );
    drop(fixture.session);
    let mut offline = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Offline,
        true,
        &query,
    )?;
    let restored = offline.reattach(fixture.original.clone(), &query)?;
    assert_eq!(restored.seal(), review.seal());
    assert_eq!(
        restored.original_object_digest(),
        review.original_object_digest()
    );
    let historical = offline.inspect(restored.seal(), &query)?;
    assert_eq!(
        historical.placement(),
        &NativeFurniturePlacement::Unverified
    );
    assert!(!historical.original_goal_proven());
    assert_eq!(
        syncs,
        (fixture.native_disk.syncs(), fixture.associations.syncs())
    );
    assert_eq!(
        calls,
        (
            fixture.native.borrow().connections,
            fixture.native.borrow().queries,
            fixture.evidence_state.borrow().reads
        )
    );
    assert!(
        offline
            .reconcile(
                restored.seal(),
                &query,
                factory(fixture.native.clone(), fixture.native_disk.clone()),
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().connections, calls.0);
    drop(offline);
    let mut recovered = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Recover,
        false,
        &query,
    )?;
    let restored = recovered.reattach(fixture.original.clone(), &query)?;
    let result = recovered.reconcile(
        restored.seal(),
        &query,
        factory(fixture.native.clone(), fixture.native_disk.clone()),
        &mut fixture.guard,
    )?;
    assert!(matches!(
        result.placement(),
        NativeFurniturePlacement::Placed { .. }
    ));
    assert_eq!(result.review().original_plan(), &fixture.original);
    assert!(!result.original_goal_proven());
    assert_eq!(fixture.native.borrow().queries, calls.1 + 1);
    assert_eq!(fixture.evidence_state.borrow().reads, calls.2);
    let after = fixture.native.borrow().queries;
    recovered.reconcile(
        restored.seal(),
        &query,
        factory(fixture.native.clone(), fixture.native_disk.clone()),
        &mut fixture.guard,
    )?;
    assert_eq!(fixture.native.borrow().queries, after);
    assert!(
        recovered
            .commit(
                restored.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn missing_native_record_preserves_uncertainty_and_never_creates_retry_permission() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    fixture.native.borrow_mut().lost_commit_reply = true;
    assert!(
        fixture
            .session
            .commit(
                review.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    fixture.native.borrow_mut().forget_query = true;
    let query = query_context(&fixture.context);
    drop(fixture.session);
    let mut recovered = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Recover,
        false,
        &query,
    )?;
    let restored = recovered.reattach(fixture.original.clone(), &query)?;
    let before = fixture.native.borrow().queries;
    let _missing = recovered.reconcile(
        restored.seal(),
        &query,
        factory(fixture.native.clone(), fixture.native_disk.clone()),
        &mut fixture.guard,
    );
    assert_eq!(fixture.native.borrow().queries, before + 1);
    let pending = recovered.inspect(restored.seal(), &query)?;
    assert!(pending.native_record().unresolved());
    assert_eq!(pending.placement(), &NativeFurniturePlacement::Unverified);
    assert!(!pending.original_goal_proven());
    assert!(
        recovered
            .commit(
                restored.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().commits, 1);
    assert_eq!(fixture.native.borrow().prepares, 1);
    Ok(())
}

#[test]
fn post_dispatch_marker_policy_source_and_original_custody_loss_block_the_writer() -> Result<()> {
    for fault in 0..5 {
        let mut fixture = Fixture::new()?;
        let review = fixture.prepared()?;
        let policy = fixture.guard.permitted.clone();
        let evidence = fixture.evidence_state.clone();
        let associations = fixture.associations.clone();
        let changed = spatial_observation(&fixture.capture, SPATIAL_GENERATION + 1)?;
        fixture.native_disk.after_next_sync(move || match fault {
            0 => policy.set(false),
            1 => evidence.borrow_mut().permitted = false,
            2 => evidence.borrow_mut().observation = changed,
            3 => associations.lose_identity(),
            _ => associations.corrupt(),
        });
        assert!(
            fixture
                .session
                .commit(
                    review.seal(),
                    true,
                    &mut fixture.evidence,
                    &fixture.context,
                    &mut fixture.guard
                )
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(fixture.native.borrow().commits, 0, "fault {fault}");
        let durable = durable_inventory(&fixture.native_disk, &fixture.binding, &fixture.context)?;
        assert_eq!(
            durable
                .entry(review.native_plan().key())
                .ok_or_else(|| invalid("marker missing"))?
                .state(),
            BuildState::DispatchStarted,
            "fault {fault}"
        );
        fixture.guard.permitted.set(true);
        fixture.evidence_state.borrow_mut().permitted = true;
        assert!(
            fixture
                .session
                .commit(
                    review.seal(),
                    true,
                    &mut fixture.evidence,
                    &fixture.context,
                    &mut fixture.guard
                )
                .is_err(),
            "retry fault {fault}"
        );
        assert_eq!(fixture.native.borrow().commits, 0);
        assert_eq!(fixture.native.borrow().prepares, 1);
    }
    Ok(())
}

#[test]
fn reattachment_rejects_changed_full_original_under_the_same_native_key() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    let query = query_context(&fixture.context);
    drop(fixture.session);
    for change in 0..4 {
        let changed = reseal(&fixture.original, |plan| match change {
            0 => plan.summary.push_str(" with changed intent"),
            1 => plan.terminal_condition = Predicate::Paused(false),
            2 => plan.steps[0].postconditions.push(Predicate::Paused(false)),
            _ => {
                if let Some(obligation) = plan.steps[0].obligation.as_mut() {
                    obligation.stable_for_observations += 1;
                }
            }
        });
        changed.validate_structure()?;
        assert_eq!(
            changed.steps[0].idempotency_key,
            fixture.original.steps[0].idempotency_key
        );
        let mut reopened = reopen(
            fixture.native_disk.clone(),
            fixture.associations.clone(),
            &fixture.binding,
            BuildMode::Offline,
            true,
            &query,
        )?;
        assert!(
            reopened.reattach(changed, &query).is_err(),
            "changed field {change}"
        );
        let restored = reopened.reattach(fixture.original.clone(), &query)?;
        assert_eq!(restored.seal(), review.seal());
        assert_eq!(restored.original_plan(), &fixture.original);
    }
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn association_loss_or_another_native_journal_cannot_adopt_existing_preparation() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.prepared()?;
    drop(fixture.session);
    let mut original = BuildJournal::open(
        fixture.native_disk.clone(),
        &fixture.context,
        BuildMode::Control,
        Some(fixture.binding.clone()),
        None,
    )?;
    let id = original.inventory(&fixture.context)?.journal_id;
    let native = BuildSession::new(original, &fixture.context)?;
    let replacement = AssociationStore::open(Memory::default(), id, true, false, &fixture.context)?;
    assert!(Session::new(native, replacement, &fixture.context).is_err());
    let mut other = BuildJournal::open(
        Memory::default(),
        &fixture.context,
        BuildMode::Control,
        Some(fixture.binding.clone()),
        Some([88; 32]),
    )?;
    let other_id = other.inventory(&fixture.context)?.journal_id;
    assert_ne!(id, other_id);
    assert!(
        AssociationStore::open(
            fixture.associations.clone(),
            other_id,
            false,
            true,
            &fixture.context
        )
        .is_err()
    );
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn association_sync_failure_precedes_native_intent_and_preparation() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.observe()?;
    let native_before = fixture.native_disk.bytes();
    fixture.associations.fail_next_sync();
    assert!(
        fixture
            .session
            .prepare(
                review.seal(),
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native_disk.bytes(), native_before);
    assert_eq!(fixture.native.borrow().prepares, 0);
    assert_eq!(fixture.native.borrow().commits, 0);
    assert!(
        fixture
            .session
            .prepare(
                review.seal(),
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().prepares, 0);
    Ok(())
}

#[test]
fn read_only_association_never_controls_even_with_full_current_grants() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    drop(fixture.session);
    let mut readonly = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Control,
        true,
        &fixture.context,
    )?;
    readonly.reattach(fixture.original.clone(), &fixture.context)?;
    let before = (
        fixture.native.borrow().connections,
        fixture.native.borrow().prepares,
        fixture.native.borrow().commits,
    );
    assert!(
        readonly
            .commit(
                review.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert!(
        readonly
            .prepare(
                review.seal(),
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert!(
        readonly
            .observe(
                fixture.original.clone(),
                fixture.capture.selection(),
                &mut fixture.evidence,
                &fixture.context,
                factory(fixture.native.clone(), fixture.native_disk.clone()),
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(
        before,
        (
            fixture.native.borrow().connections,
            fixture.native.borrow().prepares,
            fixture.native.borrow().commits
        )
    );
    Ok(())
}

#[test]
fn paired_confirmation_budget_and_original_expiry_refuse_before_effect() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    let bytes = fixture.native_disk.bytes();
    let wrong = Digest32::of_bytes(b"another paired furniture review");
    assert!(
        fixture
            .session
            .commit(
                wrong,
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert!(
        fixture
            .session
            .commit(
                review.seal(),
                false,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native_disk.bytes(), bytes);
    let mut bounded = fixture.context.clone();
    bounded.budget.max_bytes = 1024;
    assert!(
        fixture
            .session
            .commit(
                review.seal(),
                true,
                &mut fixture.evidence,
                &bounded,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native_disk.bytes(), bytes);
    assert_eq!(fixture.native.borrow().commits, 0);
    let mut expired = Fixture::new()?;
    let review = expired.prepared()?;
    let bytes = expired.native_disk.bytes();
    let mut later = expired.context.clone();
    later.anchor.tick = GameTick(expired.original.expires_at_tick.get() + 1);
    assert!(
        expired
            .session
            .commit(
                review.seal(),
                true,
                &mut expired.evidence,
                &later,
                &mut expired.guard
            )
            .is_err()
    );
    assert_eq!(expired.native_disk.bytes(), bytes);
    assert_eq!(expired.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn cancellation_after_original_expiry_needs_query_and_never_constructs() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    let mut query = query_context(&fixture.context);
    query.anchor.tick = GameTick(fixture.original.expires_at_tick.get() + 1);
    assert!(
        !query
            .grants
            .iter()
            .any(|g| g.capability == Capability::Construct)
    );
    let reads = fixture.evidence_state.borrow().reads;
    let cancelled = fixture.session.cancel(
        review.seal(),
        &query,
        factory(fixture.native.clone(), fixture.native_disk.clone()),
        &mut fixture.guard,
    )?;
    assert_eq!(
        cancelled
            .native_record()
            .native()
            .ok_or_else(|| invalid("cancelled receipt"))?
            .phase(),
        BuildPhase::Cancelled
    );
    assert_eq!(cancelled.review().original_plan(), &fixture.original);
    assert_eq!(cancelled.placement(), &NativeFurniturePlacement::Unverified);
    assert!(!cancelled.original_goal_proven());
    assert_eq!(fixture.native.borrow().commits, 0);
    assert_eq!(fixture.native.borrow().cancels, 1);
    assert_eq!(fixture.evidence_state.borrow().reads, reads);
    Ok(())
}

#[test]
fn independent_spatial_generation_and_closed_sources_are_required_for_every_review() -> Result<()> {
    let mut fixture = Fixture::new()?;
    assert_ne!(fixture.capture.generation(), SPATIAL_GENERATION);
    let review = fixture.prepared()?;
    assert_eq!(
        review.native_plan().before().generation(),
        fixture.capture.generation()
    );
    let evidence = fixture.evidence_state.borrow();
    assert_eq!(evidence.active, 0);
    assert!(evidence.reads >= 3);
    assert_eq!(evidence.opened, evidence.reads);
    assert!(evidence.authorizations >= evidence.reads * 2);
    drop(evidence);
    assert_eq!(
        fixture.evidence.routing_evidence()?.source_generation(),
        SPATIAL_GENERATION
    );
    assert_eq!(review.original_plan().anchor, fixture.context.anchor);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn changed_spatial_basis_cannot_be_revived_by_restoring_old_capture_bytes() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let original_observation = fixture.evidence_state.borrow().observation.clone();
    fixture.evidence_state.borrow_mut().observation =
        spatial_observation(&fixture.capture, SPATIAL_GENERATION + 1)?;
    assert!(
        fixture
            .evidence
            .refresh_after_capture(&fixture.capture, &fixture.context)
            .is_err()
    );
    fixture.evidence.invalidate();
    fixture.evidence_state.borrow_mut().observation = original_observation;
    assert!(
        fixture
            .evidence
            .refresh_after_capture(&fixture.capture, &fixture.context)
            .is_err()
    );
    assert!(fixture.evidence.routing_evidence().is_err());
    let replacement = evidence_owner(
        fixture.original_state.clone(),
        fixture.evidence_state.clone(),
        &fixture.context,
    )?;
    fixture.evidence = replacement;
    let review = fixture.observe()?;
    assert_eq!(review.original_plan(), &fixture.original);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn unsupported_checkpoint_and_material_constraints_are_retained_and_refused() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let original = fixture.original.clone();
    let checkpoint = reseal(&original, |plan| plan.requires_checkpoint = true);
    assert!(
        fixture
            .session
            .observe(
                checkpoint.clone(),
                fixture.capture.selection(),
                &mut fixture.evidence,
                &fixture.context,
                factory(fixture.native.clone(), fixture.native_disk.clone()),
                &mut fixture.guard
            )
            .is_err()
    );
    assert!(checkpoint.requires_checkpoint);
    assert_eq!(fixture.native.borrow().connections, 0);
    let mut material = original.steps[0].action.clone();
    if let dfmcp_intent::Action::Build { material, .. } = &mut material {
        material.prefer_nearest = true;
    }
    let constrained = reseal(&original, |plan| {
        plan.steps[0].action = material;
        plan.steps[0].idempotency_key = dfmcp_intent::derive_step_idempotency_key(
            plan.intent_id,
            plan.anchor,
            plan.steps[0].id,
            &plan.steps[0].action,
        );
    });
    constrained.validate_structure()?;
    assert!(
        fixture
            .session
            .observe(
                constrained.clone(),
                fixture.capture.selection(),
                &mut fixture.evidence,
                &fixture.context,
                factory(fixture.native.clone(), fixture.native_disk.clone()),
                &mut fixture.guard
            )
            .is_err()
    );
    assert!(
        matches!(&constrained.steps[0].action, dfmcp_intent::Action::Build { material, .. } if material.prefer_nearest)
    );
    assert_eq!(fixture.native.borrow().prepares, 0);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn reopened_duplicate_preparation_retains_exact_evidence_without_restoring_permission() -> Result<()>
{
    let mut fixture = Fixture::new()?;
    let review = fixture.prepared()?;
    let prior = fixture.session.inspect(review.seal(), &fixture.context)?;
    drop(fixture.session);
    let mut reopened = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Control,
        false,
        &fixture.context,
    )?;
    reopened.reattach(fixture.original.clone(), &fixture.context)?;
    let calls = (
        fixture.native.borrow().connections,
        fixture.native.borrow().prepares,
        fixture.evidence_state.borrow().reads,
    );
    let history = (fixture.native_disk.bytes(), fixture.associations.bytes());
    let retained = reopened.prepare(
        review.seal(),
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    assert_eq!(retained.native_record(), prior.native_record());
    assert_eq!(retained.review().original_plan(), &fixture.original);
    assert_eq!(
        calls,
        (
            fixture.native.borrow().connections,
            fixture.native.borrow().prepares,
            fixture.evidence_state.borrow().reads
        )
    );
    assert_eq!(
        history,
        (fixture.native_disk.bytes(), fixture.associations.bytes())
    );
    assert!(
        reopened
            .commit(
                review.seal(),
                true,
                &mut fixture.evidence,
                &fixture.context,
                &mut fixture.guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn unchanged_native_guard_default_rechecks_authority_after_durable_dispatch_marker() -> Result<()> {
    use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildStage};
    use dfmcp_adapter::build_placement::{BuildBinding, BuildPlan, BuildSelection};
    use dfmcp_core::OperationContext;
    use std::time::Duration;
    struct ExistingGuard(Guard);
    impl BuildGuard for ExistingGuard {
        fn check(
            &mut self,
            stage: BuildStage,
            binding: &BuildBinding,
            plan: Option<&BuildPlan>,
            selection: BuildSelection,
            context: &OperationContext,
        ) -> Result<()> {
            self.0.check(stage, binding, plan, selection, context)
        }
    }
    let fixture = Fixture::new()?;
    drop(fixture.session);
    let plan = BuildPlan::new(
        &fixture.original.steps[0].idempotency_key,
        fixture.capture.clone(),
    )?;
    let mut journal = BuildJournal::open(
        fixture.native_disk.clone(),
        &fixture.context,
        BuildMode::Control,
        Some(fixture.binding.clone()),
        None,
    )?;
    let mut native = factory(fixture.native.clone(), fixture.native_disk.clone())(
        &fixture.binding,
        &fixture.context,
        Duration::from_secs(10),
    )?;
    let mut guard = ExistingGuard(fixture.guard);
    journal.prepare(&mut native, &plan, &fixture.context, &mut guard)?;
    let permitted = guard.0.permitted.clone();
    fixture
        .native_disk
        .after_next_sync(move || permitted.set(false));
    assert!(
        journal
            .commit(
                &mut native,
                plan.key(),
                plan.digest(),
                &fixture.context,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 0);
    assert_eq!(
        journal
            .inventory(&fixture.context)?
            .entry(plan.key())
            .ok_or_else(|| invalid("retained dispatch marker"))?
            .state(),
        BuildState::DispatchStarted
    );
    assert_eq!(
        durable_inventory(&fixture.native_disk, &fixture.binding, &fixture.context)?
            .entry(plan.key())
            .ok_or_else(|| invalid("durable dispatch marker"))?
            .state(),
        BuildState::DispatchStarted
    );
    assert_eq!(guard.0.dispatches.get(), 0);
    Ok(())
}

#[path = "semantic_furniture/goal_tests.rs"]
mod goal_tests;

#[path = "semantic_furniture/archive_tests.rs"]
mod archive_tests;
