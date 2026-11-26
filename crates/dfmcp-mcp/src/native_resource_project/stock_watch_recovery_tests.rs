//! Original no-order watch custody is carried by the actual parent journal and
//! its independent acknowledgement, including canonical replay and real files.
use super::*;
use crate::live_production_runtime::project_stock_watch::tests::reference_fixture;

fn watch_context(portfolio: bool) -> TestResult<OperationContext> {
    let (_, _, source) = reference_fixture(portfolio, &[], false)?;
    let mut c = context();
    c.anchor.fortress_id = source.anchor.fortress_id;
    for grant in &mut c.grants {
        grant.scope.fortress_id = Some(c.anchor.fortress_id);
    }
    Ok(c)
}

fn watch_original(portfolio: bool) -> TestResult<Value> {
    let (_, goal, _) = reference_fixture(portfolio, &[], false)?;
    let mut original = original();
    original["goal"] = goal;
    Ok(original)
}

fn watch_checkpoint(portfolio: bool, ticks: &[u64], cancelled: bool) -> TestResult<Checkpoint> {
    let (reference, _, _) = reference_fixture(portfolio, ticks, cancelled)?;
    let mut checkpoint = handoff();
    checkpoint.production = Some(reference);
    Ok(checkpoint)
}

fn enter_production<S: EffectJournalStorage, A: EffectJournalStorage>(
    journal: &mut Journal<S, A>,
    c: &OperationContext,
    portfolio: bool,
) -> TestResult {
    journal.advance(c, Stage::Workshop, workshop())?;
    journal.advance(c, Stage::Transition, handoff())?;
    journal.advance(c, Stage::ProductionOpening, handoff())?;
    let mut checkpoint = watch_checkpoint(portfolio, &[], false)?;
    checkpoint
        .production
        .as_mut()
        .ok_or("missing production")?
        .as_object_mut()
        .ok_or("missing production object")?
        .remove("stock_watch");
    journal.advance(c, Stage::Production, checkpoint)?;
    Ok(())
}

fn seeded_watch(
    portfolio: bool,
) -> TestResult<(
    Journal<Memory, Memory>,
    Memory,
    Memory,
    OperationContext,
    Value,
)> {
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    let c = watch_context(portfolio)?;
    let original = watch_original(portfolio)?;
    let mut journal = Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &c,
        Mode::Control,
        true,
        original.clone(),
        None,
    )?;
    enter_production(&mut journal, &c, portfolio)?;
    Ok((journal, storage, acknowledgements, c, original))
}

#[test]
fn original_scalar_and_portfolio_watch_history_reopens_only_from_acknowledged_parent() -> TestResult
{
    for portfolio in [false, true] {
        let (mut journal, storage, acknowledgements, mut c, original) = seeded_watch(portfolio)?;
        journal.advance(
            &c,
            Stage::Production,
            watch_checkpoint(portfolio, &[], false)?,
        )?;
        let initial = journal.pin();
        c.anchor.tick = GameTick(140);
        let achieved = watch_checkpoint(portfolio, &[130, 140], false)?;
        journal.advance(&c, Stage::Production, achieved.clone())?;
        let expected = journal.pin();
        let retained = journal.snapshot().clone();
        drop(journal);
        let before = (storage.raw(), acknowledgements.raw());
        let writes = (
            storage.state.borrow().writes,
            acknowledgements.state.borrow().writes,
        );
        c.session_id = SessionId::new(7002);
        c.grants
            .retain(|grant| grant.capability == Capability::Query);
        for pin in [initial, expected.clone()] {
            let recovered = Journal::open(
                storage.clone(),
                acknowledgements.clone(),
                &c,
                Mode::Offline,
                false,
                original.clone(),
                Some(pin),
            )?;
            assert_eq!(recovered.snapshot(), &retained);
            assert_eq!(recovered.pin(), expected);
            assert_eq!(
                recovered.snapshot().checkpoint.production,
                achieved.production
            );
        }
        assert_eq!((storage.raw(), acknowledgements.raw()), before);
        assert_eq!(
            (
                storage.state.borrow().writes,
                acknowledgements.state.borrow().writes
            ),
            writes
        );
    }
    Ok(())
}

#[test]
fn first_stock_watch_requires_control_and_plan_before_any_journal_write() -> TestResult {
    for portfolio in [false, true] {
        let (mut journal, storage, acknowledgements, c, original) = seeded_watch(portfolio)?;
        let checkpoint = watch_checkpoint(portfolio, &[], false)?;
        let before = (storage.raw(), acknowledgements.raw());
        let mut read_only = c.clone();
        read_only
            .grants
            .retain(|grant| grant.capability != Capability::Plan);
        assert!(
            journal
                .advance(&read_only, Stage::Production, checkpoint.clone())
                .is_err()
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), before);
        let pin = journal.pin();
        drop(journal);
        let mut recovered = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Recover,
            false,
            original,
            Some(pin),
        )?;
        assert!(
            recovered
                .advance(&c, Stage::Production, checkpoint)
                .is_err()
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), before);
    }
    Ok(())
}

