//! Real private donor custody and canonical native doubles, never live DF proof.
use super::super::journal_tests::{Directory, binding, context};
use super::*;
use crate::manager_appointment::tests::native_capture;
use crate::workshop_construction::acknowledgement::open_private_manager_acknowledgements_for_protocol;
use dfmcp_core::{MapCoord, MapCuboid};

fn fixture_capture() -> Result<Capture> {
    let text = include_str!("../../../../tests/fixtures/manager_v1_23_component.json")
        .lines()
        .find_map(|line| line.trim().strip_prefix("\"capture\": \""))
        .and_then(|value| value.split('"').next())
        .ok_or_else(corrupt)?;
    let raw = text
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| corrupt())?;
            u8::from_str_radix(pair, 16).map_err(|_| corrupt())
        })
        .collect::<Result<Vec<_>>>()?;
    Capture::decode(&raw)
}
fn canonical(capture: Capture) -> Result<Capture> {
    Capture::decode(&capture.encode())
}
fn at(mut capture: Capture, tick: u64) -> Result<Capture> {
    capture.tick = tick;
    capture.year = (tick / 403200) as u32;
    capture.year_tick = (tick % 403200) as u32;
    canonical(capture)
}
fn record(plan: &Plan, complete: bool) -> Result<Record> {
    let mut bytes = plan.before.protocol.magic(b'R').to_vec();
    field(&mut bytes, plan.key().as_bytes());
    bytes.extend_from_slice(plan.digest.as_bytes());
    bytes.extend_from_slice(&plan.token);
    blob(&mut bytes, plan.before.bytes());
    bytes.extend([if complete { 2 } else { 0 }, 0, u8::from(complete)]);
    if complete {
        blob(&mut bytes, plan.before.expected_after()?.bytes());
    }
    bytes.push(u8::from(complete));
    if complete {
        bytes.extend_from_slice(&plan.before.protocol.magic(b'I'));
        for value in [
            plan.before.selection.unit,
            plan.before.selection.figure,
            plan.before.group.id as u32,
            plan.before.target().ok_or_else(corrupt)?.1.id as u32,
            plan.before.next_event,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend([1, 1]);
    }
    let receipt = hash(b"dfmcp-manager-receipt/1", &bytes);
    bytes.extend_from_slice(receipt.as_bytes());
    Record::decode(&bytes, plan)
}
struct Donor {
    directory: Directory,
    journal: Journal<PrivateBuildFile>,
    ack: WorkshopAcknowledgements<PrivateBuildFile>,
    context: OperationContext,
    binding: Binding,
    original: Original,
}
impl Donor {
    fn new() -> Result<Self> {
        Self::from_capture(fixture_capture()?, true, Mode::Offline)
    }
    fn from_capture(before: Capture, complete: bool, mode: Mode) -> Result<Self> {
        let directory = Directory::new()?;
        let context = context()?;
        let mut binding = binding("127.0.0.1:5000".parse().map_err(|_| corrupt())?)?;
        binding.protocol = before.protocol;
        let plan = Plan::new("manager-production-original", before)?;
        let original = Original::new(plan.clone(), plan.before.tick + 12000)?;
        let (mut journal, mut ack) = open_private(
            &directory.0,
            &context,
            Mode::Control,
            &binding,
            binding.selection,
            true,
            None,
            [7; 32],
        )?;
        journal.intent(original.clone(), &context, &mut ack)?;
        journal.prepared(record(&plan, false)?, &context, &mut ack)?;
        if complete {
            journal.dispatch(
                original.seal(),
                &context,
                &mut ack,
                &mut |_| Ok(()),
                |_, _| record(&plan, true),
            )?;
        }
        if mode != Mode::Control {
            let pin = journal.pin();
            drop(journal);
            drop(ack);
            (journal, ack) = open_private(
                &directory.0,
                &context,
                mode,
                &binding,
                binding.selection,
                false,
                Some(pin),
                [0; 32],
            )?;
        }
        Ok(Self {
            directory,
            journal,
            ack,
            context,
            binding,
            original,
        })
    }
    fn import(&mut self) -> Result<ManagerProductionOrigin> {
        self.journal.production_origin(
            &mut self.ack,
            self.original.plan.key(),
            self.original.plan.digest,
            &self.context,
        )
    }
    fn bytes(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            std::fs::read(self.directory.0.join(self.binding.protocol.journal_name()))
                .map_err(|_| corrupt())?,
            std::fs::read(self.directory.0.join(self.binding.protocol.ack_name()))
                .map_err(|_| corrupt())?,
        ))
    }
    fn after(&self) -> Result<Capture> {
        self.original.plan.before.expected_after()
    }
    fn refresh(
        &mut self,
        origin: &mut ManagerProductionOrigin,
        capture: Capture,
    ) -> Result<ReadyManagerProduction> {
        let mut context = self.context.clone();
        context.anchor.tick = GameTick(capture.tick);
        let mut source = Source::new(self.binding.clone(), capture);
        let result = origin.refresh(
            &mut self.journal,
            &mut self.ack,
            &mut source,
            context.anchor.tick,
            &context,
        );
        assert_eq!(source.fences, 1);
        result
    }
}
struct Source {
    binding: Binding,
    capture: Capture,
    reads: usize,
    fences: usize,
    fail: bool,
    observed: Option<ManagerProductionFloor>,
    hook: Option<Box<dyn FnMut() -> Result<()>>>,
}
impl Source {
    fn new(binding: Binding, capture: Capture) -> Self {
        Self {
            binding,
            capture,
            reads: 0,
            fences: 0,
            fail: false,
            observed: None,
            hook: None,
        }
    }
}
impl ManagerReadSource for Source {
    fn binding(&self) -> &Binding {
        &self.binding
    }
    fn observed_generation(&self) -> Option<u64> {
        Some(self.binding.generation)
    }
    fn observed_floor(&self) -> Option<ManagerProductionFloor> {
        self.observed
    }
    fn read_manager(&mut self, _: &OperationContext) -> Result<Capture> {
        self.reads += 1;
        self.observed = Some(ManagerProductionFloor::capture(&Capture::decode(
            self.capture.bytes(),
        )?));
        if let Some(hook) = self.hook.as_mut() {
            hook()?;
        }
        if self.fail {
            Err(denied("injected late reader refusal"))
        } else {
            Ok(self.capture.clone())
        }
    }
    fn fence(&mut self) {
        self.fences += 1;
    }
}

