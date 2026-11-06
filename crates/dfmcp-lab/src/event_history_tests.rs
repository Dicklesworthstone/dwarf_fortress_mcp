//! Exercise the actual adapter's normal effect, publication and recovery routes.

use super::*;
use crate::{MemoryAdapter, durable::DurableLabStore};
use dfmcp_adapter::{CancelMode, GameAdapter, ObservationRequest, Projection};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{Predicate, WorldGraph};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const UNIT: EntityId = EntityId::new(11);
const LEDGER: EntityId = EntityId::new(91);

fn scenario_fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(1),
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn entity(id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        label: kind.as_str().to_owned(),
        kind,
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), scenario_fact(value)))
            .collect(),
    }
}

fn world(enabled: bool, paused: bool) -> Result<MemoryAdapter> {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        UNIT,
        entity(
            UNIT,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.BREW", Value::Bool(true)),
            ],
        ),
    );
    let workshop = EntityId::new(51);
    graph.entities.insert(
        workshop,
        entity(
            workshop,
            EntityKind::Building,
            vec![
                ("building_kind", Value::Text("workshop:Still".to_owned())),
                (
                    effects::CONSTRUCTION_STAGE_FIELD,
                    Value::Text(effects::STAGE_COMPLETE.to_owned()),
                ),
            ],
        ),
    );
    graph.entities.insert(
        LEDGER,
        entity(
            LEDGER,
            EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            vec![
                (effects::STOCK_DRINK_FIELD, Value::U64(100)),
                (effects::STOCK_FOOD_FIELD, Value::U64(100)),
                (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
            ],
        ),
    );
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(77),
        GameTick(1),
        ObservationCursor::ORIGIN,
        paused,
        graph,
    );
    if enabled {
        enable(&mut snapshot)?;
    }
    Ok(MemoryAdapter::new(snapshot))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::Restore, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn plan(adapter: &MemoryAdapter, action: Action, id: u128) -> Result<PreparedPlan> {
    let intent_id = IntentId::new(id);
    let snapshot = adapter.snapshot();
    let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), StepId::new(0), &action);
    let postconditions = effects::default_postconditions(&action, &key, snapshot.fortress_id);
    let obligation =
        effects::default_obligation(&action, &key, snapshot.fortress_id, snapshot.tick)?;
    let intent = Intent {
        id: intent_id,
        anchor: snapshot.anchor(),
        summary: "exercise canonical event production".to_owned(),
        terminal_condition: Predicate::All(postconditions.clone()).normalized(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions,
            compensation: None,
            obligation,
            depends_on: Vec::new(),
        }],
    };
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(adapter))
}

fn dispatch(adapter: &mut MemoryAdapter, action: Action, id: u128) -> Result<(ActionId, EntityId)> {
    let plan = plan(adapter, action, id)?;
    let entity = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let prepared = adapter.prepare(&plan, &context(adapter))?;
    let committed = adapter.commit(&plan, &prepared, &context(adapter))?;
    Ok((committed.actions[0].action_id, entity))
}

fn order(amount: u32) -> Action {
    Action::CreateWorkOrder {
        name: "retained brewing history".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount,
        conditions: Vec::new(),
    }
}

fn history(adapter: &MemoryAdapter) -> Result<HistoryCoverage> {
    coverage(adapter.snapshot())?.ok_or_else(|| invalid("test world has no event history"))
}

fn last_notice(adapter: &MemoryAdapter, transition: &str) -> Result<WorldEvent> {
    adapter
        .snapshot()
        .graph
        .events
        .values()
        .filter(|event| event.fields.get("transition") == Some(&Value::Text(transition.to_owned())))
        .max_by_key(|event| value_number(&event.fields, "ordinal").ok())
        .cloned()
        .ok_or_else(|| invalid("expected publication event missing"))
}

