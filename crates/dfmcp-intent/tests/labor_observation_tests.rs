#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId, Result,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{LaborAllocationLimits, LaborAllocator, LaborDemand};
use dfmcp_intent::labor_observation::{
    LaborPreference, LaborReadBudget, LaborReadProblemKind, ObservedLaborRoster,
};
use dfmcp_world::{
    EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactPresence, FactSource, PredicateEvidence, Value, WorldGraph, WorldSnapshot,
};

fn missing() -> DfmcpError { DfmcpError::new(ErrorCode::InternalInvariantViolation, "test fixture missing") }
fn preferences() -> Vec<LaborPreference> {
    vec![
        LaborPreference { id: EntityId::new(1), skills: BTreeMap::from([("A".to_owned(), 20), ("B".to_owned(), 10)]), stress_level: 100 },
        LaborPreference { id: EntityId::new(2), skills: BTreeMap::from([("B".to_owned(), 9)]), stress_level: 100 },
    ]
}
fn demands() -> Vec<LaborDemand> {
    ["A", "B"].into_iter().enumerate().map(|(i, name)| LaborDemand {
        labor: name.to_owned(), skill: name.to_owned(), workers: 1, minimum_skill: 1, priority: i as u8,
    }).collect()
}
fn fact(value: Value) -> Fact {
    Fact::known(value, GameTick(10), FactSource::Derived("dfmcp.lab-scenario/1".to_owned()), Digest32::ZERO)
}
fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in [EntityId::new(1), EntityId::new(2)] {
        graph.entities.insert(id, EntityRecord {
            id, generation: 1, revision: 1, kind: EntityKind::Unit, label: "worker".to_owned(),
            fields: BTreeMap::from([
                ("alive".to_owned(), fact(Value::Bool(true))),
                ("labor.A".to_owned(), fact(Value::Bool(false))),
                ("labor.B".to_owned(), fact(Value::Bool(id == EntityId::new(1)))),
            ]),
        });
    }
    WorldSnapshot::new(FortressId::new(1), GameTick(10), ObservationCursor::ORIGIN, true, graph)
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget::default(), cancellation_requested: false,
        grants: [Capability::Query, Capability::Observe, Capability::Plan].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope { fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None,
        }).collect(),
    }
}
fn inspect(snapshot: &WorldSnapshot) -> Result<ObservedLaborRoster> {
    ObservedLaborRoster::inspect(&PredicateEvidence::laboratory(snapshot)?, &preferences(), &demands(), LaborReadBudget::default())
}

#[test]
fn observed_false_is_preserved_and_the_real_compiler_seals_the_result() -> Result<()> {
    let snapshot = world();
    let original = snapshot.clone();
    let observed = inspect(&snapshot)?;
    assert_eq!(observed.anchor(), snapshot.anchor());
    assert_eq!((observed.known_enabled(), observed.known_disabled()), (1, 3));
    let profiles = observed.profiles().ok_or_else(missing)?;
    assert_eq!(profiles[0].assigned_labors, BTreeSet::from(["B".to_owned()]));
    assert!(profiles[1].assigned_labors.is_empty());
    let compiled = LaborAllocator.compile_execution(profiles, &demands(), LaborAllocationLimits::default())?;
    let plan = compiled.prepare(IntentId::new(71), "observed staffing".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot))?;
    assert_eq!(plan.steps.len(), 3);
    assert!(plan.digest_is_valid());
    assert_eq!(original, snapshot);
    Ok(())
}

#[test]
fn source_presence_future_and_type_failures_never_manufacture_false_bits() -> Result<()> {
    for fault in 0..6 {
        let mut snapshot = world();
        let fields = &mut snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or_else(missing)?.fields;
        if fault == 0 { fields.remove("labor.A"); } else {
            let record = fields.get_mut("labor.A").ok_or_else(missing)?;
            match fault {
                1 => record.source = FactSource::AgentAssertion("pretend disabled".to_owned()),
                2 => record.source_digest = Digest32::of_bytes(b"unregistered producer"),
                3 => record.observed_at = GameTick(11),
                4 => record.presence = Some(FactPresence::Omitted("not observed".to_owned())),
                _ => { record.value = Value::U64(0); record.presence = None; }
            }
        }
        snapshot.refresh_hash();
        let observed = inspect(&snapshot)?;
        assert!(observed.profiles().is_none(), "fault {fault}");
        assert!(observed.problems().iter().any(|p| p.entity_id == EntityId::new(2)
            && p.field.as_deref() == Some("labor.A") && p.kind == LaborReadProblemKind::UnprovenLabor));
    }
    Ok(())
}

#[test]
fn an_untrusted_world_and_partial_roster_never_yield_a_partial_executable_roster() -> Result<()> {
    let mut snapshot = world();
    let unknown = ObservedLaborRoster::inspect(&PredicateEvidence::untrusted(&snapshot)?, &preferences(), &demands(), LaborReadBudget::default())?;
    assert!(unknown.profiles().is_none());
    assert!(unknown.problems().iter().all(|p| p.kind == LaborReadProblemKind::UnprovenUnit));
    snapshot.graph.entities.remove(&EntityId::new(2));
    snapshot.refresh_hash();
    assert!(inspect(&snapshot)?.profiles().is_none());
    Ok(())
}

