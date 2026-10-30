//! One original room goal, one read intent and one shared sampled condition.
//! Terrain and every original furnishing are assessed together. This local
//! monitor neither dispatches placements nor promotes a furniture-only clock.
use super::*;
use dfmcp_adapter::build_placement::journal::private_file::PrivateFileIdentity;
use dfmcp_adapter::construction_plan::origin::{MonitorDefinition, Origin};
use dfmcp_adapter::construction_plan::{Goal, Timing};
use dfmcp_adapter::room_condition::store::{
    MonitorStore, Recovery, initial_recovery, open_private_monitor,
};
use dfmcp_adapter::room_condition::{
    Counts, Definition, MAX_DEFICITS, MAX_SAMPLE, Progress, Sample,
};
use std::cell::{Cell, RefCell};

pub(super) enum Action {
    Start(Timing),
    Sample,
    Inspect,
    Cancel,
}

pub(super) trait Io {
    fn sample(
        &mut self,
        binding: &BuildBinding,
        definition: &Definition,
        c: &OperationContext,
    ) -> Result<Sample>;
    fn checkpoint(&mut self, c: &OperationContext) -> Result<()>;
    fn observed_tick_floor(&self) -> u64 {
        0
    }
}

pub(super) struct Monitor {
    pub store: MonitorStore<PrivateBuildFile>,
    pub verified: bool,
    pub store_verified: bool,
    pub parent: PrivateFileIdentity,
    pub child: PrivateFileIdentity,
}

fn missing() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::InvalidRequest,
        "original joint room monitor is unavailable",
    )
}
fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "original room, complete furnishing receipts or monitor custody differs",
    )
}
fn current(c: &OperationContext, floor: u64) -> Result<OperationContext> {
    let mut c = c.clone();
    c.anchor.tick = GameTick(c.anchor.tick.get().max(floor));
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(c)
}
pub(super) fn recovery(pin: Recovery) -> Value {
    json!({"monitor_id":pin.monitor_id.to_string(),"head":pin.head.to_string()})
}
fn counts(value: &Counts) -> Value {
    json!({"total":value.total,"matched":value.matched,"hidden":value.hidden,
        "missing":value.missing,"wrong_shape":value.wrong_shape,"liquid":value.liquid,
        "designated":value.designated,"occupied":value.occupied,"unoccupied":value.unoccupied})
}
pub(super) fn terrain(value: &dfmcp_adapter::room_condition::TerrainAssessment) -> Value {
    json!({"floors":counts(&value.floors),"walls":counts(&value.walls),
        "furnishings":counts(&value.furnishings),"condition_met":value.condition_met(),
        "unknown":value.unknown(),"deficit_count":value.deficit_count(),
        "deficits_complete":value.deficit_count() as usize <= MAX_DEFICITS,
        "deficits":value.deficits.iter().map(|d|json!({"position":d.position,
            "requirement":d.requirement,"status":d.status})).collect::<Vec<_>>()})
}
pub(super) fn projected(definition: &Definition, progress: &Progress, verified: bool) -> Value {
    let mut value = completion::projected(definition.original(), &progress.construction, verified);
    value["schema"] = json!("dfmcp.room-completion-mcp/1");
    value["policy"] = json!(dfmcp_adapter::room_condition::POLICY);
    value["monitor_id"] = json!(definition.id().to_string());
    value["room_intent_digest"] = json!(definition.room().digest().to_string());
    value["authority_tick_floor"] = json!(progress.observed_tick_floor);
    value["evidence_scope"] =
        json!("historical_joint_original_room_and_furnishings_sampled_condition");
    value["terrain"] = terrain(&progress.terrain);
    value["terrain"]["region"] = json!({"origin":definition.room().terrain().region().origin,
        "size":definition.room().terrain().region().size});
    value["source"] = progress.source.as_ref().map_or(Value::Null, |source| {
        json!({
        "furniture_generation":source.furniture.furniture_generation,
        "operations_generation":source.furniture.operations.generation,
        "map_generation":source.map.generation,
        "df_version":source.map.df_version,"dfhack_version":source.map.dfhack_version})
    });
    value["fresh_joint_sample_acquired"] = json!(false);
    value["sampling_permitted"] = json!(false);
    value["local_cancel_permitted"] = json!(false);
    value["room_assignment_created"] = json!(false);
    value["native_pathfinding_proven"] = json!(false);
    value["current_room_usability_proven"] = json!(false);
    value["atomic_cross_profile_snapshot_proven"] = json!(false);
    value["construction_clock_goal_promoted"] = json!(false);
    value
}
pub(super) fn display(monitor: &Monitor, verified: bool) -> Value {
    let mut value = projected(
        monitor.store.definition(),
        monitor.store.progress(),
        verified && !monitor.store.is_fenced(),
    );
    value["advancement_fenced"] = json!(monitor.store.is_fenced());
    value["monitor_inventory_verified"] =
        json!(monitor.store_verified && !monitor.store.is_fenced());
    value["monitor_terminal"] = json!(
        monitor.store_verified && !monitor.store.is_fenced() && monitor.store.progress().terminal()
    );
    value["authority_tick_floor"] = json!(monitor.store.authority_tick_floor());
    value["journal"] = json!({"head":monitor.store.head().to_string(),
        "frames":monitor.store.frames(),"byte_length":monitor.store.byte_len()});
    value["recovery"] = recovery(monitor.store.recovery());
    value
}

