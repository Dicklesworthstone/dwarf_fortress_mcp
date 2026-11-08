#![allow(dead_code, unused_imports)]
include!("../semantic_workforce_runtime/fixtures.rs");
use super::ExistingLaborRequest;
use dfmcp_adapter::live_production::ExistingLaborRequirement;

fn request() -> Result<ExistingLaborRequest> {
    ExistingLaborRequest::parse(
        &serde_json::json!({
            "kind":"existing_labor","expected_observation":"11".repeat(32),
            "units":["44","43"],"labor":"MINE","recipe":"wooden_bed","amount":2,
            "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2
        })
        .to_string(),
    )
}

fn requirement() -> Result<ExistingLaborRequirement> {
    ExistingLaborRequirement::new(
        "MINE".into(),
        vec![(EntityId::new(43), 142), (EntityId::new(44), 143)],
    )
}

#[test]
fn workshop_bound_existing_labor_rebuilds_order_identity_without_an_assignment() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let c = context(&projection.snapshot);
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let baselines =
        crate::semantic_work_order_runtime::inventory_baseline::InventoryBaselines::default();
    let request = request()?;
    let required = requirement()?;
    let legacy = request.compile(&evidence, &required, &c)?;
    assert_eq!(
        legacy,
        request.compile_with_workshop(&evidence, &required, &baselines, &c, None)?
    );
    let first_origin = Digest32::of_bytes(b"first completed workshop");
    let second_origin = Digest32::of_bytes(b"second completed workshop");
    let first =
        request.compile_with_workshop(&evidence, &required, &baselines, &c, Some(first_origin))?;
    let second =
        request.compile_with_workshop(&evidence, &required, &baselines, &c, Some(second_origin))?;
    super::super::workshop_binding::assert_compiled_binding(
        &legacy,
        &first,
        &second,
        first_origin,
        second_origin,
    )?;
    assert_eq!(
        ExistingLaborRequirement::from_plan(&first)?,
        Some(required.clone())
    );
    assert!(
        request
            .compile_with_workshop(&evidence, &required, &baselines, &c, Some(Digest32::ZERO))
            .is_err()
    );
    Ok(())
}

#[test]
fn original_existing_labor_plan_has_only_the_exact_finite_order() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let c = context(&projection.snapshot);
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let request = request()?;
    let requirement = requirement()?;
    let plan = request.compile(&evidence, &requirement, &c)?;
    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.id, StepId::ZERO);
    assert!(matches!(
        step.action,
        Action::CreateWorkOrder { amount: 2, .. }
    ));
    assert!(step.depends_on.is_empty());
    assert!(step.compensation.is_none());
    assert_eq!(step.preconditions, [Predicate::Paused(true)]);
    assert!(
        !plan
            .required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureProduction)
    );
    assert_eq!(
        ExistingLaborRequirement::from_plan(&plan)?,
        Some(requirement.clone())
    );
    assert_eq!(plan.anchor, c.anchor);
    assert_eq!(
        step.idempotency_key,
        derive_step_idempotency_key(plan.intent_id, c.anchor, step.id, &step.action)
    );
    let obligation = step
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("obligation missing"))?;
    assert_eq!(obligation.deadline_tick, GameTick(300));
    assert_eq!(obligation.poll_interval_ticks, 5);
    assert_eq!(obligation.stable_for_observations, 2);
    assert_eq!(
        plan.terminal_condition,
        Predicate::All(vec![requirement.predicate(), obligation.terminal.clone()]).normalized()
    );
    let limits = dfmcp_adapter::live_production::archive_limits();
    let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
    assert_eq!(
        dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?,
        plan
    );
    assert_eq!(super::super::host_plan(&plan)?.archive_seal(), seal);
    Ok(())
}

#[test]
fn identity_binds_historical_citizens_objective_and_timing_without_input_order() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_actions = 2;
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let request = request()?;
    let requirement = requirement()?;
    let original = request.compile(&evidence, &requirement, &c)?;
    let mut reordered = request.clone();
    reordered.units.reverse();
    assert_eq!(original, reordered.compile(&evidence, &requirement, &c)?);
    let replaced = ExistingLaborRequirement::new(
        "MINE".into(),
        vec![(EntityId::new(43), 242), (EntityId::new(44), 143)],
    )?;
    assert_ne!(
        original.intent_id,
        request.compile(&evidence, &replaced, &c)?.intent_id
    );
    let mut changed = request.clone();
    changed.objective = Some(
        crate::semantic_work_order_runtime::request::Objective::ObservedInventoryMinimum {
            item_type: "BED".into(),
            minimum_stack_units: 2,
        },
    );
    let inventory = changed.compile(&evidence, &requirement, &c)?;
    assert_ne!(original.intent_id, inventory.intent_id);
    assert!(
        dfmcp_adapter::semantic_work_orders::ObservedInventoryObjective::from_selected_plan(
            &inventory,
            StepId::ZERO
        )
        .is_some()
    );
    changed = request.clone();
    changed.deadline_tick += 1;
    assert_ne!(
        original.intent_id,
        changed.compile(&evidence, &requirement, &c)?.intent_id
    );
    let legacy = request.common().compile(&evidence, &c)?;
    assert_eq!(legacy.steps.len(), 2);
    assert_ne!(legacy.intent_id, original.intent_id);
    Ok(())
}

