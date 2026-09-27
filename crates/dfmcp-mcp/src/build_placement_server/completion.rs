//! Original-batch completion through the existing furniture MCP session.
//! Query authority covers local monitor custody and one foreground native read.
//! No monitor state grants, repeats or discharges a placement effect.
use super::*;
use dfmcp_adapter::build_placement::journal::MAX_BODY_BYTES;
use dfmcp_adapter::build_placement::journal::private_file::PrivateFileIdentity;
use dfmcp_adapter::construction_plan::origin::{MonitorDefinition, Origin};
use dfmcp_adapter::construction_plan::store::{MonitorStore, open_private_monitor};
use dfmcp_adapter::construction_plan::{Goal, Progress, Timing};
use std::cell::RefCell;
use std::path::Path;

pub(super) enum Action {
    Start(Timing),
    Sample,
    Inspect,
    Cancel,
}

pub(super) struct Monitor {
    pub store: MonitorStore<PrivateBuildFile>,
    pub verified: bool,
    pub store_verified: bool,
    parent: PrivateFileIdentity,
    child: PrivateFileIdentity,
}

/// A separate owner keeps local cancellation reachable after the original
/// placement files are lost. Opening it never opens either original path and
/// cannot establish current origin custody or obtain a native connection.
pub(super) struct Recovery {
    pub id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    binding: BuildBinding,
    store: MonitorStore<PrivateBuildFile>,
}
impl Recovery {
    fn high_tick(&self) -> u64 {
        self.store
            .definition()
            .origin()
            .receipts()
            .iter()
            .flat_map(|r| [r.plan().before().tick(), r.after().map_or(0, |v| v.tick())])
            .chain(self.store.progress().last_tick)
            .max()
            .unwrap_or(0)
    }
    fn context(&mut self, mode: BuildMode, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(wall) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(wall);
        }
        let mut c = context(
            self.id,
            RequestId::new(self.request),
            self.binding.fortress().fortress_id(),
            self.high_tick(),
            budget,
            mode,
            false,
        );
        c.grants = self.grants.clone();
        authorize(&c, Some(self.store.progress()))?;
        Ok(c)
    }
    fn render(&self, op: &str, c: &OperationContext, result: Value, verified: bool) -> String {
        recovery_packet(
            op,
            c,
            &self.binding,
            self.store.definition(),
            self.store.progress(),
            result,
            verified && !self.store.is_fenced(),
            Some(&self.store),
        )
    }
}

fn recovery_batch(origin: &Origin) -> Value {
    let definition = origin.definition();
    let rows = definition.plan().ordered_steps().zip(origin.receipts()).map(|(step, record)| {
        json!({"name":step.name,"idempotency_key":definition.key(step),
            "native_plan_digest":record.plan().digest().to_string(),
            "state":"unverified","historical_outcome":"placed",
            "receipt_digest":record.receipt().to_string(),
            "insertion":record.insertion().map(|v|json!({"building_id":v.building_id(),"job_id":v.job_id()}))})
    }).collect::<Vec<_>>();
    let plan: Value =
        serde_json::from_slice(definition.plan().canonical_bytes()).unwrap_or(Value::Null);
    json!({"schema":"dfmcp.furniture-batch-mcp/1","batch_id":definition.id().to_string(),
        "plan_digest":definition.plan().digest().to_string(),"plan":plan,
        "journal_id":definition.journal_id().to_string(),"head":origin.journal_head().to_string(),
        "inventory_verified":false,"historical_evidence_only":true,"status":"unverified",
        "historical_placed":rows.len(),"placed":null,"total":rows.len(),"steps":rows,
        "stopped":null,"advancement_fenced":true,"pending_step":null,"next":null,
        "atomic":false,"construction_completion_proven":false,"retry_permitted":false,
        "reopening_restores_dispatch_permission":false})
}