#[test]
fn first_watch_cannot_be_adopted_with_samples_after_cancellation_or_pending_production()
-> TestResult {
    for portfolio in [false, true] {
        let (mut journal, storage, acknowledgements, c, _) = seeded_watch(portfolio)?;
        let before = (storage.raw(), acknowledgements.raw());
        for (ticks, cancelled) in [
            (&[130][..], false),
            (&[130, 140][..], false),
            (&[][..], true),
        ] {
            assert!(
                journal
                    .advance(
                        &c,
                        Stage::Production,
                        watch_checkpoint(portfolio, ticks, cancelled)?
                    )
                    .is_err()
            );
            assert_eq!((storage.raw(), acknowledgements.raw()), before);
        }
        journal.cancel(&c)?;
        let stopped = (storage.raw(), acknowledgements.raw());
        assert!(
            journal
                .advance(
                    &c,
                    Stage::Cancelled,
                    watch_checkpoint(portfolio, &[], false)?
                )
                .is_err()
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), stopped);
    }
    let (mut journal, storage, acknowledgements) = production_seeded()?;
    journal.advance(&context(), Stage::Production, intended_production())?;
    let before = (storage.raw(), acknowledgements.raw());
    let mut checkpoint = journal.snapshot().checkpoint.clone();
    let (watch, _, _) = reference_fixture(false, &[], false)?;
    checkpoint.production.as_mut().ok_or("missing production")?["stock_watch"] =
        watch["stock_watch"].clone();
    assert!(
        journal
            .advance(&context(), Stage::Production, checkpoint)
            .is_err()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), before);
    Ok(())
}

#[test]
fn cancelled_watch_and_achievement_cannot_be_erased_even_by_checksummed_replay() -> TestResult {
    for portfolio in [false, true] {
        let (mut journal, storage, acknowledgements, mut c, original) = seeded_watch(portfolio)?;
        journal.advance(
            &c,
            Stage::Production,
            watch_checkpoint(portfolio, &[], false)?,
        )?;
        c.anchor.tick = GameTick(140);
        journal.advance(
            &c,
            Stage::Production,
            watch_checkpoint(portfolio, &[130, 140], false)?,
        )?;
        journal.cancel(&c)?;
        journal.advance(
            &c,
            Stage::Cancelled,
            watch_checkpoint(portfolio, &[130, 140], true)?,
        )?;
        let pin = journal.pin();
        let ack_chain = journal.ack_chain;
        let snapshot = journal.snapshot().clone();
        drop(journal);
        let before = (storage.raw(), acknowledgements.raw());
        for changed in 0..6 {
            storage.replace(before.0.clone());
            acknowledgements.replace(before.1.clone());
            let mut candidate = snapshot.clone();
            let reference = candidate
                .checkpoint
                .production
                .as_mut()
                .ok_or("missing production")?;
            match changed {
                0 => {
                    reference
                        .as_object_mut()
                        .ok_or("missing object")?
                        .remove("stock_watch");
                }
                1 => reference["stock_watch"]["cancelled"] = json!(false),
                2 => reference["stock_watch"]["request"]["units"] = json!(["43"]),
                3 => reference["stock_watch"]["request"]["deadline_tick"] = json!(201),
                4 => {
                    let (older, _, _) = reference_fixture(portfolio, &[130], true)?;
                    reference["stock_watch"] = older["stock_watch"].clone();
                }
                _ => reference["stock_watch"]["checkpoint_hex"] = json!("ab".repeat(2049)),
            }
            let body = serde_json::to_vec(&Body {
                high_tick: 140,
                snapshot: candidate,
            })?;
            super::stock_clock_recovery::append_stock_replay_frame(
                &storage,
                &acknowledgements,
                &pin,
                ack_chain,
                &body,
            )?;
            assert!(
                Journal::open(
                    storage.clone(),
                    acknowledgements.clone(),
                    &c,
                    Mode::Recover,
                    false,
                    original.clone(),
                    Some(pin.clone()),
                )
                .is_err(),
                "forged successor {changed}"
            );
        }
    }
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn actual_private_stock_watch_ack_recovers_without_writes_and_missing_ack_refuses() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = Directory(std::env::temp_dir().join(format!(
        "dfmcp-project-stock-watch-{}-{nonce}",
        std::process::id(),
    )));
    std::fs::create_dir(&directory.0)?;
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
    let mut c = watch_context(true)?;
    let original = watch_original(true)?;
    let mut journal = open_private(
        &directory.0,
        &c,
        Mode::Control,
        true,
        original.clone(),
        None,
        Stage::WorkshopOpening,
    )?;
    enter_production(&mut journal, &c, true)?;
    journal.advance(&c, Stage::Production, watch_checkpoint(true, &[], false)?)?;
    c.anchor.tick = GameTick(130);
    journal.advance(
        &c,
        Stage::Production,
        watch_checkpoint(true, &[130], false)?,
    )?;
    let pin = journal.pin();
    let expected = journal.snapshot().clone();
    drop(journal);
    let journal_path = directory.0.join("resource-project.journal");
    let ack_path = directory.0.join("resource-project.ack");
    let bytes = (std::fs::read(&journal_path)?, std::fs::read(&ack_path)?);
    c.session_id = SessionId::new(7002);
    c.grants
        .retain(|grant| grant.capability == Capability::Query);
    let reopened = open_private(
        &directory.0,
        &c,
        Mode::Offline,
        false,
        original.clone(),
        Some(pin.clone()),
        Stage::WorkshopOpening,
    )?;
    assert_eq!(reopened.snapshot(), &expected);
    drop(reopened);
    assert_eq!(
        (std::fs::read(&journal_path)?, std::fs::read(&ack_path)?),
        bytes
    );
    std::fs::remove_file(&ack_path)?;
    assert!(
        open_private(
            &directory.0,
            &c,
            Mode::Offline,
            false,
            original,
            Some(pin),
            Stage::WorkshopOpening,
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&journal_path)?, bytes.0);
    assert!(!ack_path.exists());
    Ok(())
}

