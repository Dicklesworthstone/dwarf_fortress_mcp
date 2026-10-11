//! Exercise original production recovery with real plans, effects and storage.
use super::*;
use crate::lab_world::{ProductionPlanner, ProductionRequest, scenario_snapshot, starter};
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, GameTick, IntentId,
    OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{EffectWorkState, StaticPlanner};
use dfmcp_lab::{
    MemoryAdapter,
    durable::{DurableLabStore, DurablePlanSource, DurableStepUpdate},
};
use dfmcp_world::{Predicate, PredicateEvidence, PredicateTruth};

use super::super::{goal_continuation, lab_progress, production_source};

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-original-custody-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_game_ticks: 2_000,
            max_wall_millis: 60_000,
            max_actions: 64,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

struct Fixture {
    store: DurableLabStore,
    directory: StateDir,
    request: ProductionRequest,
    plan: PreparedPlan,
    retired: WorldSnapshot,
    order_step: u32,
}

fn fixture(label: &str) -> Result<Fixture> {
    let directory = StateDir::new(label);
    let mut sealed = scenario_snapshot("starter_fortress", FortressId::new(753), false)?;
    sealed.graph.entities.remove(&starter::STILL);
    sealed.refresh_hash();
    let request = ProductionRequest::parse(
        r#"{
        "template":"production", "quotas":[{"item":"DRINK","minimum":50}],
        "prerequisites":{"workshops":[{"building":"workshop:Still","location":[0,0,10]}]}
    }"#,
    )?
    .with_planner(ProductionPlanner::ConsumptionAwareV1);
    let (intent, _) = production_source::compile(
        &request,
        IntentId::new(753),
        &sealed,
        "recover unfinished original setup",
    )?;
    let plan = StaticPlanner::default().prepare_laboratory(&sealed, &intent, &context(&sealed))?;
    assert_eq!(plan.steps.len(), 2);
    let build = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::Build { .. }))
        .ok_or_else(|| corrupt("fixture lacks the original workshop step"))?;
    let order = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        .ok_or_else(|| corrupt("fixture lacks the original order step"))?;
    assert!(order.depends_on.contains(&build.id));
    let mut store = DurableLabStore::open(&directory.0)?;
    store.persist_head("starter_fortress", &sealed)?;
    store.persist_objective_commit(
        &sealed,
        plan.digest,
        plan.intent_id.get(),
        DurablePlanSource::Production {
            summary: "recover unfinished original setup".to_owned(),
            raw: request.canonical_json(),
        },
        SessionId::new(1),
        &[],
    )?;
    let mut adapter = MemoryAdapter::new(sealed);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(
        adapter
            .step_receipt(plan.id, order.id)
            .map(|receipt| receipt.state),
        Some(CommitState::Prepared),
    );
    store.persist_progress(
        "starter_fortress",
        adapter.snapshot(),
        &[
            DurableStepUpdate {
                plan_digest: plan.digest,
                step: build.id.get(),
                state: "dispatched".to_owned(),
            },
            DurableStepUpdate {
                plan_digest: plan.digest,
                step: order.id.get(),
                state: "not_dispatched".to_owned(),
            },
        ],
        &[],
    )?;

    // Crash before a deferred order could start. Physical construction remains
    // in the recovered world; the lost adapter handle is never reconstructed.
    adapter = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    adapter.advance_ticks(effects::BUILD_TICKS)?;
    let mut predicates = build.postconditions.clone();
    if let Some(spec) = &build.obligation {
        predicates.push(spec.terminal.clone());
    }
    assert!(
        PredicateEvidence::laboratory(adapter.snapshot())?
            .establishes(&Predicate::All(predicates).normalized())?
    );
    assert!(
        dfmcp_intent::inspect_effect_work(
            adapter.snapshot(),
            &build.action,
            &build.idempotency_key,
            true,
        )?
        .is_quiescent()
    );
    assert!(
        matches!(
            dfmcp_intent::inspect_effect_work(
                adapter.snapshot(),
                &order.action,
                &order.idempotency_key,
                true,
            )?,
            EffectWorkState::Unknown { .. }
        ),
        "missing effects alone cannot establish nondispatch"
    );
    store.persist_progress(
        "starter_fortress",
        adapter.snapshot(),
        &[DurableStepUpdate {
            plan_digest: plan.digest,
            step: build.id.get(),
            state: "verified".to_owned(),
        }],
        &[plan.digest],
    )?;
    let order_step = order.id.get();
    Ok(Fixture {
        store,
        directory,
        request,
        plan,
        retired: adapter.snapshot().clone(),
        order_step,
    })
}