#[allow(clippy::too_many_arguments)]
fn recovery_packet(
    op: &str,
    c: &OperationContext,
    binding: &BuildBinding,
    definition: &MonitorDefinition,
    progress: &Progress,
    mut result: Value,
    verified: bool,
    store: Option<&MonitorStore<PrivateBuildFile>>,
) -> String {
    result["batch"] = recovery_batch(definition.origin());
    let mut completion = projected(definition, progress, false);
    completion["monitor_inventory_verified"] = json!(verified);
    completion["monitor_terminal"] = json!(verified && progress.terminal());
    completion["advancement_fenced"] = json!(store.is_some_and(|v| v.is_fenced()));
    completion["monitor_only_recovery"] = json!(true);
    if let Some(store) = store {
        completion["journal"] = json!({"head":store.head().to_string(),
            "frames":store.frames(),"byte_length":store.byte_len()});
    }
    result["completion"] = completion;
    result["effect_may_have_occurred"] = json!(false);
    result["game_mutation_dispatched"] = json!(false);
    result["native_calls"] = json!(0);
    packet(
        op,
        result,
        Some(c),
        Some(binding),
        None,
        None,
        None,
        None,
        None,
    )
}

pub(super) fn open_recovery(
    config: &Config,
    request: &runtime::RequestControl,
    c: &OperationContext,
    work: &mut Work,
) -> Result<(Recovery, String)> {
    authorize(c, None)?;
    let path = config.completion_path.as_ref().ok_or_else(missing)?;
    let store = open_private_monitor(
        path,
        &work.take(c, length_reserve(path, 128 * 1024 * 1024, 7)?)?,
        config.mode,
        None,
    )?;
    let origin = store.definition().origin();
    let binding = origin.definition().binding().clone();
    config.matches(&binding)?;
    if origin.child_identity().path != config.path
        || config.batch_path.as_ref() != Some(&origin.parent_identity().path)
    {
        return Err(corrupt());
    }
    let state = Recovery {
        id: c.session_id,
        request: c.request_id.get(),
        budget: c.budget,
        grants: c.grants.clone(),
        binding,
        store,
    };
    let mut current = work.current(c)?;
    current.anchor.tick = GameTick(current.anchor.tick.get().max(state.high_tick()));
    authorize(&current, Some(state.store.progress()))?;
    let output = state.render(
        "fortress.open_session",
        &current,
        json!({"ok":true,
        "session_id":state.id.to_string(),"mode":if config.mode==BuildMode::Offline {
            "completion-offline"} else {"completion-recover"},
        "capabilities":current.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),
        "planning_observation_retained":false,"native_preparation_dispatched":false}),
        true,
    );
    if output.len() as u64 > OUTPUT_BYTES {
        return Err(exhausted());
    }
    // Admit inspect/schema and local cancellation disclosures before keeping
    // custody. The original targets remain complete in every retained result.
    let reserve = state.render(
        "fortress.query",
        &current,
        json!({"ok":true,"query_schema":schema()}),
        true,
    );
    if reserve.len() + 2048 > OUTPUT_BYTES as usize {
        return Err(exhausted());
    }
    work.current(&current)?;
    runtime::boundary(request, config, false)?;
    Ok((state, output))
}

pub(super) fn run_recovery(
    state: &mut Recovery,
    config: &Config,
    request: &runtime::RequestControl,
    op: &str,
    action: Result<super::Action>,
    wall: Option<u64>,
) -> String {
    let c = match state.context(config.mode, wall) {
        Ok(c) => c,
        Err(e) => return read_only_packet(unbound(op, &e)),
    };
    let result = (|| -> Result<String> {
        runtime::boundary(request, config, false)?;
        let mut work = Work::new(&c, request.started)?;
        let cancel = match action? {
            super::Action::Completion(Action::Cancel) => true,
            super::Action::Completion(Action::Inspect)
            | super::Action::Query(Query::Batch {})
            | super::Action::Query(Query::Schema {})
            | super::Action::Inventory => false,
            _ => {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "monitor-only recovery permits local inspect, cancel and session release",
                ));
            }
        };
        let mut result = json!({"ok":true,"scope":"completion","monitor_only_recovery":true});
        if cancel {
            result["cancel_requested"] = json!(true);
            let current = work.take(&c, state.store.operation_reserve(false))?;
            let binding = &state.binding;
            let mut admit = |definition: &MonitorDefinition, progress: &Progress| {
                runtime::boundary(request, config, false)?;
                authorize(&work.current(&c)?, Some(progress))?;
                if recovery_packet(
                    op,
                    &c,
                    binding,
                    definition,
                    progress,
                    result.clone(),
                    true,
                    None,
                )
                .len()
                    + 512
                    > OUTPUT_BYTES as usize
                {
                    return Err(exhausted());
                }
                Ok(())
            };
            state.store.cancel(&current, &mut admit)?;
        } else {
            state
                .store
                .verify(&work.take(&c, state.store.byte_len() as u64 + 8192)?)?;
            result["query_schema"] = schema();
        }
        authorize(&work.current(&c)?, Some(state.store.progress()))?;
        runtime::boundary(request, config, false)?;
        let output = state.render(op, &c, result, true);
        if output.len() as u64 > OUTPUT_BYTES {
            return Err(exhausted());
        }
        Ok(output)
    })();
    match result {
        Ok(output) => output,
        Err(e) => {
            state.store.abandon_read();
            match authorize(&c, Some(state.store.progress())) {
                Ok(()) => {
                    let output = state.render(op, &c, failure(&e), false);
                    if output.len() as u64 > OUTPUT_BYTES {
                        read_only_packet(unbound(op, &exhausted()))
                    } else {
                        output
                    }
                }
                Err(denied) => read_only_packet(unbound(op, &denied)),
            }
        }
    }
}

