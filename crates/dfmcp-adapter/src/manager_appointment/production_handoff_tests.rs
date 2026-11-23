//! Active-donor exports use actual private journal/ACK custody and one bounded
//! native-source double. These tests establish no live DFHack execution.
use super::*;

#[test]
fn active_handoff_is_read_only_and_matches_later_exact_offline_import() -> Result<()> {
    for mode in [Mode::Control, Mode::Recover] {
        let mut donor = Donor::from_capture(fixture_capture()?, true, mode)?;
        let bytes = donor.bytes()?;
        donor.context.grants.retain(|grant| {
            matches!(grant.capability, Capability::Query | Capability::Observe)
        });
        let capture = at(donor.after()?, donor.context.anchor.tick.get() + 10)?;
        let mut source = Source::new(donor.binding.clone(), capture.clone());
        let mut retained = None;
        let handoff = donor.journal.export_production_handoff(
            &mut donor.ack,
            &mut source,
            &mut retained,
            &donor.context,
        )?;
        assert_eq!((source.reads, source.fences), (1, 1));
        assert_eq!(handoff.capture(), &capture);
        assert_eq!(handoff.plan(), &donor.original.plan);
        assert_eq!(handoff.original_seal(), donor.original.seal());
        assert_eq!(handoff.floor(), retained.ok_or_else(corrupt)?);
        assert_eq!(handoff.floor().tick, capture.tick);
        assert_eq!(
            handoff.origin_witness(),
            Digest32::of_bytes(handoff.origin_canonical_bytes())
        );
        assert!(!donor.journal.dispatchable());
        assert_eq!(donor.bytes()?, bytes);
        let pin = handoff.pin();
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
        let mut imported = donor.import()?;
        assert_eq!(imported.canonical_bytes(), handoff.origin_canonical_bytes());
        assert_eq!(imported.receipt(), handoff.receipt());
        assert_eq!(imported.witness(), handoff.origin_witness());
        // Import never inherits the export's fresh sample. Only its rejection
        // frontier may be retained before the later owner's independent read.
        imported.retain_recovery_floor(handoff.floor(), &donor.context)?;
        assert_eq!(imported.floor(), handoff.floor());
        assert_eq!(donor.bytes()?, bytes);
    }
    Ok(())
}

#[test]
fn handoff_refuses_legacy_offline_and_forged_pending_state_before_source_contact() -> Result<()> {
    for which in 0..4 {
        let mut donor = match which {
            0 => Donor::from_capture(native_capture()?, true, Mode::Control)?,
            1 => Donor::from_capture(fixture_capture()?, true, Mode::Offline)?,
            _ => Donor::from_capture(fixture_capture()?, false, Mode::Control)?,
        };
        if which == 3 {
            donor.journal.abandon();
            donor.journal.state = State::Terminal;
            donor.journal.native = Some(record(&donor.original.plan, true)?);
            donor.journal.effect_uncertain = false;
        }
        let bytes = donor.bytes()?;
        let mut source = Source::new(donor.binding.clone(), donor.after()?);
        let mut retained = None;
        assert!(
            donor.journal.export_production_handoff(
                &mut donor.ack,
                &mut source,
                &mut retained,
                &donor.context,
            ).is_err(),
            "case {which}"
        );
        assert_eq!((source.reads, source.fences), (0, 1));
        assert!(retained.is_none());
        assert_eq!(donor.bytes()?, bytes);
    }
    Ok(())
}

#[test]
fn handoff_distinguishes_temporary_unavailability_from_sticky_identity_loss() -> Result<()> {
    for which in 0..4 {
        let mut donor = Donor::from_capture(fixture_capture()?, true, Mode::Control)?;
        let bytes = donor.bytes()?;
        let mut capture = donor.after()?;
        match which {
            0 => capture.paused = false,
            1 => capture.group.role_buckets[MANAGE_PRODUCTION].clear(),
            2 => capture.dual_getters = false,
            _ => capture.generation += 1,
        }
        let capture = at(capture, donor.context.anchor.tick.get() + 5)?;
        let mut source = Source::new(donor.binding.clone(), capture.clone());
        let mut retained = None;
        assert!(
            donor.journal.export_production_handoff(
                &mut donor.ack,
                &mut source,
                &mut retained,
                &donor.context,
            ).is_err()
        );
        assert_eq!(source.reads, 1);
        let floor = retained.ok_or_else(corrupt)?;
        assert_eq!(floor.tick, capture.tick);
        assert_eq!(floor.fenced, which >= 2);
        let restored = at(donor.after()?, capture.tick + 1)?;
        let mut source = Source::new(donor.binding.clone(), restored);
        let again = donor.journal.export_production_handoff(
            &mut donor.ack,
            &mut source,
            &mut retained,
            &donor.context,
        );
        assert_eq!(again.is_ok(), which < 2);
        assert_eq!(source.reads, usize::from(which < 2));
        assert_eq!(donor.bytes()?, bytes);
    }
    Ok(())
}