fn retained(fixture: &Fixture) -> Result<DurableObjective> {
    fixture
        .store
        .objective(fixture.plan.anchor.fortress_id, fixture.plan.digest)
        .cloned()
        .ok_or_else(|| corrupt("fixture lost the original goal"))
}

fn recovered(snapshot: &WorldSnapshot) -> Result<(MemoryAdapter, RecoveredGoalLineage)> {
    let adapter = MemoryAdapter::recovered(snapshot.clone())?;
    let lineage =
        RecoveredGoalLineage::verify(snapshot, snapshot.anchor(), adapter.snapshot().anchor())?;
    Ok((adapter, lineage))
}

#[test]
fn original_setup_can_continue_after_retirement_compaction_and_two_reopens() -> Result<()> {
    let f = fixture("continue-twice")?;
    let immutable = f.plan.clone();
    let mut store = f.store;
    let mut current = f.retired.clone();
    for _ in 0..2 {
        store.compact()?;
        drop(store);
        store = DurableLabStore::open(&f.directory.0)?;
        assert!(store.commit(current.fortress_id, f.plan.digest).is_none());
        let retained = store
            .objective(current.fortress_id, f.plan.digest)
            .cloned()
            .ok_or_else(|| corrupt("retired original goal disappeared"))?;
        assert_eq!(retained.retired_never_dispatched.len(), 1);
        let anchor = retained.retired_never_dispatched[&f.order_step];
        let archived = store.load_snapshot(anchor.state_hash)?;
        let (adapter, lineage) = recovered(&current)?;
        let proof =
            RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &archived, &lineage)?;
        let quiet = f.plan.steps.iter().try_fold(true, |quiet, step| {
            if proof.covers(&f.plan, step, adapter.snapshot()) {
                return Ok(quiet);
            }
            dfmcp_intent::inspect_effect_work(
                adapter.snapshot(),
                &step.action,
                &step.idempotency_key,
                true,
            )
            .map(|work| quiet && work.is_quiescent())
        })?;
        assert!(quiet);
        goal_continuation::validate_goal_evidence_in_active_lineage(
            &f.plan,
            lineage.active_anchor(),
            adapter.snapshot(),
            &context(adapter.snapshot()),
            quiet,
        )?;
        current = adapter.snapshot().clone();
        store.persist_head("starter_fortress", &current)?;
    }

    let continuation =
        goal_continuation::ProductionContinuation::new(f.plan.digest, f.plan.digest, f.request)?;
    let (intent, _) =
        continuation.compile(IntentId::new(754), &current, "finish original drinks")?;
    let next =
        StaticPlanner::default().prepare_laboratory(&current, &intent, &context(&current))?;
    assert_eq!(next.terminal_condition, f.plan.terminal_condition);
    assert_eq!(
        next.steps.len(),
        1,
        "the completed original workshop is reused"
    );
    let original_order = f
        .plan
        .steps
        .iter()
        .find(|step| step.id.get() == f.order_step)
        .ok_or_else(|| corrupt("original order missing"))?;
    assert_ne!(
        next.steps[0].idempotency_key,
        original_order.idempotency_key
    );
    let mut adapter = MemoryAdapter::new(current);
    let prepared = adapter.prepare(&next, &context(adapter.snapshot()))?;
    let committed = adapter.commit(&next, &prepared, &context(adapter.snapshot()))?;
    let actions: Vec<_> = committed
        .actions
        .iter()
        .map(|receipt| receipt.action_id)
        .collect();
    let mut progress =
        lab_progress::ForegroundProgress::new(&adapter, 200, &context(adapter.snapshot()))?;
    loop {
        let ctx = context(adapter.snapshot());
        let Some(slice) = progress.step(&mut adapter, &actions, &ctx, None)? else {
            break;
        };
        if slice.finished {
            break;
        }
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&f.plan.terminal_condition)?,
        PredicateTruth::True
    );
    assert!(effect_identity_absent(original_order, adapter.snapshot()));
    assert_eq!(f.plan, immutable);
    Ok(())
}