pub(super) fn close_recovery(
    state: &mut Recovery,
    config: &Config,
    request: &runtime::RequestControl,
    release: bool,
) -> Result<String> {
    let result = json!({"ok":true,"scope":"session","closed":true,
        "release_for_recovery":release,"effects_cancelled":false,"history_erased":false,
        "native_quiescence_proven":false,"effect_may_have_occurred":false,"game_mutation_dispatched":false});
    let c = match state.context(config.mode, None) {
        Ok(c) => c,
        Err(_) if release => {
            return Ok(packet(
                "fortress.cancel",
                result,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ));
        }
        Err(e) => return Err(e),
    };
    if !release {
        runtime::boundary(request, config, false)?;
        let mut work = Work::new(&c, request.started)?;
        state
            .store
            .verify(&work.take(&c, state.store.byte_len() as u64 + 8192)?)?;
        work.current(&c)?;
        runtime::boundary(request, config, false)?;
    }
    let output = state.render("fortress.cancel", &c, result, !release);
    if output.len() as u64 > OUTPUT_BYTES {
        return Err(exhausted());
    }
    Ok(output)
}

/// The native read monitor cannot dispatch a game effect even when its local
/// cancellation or the supervising worker fails.
pub(super) fn read_only_packet(raw: String) -> String {
    match serde_json::from_str::<Value>(&raw) {
        Ok(mut packet) => {
            packet["result"]["effect_may_have_occurred"] = json!(false);
            packet["result"]["game_mutation_dispatched"] = json!(false);
            packet.to_string()
        }
        Err(_) => raw,
    }
}
fn missing() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::InvalidRequest,
        "original batch completion monitor is unavailable",
    )
}
fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "original complete batch or monitor custody differs",
    )
}
fn authorize(c: &OperationContext, progress: Option<&Progress>) -> Result<()> {
    let mut current = c.clone();
    current.anchor.tick = GameTick(
        current
            .anchor
            .tick
            .get()
            .max(progress.and_then(|p| p.last_tick).unwrap_or(0)),
    );
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
}

/// This is only admission arithmetic. The private opener still performs every
/// no-follow, mode, owner, file identity and complete-byte check itself.
pub(super) fn open_reserve(path: &Path, maximum: u64) -> Result<u64> {
    length_reserve(path, maximum, 8)
}
fn length_reserve(path: &Path, maximum: u64, copies: u64) -> Result<u64> {
    let length = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= maximum => meta.len(),
        Ok(_) => return Err(corrupt()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => return Err(corrupt()),
    };
    length
        .checked_mul(copies)
        .and_then(|n| n.checked_add(2 * 1024 * 1024))
        .ok_or_else(exhausted)
}