#[test]
fn terminal_parent_ack_survives_crash_before_child_watch_cancellation() -> TestResult {
    for portfolio in [false, true] {
        let (mut journal, storage, acknowledgements, mut c, original) = seeded_watch(portfolio)?;
        journal.advance(
            &c,
            Stage::Production,
            watch_checkpoint(portfolio, &[], false)?,
        )?;
        c.anchor.tick = GameTick(130);
        journal.advance(
            &c,
            Stage::Production,
            watch_checkpoint(portfolio, &[130], false)?,
        )?;
        journal.cancel(&c)?;
        let pin = journal.pin();
        let stopped = journal.snapshot().clone();
        assert_eq!(stopped.stage, Stage::Cancelled);
        assert_eq!(stopped.cancelled_from, Some(Stage::Production));
        assert_eq!(
            stopped
                .checkpoint
                .production
                .as_ref()
                .ok_or("missing production")?["stock_watch"]["cancelled"],
            false,
        );
        drop(journal);
        let bytes = (storage.raw(), acknowledgements.raw());
        let writes = (
            storage.state.borrow().writes,
            acknowledgements.state.borrow().writes,
        );
        c.session_id = SessionId::new(7003);
        c.grants
            .retain(|grant| grant.capability == Capability::Query);
        let offline = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Offline,
            false,
            original.clone(),
            Some(pin.clone()),
        )?;
        assert_eq!(offline.snapshot(), &stopped);
        drop(offline);
        assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
        assert_eq!(
            (
                storage.state.borrow().writes,
                acknowledgements.state.borrow().writes
            ),
            writes
        );

        // Local child retirement needs no new Plan/effect authority. Its
        // sticky flag is the only change acknowledged after the terminal ACK.
        let mut recovered = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Recover,
            false,
            original,
            Some(pin),
        )?;
        let cancelled = watch_checkpoint(portfolio, &[130], true)?;
        recovered.advance(&c, Stage::Cancelled, cancelled.clone())?;
        assert_eq!(recovered.snapshot().checkpoint, cancelled);
        assert_eq!(recovered.snapshot().cancelled_from, Some(Stage::Production));
        let terminal = (storage.raw(), acknowledgements.raw());
        assert!(
            recovered
                .advance(
                    &c,
                    Stage::Cancelled,
                    watch_checkpoint(portfolio, &[130], false)?,
                )
                .is_err()
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), terminal);
    }
    Ok(())
}