#[test]
fn missing_and_unrelated_retirement_rows_never_prove_original_absence() -> Result<()> {
    let f = fixture("no-absence-inference")?;
    let mut retained = retained(&f)?;
    let anchor = retained.retired_never_dispatched[&f.order_step];
    let archived = f.store.load_snapshot(anchor.state_hash)?;
    let (_, lineage) = recovered(&f.retired)?;
    retained.retired_never_dispatched.clear();
    assert!(
        RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &archived, &lineage)
            .is_err()
    );
    retained.retired_never_dispatched.insert(255, anchor);
    assert!(RetiredNondispatchProof::verify(&f.plan, &retained, 255, &archived, &lineage).is_err());
    retained
        .retired_never_dispatched
        .insert(f.order_step, anchor);
    retained.intent_id += 1;
    assert!(
        RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &archived, &lineage)
            .is_err()
    );
    Ok(())
}

#[test]
fn altered_archives_and_future_or_forked_custody_are_rejected() -> Result<()> {
    let f = fixture("archive-binding")?;
    let retained = retained(&f)?;
    let anchor = retained.retired_never_dispatched[&f.order_step];
    let archived = f.store.load_snapshot(anchor.state_hash)?;
    let (_, lineage) = recovered(&f.retired)?;
    let mut invalid_hash = archived.clone();
    invalid_hash.paused = !invalid_hash.paused;
    assert!(
        RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &invalid_hash, &lineage)
            .is_err()
    );
    for (index, mut other) in [archived.clone(), f.retired.clone(), archived.clone()]
        .into_iter()
        .enumerate()
    {
        match index {
            0 => other.tick = GameTick(f.retired.tick.0 + 1),
            1 => other.cursor = f.plan.anchor.cursor,
            _ => other.tick = GameTick(0),
        }
        other.refresh_hash();
        let mut changed = retained.clone();
        changed
            .retired_never_dispatched
            .insert(f.order_step, other.anchor());
        assert!(
            RetiredNondispatchProof::verify(&f.plan, &changed, f.order_step, &other, &lineage)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn current_effect_identity_contradicts_historical_nondispatch() -> Result<()> {
    let f = fixture("current-contradiction")?;
    let retained = retained(&f)?;
    let anchor = retained.retired_never_dispatched[&f.order_step];
    let archived = f.store.load_snapshot(anchor.state_hash)?;
    let (adapter, lineage) = recovered(&f.retired)?;
    let proof =
        RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &archived, &lineage)?;
    let step = f
        .plan
        .steps
        .iter()
        .find(|step| step.id.get() == f.order_step)
        .ok_or_else(|| corrupt("original order missing"))?;
    assert!(proof.covers(&f.plan, step, adapter.snapshot()));
    let mut contradicted = adapter.snapshot().clone();
    let mut entity = contradicted
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .cloned()
        .ok_or_else(|| corrupt("fixture stock ledger missing"))?;
    entity.id = effects::created_entity_id(&step.idempotency_key, 0);
    contradicted.graph.entities.insert(entity.id, entity);
    contradicted.cursor = contradicted
        .cursor
        .checked_next()
        .ok_or_else(|| corrupt("fixture cursor exhausted"))?;
    contradicted.refresh_hash();
    assert!(!proof.covers(&f.plan, step, &contradicted));
    let mut bad_archive = retained;
    bad_archive
        .retired_never_dispatched
        .insert(f.order_step, contradicted.anchor());
    let (_, later) = recovered(&contradicted)?;
    assert!(
        RetiredNondispatchProof::verify(&f.plan, &bad_archive, f.order_step, &contradicted, &later)
            .is_err()
    );
    Ok(())
}

#[test]
fn abandoned_historical_custody_cannot_prove_quiet_in_a_rewound_world() -> Result<()> {
    let f = fixture("abandoned-rewind")?;
    let mut retained = retained(&f)?;
    let anchor = retained.retired_never_dispatched[&f.order_step];
    let archived = f.store.load_snapshot(anchor.state_hash)?;
    let mut rewound = archived.clone();
    rewound.tick = GameTick(0);
    rewound.cursor = rewound
        .cursor
        .checked_reset_epoch()
        .ok_or_else(|| corrupt("fixture epoch exhausted"))?;
    rewound.refresh_hash();
    retained.restore_abandoned_anchor = Some(rewound.anchor());
    let (adapter, lineage) = recovered(&rewound)?;
    let proof =
        RetiredNondispatchProof::verify(&f.plan, &retained, f.order_step, &archived, &lineage)?;
    let step = f
        .plan
        .steps
        .iter()
        .find(|step| step.id.get() == f.order_step)
        .ok_or_else(|| corrupt("original order missing"))?;
    assert!(!proof.covers(&f.plan, step, adapter.snapshot()));
    Ok(())
}