fn reserve_output<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
    c: &OperationContext,
    view: &BuildInventory,
    definition: &Definition,
) -> Result<()> {
    let mut result = json!({"ok":true});
    state.attach_batch(&mut result, Some(view), false);
    let mut room = projected(definition, &Progress::new(definition), true);
    room["phase"] = json!("invalidated");
    room["reason"] = json!("original_room_source_or_receipt_identity_mismatch");
    for row in room["assessments"].as_array_mut().ok_or_else(corrupt)? {
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
    room["terrain"]["deficits"] = json!(
        (0..MAX_DEFICITS)
            .map(|_| json!({
        "position":[u32::MAX,u32::MAX,u32::MAX],"requirement":"furnishing","status":"wrong_shape"}))
            .collect::<Vec<_>>()
    );
    result["room_completion"] = room;
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
    if bytes
        .checked_add(8192)
        .is_none_or(|n| n > OUTPUT_BYTES as usize)
    {
        return Err(exhausted());
    }
    Ok(())
}

pub(super) fn verify<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    view: &BuildInventory,
    parent_verified: bool,
) -> Result<()> {
    let monitor = state.room_completion.as_mut().ok_or_else(missing)?;
    monitor.verified = false;
    monitor.store_verified = false;
    let checked = monitor
        .store
        .verify(&work.take(c, monitor.store.byte_len() as u64 + 8192)?);
    state.authority_tick = state
        .authority_tick
        .max(monitor.store.authority_tick_floor());
    state.room_recovery = Some(monitor.store.recovery());
    checked?;
    monitor.store_verified = true;
    if !parent_verified {
        return Err(corrupt());
    }
    let parent = state.batch.as_ref().ok_or_else(corrupt)?;
    monitor.store.definition().origin().verify_batch(
        parent.definition(),
        &state.binding,
        view,
        &monitor.parent,
        &monitor.child,
    )?;
    current(&work.current(c)?, state.authority_tick)?;
    monitor.verified = true;
    Ok(())
}

