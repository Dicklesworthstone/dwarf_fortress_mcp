use super::*;
use dfmcp_core::{MapCoord, MapCuboid};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Prepare,
    Commit,
    Reconcile,
    Cancel,
}

struct Checked {
    inner: Fake,
    advance: Option<Boundary>,
    seen: Vec<OperationContext>,
}
impl Checked {
    fn check(&mut self, boundary: Boundary, step: &PlanStep, c: &OperationContext) -> Result<()> {
        self.seen.push(c.clone());
        if self.advance == Some(boundary) {
            let mut acquired = c.clone();
            acquired.anchor.tick = GameTick(300);
            self.inner.floor = 300;
            let mut broader = acquired.clone();
            broader.grants = context().grants;
            if boundary == Boundary::Reconcile {
                broader.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                acquired.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            } else {
                let scope = step.action.scope();
                broader.authorize(
                    step.required_capability,
                    step.risk,
                    &scope.entity_ids,
                    scope.map_area,
                )?;
                acquired.authorize(
                    step.required_capability,
                    step.risk,
                    &scope.entity_ids,
                    scope.map_area,
                )?;
            }
        }
        Ok(())
    }
}
impl LiveFamilyExecutor for Checked {
    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<FamilyPreparation> {
        self.check(Boundary::Prepare, step, c)?;
        self.inner.prepare(plan, step, route, c)
    }
    fn commit(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<FamilyReceipt> {
        self.check(Boundary::Commit, step, c)?;
        self.inner.commit(plan, step, route, preparation, c)
    }
    fn reconcile(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<super::super::super::ReconcileOutcome> {
        self.inner.reconcile(plan, step, route, c)
    }
    fn cancel_prepared(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<Option<FamilyReceipt>> {
        self.check(Boundary::Cancel, step, c)?;
        self.inner
            .cancel_prepared(plan, step, route, preparation, c)
    }
}
impl DurableFamilyExecutor for Checked {
    fn recover_step(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<RecoveredFamily> {
        self.check(Boundary::Reconcile, step, c)?;
        self.inner.recover_step(plan, step, route, c)
    }
    fn observed_tick_floor(&self) -> u64 {
        self.inner.floor
    }
    fn abandon_preparation(&mut self) {
        self.inner.abandon_preparation();
    }
}
fn opened(opening: &OperationContext) -> Result<(Coordinator<Memory>, Checked)> {
    let original = plan(2);
    let pin = initial_recovery(&original, opening)?;
    let disk = Memory::default();
    let owner = Coordinator::open(
        disk.clone(),
        opening,
        BuildMode::Control,
        Some(original),
        pin,
        true,
    )?;
    Ok((
        owner,
        Checked {
            inner: Fake {
                disk,
                pin,
                ..Fake::default()
            },
            advance: None,
            seen: Vec::new(),
        },
    ))
}

#[test]
fn opening_expiry_is_enforced_inside_all_four_family_boundaries() -> Result<()> {
    for boundary in [
        Boundary::Prepare,
        Boundary::Commit,
        Boundary::Reconcile,
        Boundary::Cancel,
    ] {
        let mut opening = context();
        let capability = if boundary == Boundary::Reconcile {
            Capability::Query
        } else {
            Capability::ControlClock
        };
        for grant in &mut opening.grants {
            if grant.capability == capability {
                grant.expires_at_tick = Some(GameTick(200));
            }
        }
        let c = context();
        let (mut owner, mut executor) = opened(&opening)?;
        let review = if boundary == Boundary::Prepare {
            None
        } else {
            Some(owner.prepare_next(&mut executor, &c)?)
        };
        executor.advance = Some(boundary);
        let result = match boundary {
            Boundary::Prepare => owner.prepare_next(&mut executor, &c).map(|_| ()),
            Boundary::Commit => owner
                .commit(review.ok_or_else(conflict)?.seal, true, &mut executor, &c)
                .map(|_| ()),
            Boundary::Reconcile => owner.reconcile(StepId::ZERO, &mut executor, &c).map(|_| ()),
            Boundary::Cancel => owner
                .cancel_prepared(StepId::ZERO, &mut executor, &c)
                .map(|_| ()),
        };
        assert_eq!(
            result.err().map(|cause| cause.code),
            Some(ErrorCode::CapabilityDenied)
        );
        assert_eq!(owner.high_tick(), 300);
        assert_eq!(executor.inner.commit, 0);
        assert_eq!(executor.inner.recover, 0);
        assert_eq!(executor.inner.cancel, 0);
        assert_eq!(
            executor.inner.prepare,
            usize::from(boundary != Boundary::Prepare)
        );
        assert!(!executor.inner.local_permit);
    }
    Ok(())
}

fn area(min: i32, max: i32) -> Result<MapCuboid> {
    MapCuboid::new(MapCoord::new(min, min, 0), MapCoord::new(max, max, 0))
}
fn scoped(
    capability: Capability,
    entities: &[u64],
    bounds: MapCuboid,
    risk: RiskTier,
    expiry: u64,
) -> CapabilityGrant {
    CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(anchor().fortress_id),
            entity_ids: entities.iter().copied().map(EntityId::new).collect(),
            map_area: Some(bounds),
        },
        max_risk: risk,
        expires_at_tick: Some(GameTick(expiry)),
        remaining_uses: None,
    }
}

#[test]
fn delegated_admin_entity_map_and_risk_intersections_never_widen_either_owner() -> Result<()> {
    for admin in 0..3 {
        let mut opening = context();
        opening.grants.push(scoped(
            if admin == 2 {
                Capability::Admin
            } else {
                Capability::Observe
            },
            &[17, 18],
            area(0, 9)?,
            RiskTier::ReadOnly,
            180,
        ));
        let mut current = context();
        current.grants.push(scoped(
            if admin == 1 {
                Capability::Admin
            } else {
                Capability::Observe
            },
            &[18, 19],
            area(5, 14)?,
            RiskTier::Guarded,
            240,
        ));
        let (mut owner, mut executor) = opened(&opening)?;
        owner.prepare_next(&mut executor, &current)?;
        let delegated = executor.seen.first().ok_or_else(corrupt)?;
        delegated.authorize(
            Capability::Observe,
            RiskTier::ReadOnly,
            &[EntityId::new(18)],
            Some(area(5, 9)?),
        )?;
        for entity in [17, 19] {
            assert!(
                delegated
                    .authorize(
                        Capability::Observe,
                        RiskTier::ReadOnly,
                        &[EntityId::new(entity)],
                        Some(area(5, 9)?)
                    )
                    .is_err()
            );
        }
        assert!(
            delegated
                .authorize(
                    Capability::Observe,
                    RiskTier::Guarded,
                    &[EntityId::new(18)],
                    Some(area(5, 9)?)
                )
                .is_err()
        );
        assert!(
            delegated
                .authorize(
                    Capability::Observe,
                    RiskTier::ReadOnly,
                    &[EntityId::new(18)],
                    Some(area(0, 14)?)
                )
                .is_err()
        );
        assert!(
            delegated
                .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
                .is_err()
        );
        let mut later = delegated.clone();
        later.anchor.tick = GameTick(181);
        assert!(
            later
                .authorize(
                    Capability::Observe,
                    RiskTier::ReadOnly,
                    &[EntityId::new(18)],
                    Some(area(5, 9)?)
                )
                .is_err()
        );
        let mut foreign = delegated.clone();
        foreign.anchor.fortress_id = FortressId::new(999);
        assert!(
            foreign
                .authorize(
                    Capability::Observe,
                    RiskTier::ReadOnly,
                    &[EntityId::new(18)],
                    Some(area(5, 9)?)
                )
                .is_err()
        );
        assert!(
            !delegated
                .grants
                .iter()
                .any(|grant| grant.capability == Capability::Admin)
        );
    }
    Ok(())
}

#[test]
fn disjoint_or_limited_scopes_cannot_turn_into_unrestricted_delegated_grants() -> Result<()> {
    for fault in 0..3 {
        let mut opening = context();
        let mut retained = scoped(
            Capability::Observe,
            &[17],
            area(0, 4)?,
            RiskTier::ReadOnly,
            200,
        );
        let mut current = context();
        let granted = match fault {
            0 => scoped(
                Capability::Observe,
                &[18],
                area(0, 4)?,
                RiskTier::ReadOnly,
                200,
            ),
            1 => scoped(
                Capability::Observe,
                &[17],
                area(5, 9)?,
                RiskTier::ReadOnly,
                200,
            ),
            _ => {
                retained.remaining_uses = Some(1);
                scoped(
                    Capability::Observe,
                    &[17],
                    area(0, 4)?,
                    RiskTier::ReadOnly,
                    200,
                )
            }
        };
        opening.grants.push(retained);
        current.grants.push(granted);
        let (mut owner, mut executor) = opened(&opening)?;
        owner.prepare_next(&mut executor, &current)?;
        let delegated = executor.seen.first().ok_or_else(corrupt)?;
        assert!(
            !delegated
                .grants
                .iter()
                .any(|grant| grant.capability == Capability::Observe)
        );
    }
    Ok(())
}