fn source_reserve(origin: &Origin) -> Result<u64> {
    (origin.journal_bytes() as u64)
        .checked_add((origin.receipts().len() * MAX_BODY_BYTES) as u64)
        .and_then(|n| n.checked_add(batch::GUARD_BYTES))
        .ok_or_else(exhausted)
}
fn verify_source<S: EffectJournalStorage, N: BuildSource>(
    child: &mut BuildSession<S, N>,
    parent: &mut Option<batch::Parent>,
    binding: &BuildBinding,
    origin: &Origin,
    identities: (&PrivateFileIdentity, &PrivateFileIdentity),
    c: &OperationContext,
) -> Result<BuildInventory> {
    authorize(c, None)?;
    let parent = parent.as_mut().ok_or_else(corrupt)?;
    let mut parent_context = c.clone();
    parent_context.budget.max_bytes = batch::GUARD_BYTES;
    parent.verify(&parent_context)?;
    let mut child_context = c.clone();
    child_context.budget.max_bytes = c
        .budget
        .max_bytes
        .checked_sub(batch::GUARD_BYTES)
        .ok_or_else(exhausted)?;
    let view = child.inventory(&child_context)?;
    origin.verify_batch(
        parent.definition(),
        binding,
        &view,
        identities.0,
        identities.1,
    )?;
    Ok(view)
}

fn projected(definition: &MonitorDefinition, progress: &Progress, verified: bool) -> Value {
    let timing = definition.goal().timing();
    let original = definition.origin();
    let rows = definition.goal().records().iter().map(|record| {
        let insertion = record.insertion();
        let finding = progress.assessments.iter().find(|row| row.key == record.plan().key());
        let condition = finding.map(|row| &row.condition);
        let step = original.definition().plan().ordered_steps()
            .find(|step| original.definition().key(step) == record.plan().key());
        json!({
            "name":step.map(|step|step.name.as_str()),"key":record.plan().key(),
            "building_id":insertion.map(|v|v.building_id()),"item_id":insertion.map(|v|v.item_id()),
            "job_id":insertion.map(|v|v.job_id()),"receipt_digest":record.receipt().to_string(),
            "status":condition.map_or("not_sampled", |v|v.status),
            "stage":condition.and_then(|v|v.stage),"max_stage":insertion.map(|v|v.max_stage()),
            "building_type":condition.and_then(|v|v.building_type),
            "construction_jobs":condition.map(|v|v.construction_jobs),
            "removal_jobs":condition.map(|v|v.removal_jobs),
            "suspended_jobs":condition.map(|v|v.suspended_jobs),
            "item_job_links":condition.map(|v|v.item_job_links)
        })
    }).collect::<Vec<_>>();
    json!({
        "schema":"dfmcp.furniture-completion-mcp/1",
        "policy":dfmcp_adapter::construction_plan::POLICY,
        "monitor_id":definition.id().to_string(),"goal_digest":definition.goal().digest().to_string(),
        "origin_digest":original.digest().to_string(),
        "batch_id":original.definition().id().to_string(),
        "original_journal_head":original.journal_head().to_string(),
        "origin_verified":verified,"inventory_verified":verified,
        "phase":if verified {progress.phase} else {"unverified"},
        "historical_phase":progress.phase,"reason":progress.reason,"reason_building":progress.reason_building,
        "observations":progress.observations,"streak":progress.streak,"first_tick":progress.first_tick,
        "last_counted_tick":progress.counted_tick,"last_tick":progress.last_tick,
        "capture_sha256":progress.last_capture.map(|v|v.to_string()),
        "interrupted_reads":progress.interruptions,"read_outcome_unknown":progress.reading,
        "terminal":verified && progress.terminal(),
        "sampled_condition_satisfied":verified && progress.phase == "satisfied",
        "timing":{"deadline":timing.deadline,"interval":timing.interval,
            "stable_samples":timing.stable_samples,"stable_span":timing.stable_span,
            "max_gap":timing.max_gap,"max_observations":timing.max_observations},
        "selection_count":rows.len(),"condition_met_count":progress.condition_met_count(),
        "assessments_complete":progress.assessments.len()==rows.len(),"assessments":rows,
        "source":progress.source.as_ref().map(|s|json!({
            "furniture_generation":s.furniture_generation,"operations_generation":s.operations.generation,
            "df_version":s.operations.df_version,"dfhack_version":s.operations.dfhack_version
        })),
        "evidence_scope":"historical_same_capture_whole_plan_sampled_condition",
        "placement_effect_discharged":false,"current_usability_proven":false,
        "continuous_stability_proven":false,"retry_placement_permitted":false,
        "game_mutation_dispatched":false
    })
}