/// Joint replay validates both maps and the complete operations evidence in
/// every sample. Its semantic work exceeds the furniture-only replay reserve.
/// Keep the full conservative allowance inside the caller's fixed Work budget.
fn open_reserve(path: &std::path::Path) -> Result<u64> {
    let length = match std::fs::symlink_metadata(path) {
        Ok(meta)
            if meta.is_file()
                && meta.len() <= dfmcp_adapter::room_condition::store::MAX_STORE_BYTES as u64 =>
        {
            meta.len()
        }
        Ok(_) => return Err(corrupt()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => return Err(corrupt()),
    };
    length
        .checked_mul(9)
        .and_then(|n| {
            n.checked_add(
                u64::from(dfmcp_adapter::room_condition::store::MAX_STORE_FRAMES) * 16_384,
            )
        })
        .and_then(|n| n.checked_add(2 * 1024 * 1024))
        .ok_or_else(exhausted)
}

pub(super) fn reopen<N: BuildSource>(
    state: &mut State<PrivateBuildFile, N>,
    config: &Config,
    c: &OperationContext,
    work: &mut Work,
    view: &BuildInventory,
) -> Result<()> {
    let Some(path) = &config.room_completion_path else {
        return Ok(());
    };
    let expected = state.room_recovery.or(config.room_completion_recovery);
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && expected.is_none() => return Ok(()),
        Err(_) => return Err(corrupt()),
        Ok(_) => {}
    }
    let parent = state.batch.as_ref().ok_or_else(corrupt)?;
    let parent_identity = parent.private_identity(&work.current(c)?)?;
    let child_identity = state.control.private_identity(&work.current(c)?)?;
    let expected = expected.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "reopening a joint room monitor requires its acknowledged recovery identity",
        )
    })?;
    let mut store =
        open_private_monitor(path, &work.take(c, open_reserve(path)?)?, config.mode, None)?;
    state.authority_tick = state.authority_tick.max(store.authority_tick_floor());
    // Exact ancestry verification only reads the retained bytes; mutation's
    // five-copy reservation is unnecessary for this read-only checkpoint.
    store.verify_recovery(&work.take(c, store.byte_len() as u64 + 8192)?, expected)?;
    state.room_recovery = Some(store.recovery());
    let scoped = room_intake::validate(store.definition().room(), config);
    let output = reserve_output(
        state,
        &current(c, state.authority_tick)?,
        view,
        store.definition(),
    );
    state.room_completion = Some(Monitor {
        store,
        verified: false,
        store_verified: false,
        parent: parent_identity,
        child: child_identity,
    });
    scoped?;
    output?;
    verify(state, c, work, view, true)
}