#[test]
fn old_worlds_remain_uninstrumented_and_exactly_canonical() -> Result<()> {
    let mut adapter = world(false, true)?;
    let old = adapter.snapshot().clone();
    assert_eq!(coverage(&old)?, None);
    adapter.advance_ticks(2)?;
    let expected = WorldSnapshot::new(
        old.fortress_id,
        GameTick(3),
        old.cursor.next(),
        true,
        old.graph,
    );
    assert_eq!(adapter.snapshot(), &expected);
    assert!(adapter.snapshot().graph.events.is_empty());
    let checkpoint = adapter.checkpoint("old world", &context(&adapter))?;
    let restored = adapter.restore(checkpoint.checkpoint_id, &context(&adapter))?;
    assert_eq!(
        restored.restored_anchor.state_hash,
        adapter.snapshot().state_hash
    );
    assert_eq!(coverage(adapter.snapshot())?, None);
    let recovered = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    assert_eq!(coverage(recovered.snapshot())?, None);
    assert!(recovered.snapshot().graph.events.is_empty());
    Ok(())
}

#[test]
fn initialize_seals_a_distinct_model_and_refuses_foreign_history() -> Result<()> {
    let original = world(false, true)?.snapshot().clone();
    let mut enabled = original.clone();
    enable(&mut enabled)?;
    assert_ne!(enabled.state_hash, original.state_hash);
    assert!(enabled.hash_is_valid());
    assert_eq!(coverage(&enabled)?.map(|state| state.total_events), Some(1));
    assert_eq!(
        WorldSnapshot::from_canonical_bytes(&enabled.canonical_bytes())?,
        enabled
    );
    let prior = enabled.clone();
    assert!(enable(&mut enabled).is_err());
    assert_eq!(enabled, prior);
    let mut foreign = original;
    foreign.graph.events.insert(
        EventId::new(9),
        WorldEvent {
            id: EventId::new(9),
            tick: foreign.tick,
            kind: WorldEventKind::Announcement,
            subject: None,
            summary: "pre-existing source".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    foreign.refresh_hash();
    let prior = foreign.clone();
    assert!(enable(&mut foreign).is_err());
    assert_eq!(foreign, prior);
    Ok(())
}

#[test]
fn real_prepare_commit_observe_poll_and_replay_emit_only_actual_publication() -> Result<()> {
    let mut adapter = world(true, true)?;
    let plan = plan(
        &adapter,
        Action::SetLabor {
            units: vec![UNIT],
            labor: "BREW".to_owned(),
            enabled: false,
        },
        1,
    )?;
    let before = adapter.snapshot().clone();
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    assert_eq!(adapter.snapshot(), &before);
    let committed = adapter.commit(&plan, &prepared, &context(&adapter))?;
    let after = adapter.snapshot().clone();
    let event = after
        .graph
        .events
        .values()
        .find(|event| event.kind == WorldEventKind::UnitChanged)
        .ok_or_else(|| invalid("normal labor commit did not publish a unit event"))?;
    assert_eq!(event.subject, Some(UNIT));
    assert_eq!(
        event.fields["predecessor_anchor"],
        anchor_value(before.anchor())
    );
    assert_eq!(
        event.fields["publication_sequence"],
        Value::U64(after.cursor.sequence)
    );
    assert_eq!(event.fields["change"], Value::Text("updated".to_owned()));
    let again = adapter.commit(&plan, &prepared, &context(&adapter))?;
    assert_eq!(again, committed);
    adapter.poll_action(committed.actions[0].action_id, &context(&adapter))?;
    adapter.observe(
        &ObservationRequest {
            since: Some(after.cursor),
            projection: Projection::Summary,
            interest: Default::default(),
            max_entities: 20,
            max_bytes: 10_000,
            max_output_tokens: 1_000,
            continuation: None,
        },
        &context(&adapter),
    )?;
    adapter.checkpoint("read-only source boundary", &context(&adapter))?;
    assert_eq!(adapter.snapshot(), &after);
    assert_eq!(history(&adapter)?.total_events, 3);
    Ok(())
}

#[test]
fn normal_work_order_completion_publishes_job_and_inventory_changes() -> Result<()> {
    let mut adapter = world(true, false)?;
    let (action, order) = dispatch(&mut adapter, order(1), 2)?;
    let dispatched = adapter.snapshot().anchor();
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    let after = adapter.snapshot().clone();
    let completed = after
        .graph
        .events
        .values()
        .find(|event| {
            event.kind == WorldEventKind::JobChanged
                && event.subject == Some(order)
                && event.fields.get("predecessor_anchor") == Some(&anchor_value(dispatched))
        })
        .ok_or_else(|| invalid("normal tick advancement did not publish work-order change"))?;
    assert_eq!(completed.tick, after.tick);
    let Value::List(changes) = &completed.fields["changes"] else {
        return Err(invalid("missing typed changes"));
    };
    assert!(changes.iter().any(|change| {
        matches!(change, Value::Object(fields) if fields.get("field") == Some(&Value::Text(effects::STATUS_FIELD.to_owned()))
            && matches!(fields.get("after"), Some(Value::Object(after)) if after.get("value") == Some(&Value::Text(effects::STATUS_COMPLETE.to_owned()))))
    }));
    assert!(
        after
            .graph
            .events
            .values()
            .any(|event| event.subject == Some(LEDGER)
                && event.fields.get("predecessor_anchor") == Some(&anchor_value(dispatched)))
    );
    assert_eq!(
        adapter.poll_action(action, &context(&adapter))?.state,
        CommitState::Verified
    );
    assert_eq!(adapter.snapshot(), &after);
    assert!(after.hash_is_valid());
    Ok(())
}

#[test]
fn cancellation_publishes_stopped_work_once_and_preserves_immutable_events() -> Result<()> {
    let mut adapter = world(true, false)?;
    let (action, order) = dispatch(&mut adapter, order(100), 3)?;
    let previous = adapter.snapshot().graph.events.clone();
    adapter.request_cancel(action, CancelMode::StopFutureSteps, &context(&adapter))?;
    assert_eq!(adapter.snapshot().graph.events, previous);
    adapter.finalize_cancel(action, &context(&adapter))?;
    assert!(adapter.action_work_state(action)?.is_quiescent());
    assert!(
        adapter
            .snapshot()
            .graph
            .events
            .values()
            .any(|event| event.subject == Some(order)
                && event.fields.get("change") == Some(&Value::Text("updated".to_owned())))
    );
    let notice = last_notice(&adapter, "cancel_work")?;
    assert_eq!(notice.fields["entity_changes_recorded"], Value::U64(1));
    for (id, event) in previous {
        assert_eq!(adapter.snapshot().graph.events.get(&id), Some(&event));
    }
    let after = adapter.snapshot().clone();
    adapter.finalize_cancel(action, &context(&adapter))?;
    assert_eq!(adapter.snapshot(), &after);
    Ok(())
}

#[test]
fn retained_history_is_bounded_by_ordinal_and_restoration_drops_abandoned_future() -> Result<()> {
    let mut adapter = world(true, true)?;
    adapter.advance_ticks(1)?;
    let checkpoint = adapter.checkpoint("retained branch", &context(&adapter))?;
    let checkpoint_world = adapter.snapshot().clone();
    for _ in 0..MAX_RETAINED_EVENTS + 2 {
        adapter.advance_ticks(1)?;
    }
    let future = adapter.snapshot().clone();
    let state = history(&adapter)?;
    assert_eq!(state.retained_events, MAX_RETAINED_EVENTS);
    assert_eq!(
        state.total_events,
        state.evicted_events + MAX_RETAINED_EVENTS as u64
    );
    assert!(state.evicted_events > 0);
    assert_eq!(state.oldest_ordinal, Some(state.evicted_events + 1));
    assert_eq!(state.newest_ordinal, Some(state.total_events));
    let restored = adapter.restore(checkpoint.checkpoint_id, &context(&adapter))?;
    assert_eq!(restored.content_digest, checkpoint_world.state_hash);
    assert_eq!(adapter.snapshot().tick, checkpoint_world.tick);
    assert_eq!(adapter.snapshot().cursor.epoch, future.cursor.epoch + 1);
    assert_eq!(history(&adapter)?.evicted_events, 0);
    assert_eq!(history(&adapter)?.total_events, 3);
    let reset = last_notice(&adapter, "restore")?;
    assert_eq!(
        reset.fields["predecessor_anchor"],
        anchor_value(future.anchor())
    );
    assert_eq!(
        reset.fields["retained_history_anchor"],
        anchor_value(checkpoint_world.anchor())
    );
    for (id, event) in &checkpoint_world.graph.events {
        assert_eq!(adapter.snapshot().graph.events.get(id), Some(event));
    }
    for (id, event) in future.graph.events {
        if event.tick > checkpoint_world.tick {
            assert!(!adapter.snapshot().graph.events.contains_key(&id));
        }
    }
    Ok(())
}

#[test]
fn typed_presence_never_exposes_unavailable_backing_values_or_invents_absence() -> Result<()> {
    let mut before = world(true, true)?.snapshot().clone();
    let source = before
        .graph
        .entities
        .get_mut(&UNIT)
        .ok_or_else(|| invalid("missing test unit"))?;
    let mut redacted = scenario_fact(Value::Text("private backing".to_owned()));
    redacted.presence = Some(FactPresence::Redacted("policy".to_owned()));
    source.fields.insert("secret".to_owned(), redacted);
    source
        .fields
        .insert("empty".to_owned(), scenario_fact(Value::Null));
    before.refresh_hash();
    let mut next = before.clone();
    next.cursor = next.cursor.next();
    let target = next
        .graph
        .entities
        .get_mut(&UNIT)
        .ok_or_else(|| invalid("missing test unit"))?;
    target.revision += 1;
    target.fields.insert(
        "secret".to_owned(),
        Fact::with_presence(
            FactPresence::Absent,
            GameTick(1),
            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
            Digest32::ZERO,
        ),
    );
    target.fields.remove("empty");
    record_publication(&before, &mut next, Publication::AdvanceTicks { amount: 1 })?;
    next.refresh_hash();
    let event = next
        .graph
        .events
        .values()
        .find(|event| event.subject == Some(UNIT))
        .ok_or_else(|| invalid("missing presence event"))?;
    let Value::List(changes) = &event.fields["changes"] else {
        return Err(invalid("missing changes"));
    };
    let mut seen = BTreeMap::new();
    for change in changes {
        let Value::Object(change) = change else {
            return Err(invalid("invalid change"));
        };
        let Value::Text(name) = &change["field"] else {
            return Err(invalid("invalid field"));
        };
        seen.insert(name.as_str(), change);
    }
    let Value::Object(redacted_before) = &seen["secret"]["before"] else {
        return Err(invalid("invalid state"));
    };
    assert_eq!(
        redacted_before["presence"],
        Value::Text("redacted".to_owned())
    );
    assert!(!redacted_before.contains_key("value"));
    let Value::Object(absent_after) = &seen["secret"]["after"] else {
        return Err(invalid("invalid state"));
    };
    assert_eq!(absent_after["presence"], Value::Text("absent".to_owned()));
    let Value::Object(null_before) = &seen["empty"]["before"] else {
        return Err(invalid("invalid state"));
    };
    assert_eq!(null_before["presence"], Value::Text("known".to_owned()));
    assert_eq!(null_before["value"], Value::Null);
    let Value::Object(missing_after) = &seen["empty"]["after"] else {
        return Err(invalid("invalid state"));
    };
    assert_eq!(missing_after["presence"], Value::Text("unknown".to_owned()));
    let bytes = next.canonical_bytes();
    assert!(
        !bytes
            .windows("private backing".len())
            .any(|window| window == b"private backing")
    );
    Ok(())
}

#[test]
fn bounds_report_dropped_entity_field_and_value_details_without_partial_records() -> Result<()> {
    let before = world(true, true)?.snapshot().clone();
    let mut next = before.clone();
    next.cursor = next.cursor.next();
    for index in 0..MAX_ENTITY_EVENTS_PER_PUBLICATION + 5 {
        let id = EntityId::new(1000 + index as u64);
        let mut record = entity(id, EntityKind::Item, Vec::new());
        for field in 0..MAX_CHANGED_FIELDS + 3 {
            record.fields.insert(
                format!("field_{field:02}"),
                scenario_fact(Value::Text("x".repeat(600))),
            );
        }
        next.graph.entities.insert(id, record);
    }
    record_publication(&before, &mut next, Publication::AdvanceTicks { amount: 1 })?;
    next.refresh_hash();
    let state = coverage(&next)?.ok_or_else(|| invalid("missing history"))?;
    assert_eq!(state.omitted_entity_changes, 5);
    assert_eq!(
        state.omitted_fields,
        (MAX_ENTITY_EVENTS_PER_PUBLICATION * 3) as u64
    );
    assert_eq!(
        state.omitted_values,
        (MAX_ENTITY_EVENTS_PER_PUBLICATION * MAX_CHANGED_FIELDS) as u64
    );
    assert_eq!(
        state.retained_events,
        1 + MAX_ENTITY_EVENTS_PER_PUBLICATION + 1
    );
    assert!(
        next.graph
            .events
            .values()
            .all(|event| event_id(event).ok() == Some(event.id))
    );
    Ok(())
}

#[test]
fn changed_generation_and_different_forks_have_distinct_event_identity() -> Result<()> {
    let before = world(true, true)?.snapshot().clone();
    let mut a = before.clone();
    a.cursor = a.cursor.next();
    let entity = a
        .graph
        .entities
        .get_mut(&UNIT)
        .ok_or_else(|| invalid("missing unit"))?;
    entity.generation = 2;
    entity.revision = 1;
    let mut b = a.clone();
    b.graph
        .entities
        .get_mut(&UNIT)
        .ok_or_else(|| invalid("missing unit"))?
        .generation = 3;
    record_publication(&before, &mut a, Publication::AdvanceTicks { amount: 1 })?;
    record_publication(&before, &mut b, Publication::AdvanceTicks { amount: 1 })?;
    let first = a
        .graph
        .events
        .values()
        .find(|event| event.subject == Some(UNIT))
        .ok_or_else(|| invalid("missing generation event"))?;
    let second = b
        .graph
        .events
        .values()
        .find(|event| event.subject == Some(UNIT))
        .ok_or_else(|| invalid("missing generation event"))?;
    assert_eq!(first.fields["generation_before"], Value::U64(1));
    assert_eq!(first.fields["generation_after"], Value::U64(2));
    assert_ne!(first.id, second.id);
    let first_notice = a
        .graph
        .events
        .values()
        .find(|event| {
            event.fields.get("transition") == Some(&Value::Text("advance_ticks".to_owned()))
        })
        .ok_or_else(|| invalid("missing first notice"))?;
    let second_notice = b
        .graph
        .events
        .values()
        .find(|event| {
            event.fields.get("transition") == Some(&Value::Text("advance_ticks".to_owned()))
        })
        .ok_or_else(|| invalid("missing second notice"))?;
    assert_ne!(first_notice.id, second_notice.id);
    Ok(())
}

#[test]
fn corrupt_history_and_publication_exhaustion_leave_the_entire_effect_unpublished() -> Result<()> {
    let mut snapshot = world(true, true)?.snapshot().clone();
    let id = *snapshot
        .graph
        .events
        .keys()
        .next()
        .ok_or_else(|| invalid("missing event"))?;
    snapshot
        .graph
        .events
        .get_mut(&id)
        .ok_or_else(|| invalid("missing event"))?
        .summary
        .push_str("tampered");
    snapshot.refresh_hash();
    let mut adapter = MemoryAdapter::new(snapshot.clone());
    assert!(adapter.advance_ticks(1).is_err());
    assert_eq!(adapter.snapshot(), &snapshot);
    let mut snapshot = world(true, true)?.snapshot().clone();
    snapshot
        .graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(snapshot.fortress_id))
        .ok_or_else(|| invalid("missing settings"))?
        .revision = u64::MAX;
    snapshot.refresh_hash();
    let mut adapter = MemoryAdapter::new(snapshot.clone());
    assert!(adapter.advance_ticks(1).is_err());
    assert_eq!(adapter.snapshot(), &snapshot);
    let plan = plan(&adapter, Action::Pause { paused: false }, 8)?;
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    assert!(
        adapter
            .commit(&plan, &prepared, &context(&adapter))
            .is_err()
    );
    assert_eq!(adapter.snapshot(), &snapshot);
    Ok(())
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
struct StateDirectory(PathBuf);
impl StateDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "dfmcp-event-history-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for StateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn durable_objects_retain_exact_events_and_recovery_continues_in_a_new_epoch() -> Result<()> {
    let directory = StateDirectory::new();
    let mut adapter = world(true, false)?;
    let (_, order) = dispatch(&mut adapter, order(2), 9)?;
    adapter.advance_ticks(10)?;
    let checkpoint = adapter.checkpoint("before restart", &context(&adapter))?;
    let saved = adapter.snapshot().clone();
    {
        let mut store = DurableLabStore::open(&directory.0)?;
        store.persist_head("starter_fortress_history", &saved)?;
        store.persist_checkpoint(checkpoint.checkpoint_id, "before restart", &saved)?;
    }
    let store = DurableLabStore::open(&directory.0)?;
    let head = store
        .head(saved.fortress_id)
        .ok_or_else(|| invalid("missing durable head"))?;
    let loaded = store.load_snapshot(head.anchor.state_hash)?;
    assert_eq!(loaded, saved);
    let mut recovered = MemoryAdapter::recovered(loaded)?;
    assert_eq!(recovered.snapshot().cursor.epoch, saved.cursor.epoch + 1);
    for (id, event) in &saved.graph.events {
        assert_eq!(recovered.snapshot().graph.events.get(id), Some(event));
    }
    assert_eq!(
        history(&recovered)?.total_events,
        history(&adapter)?.total_events + 1
    );
    assert_eq!(
        last_notice(&recovered, "recovery")?.fields["retained_history_anchor"],
        anchor_value(saved.anchor())
    );
    recovered.advance_ticks(90)?;
    assert!(
        recovered
            .snapshot()
            .graph
            .events
            .values()
            .any(|event| event.subject == Some(order)
                && event.fields.get("publication_epoch")
                    == Some(&Value::U64(recovered.snapshot().cursor.epoch)))
    );
    recovered.adopt_checkpoint(checkpoint.checkpoint_id, saved.clone())?;
    recovered.restore(checkpoint.checkpoint_id, &context(&recovered))?;
    assert_eq!(recovered.snapshot().tick, saved.tick);
    assert_eq!(
        last_notice(&recovered, "restore")?.fields["retained_history_anchor"],
        anchor_value(saved.anchor())
    );
    assert_eq!(
        WorldSnapshot::from_canonical_bytes(&recovered.snapshot().canonical_bytes())?,
        *recovered.snapshot()
    );
    Ok(())
}

#[test]
fn invalid_restored_model_cannot_replace_current_world_or_invalidate_its_handles() -> Result<()> {
    let mut adapter = world(true, false)?;
    let mut checkpoint = adapter.snapshot().clone();
    let settings = checkpoint
        .graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(
            checkpoint.fortress_id,
        ))
        .ok_or_else(|| invalid("missing settings"))?;
    settings
        .fields
        .insert(TOTAL.to_owned(), fact(Value::U64(99), checkpoint.tick));
    checkpoint.refresh_hash();
    adapter.adopt_checkpoint(CheckpointId::new(99), checkpoint)?;
    let (action, _) = dispatch(&mut adapter, order(2), 10)?;
    let before = adapter.snapshot().clone();
    assert!(
        adapter
            .restore(CheckpointId::new(99), &context(&adapter))
            .is_err()
    );
    assert_eq!(adapter.snapshot(), &before);
    assert!(matches!(
        adapter.action_work_state(action)?,
        dfmcp_intent::EffectWorkState::Active { .. }
    ));
    Ok(())
}