pub(super) fn display(monitor: &Monitor, verified: bool) -> Value {
    let mut result = projected(
        monitor.store.definition(),
        monitor.store.progress(),
        verified && !monitor.store.is_fenced(),
    );
    result["advancement_fenced"] = json!(monitor.store.is_fenced());
    let local_verified = monitor.store_verified && !monitor.store.is_fenced();
    result["monitor_inventory_verified"] = json!(local_verified);
    result["monitor_terminal"] = json!(local_verified && monitor.store.progress().terminal());
    result["journal"] = json!({"head":monitor.store.head().to_string(),
        "frames":monitor.store.frames(),"byte_length":monitor.store.byte_len()});
    result
}

/// Ordinary furniture results also recheck the retained monitor. Its original
/// identities were compared with the actual held owners at import/reopen; each
/// current child/parent read independently validates those same held identities.
pub(super) fn verify<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    view: &BuildInventory,
    parent_verified: bool,
) -> Result<()> {
    let monitor = state.completion.as_mut().ok_or_else(missing)?;
    monitor.verified = false;
    monitor.store_verified = false;
    if !parent_verified {
        return Err(corrupt());
    }
    let parent = state.batch.as_ref().ok_or_else(corrupt)?;
    monitor
        .store
        .verify(&work.take(c, monitor.store.byte_len() as u64 + 8192)?)?;
    monitor.store_verified = true;
    monitor.store.definition().origin().verify_batch(
        parent.definition(),
        &state.binding,
        view,
        &monitor.parent,
        &monitor.child,
    )?;
    authorize(&work.current(c)?, Some(monitor.store.progress()))?;
    monitor.verified = true;
    Ok(())
}

pub(super) fn reopen<N: BuildSource>(
    state: &mut State<PrivateBuildFile, N>,
    config: &Config,
    c: &OperationContext,
    work: &mut Work,
    view: &BuildInventory,
) -> Result<()> {
    let Some(path) = &config.completion_path else {
        return Ok(());
    };
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(corrupt()),
        Ok(_) => {}
    }
    let parent = state.batch.as_ref().ok_or_else(corrupt)?;
    let parent_identity = parent.private_identity(&work.current(c)?)?;
    let child_identity = state.control.private_identity(&work.current(c)?)?;
    let store = open_private_monitor(
        path,
        &work.take(c, length_reserve(path, 128 * 1024 * 1024, 7)?)?,
        config.mode,
        None,
    )?;
    store.definition().origin().verify_batch(
        parent.definition(),
        &state.binding,
        view,
        &parent_identity,
        &child_identity,
    )?;
    reserve_output(state, c, view, store.definition())?;
    let current = work.take(c, source_reserve(store.definition().origin())?)?;
    let refreshed = verify_source(
        &mut state.control,
        &mut state.batch,
        &state.binding,
        store.definition().origin(),
        (&parent_identity, &child_identity),
        &current,
    )?;
    state.historical = Some(refreshed);
    authorize(&work.current(c)?, Some(store.progress()))?;
    state.completion = Some(Monitor {
        store,
        verified: true,
        store_verified: true,
        parent: parent_identity,
        child: child_identity,
    });
    Ok(())
}

/// Admit worst per-target integer widths and retained identity strings before a
/// monitor is created. The complete original batch appears only once.
fn reserve_output<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
    c: &OperationContext,
    view: &BuildInventory,
    definition: &MonitorDefinition,
) -> Result<()> {
    let mut result = json!({"ok":true});
    if let Some(parent) = &state.batch {
        result["batch"] = batch::display(parent, Some(view), true);
    }
    let mut completion = projected(definition, &Progress::new(definition.goal()), true);
    completion["phase"] = json!("invalidated");
    completion["reason"] = json!("original_job_identity_mismatch");
    for row in completion["assessments"]
        .as_array_mut()
        .ok_or_else(corrupt)?
    {
        row["status"] = json!("original_job_identity_mismatch");
        for name in [
            "stage",
            "max_stage",
            "building_type",
            "construction_jobs",
            "removal_jobs",
            "suspended_jobs",
            "item_job_links",
        ] {
            row[name] = json!(u32::MAX);
        }
    }
    result["completion"] = completion;
    let bytes = packet(
        "fortress.query",
        result,
        Some(c),
        Some(&state.binding),
        Some(view),
        None,
        Some(&state.policy),
        None,
        None,
    )
    .len();
    // This admission covers the complete monitor and its handoff, including
    // future source identities, journal metadata, counters and errors. Exact
    // native-record queries have their own whole-packet check; reserving their
    // optional payload here would prevent ordinary 32-target monitoring even
    // when every target and all completion evidence fit together.
    if bytes + 8192 > OUTPUT_BYTES as usize {
        return Err(exhausted());
    }
    Ok(())
}

