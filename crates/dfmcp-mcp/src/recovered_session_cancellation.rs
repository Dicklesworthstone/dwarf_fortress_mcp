//! One bounded cancellation over current owned originals and durable recovery.
//!
//! The union is fully admitted before the emergency pause or first physical
//! stop. Each stop publishes its own frontier; a failed append ends the request.
//! Known foreign live originals are excluded, while unattributed world work
//! prevents a session-wide quiescence claim.
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{ActionId, SessionId};

use super::super as server;
use super::*;

struct LiveOriginal {
    selected: server::plan_cancellation::Selection,
    order: (u64, u64, Digest32),
    pending: bool,
}

struct SessionSelection {
    live: Vec<LiveOriginal>,
    recovered: Vec<Selection>,
    recovered_digests: BTreeSet<Digest32>,
    requested: Option<Vec<Digest32>>,
    scope: String,
    identities: Value,
    digest: Digest32,
    live_actions: usize,
    recovered_steps: usize,
    originals: usize,
    source_bytes: u64,
    source_records: u64,
    final_census_allowance: u64,
    final_census_records_allowance: u64,
    census_prefix_bytes: u64,
    census_prefix_records: u64,
    started: Instant,
}

fn charge(session: &LabSession, started: Instant, bytes: u64, records: u64) -> Result<()> {
    check_time(session, started)?;
    if bytes > session.budget.max_bytes.min(MAX_SOURCE_BYTES)
        || records > u64::from(session.budget.max_entities)
    {
        return Err(exhausted(
            "complete session cancellation exceeds its shared source allowance; drain oldest_open_plan or one exact original",
        ));
    }
    Ok(())
}

fn live_effects(session: &LabSession, original: &LiveOriginal, mode: CancelMode) -> Result<usize> {
    let mut effects = 0usize;
    for id in &original.selected.actions {
        let receipt = session
            .adapter
            .action_receipt(*id)
            .ok_or_else(|| refused("a session original lost its action receipt"))?;
        let work = session.adapter.action_work_state(*id)?;
        if matches!(work, EffectWorkState::Unknown { .. }) {
            return Err(refused(
                "selected session work has unresolved physical identity",
            ));
        }
        effects += usize::from(matches!(work, EffectWorkState::Active { .. }));
        if mode == CancelMode::CompensateReversible
            && !receipt.state.is_terminal()
            && work != EffectWorkState::NeverDispatched
            && session
                .adapter
                .action_step(*id)
                .is_some_and(|step| step.compensation.is_some())
        {
            effects += 1;
        }
    }
    Ok(effects)
}

/// Conservative count without first cloning variable-length action scopes.
/// Assignment scopes may deduplicate IDs, so their upper bound is sufficient.
pub(crate) fn live_scope_lease_count(action: &Action) -> u64 {
    match action {
        Action::Pause { .. }
        | Action::CreateWorkOrder { .. }
        | Action::SetStandingOrder { .. }
        | Action::Extension { .. } => 0,
        Action::DesignateDig { .. }
        | Action::Build { .. }
        | Action::HarvestResource { .. }
        | Action::ConfigureStockpile { .. } => 1,
        Action::PlaceFurniture { .. } => 2,
        Action::SetLabor { units, .. } => units.len() as u64,
        Action::AssignSquad { units, .. } | Action::SetBurrowMembership { units, .. } => {
            (units.len() as u64).saturating_add(1)
        }
    }
}

/// Check a selected live effect against current reservations and the retained
/// physical work of other owners. The private manager never publishes a lease
/// or consumes a real identifier when complete session admission is refused.
pub(crate) fn admit_live_scope(
    session: &LabSession,
    leases: &mut dfmcp_core::lease::LeaseManager,
    action: &Action,
    started: Instant,
) -> Result<()> {
    check_time(session, started)?;
    let scope = action.scope();
    let now = session.adapter.snapshot().tick;
    if let Some(area) = scope.map_area {
        leases.acquire_spatial_lease(session.session_id, area, true, now, 1)?;
    }
    for entity in &scope.entity_ids {
        leases.acquire_entity_lease(session.session_id, *entity, true, now, 1)?;
    }
    // TTL expiry does not release another member's unresolved physical work.
    for (id, (owner, _)) in &session.leases.by_action {
        check_time(session, started)?;
        if *owner == session.session_id || server::action_fully_drained(&session.adapter, *id) {
            continue;
        }
        let held = session
            .adapter
            .action_step(*id)
            .ok_or_else(|| refused("foreign physical ownership lost its original action"))?
            .action
            .scope();
        if scope
            .entity_ids
            .iter()
            .any(|entity| held.entity_ids.contains(entity))
            || scope
                .map_area
                .zip(held.map_area)
                .is_some_and(|(area, held)| dfmcp_core::lease::cuboids_intersect(&area, &held))
        {
            return Err(refused(
                "another member retains unresolved physical work overlapping selected live cleanup",
            ));
        }
    }
    // Derived lease entries can be absent while immutable original custody
    // still identifies another current member. The same positive ownership
    // used to exclude that work from the final certificate also fences stops.
    for id in session.adapter.retained_action_ids() {
        check_time(session, started)?;
        if session.leases.by_action.contains_key(&id)
            || server::action_fully_drained(&session.adapter, id)
            || !known_foreign(session, id)
        {
            continue;
        }
        let held = session
            .adapter
            .action_step(id)
            .ok_or_else(|| refused("retained foreign work lost its original action"))?
            .action
            .scope();
        if scope
            .entity_ids
            .iter()
            .any(|entity| held.entity_ids.contains(entity))
            || scope
                .map_area
                .zip(held.map_area)
                .is_some_and(|(area, held)| dfmcp_core::lease::cuboids_intersect(&area, &held))
        {
            return Err(refused(
                "foreign original physical work overlaps selected live cleanup despite missing derived leases",
            ));
        }
    }
    Ok(())
}