#[test]
fn closed_existing_labor_request_cannot_supply_identity_facts_or_assignment_flags() -> Result<()> {
    let raw = serde_json::json!({"kind":"existing_labor","expected_observation":"11".repeat(32),
        "units":["43"],"labor":"MINE","recipe":"wooden_bed","amount":1,"deadline_tick":300});
    for key in [
        "enabled",
        "historical_figure_id",
        "eligible",
        "capture",
        "native_unit_id",
        "orders",
    ] {
        let mut changed = raw.clone();
        changed[key] = true.into();
        assert!(
            ExistingLaborRequest::parse(&changed.to_string()).is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("kind", serde_json::json!("portfolio")),
        ("units", serde_json::json!(["43", "43"])),
        ("units", serde_json::json!(["043"])),
        ("amount", serde_json::json!(101)),
    ] {
        let mut changed = raw.clone();
        changed[key] = value;
        assert!(ExistingLaborRequest::parse(&changed.to_string()).is_err());
    }
    assert!(ExistingLaborRequest::parse(&" ".repeat(4097)).is_err());
    Ok(())
}

#[test]
fn mismatched_selection_unqualified_identity_and_unachievable_deadlines_refuse() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let c = context(&projection.snapshot);
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let request = request()?;
    let requirement = requirement()?;
    let untrusted = PredicateEvidence::untrusted(&projection.snapshot)?;
    assert!(request.compile(&untrusted, &requirement, &c).is_err());
    let different = ExistingLaborRequirement::new("MINE".into(), vec![(EntityId::new(43), 142)])?;
    assert!(request.compile(&evidence, &different, &c).is_err());
    let mut changed = request.clone();
    changed.deadline_tick = 129;
    assert!(changed.compile(&evidence, &requirement, &c).is_err());
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(request.compile(&evidence, &requirement, &stale).is_err());
    Ok(())
}

#[test]
fn existing_labor_requires_independent_current_source_policy_without_assignment_authority()
-> Result<()> {
    use crate::semantic_workforce_runtime::policy::{ACCEPTANCE_KIND, SourceAcceptance};
    use dfmcp_adapter::live_production::ExistingLaborProjection;
    let (capsule, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.grants
        .retain(|grant| grant.capability != Capability::ConfigureLabor);
    c.grants.push(CapabilityGrant {
        capability: Capability::Observe,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    });
    let acceptance = SourceAcceptance {
        kind: ACCEPTANCE_KIND.into(),
        world_folder: "region1".into(),
        site_id: 4,
        df_version: "0.51.11".into(),
        dfhack_version: "0.51.11-r1".into(),
        citizen_bridge_version: "0.1.0".into(),
        citizen_generation: CITIZEN_GENERATION,
        workforce_generation: 7,
    };
    let evidence = PredicateEvidence::scoped(
        &projection.snapshot,
        acceptance.citizens(&capsule, &projection, &c)?,
    )?;
    let identity = LiveRoutingEvidence::citizens_v1(evidence.clone(), &projection, &capsule)?;
    let mut request = request()?;
    request.units = vec!["43".into()];
    let required = ExistingLaborRequirement::new("MINE".into(), vec![(EntityId::new(43), 142)])?;
    let original = request.compile(&evidence, &required, &c)?;
    let mut source = WorkforceFixture::default();
    source.automatic = false;
    source.details.clear();
    source.citizens[0].labors = vec![1, 0];
    source.citizens.push(CitizenFixture {
        id: 43,
        eligible: false,
        labors: vec![0, 0],
    });
    let observed =
        ExistingLaborProjection::new(&original, &identity, source.capture()?, c.anchor.cursor)?;
    assert_eq!(observed.snapshot().graph.entities.len(), 1);
    let granted = acceptance.existing_labor(&observed, &c)?;
    assert!(
        observed
            .evidence(granted.clone())?
            .establishes(&required.predicate())?
    );
    let mut unqualified = granted;
    unqualified.sources.clear();
    assert!(
        !observed
            .evidence(unqualified)?
            .establishes(&required.predicate())?
    );
    let mut wrong = acceptance.clone();
    wrong.workforce_generation += 1;
    assert!(wrong.existing_labor(&observed, &c).is_err());
    c.grants
        .retain(|grant| grant.capability != Capability::Observe);
    assert!(acceptance.existing_labor(&observed, &c).is_err());
    Ok(())
}