#[test]
fn stopped_watch_retains_independent_clock_custody_and_reconciliation_floor() -> TestResult {
    use crate::native_resource_project::runtime::stock_clock::{
        ClockCheckpoint, WatchClockTarget, original_digest, production_source_digest,
    };
    for portfolio in [false, true] {
        let storage = Memory::default();
        let acknowledgements = Memory::default();
        let mut c = watch_context(portfolio)?;
        let mut original = watch_original(portfolio)?;
        original["schema"] = json!("dfmcp.native-resource-project-original/1");
        original["stock_watch_clock"] = json!(true);
        let mut journal = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Control,
            true,
            original.clone(),
            None,
        )?;
        let with_donor = |ticks: &[u64], cancelled: bool| -> TestResult<Checkpoint> {
            let mut checkpoint = watch_checkpoint(portfolio, ticks, cancelled)?;
            let mut donor = production_publication()["workshop_prerequisite"].clone();
            donor["checkpoint"]["tick"] = json!(120);
            checkpoint.production.as_mut().ok_or("missing production")?["workshop_prerequisite"] =
                donor;
            Ok(checkpoint)
        };
        journal.advance(&c, Stage::Workshop, workshop())?;
        journal.advance(&c, Stage::Transition, handoff())?;
        journal.advance(&c, Stage::ProductionOpening, handoff())?;
        let mut initial = with_donor(&[], false)?;
        initial
            .production
            .as_mut()
            .ok_or("missing production")?
            .as_object_mut()
            .ok_or("missing production object")?
            .remove("stock_watch");
        journal.advance(&c, Stage::Production, initial)?;
        journal.advance(&c, Stage::Production, with_donor(&[], false)?)?;
        c.anchor.tick = GameTick(130);
        let mut checkpoint = with_donor(&[130], false)?;
        journal.advance(&c, Stage::Production, checkpoint.clone())?;
        let goal = serde_json::from_value(original["goal"].clone())?;
        let target = WatchClockTarget::new(
            parsed_digest(&journal.pin().journal_id)?,
            original_digest(&original)?,
            &goal,
            production_source_digest(checkpoint.production.as_ref().ok_or("missing production")?)?,
            c.anchor,
        )?;
        checkpoint.stock_clock = Some(
            ClockCheckpoint::prospective(
                target,
                Digest32::of_bytes(b"retained clock beside a stopped stock watch"),
            )?
            .to_json(),
        );
        journal.advance(&c, Stage::Production, checkpoint.clone())?;
        let clock = checkpoint
            .stock_clock
            .as_mut()
            .ok_or("missing stock clock")?;
        clock["head"] = json!(Digest32::of_bytes(b"existing stock clock progress").to_hex());
        clock["authority_tick_floor"] = json!(145);
        clock["high_source_sequence"] = json!(1);
        ClockCheckpoint::from_json(clock)?;
        journal.advance(&c, Stage::Production, checkpoint.clone())?;
        journal.cancel(&c)?;
        let pin = journal.pin();
        let stopped = journal.snapshot().clone();
        drop(journal);
        let bytes = (storage.raw(), acknowledgements.raw());
        c.session_id = SessionId::new(7004);
        c.grants
            .retain(|grant| grant.capability == Capability::Query);
        let offline = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Offline,
            false,
            original.clone(),
            Some(pin.clone()),
        )?;
        assert_eq!(offline.snapshot(), &stopped);
        assert_eq!(
            offline.snapshot().checkpoint.stock_clock,
            checkpoint.stock_clock
        );
        assert_eq!(offline.pin().authority_tick_floor, 145);
        drop(offline);
        assert_eq!((storage.raw(), acknowledgements.raw()), bytes);

        let mut recovered = Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Recover,
            false,
            original,
            Some(pin),
        )?;
        checkpoint.production = with_donor(&[130], true)?.production;
        recovered.advance(&c, Stage::Cancelled, checkpoint.clone())?;
        let original_target = checkpoint
            .stock_clock
            .as_ref()
            .ok_or("missing stock clock")?["target"]
            .clone();
        // Existing time custody can advance its safety/reconciliation floor
        // beyond the unchanged goal deadline after local monitor retirement.
        let clock = checkpoint
            .stock_clock
            .as_mut()
            .ok_or("missing stock clock")?;
        clock["head"] =
            json!(Digest32::of_bytes(b"existing stock clock safety reconciliation").to_hex());
        clock["authority_tick_floor"] = json!(220);
        clock["high_source_sequence"] = json!(2);
        ClockCheckpoint::from_json(clock)?;
        recovered.advance(&c, Stage::Cancelled, checkpoint.clone())?;
        assert_eq!(recovered.pin().authority_tick_floor, 220);
        assert_eq!(recovered.snapshot().checkpoint, checkpoint);
        let clock = checkpoint
            .stock_clock
            .as_ref()
            .ok_or("missing stock clock")?;
        assert_eq!(clock["target"], original_target);
        assert_eq!(clock["target"]["deadline_tick"], 200);
        assert!(clock.get("current_goal_proven").is_none());
        assert!(clock.get("review_seal").is_none());
        let production = checkpoint.production.as_ref().ok_or("missing production")?;
        assert_eq!(production["stock_watch"]["cancelled"], true);
        assert_eq!(production["original_goal_proven"], false);
        assert!(production["recovery"].is_null());
    }
    Ok(())
}