fn prepare(
    session: &LabSession,
    mode: CancelMode,
    started: Instant,
    requested: Option<&[Digest32]>,
) -> Result<SessionSelection> {
    session.budget.validate()?;
    let context = remaining_context(session, started)?;
    server::authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    if session.budget.max_output_tokens < 1_500 || session.budget.max_bytes < 6_000 {
        return Err(exhausted(
            "session cancellation requires at least 1500 output tokens and 6000 bytes for the complete outcome",
        ));
    }
    let source =
        server::query_budget::admit_observation_sources(&[session.adapter.snapshot()], &context)?;
    let mut bytes = source["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| refused("session cancellation source size is unavailable"))?;
    let mut records = source["records"]
        .as_u64()
        .ok_or_else(|| refused("session cancellation source count is unavailable"))?;
    records = records
        .saturating_add(session.objectives.len() as u64)
        .saturating_add(session.carried.len() as u64)
        .saturating_add(session.commit_receipts.len() as u64)
        .saturating_add(session.open_actions.len() as u64)
        .saturating_add(session.last_plan_actions.len() as u64)
        .saturating_add(session.leases.by_action.len() as u64);
    let lease_records = session.leases.manager.active_lease_count() as u64;
    records = records.saturating_add(lease_records);
    bytes = bytes.saturating_add(lease_records.saturating_mul(192));
    bytes = bytes.saturating_add(records.saturating_mul(64));
    charge(session, started, bytes, records)?;
    for _ in session.adapter.retained_action_ids() {
        records = records.saturating_add(1);
        bytes = bytes.saturating_add(16);
        charge(session, started, bytes, records)?;
    }

    // Current-session commit custody is complete even if a foreground action
    // list was lost. Alias receipts are validated, then deduplicated by actual
    // sealed original rather than being dispatched or counted twice.
    let mut live = BTreeMap::<Digest32, LiveOriginal>::new();
    let mut owned_actions = BTreeSet::new();
    for (digest, raw) in &session.commit_receipts {
        if requested.is_some_and(|requested| !requested.iter().any(|item| item.to_hex() == *digest))
        {
            continue;
        }
        bytes = bytes.saturating_add(raw.len() as u64);
        charge(session, started, bytes, records)?;
        let requested_digest = server::plan_cancellation::parse_scope(&format!("plan:{digest}"))?;
        let selected = server::plan_cancellation::select_for_census(session, requested_digest)?;
        if let Some(previous) = live.get(&selected.actual) {
            if requested.is_some() {
                // Session discovery deduplicates aliases. An explicit set must
                // name each actual original exactly once.
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "different selected digests resolve to the same original commit",
                ));
            }
            if previous.selected.actions != selected.actions
                || previous.selected.plan_id != selected.plan_id
            {
                return Err(refused(
                    "original aliases disagree about complete session action custody",
                ));
            }
            continue;
        }
        records = records.saturating_add(selected.actions.len() as u64);
        bytes = bytes.saturating_add((selected.actions.len() as u64).saturating_mul(32));
        charge(session, started, bytes, records)?;
        for id in &selected.actions {
            if !owned_actions.insert(*id) {
                return Err(refused("two session originals claim the same live action"));
            }
        }
        let first = selected
            .actions
            .first()
            .ok_or_else(|| refused("a session original has no actions"))?;
        let original = session
            .adapter
            .action_plan_receipt(*first)
            .ok_or_else(|| refused("the session original lost its immutable adapter commit"))?;
        let pending = selected
            .actions
            .iter()
            .any(|id| !server::action_fully_drained(&session.adapter, *id));
        let order = (
            original.observed_anchor.cursor.epoch,
            original.observed_anchor.cursor.sequence,
            selected.actual,
        );
        live.insert(
            selected.actual,
            LiveOriginal {
                selected,
                order,
                pending,
            },
        );
    }
    for id in session
        .open_actions
        .iter()
        .chain(&session.last_plan_actions)
        .copied()
        .chain(session.last_action)
        .chain(
            session
                .leases
                .by_action
                .iter()
                .filter_map(|(id, (owner, _))| (*owner == session.session_id).then_some(*id)),
        )
    {
        if requested.is_none() && !owned_actions.contains(&id) {
            return Err(refused(
                "owned foreground work has lost complete original commit custody",
            ));
        }
    }
    let mut live = live.into_values().collect::<Vec<_>>();
    live.sort_by_key(|original| original.order);

    let recovered_digests = if let Some(requested) = requested {
        requested
            .iter()
            .copied()
            .filter(|digest| !session.commit_receipts.contains_key(&digest.to_hex()))
            .collect::<Vec<_>>()
    } else {
        let discovery = scopes::discover_recovered(session, started, true)?;
        bytes = bytes.saturating_add(discovery.inspected_bytes);
        records = records.saturating_add(discovery.inspected_records);
        discovery.digests
    };
    charge(session, started, bytes, records)?;
    if mode == CancelMode::CompensateReversible && !recovered_digests.is_empty() {
        return Err(refused(
            "a mixed session cannot compensate recovered originals; select stop_future_steps or emergency_pause_and_drain",
        ));
    }
    let effect_mode = if mode == CancelMode::EmergencyPauseAndDrain {
        CancelMode::StopFutureSteps
    } else {
        mode
    };
    let mut effects = usize::from(
        mode == CancelMode::EmergencyPauseAndDrain && !session.adapter.snapshot().paused,
    );
    let mut live_actions = 0usize;
    let mut identities = Vec::new();
    // Reserve every lease record the private admission copy can add before
    // cloning it or acquiring the first temporary forward/inverse reservation.
    let mut temporary_leases = 0u64;
    for original in live.iter().filter(|original| original.pending) {
        for id in &original.selected.actions {
            check_time(session, started)?;
            if server::action_fully_drained(&session.adapter, *id) {
                continue;
            }
            let step = session
                .adapter
                .action_step(*id)
                .ok_or_else(|| refused("selected live cleanup lost its original action"))?;
            temporary_leases =
                temporary_leases.saturating_add(live_scope_lease_count(&step.action));
            if effect_mode == CancelMode::CompensateReversible
                && session
                    .adapter
                    .action_receipt(*id)
                    .is_some_and(|receipt| !receipt.state.is_terminal())
                && session.adapter.action_work_state(*id)? != EffectWorkState::NeverDispatched
                && let Some(inverse) = &step.compensation
            {
                temporary_leases = temporary_leases.saturating_add(live_scope_lease_count(inverse));
            }
        }
    }
    records = records.saturating_add(temporary_leases);
    bytes = bytes.saturating_add(temporary_leases.saturating_mul(192));
    charge(session, started, bytes, records)?;
    let mut cleanup_leases = session.leases.manager.clone();
    for original in live
        .iter()
        .filter(|original| original.pending || requested.is_some())
    {
        server::plan_cancellation::authorize(session, &original.selected, effect_mode)?;
        effects = effects.saturating_add(live_effects(session, original, effect_mode)?);
        for id in &original.selected.actions {
            check_time(session, started)?;
            if server::action_fully_drained(&session.adapter, *id) {
                continue;
            }
            if session
                .leases
                .by_action
                .get(id)
                .is_some_and(|(owner, _)| *owner != session.session_id)
            {
                return Err(refused(
                    "selected live action has contradictory foreign ownership",
                ));
            }
            let step = session
                .adapter
                .action_step(*id)
                .ok_or_else(|| refused("selected live cleanup lost its original action"))?;
            admit_live_scope(session, &mut cleanup_leases, &step.action, started)?;
            if effect_mode == CancelMode::CompensateReversible
                && session
                    .adapter
                    .action_receipt(*id)
                    .is_some_and(|receipt| !receipt.state.is_terminal())
                && session.adapter.action_work_state(*id)? != EffectWorkState::NeverDispatched
                && let Some(inverse) = &step.compensation
            {
                admit_live_scope(session, &mut cleanup_leases, inverse, started)?;
            }
        }
        live_actions = live_actions.saturating_add(original.selected.actions.len());
        let mut identity = json!({"plan_digest":original.selected.actual.to_hex(),
            "selection_origin":"current_process",
            "actions":original.selected.actions.iter().map(ToString::to_string).collect::<Vec<_>>()});
        if requested.is_some() {
            identity["requested_plan_digest"] = json!(original.selected.requested.to_hex());
            identity["plan_id"] = json!(original.selected.plan_id);
        }
        identities.push(identity);
        charge(session, started, bytes, records)?;
    }
    let mut recovered = Vec::new();
    let mut recovered_steps = 0usize;
    for digest in &recovered_digests {
        let digest = *digest;
        let selected = select_at(
            session,
            digest,
            CancelMode::StopFutureSteps,
            started,
            bytes,
            records,
        )?;
        bytes = selected.inspected_bytes;
        records = selected.inspected_records;
        effects = effects.saturating_add(selected.physical_effects);
        recovered_steps = recovered_steps.saturating_add(selected.steps.len());
        let mut identity = json!({"plan_digest":digest.to_hex(),"selection_origin":"durable_recovery",
            "original_owner_session_id":selected.original_owner,
            "plan_object_digest":selected.archive_digest.to_hex(),
            "custody_digest":selected.custody["digest"]});
        if requested.is_some() {
            identity["requested_plan_digest"] = json!(digest.to_hex());
            identity["plan_id"] = json!(selected.plan.id.to_string());
            identity["steps"] = json!(
                selected
                    .steps
                    .iter()
                    .map(|row| row.step.get())
                    .collect::<Vec<_>>()
            );
        }
        identities.push(identity);
        recovered.push(selected);
        charge(session, started, bytes, records)?;
    }
    if live_actions.saturating_add(recovered_steps) > session.budget.max_actions as usize
        || effects > session.budget.max_actions as usize
    {
        return Err(exhausted(
            "complete original steps, physical stops, compensation and one emergency pause exceed the aggregate session action allowance; use oldest_open_plan",
        ));
    }
    if mode == CancelMode::EmergencyPauseAndDrain {
        server::authorize_entry(&context, Capability::ControlClock, RiskTier::Reversible)?;
    }
    // Every admitted recovered seal predates current-process dispatch. Order
    // recovered originals by that authenticated cursor, never request order.
    recovered.sort_by_key(|original| {
        (
            original.plan.anchor.cursor.epoch,
            original.plan.anchor.cursor.sequence,
            original.plan.digest,
        )
    });
    let originals = identities.len();
    let scope = requested.map_or_else(
        || "session".to_owned(),
        |requested| {
            format!(
                "plans:{}",
                requested
                    .iter()
                    .map(Digest32::to_hex)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        },
    );
    let identities = if requested.is_some() {
        identities.sort_by(|left, right| {
            left["requested_plan_digest"]
                .as_str()
                .cmp(&right["requested_plan_digest"].as_str())
        });
        json!({"schema":"dfmcp-selected-originals-cancellation-selection/2",
            "session_id":session.session_id.to_string(),"originals":identities,
            "dispatch_handles_restored":false})
    } else {
        json!({"schema":"dfmcp-session-cancellation-selection/1",
        "session_id":session.session_id.to_string(),"scope":"session",
        "anchor":server::anchor_json(&session.adapter.snapshot().anchor()),
        "mode":match mode {
            CancelMode::StopFutureSteps => "stop_future_steps",
            CancelMode::CompensateReversible => "compensate_reversible",
            CancelMode::EmergencyPauseAndDrain => "emergency_pause_and_drain",
        },
        "originals":identities,"foreign_live_work_included":false,
        "dispatch_handles_restored":false})
    };
    if let Some(requested) = requested {
        let output_bytes = 8_192u64
            .saturating_add((identities.to_string().len() as u64).saturating_mul(2))
            .saturating_add((scope.len() as u64).saturating_mul(2))
            .saturating_add((requested.len() as u64).saturating_mul(68));
        if output_bytes > session.budget.max_bytes
            || output_bytes.div_ceil(4) > u64::from(session.budget.max_output_tokens)
        {
            return Err(exhausted(
                "selected recovered cancellation cannot reserve every original identity and its complete drain certificate",
            ));
        }
        bytes = bytes.saturating_add(output_bytes);
    }
    bytes = bytes.saturating_add(identities.to_string().len() as u64);
    charge(session, started, bytes, records)?;
    let digest = Digest32::of_bytes(identities.to_string().as_bytes());
    let mut selected = SessionSelection {
        live,
        recovered,
        recovered_digests: recovered_digests.into_iter().collect(),
        requested: requested.map(<[Digest32]>::to_vec),
        scope,
        identities,
        digest,
        live_actions,
        recovered_steps,
        originals,
        source_bytes: bytes,
        source_records: records,
        final_census_allowance: u64::MAX,
        final_census_records_allowance: u64::MAX,
        census_prefix_bytes: bytes,
        census_prefix_records: records,
        started,
    };
    // Admit a complete custody/physical/durable census before effects, then
    // reserve another complete pass plus bounded publication growth. Neither
    // pass gets a fresh source budget or a fresh wall-time allowance.
    let initial = final_census(session, &selected)?;
    let row_count = initial.value["live"]
        .as_array()
        .map_or(0, Vec::len)
        .saturating_add(initial.value["recovered"].as_array().map_or(0, Vec::len));
    let mut growth_bytes = (row_count as u64).saturating_mul(8 * 1_024);
    let mut growth_records = 0u64;
    selected.census_prefix_bytes = selected
        .census_prefix_bytes
        .saturating_add(initial.inspected_bytes);
    selected.census_prefix_records = selected
        .census_prefix_records
        .saturating_add(initial.inspected_records);
    // Immediate inverse actions can enlarge existing facts or introduce the
    // fortress settings entity. Bound their complete shallow typed payload and
    // every touched entity; temporal or extension inverses were refused above.
    if mode == CancelMode::CompensateReversible {
        for original in selected.live.iter().filter(|original| original.pending) {
            for id in &original.selected.actions {
                check_time(session, started)?;
                let receipt = session
                    .adapter
                    .action_receipt(*id)
                    .ok_or_else(|| refused("compensation lost its original receipt"))?;
                if receipt.state.is_terminal()
                    || session.adapter.action_work_state(*id)? == EffectWorkState::NeverDispatched
                {
                    continue;
                }
                let step = session
                    .adapter
                    .action_step(*id)
                    .ok_or_else(|| refused("compensation lost its original step"))?;
                let Some(inverse) = &step.compensation else {
                    continue;
                };
                let (payload, touched, new_records) = match inverse {
                    Action::Pause { .. } => (2u64, 1u64, 0u64),
                    Action::SetLabor { units, labor, .. } => (
                        (units.len() as u64)
                            .saturating_mul(8)
                            .saturating_add(labor.len() as u64)
                            .saturating_add(32),
                        units.len() as u64,
                        0,
                    ),
                    Action::ConfigureStockpile { accepts, .. } => (
                        accepts.iter().fold(64u64, |sum, token| {
                            sum.saturating_add(token.len() as u64).saturating_add(8)
                        }),
                        1,
                        0,
                    ),
                    Action::AssignSquad { units, .. }
                    | Action::SetBurrowMembership { units, .. } => (
                        (units.len() as u64).saturating_mul(8).saturating_add(64),
                        (units.len() as u64).saturating_add(1),
                        0,
                    ),
                    Action::SetStandingOrder { key, value } => (
                        (key.len() as u64)
                            .saturating_add(value.len() as u64)
                            .saturating_add(32),
                        1,
                        1,
                    ),
                    _ => {
                        return Err(refused(
                            "compensation has no admitted immediate source-growth bound",
                        ));
                    }
                };
                selected.census_prefix_bytes = selected.census_prefix_bytes.saturating_add(payload);
                selected.census_prefix_records =
                    selected.census_prefix_records.saturating_add(touched);
                growth_bytes = growth_bytes.saturating_add(
                    touched
                        .max(1)
                        .saturating_mul(payload.saturating_mul(4).saturating_add(1_024)),
                );
                growth_records = growth_records.saturating_add(new_records);
                charge(
                    session,
                    started,
                    selected.census_prefix_bytes,
                    selected.census_prefix_records,
                )?;
            }
        }
    }
    if effects != 0 && dfmcp_lab::event_history::coverage(session.adapter.snapshot())?.is_some() {
        // History producer limits are 32 entity events plus one notice per
        // publication and 128 retained events. Each event has <=16KiB fields;
        // 20KiB also covers its fixed identity, summary and canonical headers.
        let events = effects
            .saturating_mul(
                dfmcp_lab::event_history::MAX_ENTITY_EVENTS_PER_PUBLICATION.saturating_add(1),
            )
            .min(dfmcp_lab::event_history::MAX_RETAINED_EVENTS) as u64;
        growth_bytes = growth_bytes
            .saturating_add(events.saturating_mul(20 * 1_024))
            .saturating_add(4_096);
        growth_records = growth_records.saturating_add(events);
    }
    selected.final_census_allowance = initial.inspected_bytes.saturating_add(growth_bytes);
    selected.final_census_records_allowance =
        initial.inspected_records.saturating_add(growth_records);
    selected.source_bytes = selected
        .census_prefix_bytes
        .saturating_add(selected.final_census_allowance);
    selected.source_records = selected
        .census_prefix_records
        .saturating_add(selected.final_census_records_allowance);
    charge(
        session,
        started,
        selected.source_bytes,
        selected.source_records,
    )?;
    Ok(selected)
}