#[test]
fn import_is_compact_immutable_offline_and_does_not_append() -> Result<()> {
    let mut donor = Donor::new()?;
    let bytes = donor.bytes()?;
    donor
        .context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut origin = donor.import()?;
    assert!(origin.historical_record().complete());
    assert_eq!(origin.plan(), &donor.original.plan);
    assert_eq!(origin.original_seal(), donor.original.seal());
    assert!(origin.canonical_bytes().len() <= DESCRIPTOR_MAX_BYTES);
    let descriptor = origin.canonical_bytes().to_vec();
    let witness = origin.witness();
    origin.verify(&mut donor.journal, &mut donor.ack, &donor.context)?;
    let pin = origin.pin();
    drop(donor.journal);
    drop(donor.ack);
    donor.context.session_id = SessionId::new(77);
    donor.context.anchor.cursor.epoch += 1;
    (donor.journal, donor.ack) = open_private(
        &donor.directory.0,
        &donor.context,
        Mode::Offline,
        &donor.binding,
        donor.binding.selection,
        false,
        Some(pin),
        [0; 32],
    )?;
    let reopened = donor.import()?;
    assert_eq!(reopened.canonical_bytes(), descriptor);
    assert_eq!(reopened.witness(), witness);
    assert_eq!(reopened.floor(), origin.floor());
    assert_eq!(donor.bytes()?, bytes);
    assert!(
        origin
            .verify(&mut donor.journal, &mut donor.ack, &donor.context)
            .is_err()
    );
    Ok(())
}