#[test]
fn dead_or_unproven_citizens_do_not_supply_worker_slots() -> Result<()> {
    for kind in [EntityKind::Unit, EntityKind::Item] {
        let mut snapshot = world();
        let entity = snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or_else(missing)?;
        entity.kind = kind;
        entity.fields.insert("alive".to_owned(), fact(Value::Bool(false)));
        snapshot.refresh_hash();
        assert!(inspect(&snapshot)?.profiles().is_none());
    }
    let mut snapshot = world();
    snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or_else(missing)?.fields.remove("alive");
    snapshot.refresh_hash();
    assert!(inspect(&snapshot)?.problems().iter().any(|p| p.kind == LaborReadProblemKind::UnprovenLiveness));
    Ok(())
}

#[test]
fn exact_live_source_grants_are_required_independently_of_canonical_integrity() -> Result<()> {
    let mut snapshot = world();
    let digest = Digest32::of_bytes(b"independently qualified capture");
    for entity in snapshot.graph.entities.values_mut() {
        for fact in entity.fields.values_mut() {
            fact.source = FactSource::DfhackField("workforce.test".to_owned());
            fact.source_digest = digest;
        }
    }
    snapshot.refresh_hash();
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.entity_kinds.insert(EntityKind::Unit, EvidenceCoverage::Observed);
    assert!(ObservedLaborRoster::inspect(&PredicateEvidence::scoped(&snapshot, policy.clone())?, &preferences(), &demands(), LaborReadBudget::default())?.profiles().is_none());
    policy.sources.insert(EvidenceSource::Observed { field: "workforce.test".to_owned(), source_digest: digest });
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    assert!(ObservedLaborRoster::inspect(&evidence, &preferences(), &demands(), LaborReadBudget::default())?.profiles().is_some());
    Ok(())
}

#[test]
fn unmentioned_labors_are_positive_only_and_preferences_remain_advisory() -> Result<()> {
    let mut snapshot = world();
    let entity = snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or_else(missing)?;
    entity.fields.insert("labor.MINING".to_owned(), fact(Value::Bool(true)));
    let mut untrusted = fact(Value::Bool(true));
    untrusted.source = FactSource::Replay;
    entity.fields.insert("labor.CARPENTRY".to_owned(), untrusted);
    snapshot.refresh_hash();
    let observed = inspect(&snapshot)?;
    let profiles = observed.profiles().ok_or_else(missing)?;
    assert!(profiles[0].assigned_labors.contains("MINING"));
    assert!(!profiles[0].assigned_labors.contains("CARPENTRY"));
    assert_eq!(profiles[0].skills, preferences()[0].skills);
    assert_eq!(profiles[0].stress_level, 100);
    Ok(())
}

#[test]
fn input_permutations_have_identical_observations_and_work_accounting() -> Result<()> {
    let snapshot = world();
    let expected = inspect(&snapshot)?;
    let mut preferences = preferences(); preferences.reverse();
    let mut demands = demands(); demands.reverse();
    let actual = ObservedLaborRoster::inspect(&PredicateEvidence::laboratory(&snapshot)?, &preferences, &demands, LaborReadBudget::default())?;
    assert_eq!(expected, actual);
    Ok(())
}

#[test]
fn authorized_entry_rejects_stale_narrow_limited_expired_and_cancelled_contexts() -> Result<()> {
    let snapshot = world();
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    for fault in 0..6 {
        let mut ctx = context(&snapshot);
        match fault {
            0 => ctx.anchor.tick = GameTick(9),
            1 => ctx.grants.retain(|g| g.capability != Capability::Query),
            2 => ctx.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            3 => ctx.grants[0].remaining_uses = Some(1),
            4 => ctx.grants[0].expires_at_tick = Some(GameTick(9)),
            _ => ctx.cancellation_requested = true,
        }
        assert!(ObservedLaborRoster::inspect_authorized(&evidence, &preferences(), &demands(), LaborReadBudget::default(), &ctx).is_err(), "fault {fault}");
    }
    assert!(ObservedLaborRoster::inspect_authorized(&evidence, &preferences(), &demands(), LaborReadBudget::default(), &context(&snapshot))?.profiles().is_some());
    Ok(())
}

#[test]
fn every_read_budget_and_ambiguous_model_is_rejected_without_partial_success() -> Result<()> {
    let snapshot = world();
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    for budget in [
        LaborReadBudget { max_entities: 1, ..LaborReadBudget::default() },
        LaborReadBudget { max_bytes: 1, ..LaborReadBudget::default() },
        LaborReadBudget { max_work: 1, ..LaborReadBudget::default() },
    ] {
        assert!(matches!(ObservedLaborRoster::inspect(&evidence, &preferences(), &demands(), budget), Err(e) if e.code == ErrorCode::BudgetExceeded));
    }
    let mut preferences = preferences(); preferences[1].id = preferences[0].id;
    assert!(matches!(ObservedLaborRoster::inspect(&evidence, &preferences, &demands(), LaborReadBudget::default()), Err(e) if e.code == ErrorCode::InvalidRequest));
    Ok(())
}