fn error_value(error: DfmcpError) -> Value {
    json!({"operation":"fortress.cancel","code":error.code.as_str(),"message":error.message,
        "details":error.details,"retryable":error.retryable,
        "recovery":{"class":"reconciliation_required","next_tool":"fortress.observe"}})
}

fn emergency_pause(session: &mut LabSession, started: Instant) -> (Option<Value>, Option<Value>) {
    let before = session.adapter.snapshot().anchor();
    let changed = !session.adapter.snapshot().paused;
    let result = server::drain_context_at(session, started)
        .and_then(|context| session.adapter.pause_for_emergency_drain(&context));
    match result {
        Err(error) => (None, Some(error_value(error))),
        Ok(evidence) => {
            server::reset_emergency_unpause_consent(session, CancelMode::EmergencyPauseAndDrain);
            server::persist_durable_head(session);
            let receipt = json!({"requested":true,"changed":changed,"paused":true,
                "before_anchor":server::anchor_json(&before),
                "observed_anchor":server::anchor_json(&evidence.anchor),
                "game_ticks_advanced":0,"action_handles_created":false,
                "physical_work_drained":false,"authority":"current_observe_and_control_clock",
                "evidence_digest":evidence.digest.to_hex(),"durable":session.durable_scenario.is_some(),
                "publication_confirmed":session.durability_fault.is_none()});
            let failure = session.durability_fault.as_ref().map(|fault| {
                error_value(DfmcpError::new(
                    ErrorCode::AdapterUnavailable,
                    format!("emergency pause is current but durable publication failed: {fault}"),
                ))
            });
            if failure.is_none() {
                server::remember_version(session);
            }
            (Some(receipt), failure)
        }
    }
}