#[test]
fn import_rejects_legacy_mutating_unresolved_and_forged_diagnostics() -> Result<()> {
    for mode in [Mode::Control, Mode::Recover] {
        let mut donor = Donor::from_capture(fixture_capture()?, true, mode)?;
        assert!(donor.import().is_err());
    }
    let mut legacy = Donor::from_capture(native_capture()?, true, Mode::Offline)?;
    assert!(legacy.import().is_err());
    let mut pending = Donor::from_capture(fixture_capture()?, false, Mode::Offline)?;
    pending.journal.state = State::Terminal;
    pending.journal.native = Some(record(&pending.original.plan, true)?);
    pending.journal.effect_uncertain = false;
    assert!(pending.import().is_err());
    let mut donor = Donor::new()?;
    assert!(
        donor
            .journal
            .production_origin(
                &mut donor.ack,
                "replacement-key",
                donor.original.plan.digest,
                &donor.context
            )
            .is_err()
    );
    assert!(
        donor
            .journal
            .production_origin(
                &mut donor.ack,
                donor.original.plan.key(),
                Digest32::ZERO,
                &donor.context
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn equal_sequence_and_unrelated_catalog_roster_history_and_cache_growth_are_ready() -> Result<()> {
    let mut donor = Donor::new()?;
    let mut origin = donor.import()?;
    let descriptor = origin.canonical_bytes().to_vec();
    let mut current = donor.after()?;
    current.group.index += 2;
    current.civ.index += 2;
    current.manager_index += 2;
    current.group.positions_digest = Digest32::of_bytes(b"unrelated group position append");
    current.civ.positions_digest = Digest32::of_bytes(b"unrelated civ position append");
    current.roster_digest = Digest32::of_bytes(b"unrelated arrival");
    current.history_digest = Digest32::of_bytes(b"unrelated later event");
    current.entity_domain_digest = Digest32::of_bytes(b"unrelated entity append");
    current.next_event += 1;
    current.history_count += 1;
    current.group.next_position += 1;
    current.civ.next_position += 1;
    let mut unrelated = current.group.assignments[0].clone();
    unrelated.id += 1;
    current.group.assignments.insert(1, unrelated.clone());
    current.group.next_assignment += 1;
    current.civ.assignments.push(unrelated.clone());
    current.civ.next_assignment = current.civ.next_assignment.max(unrelated.id + 1);
    current.group.role_buckets[MANAGE_PRODUCTION].push(unrelated.id);
    current.civ.role_buckets[MANAGE_PRODUCTION].push(unrelated.id);
    let (index, assignment) = current.target().ok_or_else(corrupt)?;
    let id = assignment.id;
    current.group.assignments[index].cache = current.manager_index;
    current.group.assignments[index].flags[0] ^= 0x80;
    let link = current
        .links
        .iter_mut()
        .find(|link| link.kind == 10 && link.assignment == id)
        .ok_or_else(corrupt)?;
    link.entity_cache = current.group.index;
    link.assignment_cache = index as i32;
    current.expedition_leader = false;
    let current = at(current, origin.high_tick() + 5)?;
    let ready = donor.refresh(&mut origin, current.clone())?;
    assert_eq!(ready.capture(), &current);
    assert_eq!(ready.origin_witness(), origin.witness());
    assert_eq!(ready.anchor().tick.get(), current.tick);
    let again = donor.refresh(&mut origin, current.clone())?;
    assert_eq!(again.witness(), ready.witness());
    assert_eq!(origin.floor().sequence, current.sequence);
    assert_eq!(origin.canonical_bytes(), descriptor);
    let mut other_anchor = donor.context.clone();
    other_anchor.anchor.tick = GameTick(current.tick);
    other_anchor.anchor.cursor.sequence += 1;
    let mut source = Source::new(donor.binding.clone(), current);
    let other = origin.refresh(
        &mut donor.journal,
        &mut donor.ack,
        &mut source,
        other_anchor.anchor.tick,
        &other_anchor,
    )?;
    assert_ne!(other.witness(), ready.witness());
    Ok(())
}

#[test]
fn cache_and_availability_withdrawal_refuse_without_minting_or_sticky_identity_loss() -> Result<()>
{
    for which in 0..7 {
        let mut donor = Donor::new()?;
        let mut origin = donor.import()?;
        let mut current = donor.after()?;
        match which {
            0 => current.group.role_buckets[MANAGE_PRODUCTION].clear(),
            1 => current.paused = false,
            2 => current.ui_quiescent = false,
            3 => current.active = false,
            4 => current.citizen = false,
            5 => current.adult = false,
            _ => current.sane = false,
        }
        let current = at(current, origin.high_tick() + 1)?;
        assert_eq!(
            donor
                .refresh(&mut origin, current)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::Conflict),
            "readiness mutation {which}"
        );
        assert!(!origin.is_fenced());
        let restored = at(donor.after()?, origin.high_tick() + 1)?;
        donor.refresh(&mut origin, restored)?;
    }
    Ok(())
}

#[test]
fn original_assignment_links_getters_and_current_cache_indices_are_not_substitutable() -> Result<()>
{
    for which in 0..15 {
        let mut donor = Donor::new()?;
        let mut origin = donor.import()?;
        let mut current = donor.after()?;
        let (index, assignment) = current.target().ok_or_else(corrupt)?;
        let id = assignment.id;
        let link = current
            .links
            .iter()
            .position(|link| link.kind == 10 && link.assignment == id)
            .ok_or_else(corrupt)?;
        match which {
            0 => current.identity = false,
            1 => current.member = false,
            2 => current.role_eligible = false,
            3 => current.dual_getters = false,
            4 => current.lookup_unit = -1,
            5 => current.manager_links = 0,
            6 => current.group.assignments[index].figure += 1,
            7 => current.group.assignments[index].last_figure += 1,
            8 => current.group.assignments[index].cache += 1,
            9 => current.group.assignments[index].temp += 1,
            10 => current.links[link].assignment_cache += 1,
            11 => current.links[link].entity_cache += 1,
            12 => current.links[link].start_year += 1,
            13 => current.links[link].strength -= 1,
            _ => current.group.id += 1,
        }
        let current = at(current, origin.high_tick() + 1)?;
        assert!(
            donor.refresh(&mut origin, current).is_err(),
            "identity mutation {which}"
        );
        assert!(origin.is_fenced());
        let restored = at(donor.after()?, origin.high_tick() + 1)?;
        assert!(donor.refresh(&mut origin, restored).is_err());
        origin.verify(&mut donor.journal, &mut donor.ack, &donor.context)?;
    }
    Ok(())
}

#[test]
fn decoded_counter_regression_fences_and_retains_every_other_higher_floor() -> Result<()> {
    for which in 0..8 {
        let mut donor = Donor::new()?;
        let mut origin = donor.import()?;
        let mut first = donor.after()?;
        first.sequence += 10;
        first.next_event += 10;
        first.history_count += 10;
        first.group.next_position += 10;
        first.group.next_assignment += 10;
        first.civ.next_position += 10;
        first.civ.next_assignment += 10;
        let first = at(first, origin.high_tick() + 10)?;
        donor.refresh(&mut origin, first.clone())?;
        let mut regressed = first;
        match which {
            0 => {
                regressed.tick -= 1;
                regressed.year_tick -= 1;
            }
            1 => regressed.sequence -= 1,
            2 => regressed.next_event -= 1,
            3 => regressed.history_count -= 1,
            4 => regressed.group.next_position -= 1,
            5 => regressed.group.next_assignment -= 1,
            6 => regressed.civ.next_position -= 1,
            _ => regressed.civ.next_assignment -= 1,
        }
        // Send at a nondecreasing host anchor; the decoded older source is the
        // failure, not the caller's earlier pre-read tick check.
        let mut context = donor.context.clone();
        context.anchor.tick = GameTick(origin.high_tick() + 1);
        let mut source = Source::new(donor.binding.clone(), canonical(regressed)?);
        assert!(
            origin
                .refresh(
                    &mut donor.journal,
                    &mut donor.ack,
                    &mut source,
                    context.anchor.tick,
                    &context
                )
                .is_err()
        );
        assert_eq!(source.reads, 1);
        assert!(origin.is_fenced(), "counter {which}");
        assert_eq!(
            origin.floor().history_count,
            donor.after()?.history_count + 10
        );
    }
    Ok(())
}

#[test]
fn late_read_error_and_authority_failure_preserve_all_decoded_floors() -> Result<()> {
    for fail_reader in [false, true] {
        let mut donor = Donor::new()?;
        let mut origin = donor.import()?;
        let mut capture = donor.after()?;
        capture.sequence += 2;
        capture.next_event += 2;
        capture.history_count += 2;
        capture.group.next_position += 2;
        capture.group.next_assignment += 2;
        capture.civ.next_position += 2;
        capture.civ.next_assignment += 2;
        let capture = at(capture, origin.high_tick() + 5)?;
        let expected = ManagerProductionFloor::capture(&capture);
        let mut source = Source::new(donor.binding.clone(), capture);
        source.fail = fail_reader;
        let mut context = donor.context.clone();
        for grant in &mut context.grants {
            grant.expires_at_tick = Some(GameTick(origin.high_tick() + 1));
        }
        assert!(
            origin
                .refresh(
                    &mut donor.journal,
                    &mut donor.ack,
                    &mut source,
                    context.anchor.tick,
                    &context
                )
                .is_err()
        );
        let mut retained = origin.floor();
        retained.fenced = false;
        assert_eq!(retained, expected);
        assert_eq!(source.reads, 1);
        assert_eq!(origin.is_fenced(), fail_reader);
        assert!(
            origin
                .refresh(
                    &mut donor.journal,
                    &mut donor.ack,
                    &mut source,
                    context.anchor.tick,
                    &context
                )
                .is_err()
        );
        assert_eq!(source.reads, 1);
    }
    Ok(())
}

#[test]
fn current_tick_mismatch_consumes_new_floor_and_requires_a_new_matching_anchor() -> Result<()> {
    let mut donor = Donor::new()?;
    let mut origin = donor.import()?;
    let capture = at(donor.after()?, origin.high_tick() + 5)?;
    let mut source = Source::new(donor.binding.clone(), capture.clone());
    assert!(
        origin
            .refresh(
                &mut donor.journal,
                &mut donor.ack,
                &mut source,
                donor.context.anchor.tick,
                &donor.context
            )
            .is_err()
    );
    assert_eq!(origin.high_tick(), capture.tick);
    assert!(!origin.is_fenced());
    donor.refresh(&mut origin, capture)?;
    Ok(())
}

#[test]
fn prepaid_budgets_and_source_mismatch_refuse_before_native_read() -> Result<()> {
    let mut donor = Donor::new()?;
    let mut underfunded = donor.context.clone();
    underfunded.budget.max_bytes = IMPORT_WORK_BYTES - 1;
    assert_eq!(
        donor
            .journal
            .production_origin(
                &mut donor.ack,
                donor.original.plan.key(),
                donor.original.plan.digest,
                &underfunded
            )
            .err()
            .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    let mut origin = donor.import()?;
    let mut source = Source::new(donor.binding.clone(), donor.after()?);
    underfunded.budget.max_bytes = REFRESH_WORK_BYTES - 1;
    assert_eq!(
        origin
            .refresh(
                &mut donor.journal,
                &mut donor.ack,
                &mut source,
                underfunded.anchor.tick,
                &underfunded
            )
            .err()
            .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source.reads, 0);
    source.binding.generation += 1;
    assert!(
        origin
            .refresh(
                &mut donor.journal,
                &mut donor.ack,
                &mut source,
                underfunded.anchor.tick,
                &underfunded
            )
            .is_err()
    );
    assert_eq!(source.reads, 0);
    assert!(origin.is_fenced());
    assert_eq!(origin.floor().generation, source.binding.generation);
    Ok(())
}

#[test]
fn recovery_floor_cannot_lower_custody_clear_fence_or_silently_expire_grants() -> Result<()> {
    let mut donor = Donor::new()?;
    let mut origin = donor.import()?;
    let original = origin.floor();
    let mut advanced = original;
    advanced.tick += 10;
    advanced.sequence += 10;
    origin.retain_recovery_floor(advanced, &donor.context)?;
    assert!(
        origin
            .retain_recovery_floor(original, &donor.context)
            .is_err()
    );
    let mut later = advanced;
    later.generation += 1;
    let mut expired = donor.context.clone();
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(advanced.tick + 1));
    }
    later.tick += 10;
    assert!(origin.retain_recovery_floor(later, &expired).is_err());
    assert!(origin.is_fenced());
    assert_eq!(origin.floor().generation, later.generation);
    assert_eq!(origin.high_tick(), later.tick);
    origin.retain_recovery_floor(later, &donor.context)?;
    assert!(origin.is_fenced());
    let bad_connection = stale("changed handshake").with_detail(
        "consumed_manager_generation",
        (later.generation + 1).to_string(),
    );
    assert!(origin.retain_connection_error(&bad_connection).is_err());
    assert_eq!(origin.floor().generation, later.generation + 1);
    Ok(())
}

#[test]
fn donor_change_during_read_retains_capture_but_never_returns_ready() -> Result<()> {
    for ack_file in [false, true] {
        let mut donor = Donor::new()?;
        let mut origin = donor.import()?;
        let current = at(donor.after()?, origin.high_tick() + 5)?;
        let mut context = donor.context.clone();
        context.anchor.tick = GameTick(current.tick);
        let mut source = Source::new(donor.binding.clone(), current);
        let path = donor.directory.0.join(if ack_file {
            donor.binding.protocol.ack_name()
        } else {
            donor.binding.protocol.journal_name()
        });
        let original = std::fs::read(&path).map_err(|_| corrupt())?;
        source.hook = Some(Box::new(move || {
            let mut changed = original.clone();
            changed.push(0);
            std::fs::write(&path, changed).map_err(|_| corrupt())
        }));
        assert_eq!(
            origin
                .refresh(
                    &mut donor.journal,
                    &mut donor.ack,
                    &mut source,
                    context.anchor.tick,
                    &context
                )
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CorruptLedger)
        );
        assert!(origin.is_fenced());
        assert_eq!(origin.high_tick(), context.anchor.tick.get());
    }
    Ok(())
}