/// Prepaid final replays also serve opening and ordinary-session publication.
/// No caller callback runs after these exact source, monitor and clock reads.
pub(super) struct Publication {
    deadline: Instant,
    source: OperationContext,
    monitor: OperationContext,
    clock: Option<OperationContext>,
    room_clock: Option<OperationContext>,
    head: Digest32,
}
impl Publication {
    pub(super) fn reserve<S: EffectJournalStorage, N: BuildSource>(
        state: &State<S, N>,
        c: &OperationContext,
        work: &mut Work,
    ) -> Result<Option<Self>> {
        let Some(monitor) = &state.room_completion else {
            return Ok(None);
        };
        Ok(Some(Self::reserve_for(
            state,
            c,
            work,
            monitor.store.definition(),
            monitor.store.byte_len() as u64,
            monitor.store.head(),
        )?))
    }
    pub(super) fn reserve_growing<S: EffectJournalStorage, N: BuildSource>(
        state: &State<S, N>,
        c: &OperationContext,
        work: &mut Work,
        growth: u64,
    ) -> Result<Option<Self>> {
        let Some(monitor) = &state.room_completion else {
            return Ok(None);
        };
        Ok(Some(Self::reserve_for(
            state,
            c,
            work,
            monitor.store.definition(),
            (monitor.store.byte_len() as u64)
                .checked_add(growth)
                .ok_or_else(exhausted)?,
            monitor.store.head(),
        )?))
    }
    pub(super) fn retain_current_head<S: EffectJournalStorage, N: BuildSource>(
        &mut self,
        state: &State<S, N>,
    ) -> Result<()> {
        self.head = state
            .room_completion
            .as_ref()
            .ok_or_else(missing)?
            .store
            .head();
        Ok(())
    }
    fn reserve_for<S: EffectJournalStorage, N: BuildSource>(
        state: &State<S, N>,
        c: &OperationContext,
        work: &mut Work,
        definition: &Definition,
        monitor_bytes: u64,
        head: Digest32,
    ) -> Result<Self> {
        Ok(Self {
            deadline: work.deadline,
            source: work.take(c, completion::source_reserve(definition.origin())?)?,
            monitor: work.take(c, monitor_bytes.checked_add(8192).ok_or_else(exhausted)?)?,
            clock: if state.clock_config.is_some() || state.clock.is_some() {
                Some(work.take(c, clock::VIEW_BYTES)?)
            } else {
                None
            },
            room_clock: if state.room_clock_config.is_some() || state.room_clock.is_some() {
                Some(work.take(c, room_clock::VIEW_BYTES)?)
            } else {
                None
            },
            head,
        })
    }
    fn current<S: EffectJournalStorage, N: BuildSource>(
        &self,
        state: &State<S, N>,
        reserved: &OperationContext,
    ) -> Result<OperationContext> {
        let remaining: u64 = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis()
            .try_into()
            .map_err(|_| exhausted())?;
        if remaining == 0 {
            return Err(exhausted());
        }
        let mut c = reserved.clone();
        c.budget.max_wall_millis = c.budget.max_wall_millis.min(remaining);
        state.disclosure_context(&c)
    }
    pub(super) fn verify<S: EffectJournalStorage, N: BuildSource>(
        &self,
        state: &mut State<S, N>,
        expected_clock: Option<dfmcp_adapter::construction_plan::clock::Recovery>,
        sampling: bool,
        allow_missing_origin: bool,
        checkpoint: impl FnOnce(&OperationContext) -> Result<()>,
    ) -> Result<()> {
        let expected_room_clock = state.room_clock_recovery;
        checkpoint(&self.current(state, &self.source)?)?;
        let c = self.current(state, &self.source)?;
        let monitor = state.room_completion.as_ref().ok_or_else(missing)?;
        let source = completion::verify_source(
            &mut state.control,
            &mut state.batch,
            &state.binding,
            monitor.store.definition().origin(),
            (&monitor.parent, &monitor.child),
            &c,
        );
        state.authority_tick = state.authority_tick.max(state.control.high_tick());
        let original_verified = source.is_ok();
        match source {
            Ok(view) => state.historical = Some(view),
            Err(_) if allow_missing_origin => {}
            Err(e) => return Err(e),
        }
        let c = self.current(state, &self.monitor)?;
        let monitor = state.room_completion.as_mut().ok_or_else(missing)?;
        monitor.store.verify(&c)?;
        state.authority_tick = state
            .authority_tick
            .max(monitor.store.authority_tick_floor());
        state.room_recovery = Some(monitor.store.recovery());
        if monitor.store.head() != self.head {
            return Err(corrupt());
        }
        monitor.store_verified = true;
        monitor.verified = original_verified;
        if let Some(reserved) = &self.clock {
            let c = self.current(state, reserved)?;
            let checked = clock::inspect(state, &c);
            if !allow_missing_origin {
                checked?;
            }
            if !allow_missing_origin && state.clock_recovery != expected_clock {
                return Err(corrupt());
            }
            if sampling && clock::blocks_sampling(state) {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "settle the original construction clock before sampling the room",
                ));
            }
        }
        if let Some(reserved) = &self.room_clock {
            let c = self.current(state, reserved)?;
            let checked = room_clock::inspect(state, &c);
            if !allow_missing_origin {
                checked?;
            }
            if !allow_missing_origin && state.room_clock_recovery != expected_room_clock {
                return Err(corrupt());
            }
            // A first ordinary joint sample may publish its source before the
            // explicit Plan call opens the absent initializer. It earns no run.
            if sampling
                && room_clock::blocks_sampling(state)
                && !room_clock::bootstrap_pending(state)
            {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "settle the retained room clock before joint sampling",
                ));
            }
        }
        self.current(state, &self.monitor)?;
        Ok(())
    }
}