#[derive(Default)]
struct Progress {
    drained: u64,
    cancelled: u64,
    compensated: u64,
    terminal_work_stopped: u64,
}

fn record_result(raw: String, progress: &mut Progress, results: &mut Vec<Value>) -> Option<Value> {
    let value: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => {
            return Some(error_value(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "original cancellation returned an unreadable outcome; further effects stopped",
            )));
        }
    };
    let counters = &value["drain_progress"];
    progress.drained = progress
        .drained
        .saturating_add(counters["drained"].as_u64().unwrap_or(0));
    progress.cancelled = progress
        .cancelled
        .saturating_add(counters["cancelled"].as_u64().unwrap_or(0));
    progress.compensated = progress
        .compensated
        .saturating_add(counters["compensated"].as_u64().unwrap_or(0));
    progress.terminal_work_stopped = progress
        .terminal_work_stopped
        .saturating_add(counters["terminal_work_stopped"].as_u64().unwrap_or(0));
    let failure = (value["ok"] != true || counters["quiescent"] != true).then(|| {
        if value["error"].is_object() {
            let mut error = value["error"].clone();
            error["selected_plan_digest"] = value["plan_digest"].clone();
            error
        } else {
            error_value(refused("one original did not prove final physical quiescence; further session stops were withheld"))
        }
    });
    results.push(value);
    failure
}