#[test]
fn same_pin_under_wrong_manager_ack_protocol_or_scope_cannot_substitute() -> Result<()> {
    for wrong_scope in [false, true] {
        let mut donor = Donor::new()?;
        let pin = donor.journal.pin();
        let path = donor.directory.0.join("unrelated.ack");
        let scope = if wrong_scope {
            MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(1, 1, 1))?
        } else {
            donor.binding.scope
        };
        let protocol = if wrong_scope {
            ManagerProtocol::V1_23
        } else {
            ManagerProtocol::V1_22
        };
        let mut foreign = open_private_manager_acknowledgements_for_protocol(
            &path,
            &donor.context,
            Mode::Control,
            true,
            None,
            scope,
            protocol,
        )?;
        foreign.acknowledge(
            WorkshopPin {
                journal_id: pin.journal_id,
                head: pin.journal_id,
                frames: 0,
            },
            &donor.context,
        )?;
        foreign.acknowledge(pin, &donor.context)?;
        drop(foreign);
        let mut foreign = open_private_manager_acknowledgements_for_protocol(
            &path,
            &donor.context,
            Mode::Offline,
            false,
            Some(pin),
            scope,
            protocol,
        )?;
        assert!(
            donor
                .journal
                .production_origin(
                    &mut foreign,
                    donor.original.plan.key(),
                    donor.original.plan.digest,
                    &donor.context
                )
                .is_err()
        );
        assert!(foreign.verify_workshop_offline(&donor.context).is_err());
        donor.import()?;
    }
    Ok(())
}

#[test]
fn maximum_scale_original_record_still_has_a_small_exact_descriptor() -> Result<()> {
    let mut before = fixture_capture()?;
    let mut unrelated = before.group.assignments[0].clone();
    unrelated.flags.resize(16, 0);
    unrelated.claims = (0..32).collect();
    for id in 1000..1200 {
        unrelated.id = id;
        before.group.assignments.push(unrelated.clone());
    }
    before.group.next_assignment = 1200;
    let before = canonical(before)?;
    assert!(before.bytes().len() > 32768);
    let mut donor = Donor::from_capture(before, true, Mode::Offline)?;
    let mut origin = donor.import()?;
    assert!(origin.historical_record().bytes().len() > 65536);
    assert!(origin.canonical_bytes().len() <= DESCRIPTOR_MAX_BYTES);
    let after = donor.after()?;
    donor.refresh(&mut origin, after)?;
    Ok(())
}