#[test]
fn handoff_retains_all_decoded_floors_after_late_source_or_authority_failure() -> Result<()> {
    for read_fails in [false, true] {
        let mut donor = Donor::from_capture(fixture_capture()?, true, Mode::Control)?;
        let bytes = donor.bytes()?;
        let mut capture = donor.after()?;
        capture.sequence += 2;
        capture.next_event += 2;
        capture.history_count += 2;
        capture.group.next_position += 2;
        capture.group.next_assignment += 2;
        capture.civ.next_position += 2;
        capture.civ.next_assignment += 2;
        let capture = at(capture, donor.context.anchor.tick.get() + 5)?;
        let expected = ManagerProductionFloor::capture(&capture);
        let mut source = Source::new(donor.binding.clone(), capture);
        source.fail = read_fails;
        let mut context = donor.context.clone();
        for grant in &mut context.grants {
            grant.expires_at_tick = Some(GameTick(context.anchor.tick.get() + 1));
        }
        let mut retained = None;
        assert!(
            donor.journal.export_production_handoff(
                &mut donor.ack,
                &mut source,
                &mut retained,
                &context,
            ).is_err()
        );
        assert_eq!((source.reads, source.fences), (1, 1));
        let mut floor = retained.ok_or_else(corrupt)?;
        assert_eq!(floor.fenced, read_fails);
        floor.fenced = false;
        assert_eq!(floor, expected);
        let mut retry = Source::new(
            donor.binding.clone(),
            at(donor.after()?, expected.tick + 1)?,
        );
        assert!(
            donor.journal.export_production_handoff(
                &mut donor.ack,
                &mut retry,
                &mut retained,
                &context,
            ).is_err()
        );
        assert_eq!(retry.reads, 0);
        assert_eq!(donor.bytes()?, bytes);
    }
    Ok(())
}

#[test]
fn handoff_corruption_before_or_during_read_is_not_repaired_or_published() -> Result<()> {
    for during_read in [false, true] {
        let mut donor = Donor::from_capture(fixture_capture()?, true, Mode::Control)?;
        let path = donor.directory.0.join(donor.binding.protocol.ack_name());
        let mut corrupted = std::fs::read(&path).map_err(|_| corrupt())?;
        corrupted.push(0);
        let mut source = Source::new(donor.binding.clone(), donor.after()?);
        if during_read {
            let changed = corrupted.clone();
            source.hook = Some(Box::new(move || {
                std::fs::write(&path, &changed).map_err(|_| corrupt())
            }));
        } else {
            std::fs::write(&path, &corrupted).map_err(|_| corrupt())?;
        }
        let mut retained = None;
        let result = donor.journal.export_production_handoff(
            &mut donor.ack,
            &mut source,
            &mut retained,
            &donor.context,
        );
        assert_eq!(
            result.err().map(|cause| cause.code),
            Some(ErrorCode::CorruptLedger)
        );
        assert_eq!(source.reads, usize::from(during_read));
        assert_eq!(donor.bytes()?.1, corrupted);
        if during_read {
            assert!(retained.ok_or_else(corrupt)?.fenced);
        }
    }
    Ok(())
}

#[test]
fn handoff_authority_scope_budget_and_source_are_checked_before_any_read() -> Result<()> {
    for which in 0..5 {
        let mut donor = Donor::from_capture(fixture_capture()?, true, Mode::Control)?;
        let bytes = donor.bytes()?;
        let mut context = donor.context.clone();
        let mut source = Source::new(donor.binding.clone(), donor.after()?);
        match which {
            0 => context.budget.max_bytes = EXPORT_WORK_BYTES - 1,
            1 => context.grants.retain(|grant| grant.capability != Capability::Observe),
            2 => context.session_id = SessionId::new(999),
            3 => context.cancellation_requested = true,
            _ => source.binding.source = FortressIdentity::new("foreign-fortress", 2)?,
        }
        let mut retained = None;
        assert!(
            donor.journal.export_production_handoff(
                &mut donor.ack,
                &mut source,
                &mut retained,
                &context,
            ).is_err(),
            "case {which}"
        );
        assert_eq!((source.reads, source.fences), (0, 1));
        assert_eq!(donor.bytes()?, bytes);
    }
    Ok(())
}

#[test]
fn handoff_floor_rejection_never_erases_higher_external_counters_or_fences() -> Result<()> {
    let mut donor = Donor::from_capture(fixture_capture()?, true, Mode::Control)?;
    let mut floor = ManagerProductionFloor::capture(&donor.after()?);
    floor.sequence += 10;
    floor.next_event -= 1;
    floor.fenced = true;
    let mut retained = Some(floor);
    let mut source = Source::new(donor.binding.clone(), donor.after()?);
    assert!(
        donor.journal.export_production_handoff(
            &mut donor.ack,
            &mut source,
            &mut retained,
            &donor.context,
        ).is_err()
    );
    let saved = retained.ok_or_else(corrupt)?;
    assert!(saved.fenced);
    assert_eq!(saved.sequence, floor.sequence);
    assert_eq!(saved.next_event, donor.after()?.next_event);
    assert_eq!(source.reads, 0);
    Ok(())
}