struct Census {
    value: Value,
    remaining_nonterminal: usize,
    remaining_work: usize,
    remaining_carried: usize,
    untracked: Value,
    untracked_quiet: bool,
    remaining_durable_admissions: usize,
    durable_head_confirmed: bool,
    inspected_bytes: u64,
    inspected_records: u64,
}

/// A known foreign current-process original is excluded from this session's
/// certificate. An adapter record without that positive ownership remains
/// unattributed; a missing foreground list must not hide owned physical work.
fn foreign_original(session: &LabSession, digest: Digest32) -> Option<(SessionId, &PreparedPlan)> {
    if session.shared.is_none()
        || session.shared_members <= 1
        || session.commit_receipts.contains_key(&digest.to_hex())
    {
        return None;
    }
    let mut matching = session
        .objectives
        .iter()
        .filter(|goal| goal.plan_digest == digest);
    let goal = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let source = goal.resource_goal(session);
    let plan = source.plan?;
    let anchor = source.sealed_anchor?;
    if source.owner_session_id == session.session_id
        || source.owned_by_current_session
        || source.verification_error.is_some()
        || source.withdrawal.is_some()
        || source.restore_abandoned_anchor.is_some()
        || source.abandonment_publication_pending
        || anchor.cursor.epoch != session.adapter.snapshot().cursor.epoch
        || anchor.fortress_id != session.fortress_id
        || plan.anchor != anchor
        || plan.digest != digest
    {
        return None;
    }
    Some((source.owner_session_id, plan))
}

fn known_foreign(session: &LabSession, id: ActionId) -> bool {
    let Some(commit) = session.adapter.action_plan_receipt(id) else {
        return false;
    };
    let Some((owner, plan)) = foreign_original(session, commit.plan_digest) else {
        return false;
    };
    // Leases and immutable original custody must agree. A foreign lease alone
    // never conceals a missing or contradictory current-session owner.
    plan.id == commit.plan_id
        && session
            .leases
            .by_action
            .get(&id)
            .is_none_or(|(lease_owner, _)| *lease_owner == owner)
}