fn check_origin<S: EffectJournalStorage, N: BuildSource, I: Io>(
    state: &mut State<S, N>,
    origin: &Origin,
    identities: (&PrivateFileIdentity, &PrivateFileIdentity),
    c: &OperationContext,
    ledger: &RefCell<Work>,
    io: &RefCell<&mut I>,
    sampling: bool,
    floor: &Cell<u64>,
) -> Result<()> {
    let mut current = current(c, floor.get().max(state.authority_tick))?;
    io.borrow_mut().checkpoint(&current)?;
    if state.clock_config.is_some() || state.clock.is_some() {
        let context = ledger.borrow_mut().take(&current, clock::VIEW_BYTES)?;
        let checked = clock::inspect(state, &context);
        floor.set(floor.get().max(state.authority_tick));
        checked?;
        if sampling && clock::blocks_sampling(state) {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "settle the original construction clock before sampling the room",
            ));
        }
    }
    if state.room_clock_config.is_some() || state.room_clock.is_some() {
        let context = ledger.borrow_mut().take(&current, room_clock::VIEW_BYTES)?;
        let checked = if sampling {
            room_clock::ensure_for_joint_sample(state, &context)
        } else {
            room_clock::inspect(state, &context).map(|_| ())
        };
        floor.set(floor.get().max(state.authority_tick));
        checked?;
    }
    current.anchor.tick = GameTick(
        current
            .anchor
            .tick
            .get()
            .max(floor.get())
            .max(state.authority_tick),
    );
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let context = ledger
        .borrow_mut()
        .take(&current, completion::source_reserve(origin)?)?;
    let view = completion::verify_source(
        &mut state.control,
        &mut state.batch,
        &state.binding,
        origin,
        identities,
        &context,
    )?;
    state.authority_tick = state.authority_tick.max(state.control.high_tick());
    floor.set(floor.get().max(state.authority_tick));
    state.historical = Some(view);
    current.anchor.tick = GameTick(current.anchor.tick.get().max(floor.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    io.borrow_mut().checkpoint(&current)
}

pub(super) fn run<N: BuildSource, I: Io>(
    state: &mut State<PrivateBuildFile, N>,
    config: &Config,
    c: OperationContext,
    op: &str,
    action: Action,
    started: Instant,
    io: &mut I,
) -> String {
    state.abandon_all();
    if !matches!(action, Action::Cancel)
        && state.room_clock_config.is_some()
        && state.room_clock.is_none()
        && !room_clock::bootstrap_pending(state)
    {
        if let Err(e) = room_clock::admit_bootstrap(state, &c) {
            return completion::read_only_packet(state.failed(op, Some(&c), &e));
        }
    }
    if matches!(action, Action::Sample)
        && room_clock::bootstrap_pending(state)
        && state
            .room_completion
            .as_ref()
            .is_some_and(|m| m.store.first_source().is_some())
    {
        return completion::read_only_packet(state.failed(op,Some(&c),&error(ErrorCode::EffectIndeterminate,
            "open the original room clock through explicit planning before another joint acquisition")));
    }
    let sampling = matches!(action, Action::Sample);
    let cancelling = matches!(action, Action::Cancel);
    let floor = Cell::new(state.authority_tick);
    let mut monitor = state.room_completion.take();
    let outcome = (|| -> Result<(Value, Publication)> {
        let ledger = RefCell::new(Work::new(&c, started)?);
        let io = RefCell::new(&mut *io);
        current(&c, floor.get())?;
        io.borrow_mut().checkpoint(&c)?;
        let before = state.control.inventory(&ledger.borrow().view(&c)?);
        let before = match before {
            Ok(view) => {
                state.historical = Some(view.clone());
                Some(view)
            }
            Err(_) if cancelling => None,
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
            let definition = Definition::new(MonitorDefinition::new(origin, goal)?)?;
            room_intake::validate(definition.room(), config)?;
            reserve_output(state, &c, view, &definition)?;
            if let Some(existing) = &monitor {
                if existing.store.definition().id() != definition.id() {
                    return Err(error(
                        ErrorCode::Conflict,
                        "retained joint room goal cannot be replaced or renewed",
                    ));
                }
            } else {
                let path = config.room_completion_path.as_ref().ok_or_else(missing)?;
                if state.room_recovery.is_some() {
                    return Err(error(
                        ErrorCode::Conflict,
                        "reopen the original room monitor with its retained recovery identity",
                    ));
                }
                match std::fs::symlink_metadata(path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    _ => {
                        return Err(error(
                            ErrorCode::Conflict,
                            "joint room history already exists; reopen exact custody",
                        ));
                    }
                }
                let header = definition.canonical_bytes().len() as u64;
                // All publication work is admitted before the first local write.
                let needed = open_reserve(path)?
                    .checked_add(8 * header)
                    .and_then(|n| {
                        n.checked_add(2 * completion::source_reserve(definition.origin()).ok()?)
                    })
                    .and_then(|n| n.checked_add(header + 32768 + 2 * clock::VIEW_BYTES))
                    .ok_or_else(exhausted)?;
                if ledger.borrow().bytes < needed {
                    return Err(exhausted());
                }
                state.room_recovery = Some(initial_recovery(&definition)?);
                io.borrow_mut().checkpoint(&ledger.borrow().current(&c)?)?;
                let opening = ledger
                    .borrow_mut()
                    .take(&c, open_reserve(path)? + 8 * header)?;
                let store = open_private_monitor(path, &opening, config.mode, Some(definition))?;
                state.room_recovery = Some(store.recovery());
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
        floor.set(floor.get().max(monitor.store.authority_tick_floor()));
        let c = current(&c, floor.get())?;
        let growth = if sampling {
            MAX_SAMPLE as u64 + 128 * 1024
        } else {
            8192
        };
        let final_bytes = (monitor.store.byte_len() as u64)
            .checked_add(growth)
            .ok_or_else(exhausted)?;
        let mut publication = Publication::reserve_for(
            state,
            &c,
            &mut ledger.borrow_mut(),
            monitor.store.definition(),
            final_bytes,
            monitor.store.head(),
        )?;
        let mut template_result = json!({"ok":true});
        state.attach_batch(&mut template_result, before.as_ref(), false);
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
        let owner = RefCell::new(&mut *state);
        let parent_identity = monitor.parent.clone();
        let child_identity = monitor.child.clone();
        let mut guard = |origin: &Origin, context: &OperationContext| -> Result<()> {
            check_origin(
                &mut **owner.borrow_mut(),
                origin,
                (&parent_identity, &child_identity),
                context,
                &ledger,
                &io,
                sampling,
                &floor,
            )
        };
        let mut admit = |definition: &Definition, progress: &Progress| -> Result<()> {
            let context = current(
                &ledger.borrow().current(&c)?,
                floor.get().max(progress.observed_tick_floor),
            )?;
            io.borrow_mut().checkpoint(&context)?;
            let mut candidate: Value = serde_json::from_str(&template).map_err(|_| corrupt())?;
            candidate["result"]["room_completion"] = projected(definition, progress, true);
            if candidate
                .to_string()
                .len()
                .checked_add(8192)
                .is_none_or(|n| n > OUTPUT_BYTES as usize)
            {
                return Err(exhausted());
            }
            Ok(())
        };
        let verify = ledger
            .borrow_mut()
            .take(&c, monitor.store.byte_len() as u64 + 8192)?;
        monitor.store.verify(&verify)?;
        monitor.store_verified = true;
        let origin_context = ledger.borrow().current(&c)?;
        let origin = guard(monitor.store.definition().origin(), &origin_context);
        if !cancelling {
            origin?;
        }
        let mut acquisitions = 0;
        let mut accepted = false;
        match action {
            Action::Start(_) | Action::Inspect => {}
            Action::Sample if monitor.store.progress().terminal() => {}
            Action::Sample => {
                if config.mode == BuildMode::Offline {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "offline joint room monitor cannot acquire evidence",
                    ));
                }
                let cells = monitor
                    .store
                    .definition()
                    .room()
                    .terrain()
                    .region()
                    .volume()
                    .map_err(|_| corrupt())?;
                if cells
                    .saturating_mul(2)
                    .saturating_add(monitor.store.definition().goal().records().len())
                    > c.budget.max_entities as usize
                {
                    return Err(exhausted());
                }
                let begin_bytes = monitor.store.operation_reserve(false);
                let publish_bytes = monitor.store.operation_reserve(true);
                let source_bytes = 32 * 1024 * 1024;
                let decode_bytes = 64 * 1024 * 1024;
                let each_guard = completion::source_reserve(monitor.store.definition().origin())?
                    .checked_add(if owner.borrow().clock_config.is_some() {
                        clock::VIEW_BYTES
                    } else {
                        0
                    })
                    .and_then(|n| {
                        n.checked_add(if owner.borrow().room_clock_config.is_some() {
                            room_clock::VIEW_BYTES
                        } else {
                            0
                        })
                    })
                    .ok_or_else(exhausted)?;
                let required = begin_bytes
                    .checked_add(publish_bytes)
                    .and_then(|n| n.checked_add(source_bytes + decode_bytes))
                    .and_then(|n| {
                        each_guard
                            .checked_mul(12)
                            .and_then(|guards| n.checked_add(guards))
                    })
                    .ok_or_else(exhausted)?;
                if ledger.borrow().bytes < required {
                    return Err(exhausted());
                }
                let begin = ledger.borrow_mut().take(&c, begin_bytes)?;
                monitor.store.begin_read(&begin, &mut admit, &mut guard)?;
                let source = current(&ledger.borrow_mut().take(&c, source_bytes)?, floor.get())?;
                acquisitions = 1;
                let acquired = io.borrow_mut().sample(
                    &owner.borrow().binding,
                    monitor.store.definition(),
                    &source,
                );
                floor.set(floor.get().max(io.borrow().observed_tick_floor()));
                monitor.store.retain_observed_tick_floor(floor.get());
                let sample = acquired?;
                let decode = ledger.borrow_mut().take(&c, decode_bytes)?;
                let mut decoded = floor.get();
                let retained = sample.retain_observed_tick(&mut decoded);
                floor.set(decoded);
                monitor.store.retain_observed_tick_floor(decoded);
                retained?;
                current(&decode, decoded)?;
                let publish = current(&ledger.borrow_mut().take(&c, publish_bytes)?, decoded)?;
                monitor
                    .store
                    .publish_sample(&publish, &sample, &mut admit, &mut guard)?;
                accepted = monitor.store.progress().phase != "invalidated";
            }
            Action::Cancel => {
                let cancel = current(
                    &ledger
                        .borrow_mut()
                        .take(&c, monitor.store.operation_reserve(false))?,
                    floor.get(),
                )?;
                monitor.store.cancel(&cancel, &mut admit)?;
            }
        }
        floor.set(floor.get().max(monitor.store.authority_tick_floor()));
        let final_context = ledger.borrow().current(&c)?;
        let checked = guard(monitor.store.definition().origin(), &final_context);
        monitor.verified = checked.is_ok();
        if !cancelling {
            checked?;
        }
        current(&ledger.borrow().current(&c)?, floor.get())?;
        publication.head = monitor.store.head();
        Ok((
            json!({"ok":true,"scope":"room_completion","native_acquisitions":acquisitions,
            "fresh_joint_sample_acquired":accepted,"game_mutation_dispatched":false,
            "effect_may_have_occurred":false,"placement_effect_discharged":false,
            "effects_cancelled":false,"building_undone":false,"retry_commit_permitted":false}),
            publication,
        ))
    })();
    state.authority_tick = state.authority_tick.max(floor.get());
    if let Some(monitor) = &monitor {
        state.authority_tick = state
            .authority_tick
            .max(monitor.store.authority_tick_floor());
        state.room_recovery = Some(monitor.store.recovery());
    }
    state.room_completion = monitor;
    let (result, publication) = match outcome {
        Ok(value) => value,
        Err(e) => return completion::read_only_packet(state.failed(op, Some(&c), &e)),
    };
    let current = match state.disclosure_context(&c) {
        Ok(c) => c,
        Err(e) => return completion::read_only_packet(state.failed(op, Some(&c), &e)),
    };
    // Admit the complete packet before the final caller checkpoint. A local
    // cancellation can then tolerate original-source loss without publishing
    // the earlier positive custody flags: its final packet is rebuilt below.
    if render(state, &current, op, &result).len() as u64 > OUTPUT_BYTES {
        return completion::read_only_packet(state.failed(op, Some(&current), &exhausted()));
    }
    let expected = state.clock_recovery;
    if let Err(e) = publication.verify(state, expected, sampling, cancelling, |c| io.checkpoint(c))
    {
        return completion::read_only_packet(state.failed(op, Some(&current), &e));
    }
    let current = match publication.current(state, &publication.monitor) {
        Ok(c) => c,
        Err(e) => return completion::read_only_packet(state.failed(op, Some(&current), &e)),
    };
    let output = render(state, &current, op, &result);
    if output.len() as u64 > OUTPUT_BYTES
        || publication.current(state, &publication.monitor).is_err()
    {
        return completion::read_only_packet(state.failed(op, Some(&current), &exhausted()));
    }
    output
}

fn render<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
    c: &OperationContext,
    op: &str,
    seed: &Value,
) -> String {
    let mut result = seed.clone();
    let verified = state.room_completion.as_ref().is_some_and(|m| m.verified);
    state.attach_batch(&mut result, state.historical.as_ref(), verified);
    result["room_completion"]["fresh_joint_sample_acquired"] =
        json!(verified && result["fresh_joint_sample_acquired"] == true);
    packet(
        op,
        result,
        Some(c),
        Some(&state.binding),
        if verified {
            state.historical.as_ref()
        } else {
            None
        },
        state.historical.as_ref(),
        Some(&state.policy),
        None,
        None,
    )
}
