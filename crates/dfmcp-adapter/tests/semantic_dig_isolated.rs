#![forbid(unsafe_code)]
//! Actual original Mine plans, canonical terrain, native codecs and durable
//! owners. Only native I/O and exclusively owned storage are injected.
#[allow(dead_code)]
#[path = "semantic_dig/fixtures.rs"]
mod fixtures;
#[path = "semantic_dig/goal_tests.rs"]
mod goal_tests;
use dfmcp_adapter::dig_designation::journal::session::DigSessionGuard;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigGuard, DigMode, DigStage, DigState};
use dfmcp_adapter::dig_designation::{DigPlan, DigRegion};
use dfmcp_adapter::semantic_dig::{DigEvidenceOwner, NativeDigDesignation};
use dfmcp_core::{Capability, Digest32, OperationContext, Result};
use dfmcp_world::{Predicate, PredicateTruth};
use fixtures::*;

#[test]
fn original_mine_commits_once_and_retains_exact_unproven_goal() -> Result<()> {
    let mut f = Fixture::new()?;
    let original = f.original.clone();
    let review = f.prepared()?;
    assert_eq!(review.original_plan(), &original);
    assert_eq!(
        review.native_plan().key(),
        original.steps[0].idempotency_key
    );
    assert_eq!(review.native_plan().before(), &f.capture);
    assert!(!review.native_plan().allow_hidden_neighbors());
    assert_ne!(review.source_identity_digest(), Digest32::ZERO);
    let duplicate = f
        .session
        .prepare(review.seal(), &mut f.evidence, &f.context, &mut f.guard)?;
    assert_eq!(duplicate.native_record().state(), DigState::Prepared);
    assert_eq!(f.native.borrow().prepares, 1);
    assert!(
        f.session
            .commit(
                review.seal(),
                false,
                &mut f.evidence,
                &f.context,
                &mut f.guard
            )
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    let result = f.session.commit(
        review.seal(),
        true,
        &mut f.evidence,
        &f.context,
        &mut f.guard,
    )?;
    assert_eq!(
        result.pending_obligation(),
        original.steps[0].obligation.as_ref()
    );
    assert_eq!(result.review().original_plan(), &original);
    assert!(matches!(
        result.designation(),
        NativeDigDesignation::Designated {
            designated_count: 4,
            ..
        }
    ));
    assert!(!result.original_goal_proven());
    let old = result.native_record().clone();
    let replay = f.session.commit(
        review.seal(),
        true,
        &mut f.evidence,
        &f.context,
        &mut f.guard,
    )?;
    assert_eq!(replay.native_record(), &old);
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.evidence_state.borrow().reads, 3);
    assert_eq!(f.session.inventory(&f.context)?.total_records, 1);
    Ok(())
}
#[test]
fn lost_commit_reply_reconciles_exact_original_without_redispatch() -> Result<()> {
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    f.native.borrow_mut().lost_commit_reply = true;
    assert!(
        f.session
            .commit(
                review.seal(),
                true,
                &mut f.evidence,
                &f.context,
                &mut f.guard
            )
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 1);
    let context = query_context(&f.context);
    let mut recovered = reopen(
        f.native_disk.clone(),
        f.associations.clone(),
        &f.binding,
        DigMode::Recover,
        false,
        &context,
    )?;
    let attached = recovered.reattach(f.original.clone(), &context)?;
    assert_eq!(attached.seal(), review.seal());
    let result = recovered.reconcile(
        attached.seal(),
        &context,
        factory(f.native.clone(), f.native_disk.clone()),
        &mut f.guard,
    )?;
    assert!(matches!(
        result.designation(),
        NativeDigDesignation::Designated { .. }
    ));
    assert_eq!(f.native.borrow().queries, 1);
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn reopened_preparation_has_no_dispatch_permit_and_missing_query_is_not_absence() -> Result<()> {
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    let mut recovered = reopen(
        f.native_disk.clone(),
        f.associations.clone(),
        &f.binding,
        DigMode::Control,
        false,
        &f.context,
    )?;
    recovered.reattach(f.original.clone(), &f.context)?;
    assert!(
        recovered
            .commit(
                review.seal(),
                true,
                &mut f.evidence,
                &f.context,
                &mut f.guard
            )
            .is_err()
    );
    f.native.borrow_mut().forget_query = true;
    assert!(
        recovered
            .reconcile(
                review.seal(),
                &f.context,
                factory(f.native.clone(), f.native_disk.clone()),
                &mut f.guard
            )
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    assert!(
        recovered
            .observe(
                f.original.clone(),
                &mut f.evidence,
                &f.context,
                factory(f.native.clone(), f.native_disk.clone()),
                &mut f.guard
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn first_commit_edge_refreshes_independent_evidence_after_dispatch_started() -> Result<()> {
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    let state = f.evidence_state.clone();
    f.native_disk
        .after_next_sync(move || state.borrow_mut().permitted = false);
    assert!(
        f.session
            .commit(
                review.seal(),
                true,
                &mut f.evidence,
                &f.context,
                &mut f.guard
            )
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    assert_eq!(
        f.session
            .inspect(review.seal(), &f.context)?
            .native_record()
            .state(),
        DigState::DispatchStarted
    );
    Ok(())
}
struct TamperGuard {
    inner: Guard,
    disk: Memory,
    tampered: bool,
}
impl DigGuard for TamperGuard {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, c: &OperationContext) -> Result<()> {
        self.inner.check(stage, plan, c)?;
        if stage == DigStage::Commit && !self.tampered {
            self.tampered = true;
            self.disk.corrupt();
        }
        Ok(())
    }
}
impl DigSessionGuard for TamperGuard {
    fn connect(&mut self, b: &DigBinding, r: DigRegion, c: &OperationContext) -> Result<()> {
        self.inner.connect(b, r, c)
    }
    fn observe(&mut self, b: &DigBinding, r: DigRegion, c: &OperationContext) -> Result<()> {
        self.inner.observe(b, r, c)
    }
}
#[test]
fn native_marker_is_reverified_after_policy_callback_before_rpc() -> Result<()> {
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    let mut guard = TamperGuard {
        inner: Guard::default(),
        disk: f.native_disk.clone(),
        tampered: false,
    };
    assert!(
        f.session
            .commit(review.seal(), true, &mut f.evidence, &f.context, &mut guard)
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    Ok(())
}
#[test]
fn original_constraints_and_deadline_cannot_be_silently_dropped() -> Result<()> {
    for selector in 0..3 {
        let mut f = Fixture::new()?;
        f.original = reseal(&f.original, |p| match selector {
            0 => p.requires_checkpoint = true,
            1 => p.steps[0].preconditions.push(Predicate::RegionTerrain {
                area: f.capture.region().halo(),
                tile_code: 1,
            }),
            _ => p.expires_at_tick = p.anchor.tick,
        });
        assert!(f.observe().is_err());
        assert_eq!(f.native.borrow().connections, 0);
    }
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    let changed = reseal(&f.original, |p| p.summary.push_str(" changed"));
    assert!(f.session.reattach(changed, &f.context).is_err());
    assert!(f.session.inspect(review.seal(), &f.context).is_err());
    Ok(())
}
#[test]
fn uncertain_designation_never_starts_terrain_goal_monitor() -> Result<()> {
    let mut f = Fixture::new()?;
    let review = f.prepared()?;
    f.native.borrow_mut().unknown_commit = true;
    let result = f.session.commit(
        review.seal(),
        true,
        &mut f.evidence,
        &f.context,
        &mut f.guard,
    )?;
    assert_eq!(result.designation(), &NativeDigDesignation::Unverified);
    assert!(
        f.session
            .begin_goal_monitor(review.seal(), &f.context)
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn terrain_projection_preserves_hidden_and_padding_unknown_and_exact_coverage() -> Result<()> {
    use dfmcp_adapter::semantic_dig::terrain::DigTerrainProjection;
    use dfmcp_core::{MapCoord, ObservationCursor};
    use dfmcp_world::map_region::Cell;
    let f = Fixture::new()?;
    let raw = goal_observation(&f.capture, f.capture.tick(), |m| {
        m.map.cells[0] = Cell::Hidden
    })?;
    let p = DigTerrainProjection::new(
        raw,
        ObservationCursor {
            epoch: 0,
            sequence: 2,
        },
        &f.context,
    )?;
    let halo = f.capture.region().halo();
    assert_eq!(p.snapshot().tile_code_at(halo.min), None);
    assert_eq!(
        p.snapshot().tile_code_at(MapCoord::new(0, 0, halo.min.z)),
        None
    );
    assert_eq!(
        p.evidence(policy(&p))?
            .evaluate(&Predicate::RegionTerrain {
                area: dfmcp_core::MapCuboid::new(halo.min, halo.min)?,
                tile_code: 1
            })?,
        PredicateTruth::Unknown
    );
    let mut oversized = policy(&p);
    oversized.terrain_regions.push(dfmcp_core::MapCuboid::new(
        MapCoord::new(0, 0, 0),
        MapCoord::new(1, 1, 1),
    )?);
    assert!(p.evidence(oversized).is_err());
    let mut owner = f.evidence;
    let mut no_observe = f.context;
    no_observe
        .grants
        .retain(|g| g.capability != Capability::Observe);
    assert!(
        owner
            .refresh_after_capture(&f.capture, &no_observe)
            .is_err()
    );
    Ok(())
}

#[test]
fn original_terrain_router_rejects_laboratory_and_broader_domain_claims() -> Result<()> {
    use dfmcp_adapter::live_routing::LiveRoutingEvidence;
    use dfmcp_world::EvidenceCoverage;
    let f = Fixture::new()?;
    // The API accepts only a policy; an already laboratory-qualified evidence
    // object cannot be substituted. Broader policy domains are rejected here.
    let mut broader = policy(&f.original_state);
    broader.all_entities = EvidenceCoverage::Complete;
    assert!(LiveRoutingEvidence::dig_terrain_v1_8(broader, &f.original_state).is_err());
    let mut broader = policy(&f.original_state);
    broader.all_edges = EvidenceCoverage::Complete;
    assert!(LiveRoutingEvidence::dig_terrain_v1_8(broader, &f.original_state).is_err());
    assert!(
        LiveRoutingEvidence::dig_terrain_v1_8(policy(&f.original_state), &f.original_state).is_ok()
    );
    Ok(())
}