fn final_census(session: &LabSession, selected: &SessionSelection) -> Result<Census> {
    let context = remaining_context(session, selected.started)?;
    let source =
        server::query_budget::admit_observation_sources(&[session.adapter.snapshot()], &context)?;
    let mut inspected_bytes = source["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| refused("session census source size is unavailable"))?;
    let mut inspected_records = source["records"]
        .as_u64()
        .ok_or_else(|| refused("session census source count is unavailable"))?;
    charge(
        session,
        selected.started,
        selected.census_prefix_bytes.saturating_add(inspected_bytes),
        selected
            .census_prefix_records
            .saturating_add(inspected_records),
    )?;
    let mut nonterminal = 0usize;
    let mut work = 0usize;
    let mut live_rows = Vec::new();
    let mut live_ids = BTreeSet::new();
    for original in &selected.live {
        for id in &original.selected.actions {
            inspected_records = inspected_records.saturating_add(1);
            charge(
                session,
                selected.started,
                selected.census_prefix_bytes.saturating_add(inspected_bytes),
                selected
                    .census_prefix_records
                    .saturating_add(inspected_records),
            )?;
            let receipt = session
                .adapter
                .action_receipt(*id)
                .ok_or_else(|| refused("final original action receipt missing"))?;
            let step = session
                .adapter
                .action_step(*id)
                .ok_or_else(|| refused("final original action definition missing"))?;
            let physical = session.adapter.action_work_state(*id)?;
            nonterminal += usize::from(!receipt.state.is_terminal());
            work += usize::from(!physical.is_quiescent());
            live_ids.insert(*id);
            live_rows.push(json!({"plan_digest":original.selected.actual.to_hex(),"action_id":id.to_string(),
                "step":receipt.step_id.get(),"idempotency_key":step.idempotency_key,
                "proof_state":format!("{:?}",receipt.state),"proof_anchor":server::anchor_json(&receipt.observed_anchor),
                "proof_receipt_digest":receipt.adapter_receipt_digest.to_hex(),
                "physical_work":server::effect_work_json(&physical)}));
        }
    }
    let mut recovered_rows = Vec::new();
    let mut carried = 0usize;
    let digests = if selected.requested.is_some() {
        selected.recovered_digests.clone()
    } else {
        session
            .carried
            .iter()
            .map(|row| row.plan_digest)
            .collect::<BTreeSet<_>>()
    };
    for digest in digests {
        check_time(session, selected.started)?;
        let goal = original(session, digest)?;
        let source = goal.resource_goal(session);
        let plan = source
            .plan
            .ok_or_else(|| refused("final recovered source is unavailable"))?;
        if source.withdrawal.is_none()
            && source.restore_abandoned_anchor.is_none()
            && !source.abandonment_publication_pending
        {
            let custody = goal.selected_wait_recovery(session)?;
            inspected_bytes = inspected_bytes.saturating_add(custody.bytes()? as u64);
            charge(
                session,
                selected.started,
                selected.census_prefix_bytes.saturating_add(inspected_bytes),
                selected
                    .census_prefix_records
                    .saturating_add(inspected_records),
            )?;
            for row in custody.drain_census(session, plan)? {
                inspected_records = inspected_records.saturating_add(1);
                charge(
                    session,
                    selected.started,
                    selected.census_prefix_bytes.saturating_add(inspected_bytes),
                    selected
                        .census_prefix_records
                        .saturating_add(inspected_records),
                )?;
                let pending = row.state == "dispatched";
                nonterminal += usize::from(pending);
                work += usize::from(!row.work.is_quiescent());
                carried += usize::from(pending || !row.work.is_quiescent());
                recovered_rows.push(json!({"plan_digest":digest.to_hex(),"step":row.step.get(),
                    "custody_digest":row.custody_digest.to_hex(),"recorded_state":row.recorded_state,
                    "recorded_anchor":server::anchor_json(&row.recorded_anchor),
                    "proof_state":row.state,"proof_anchor":row.proof_anchor.as_ref().map(server::anchor_json),
                    "physical_work":server::effect_work_json(&row.work)}));
            }
        } else {
            for row in session
                .carried
                .iter()
                .filter(|row| row.plan_digest == digest)
            {
                inspected_records = inspected_records.saturating_add(1);
                charge(
                    session,
                    selected.started,
                    selected.census_prefix_bytes.saturating_add(inspected_bytes),
                    selected
                        .census_prefix_records
                        .saturating_add(inspected_records),
                )?;
                let id = row
                    .step
                    .ok_or_else(|| refused("archived recovered step identity is unavailable"))?;
                let step = plan
                    .steps
                    .iter()
                    .find(|step| step.id == id)
                    .ok_or_else(|| refused("archived recovered step has no original"))?;
                let undispatched = row.state == "not_dispatched";
                if undispatched
                    && !server::goal_custody::effect_identity_absent(
                        step,
                        session.adapter.snapshot(),
                    )
                {
                    return Err(refused(
                        "archived nondispatch contradicts current physical work",
                    ));
                }
                let physical = dfmcp_intent::inspect_effect_work(
                    session.adapter.snapshot(),
                    &step.action,
                    &step.idempotency_key,
                    !undispatched,
                )?;
                let terminal = matches!(
                    row.state.as_str(),
                    "verified"
                        | "failed"
                        | "cancelled"
                        | "compensated"
                        | "not_dispatched"
                        | "abandoned"
                );
                nonterminal += usize::from(!terminal);
                work += usize::from(!physical.is_quiescent());
                carried += usize::from(!terminal || !physical.is_quiescent());
                recovered_rows.push(json!({"plan_digest":digest.to_hex(),"step":id.get(),
                    "proof_state":row.state,"proof_anchor":row.proof_anchor.as_ref().map(server::anchor_json),
                    "archived_pursuit":true,"physical_work":server::effect_work_json(&physical)}));
            }
        }
        check_time(session, selected.started)?;
    }
    let mut untracked = server::untracked_work_json(session);
    let mut unattributed = 0usize;
    for id in session.adapter.retained_action_ids() {
        inspected_bytes = inspected_bytes.saturating_add(16);
        inspected_records = inspected_records.saturating_add(1);
        charge(
            session,
            selected.started,
            selected.census_prefix_bytes.saturating_add(inspected_bytes),
            selected
                .census_prefix_records
                .saturating_add(inspected_records),
        )?;
        if !live_ids.contains(&id)
            && !server::action_fully_drained(&session.adapter, id)
            && !known_foreign(session, id)
        {
            unattributed += 1;
        }
    }
    untracked["unattributed_live_actions"] = json!(unattributed);
    let untracked_quiet = untracked["quiescent"] == true && unattributed == 0;
    untracked["quiescent"] = json!(untracked_quiet);
    let (remaining_durable_admissions, durable_head_confirmed) =
        if session.durable_scenario.is_some() {
            server::with_durable_store(|store| {
                let frontier = store
                    .head(session.fortress_id)
                    .is_some_and(|head| head.anchor == session.adapter.snapshot().anchor());
                let mut unfinished = 0usize;
                for commit in store.commits(session.fortress_id) {
                    inspected_bytes = inspected_bytes.saturating_add(128);
                    inspected_records = inspected_records.saturating_add(1);
                    charge(
                        session,
                        selected.started,
                        selected.census_prefix_bytes.saturating_add(inspected_bytes),
                        selected
                            .census_prefix_records
                            .saturating_add(inspected_records),
                    )?;
                    if selected.requested.is_some()
                        && !selected
                            .live
                            .iter()
                            .any(|live| live.selected.actual == commit.plan_digest)
                        && !selected.recovered_digests.contains(&commit.plan_digest)
                    {
                        // Unselected durable originals keep their own admissions;
                        // they cannot prevent or be covered by this exact proof.
                        continue;
                    }
                    // A foreign current-process owner retains its own admission.
                    // Durable and unique in-memory original ownership must agree.
                    // Missing, recovered or contradictory ownership stays unresolved.
                    let foreign = !selected
                        .live
                        .iter()
                        .any(|live| live.selected.actual == commit.plan_digest)
                        && foreign_original(session, commit.plan_digest).is_some_and(
                            |(owner, plan)| {
                                store
                                    .objective(session.fortress_id, commit.plan_digest)
                                    .is_some_and(|goal| {
                                        goal.owner_session_id == owner
                                            && goal.sealed_state_hash == plan.anchor.state_hash
                                            && commit.sealed_state_hash == goal.sealed_state_hash
                                    })
                            },
                        );
                    if !foreign {
                        unfinished += 1;
                    }
                }
                Ok((unfinished, frontier))
            })?
        } else {
            (0, true)
        };
    check_time(session, selected.started)?;
    let mut value = json!({"schema":"dfmcp-session-cancellation-census/1",
        "live":live_rows,"recovered":recovered_rows,"untracked":untracked,
        "remaining_durable_admissions":remaining_durable_admissions,
        "durable_head_confirmed":durable_head_confirmed,
        "foreign_live_work_included":false,
        "anchor":server::anchor_json(&session.adapter.snapshot().anchor())});
    if selected.requested.is_some() {
        value["schema"] = json!("dfmcp-selected-originals-cancellation-census/1");
        value["scope"] = json!(selected.scope);
        value["unselected_plans_included"] = json!(false);
        value["untracked_work_included"] = json!(false);
    }
    inspected_bytes = inspected_bytes.saturating_add(value.to_string().len() as u64);
    charge(
        session,
        selected.started,
        selected.census_prefix_bytes.saturating_add(inspected_bytes),
        selected
            .census_prefix_records
            .saturating_add(inspected_records),
    )?;
    if inspected_bytes > selected.final_census_allowance
        || inspected_records > selected.final_census_records_allowance
    {
        return Err(exhausted(
            "final session census exceeded its pre-admitted complete proof allowance",
        ));
    }
    Ok(Census {
        value,
        remaining_nonterminal: nonterminal,
        remaining_work: work,
        remaining_carried: carried,
        untracked,
        untracked_quiet,
        remaining_durable_admissions,
        durable_head_confirmed,
        inspected_bytes,
        inspected_records,
    })
}

pub(crate) fn cancel_session(session: &mut LabSession, mode: CancelMode) -> String {
    cancel_scope(session, mode, None)
}

pub(crate) fn cancel_set(
    session: &mut LabSession,
    mode: CancelMode,
    requested: Vec<Digest32>,
) -> String {
    cancel_scope(session, mode, Some(requested))
}

fn cancel_scope(
    session: &mut LabSession,
    mode: CancelMode,
    requested: Option<Vec<Digest32>>,
) -> String {
    let started = Instant::now();
    let mut selected = match prepare(session, mode, started, requested.as_deref()) {
        Ok(selected) => selected,
        Err(error) => {
            let Some(requested) = requested else {
                return server::drain_refusal_payload(session, "session", &[], &error, None);
            };
            let scope = format!(
                "plans:{}",
                requested
                    .iter()
                    .map(Digest32::to_hex)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let mut refused: Value = serde_json::from_str(&server::drain_refusal_payload(
                session,
                &scope,
                &[],
                &error,
                None,
            ))
            .unwrap_or_else(|_| json!({"ok":false}));
            refused["selection_scope"] = json!("exact_original_set");
            refused["requested_plan_digests"] =
                json!(requested.iter().map(Digest32::to_hex).collect::<Vec<_>>());
            refused["selection_resolved"] = json!(false);
            refused["selected_plans"] = Value::Null;
            refused["selected_plans_digest"] = Value::Null;
            refused["dispatch_handles_restored"] = json!(false);
            refused["descendant_plans_included"] = json!(false);
            refused["unselected_plans_included"] = json!(false);
            refused["drain_progress"]["original_plans_total"] = json!(requested.len());
            for field in ["actions_total", "remaining_nonterminal", "remaining_work"] {
                refused["drain_progress"][field] = Value::Null;
            }
            refused["error"]["effect_may_have_occurred"] = json!(false);
            return refused.to_string();
        }
    };
    let (emergency, mut failure) = if mode == CancelMode::EmergencyPauseAndDrain {
        emergency_pause(session, started)
    } else {
        (None, None)
    };
    let effect_mode = if mode == CancelMode::EmergencyPauseAndDrain {
        CancelMode::StopFutureSteps
    } else {
        mode
    };
    let mut progress = Progress::default();
    let mut results = Vec::new();
    // Newer original pursuits drain before their earlier prerequisites. Every
    // original already drains its steps in reverse dependency order.
    for original in selected
        .live
        .iter()
        .rev()
        .filter(|original| original.pending)
    {
        if failure.is_some() {
            break;
        }
        let mut result: Value = match serde_json::from_str(&server::drain_actions_at(
            session,
            effect_mode,
            &format!("plan:{}", original.selected.actual),
            original.selected.actions.clone(),
            started,
        )) {
            Ok(result) => result,
            Err(_) => {
                failure = Some(error_value(refused(
                    "live cancellation outcome was unreadable",
                )));
                break;
            }
        };
        result["plan_digest"] = json!(original.selected.actual.to_hex());
        result["selection_origin"] = json!("current_process");
        failure = record_result(result.to_string(), &mut progress, &mut results);
    }
    for original in std::mem::take(&mut selected.recovered).into_iter().rev() {
        if failure.is_some() {
            break;
        }
        let digest = original.plan.digest;
        let result = cancel_selected(
            session,
            CancelMode::StopFutureSteps,
            &format!("plan:{digest}"),
            original,
        );
        failure = record_result(result, &mut progress, &mut results);
    }
    let census = match final_census(session, &selected) {
        Ok(census) => Some(census),
        Err(error) => {
            if failure.is_none() {
                failure = Some(error_value(error));
            }
            None
        }
    };
    if failure.is_none() {
        failure = check_time(session, started).err().map(error_value);
    }
    let publication = session.durability_fault.is_none()
        && census
            .as_ref()
            .is_some_and(|census| census.durable_head_confirmed);
    let quiet = failure.is_none()
        && publication
        && census.as_ref().is_some_and(|census| {
            census.remaining_nonterminal == 0
                && census.remaining_work == 0
                && census.remaining_carried == 0
                && (selected.requested.is_some() || census.untracked_quiet)
                && census.remaining_durable_admissions == 0
        });
    let anchor = server::anchor_json(&session.adapter.snapshot().anchor());
    let certificate = if quiet {
        census.as_ref().map(|census| {
            let census_digest = Digest32::of_bytes(census.value.to_string().as_bytes()).to_hex();
            let mut canonical = json!({"schema":"dfmcp-session-originals-drain-certificate/1",
                "scope":selected.scope,"session_id":session.session_id.to_string(),
                "selected_plans_digest":selected.digest.to_hex(),"census_digest":census_digest,
                "anchor":anchor,"publication_confirmed":true,"foreign_live_work_included":false});
            if selected.requested.is_some() {
                canonical["schema"] = json!("dfmcp-selected-originals-drain-certificate/2");
                canonical["selected_plans"] = selected.identities.clone();
                canonical["descendant_plans_included"] = json!(false);
                canonical["unselected_plans_included"] = json!(false);
                canonical["untracked_work_included"] = json!(false);
            }
            let mut certificate = canonical.clone();
            certificate["digest"] = json!(Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex());
            certificate["statement"] = json!(if selected.requested.is_some() {
                "every explicitly selected original disposition is final and its exact physical work is currently quiet; unselected originals, descendants and untracked work are outside this proof"
            } else {
                "every owned original disposition is final and its exact physical work is currently quiet; known foreign live originals are excluded"
            });
            certificate
        })
    } else {
        None
    };
    let mut result = json!({"ok":failure.is_none(),"session_id":session.session_id.to_string(),
        "scope":selected.scope,"selection_scope":"session_originals",
        "selected_plans_digest":selected.digest.to_hex(),"selected_plans":selected.identities,
        "dispatch_handles_restored":false,"foreign_live_work_included":false,
        "drain_progress":{"original_plans_total":selected.originals,
            "actions_total":selected.live_actions.saturating_add(selected.recovered_steps),
            "live_actions_total":selected.live_actions,"recovered_steps_total":selected.recovered_steps,
            "drained":progress.drained,"cancelled":progress.cancelled,"compensated":progress.compensated,
            "terminal_work_stopped":progress.terminal_work_stopped,
            "remaining_nonterminal":census.as_ref().map(|census| census.remaining_nonterminal),
            "remaining_work":census.as_ref().map(|census| census.remaining_work),
            "remaining_carried":census.as_ref().map(|census| census.remaining_carried),
            "remaining_durable_admissions":census.as_ref().map(|census| census.remaining_durable_admissions),
            "untracked_work_quiescent":census.as_ref().is_some_and(|census| census.untracked_quiet),
            "quiescent":quiet},
        "original_results":results,"finalize_certificate":certificate,
        "untracked_work":census.as_ref().map(|census| &census.untracked),
        "paused":session.adapter.snapshot().paused,"emergency_pause":emergency,
        "observed_anchor":anchor,"publication_confirmed":publication,"durability_pending":!publication,
        "source_work":{"admitted_source_bytes":selected.source_bytes,"admitted_source_records":selected.source_records,
            "shared_wall_millis":session.budget.max_wall_millis}});
    if let Some(requested) = &selected.requested {
        result["selection_scope"] = json!("exact_original_set");
        result["requested_plan_digests"] =
            json!(requested.iter().map(Digest32::to_hex).collect::<Vec<_>>());
        result["selection_resolved"] = json!(true);
        result["descendant_plans_included"] = json!(false);
        result["unselected_plans_included"] = json!(false);
        result["untracked_work_included"] = json!(false);
    }
    if let Some(mut error) = failure {
        error["effect_may_have_occurred"] = json!(!results.is_empty() || emergency.is_some());
        result["error"] = error;
    }
    result.to_string()
}