pub(super) fn run<N: BuildSource>(
    state: &mut State<PrivateBuildFile, N>,
    config: &Config,
    request: &runtime::RequestControl,
    c: OperationContext,
    op: &str,
    action: Action,
) -> String {
    state.abandon();
    let mut monitor = state.completion.take();
    let result = (|| -> Result<(Value, Option<BuildInventory>)> {
        let ledger = RefCell::new(Work::new(&c, request.started)?);
        authorize(&c, monitor.as_ref().map(|m| m.store.progress()))?;
        let before = state.control.inventory(&ledger.borrow().view(&c)?);
        let before = match before {
            Ok(view) => {
                state.historical = Some(view.clone());
                Some(view)
            }
            Err(_) if matches!(action, Action::Cancel) => None,
            Err(e) => return Err(e),
        };
        if let Action::Start(timing) = &action {
            let view = before.as_ref().ok_or_else(corrupt)?;
            let parent = state.batch.as_mut().ok_or_else(missing)?;
            batch::verify(
                parent,
                view,
                &ledger.borrow_mut().take(&c, batch::GUARD_BYTES)?,
            )?;
            let parent_identity = parent.private_identity(&ledger.borrow().current(&c)?)?;
            let child_identity = state
                .control
                .private_identity(&ledger.borrow().current(&c)?)?;
            let origin = Origin::from_batch(
                parent.definition().clone(),
                &state.binding,
                view,
                parent_identity.clone(),
                child_identity.clone(),
            )?;
            let goal = Goal::new(origin.receipts().to_vec(), *timing)?;
            let definition = MonitorDefinition::new(origin, goal)?;
            reserve_output(state, &c, view, &definition)?;
            if let Some(existing) = &monitor {
                if existing.store.definition().id() != definition.id() {
                    return Err(error(
                        ErrorCode::Conflict,
                        "retained completion goal cannot be replaced or renewed",
                    ));
                }
            } else {
                let path = config.completion_path.as_ref().ok_or_else(missing)?;
                runtime::boundary(request, config, false)?;
                let open_context = ledger.borrow_mut().take(
                    &c,
                    open_reserve(path, 128 * 1024 * 1024)?
                        + 8 * definition.canonical_bytes().len() as u64,
                )?;
                let store =
                    open_private_monitor(path, &open_context, config.mode, Some(definition))?;
                monitor = Some(Monitor {
                    store,
                    verified: false,
                    store_verified: false,
                    parent: parent_identity,
                    child: child_identity,
                });
            }
        }
        let monitor = monitor.as_mut().ok_or_else(missing)?;
        monitor.verified = false;
        monitor.store_verified = false;
        let template_result = json!({"ok":true});
        let mut template_result = template_result;
        if let Some(parent) = &state.batch {
            template_result["batch"] = batch::display(parent, before.as_ref(), false);
        }
        let template = packet(
            op,
            template_result,
            Some(&c),
            Some(&state.binding),
            before.as_ref(),
            state.historical.as_ref(),
            Some(&state.policy),
            None,
            None,
        );
        let last_view = RefCell::new(before);
        let parent_identity = monitor.parent.clone();
        let child_identity = monitor.child.clone();
        let child = &mut state.control;
        let parent = &mut state.batch;
        let binding = &state.binding;
        let mut guard = |origin: &Origin, context: &OperationContext| -> Result<()> {
            runtime::boundary(request, config, false)?;
            let current = ledger.borrow_mut().take(context, source_reserve(origin)?)?;
            let view = verify_source(
                child,
                parent,
                binding,
                origin,
                (&parent_identity, &child_identity),
                &current,
            )?;
            last_view.replace(Some(view));
            runtime::boundary(request, config, false)
        };
        let mut admit = |definition: &MonitorDefinition, progress: &Progress| -> Result<()> {
            runtime::boundary(request, config, false)?;
            authorize(&ledger.borrow().current(&c)?, Some(progress))?;
            let mut candidate: Value = serde_json::from_str(&template).map_err(|_| corrupt())?;
            candidate["result"]["completion"] = projected(definition, progress, true);
            if candidate.to_string().len() + 8192 > OUTPUT_BYTES as usize {
                return Err(exhausted());
            }
            Ok(())
        };
        let verify_context = ledger
            .borrow_mut()
            .take(&c, monitor.store.byte_len() as u64 + 8192)?;
        monitor.store.verify(&verify_context)?;
        monitor.store_verified = true;
        let origin_context = ledger.borrow().current(&c)?;
        let origin_result = guard(monitor.store.definition().origin(), &origin_context);
        if !matches!(action, Action::Cancel) {
            origin_result?;
        }
        let mut native_calls = 0;
        match action {
            Action::Start(_) | Action::Inspect => {}
            Action::Sample if monitor.store.progress().terminal() => {}
            Action::Sample => {
                if config.mode == BuildMode::Offline {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "offline monitor cannot acquire native evidence",
                    ));
                }
                let begin_bytes = monitor.store.operation_reserve(false);
                let publish_bytes = monitor
                    .store
                    .operation_reserve(true)
                    .checked_add(8192)
                    .ok_or_else(exhausted)?;
                let source_bytes = 24 * 1024 * 1024;
                let guard_bytes = source_reserve(monitor.store.definition().origin())?
                    .checked_mul(8)
                    .ok_or_else(exhausted)?;
                let required = begin_bytes
                    .checked_add(publish_bytes)
                    .and_then(|n| n.checked_add(source_bytes))
                    .and_then(|n| n.checked_add(guard_bytes))
                    .ok_or_else(exhausted)?;
                if ledger.borrow().bytes < required {
                    return Err(exhausted());
                }
                let begin = ledger.borrow_mut().take(&c, begin_bytes)?;
                monitor.store.begin_read(&begin, &mut admit, &mut guard)?;
                let source = ledger.borrow_mut().take(&c, source_bytes)?;
                native_calls = 1;
                let sample = runtime::completion_sample(
                    config,
                    binding,
                    monitor.store.definition().goal(),
                    &source,
                )?;
                let publish = ledger.borrow_mut().take(&c, publish_bytes)?;
                monitor
                    .store
                    .publish_sample(&publish, &sample, &mut admit, &mut guard)?;
            }
            Action::Cancel => {
                let cancel = ledger
                    .borrow_mut()
                    .take(&c, monitor.store.operation_reserve(false))?;
                monitor.store.cancel(&cancel, &mut admit)?;
            }
        }
        let final_context = ledger.borrow().current(&c)?;
        let final_origin = guard(monitor.store.definition().origin(), &final_context);
        monitor.verified = final_origin.is_ok();
        if !matches!(action, Action::Cancel) {
            final_origin?;
        }
        runtime::boundary(request, config, false)?;
        authorize(
            &ledger.borrow().current(&c)?,
            Some(monitor.store.progress()),
        )?;
        let result = json!({"ok":true,"scope":"completion","native_acquisitions":native_calls,
            "game_mutation_dispatched":false,"effects_cancelled":false,"building_undone":false,
            "placement_effect_discharged":false,"retry_commit_permitted":false});
        Ok((result, last_view.into_inner()))
    })();
    state.completion = monitor;
    let (mut result, view) = match result {
        Ok(v) => v,
        Err(e) => {
            if let Some(monitor) = &mut state.completion {
                monitor.store.abandon_read();
                monitor.store_verified = false;
            }
            return state.failed(op, Some(&c), &e);
        }
    };
    let current = match state.disclosure_context(&c) {
        Ok(v) => v,
        Err(e) => return unbound(op, &e),
    };
    if let Some(view) = &view {
        state.historical = Some(view.clone());
    }
    let verified = state.completion.as_ref().is_some_and(|m| m.verified);
    state.attach_batch(
        &mut result,
        view.as_ref().or(state.historical.as_ref()),
        verified,
    );
    let output = packet(
        op,
        result,
        Some(&current),
        Some(&state.binding),
        view.as_ref(),
        state.historical.as_ref(),
        Some(&state.policy),
        None,
        None,
    );
    if output.len() as u64 > OUTPUT_BYTES {
        return state.failed(op, Some(&current), &exhausted());
    }
    if let Err(e) = runtime::boundary(request, config, false) {
        return state.failed(op, Some(&current), &e);
    }
    output
}
