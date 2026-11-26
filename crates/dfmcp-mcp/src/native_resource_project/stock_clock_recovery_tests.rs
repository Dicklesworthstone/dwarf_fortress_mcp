//! Actual parent journal replay for the separate, immutable no-order clock.
//! These records retain rejection floors and custody, never stock proof.
use super::*;
use crate::live_production_runtime::ProjectStockGoal;
use crate::native_resource_project::runtime::stock_clock::{
    ClockCheckpoint, WatchClockTarget, original_digest, production_source_digest,
};

fn stock_original(portfolio: bool) -> Value {
    let mut value = original();
    value["schema"] = json!("dfmcp.native-resource-project-original/1");
    value["goal"] = if portfolio {
        clocked_portfolio_publication().0["goal"].clone()
    } else {
        production_intent()["goal"].clone()
    };
    value["stock_watch_clock"] = json!(true);
    value
}

fn stock_seeded_with(
    original: Value,
) -> TestResult<(Journal<Memory, Memory>, Memory, Memory, Value)> {
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    let mut journal = Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &context(),
        Mode::Control,
        true,
        original.clone(),
        None,
    )?;
    journal.advance(&context(), Stage::Workshop, workshop())?;
    journal.advance(&context(), Stage::Transition, handoff())?;
    journal.advance(&context(), Stage::ProductionOpening, handoff())?;
    journal.advance(&context(), Stage::Production, preplan_production())?;
    Ok((journal, storage, acknowledgements, original))
}

fn stock_marker(journal: &Journal<Memory, Memory>, original: &Value) -> TestResult<Checkpoint> {
    let mut checkpoint = journal.snapshot().checkpoint.clone();
    let goal: ProjectStockGoal = serde_json::from_value(original["goal"].clone())?;
    let production = checkpoint.production.as_ref().ok_or("missing production")?;
    let mut origin = context().anchor;
    origin.tick = GameTick(130);
    let target = WatchClockTarget::new(
        parsed_digest(&journal.pin().journal_id)?,
        original_digest(original)?,
        &goal,
        production_source_digest(production)?,
        origin,
    )?;
    checkpoint.stock_clock = Some(
        ClockCheckpoint::prospective(
            target,
            Digest32::of_bytes(b"separate no-order stock clock journal"),
        )?
        .to_json(),
    );
    Ok(checkpoint)
}

fn stock_progress(checkpoint: &mut Checkpoint, tick: u64, sequence: u64) -> TestResult {
    let clock = checkpoint
        .stock_clock
        .as_mut()
        .ok_or("missing stock clock")?;
    clock["head"] = json!(
        Digest32::of_bytes(format!("stock clock descendant {tick} {sequence}").as_bytes()).to_hex()
    );
    clock["authority_tick_floor"] = json!(tick);
    clock["high_source_sequence"] = json!(sequence);
    ClockCheckpoint::from_json(clock)?;
    Ok(())
}

/// Public checksums are reproducible. This deliberately bypasses append so
/// replay must independently reject an invalid, canonically encoded transition.
pub(super) fn append_stock_replay_frame(
    storage: &Memory,
    acknowledgements: &Memory,
    pin: &Pin,
    ack_chain: Digest32,
    raw_body: &[u8],
) -> TestResult {
    let mut record = FRAME.to_vec();
    record.extend_from_slice(&(raw_body.len() as u32).to_be_bytes());
    record.extend_from_slice(&(pin.frames + 1).to_be_bytes());
    record.extend_from_slice(parsed_digest(&pin.head)?.as_bytes());
    record.extend_from_slice(raw_body);
    let head = Head {
        digest: hash(b"dfmcp-resource-project-frame/1", &record),
        tick: serde_json::from_slice::<Body>(raw_body)?.high_tick,
    };
    record.extend_from_slice(head.digest.as_bytes());
    let mut raw = storage.raw();
    raw.extend_from_slice(&record);
    let mut ack = acknowledgements.raw();
    ack.extend_from_slice(&acknowledgement(
        parsed_digest(&pin.journal_id)?,
        head,
        pin.frames + 1,
        ack_chain,
    ));
    storage.replace(raw);
    acknowledgements.replace(ack);
    Ok(())
}

fn reopen_stock(
    storage: &Memory,
    acknowledgements: &Memory,
    original: &Value,
    pin: Pin,
    mode: Mode,
) -> Result<Journal<Memory, Memory>> {
    let mut c = context();
    c.session_id = SessionId::new(7002);
    Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &c,
        mode,
        false,
        original.clone(),
        Some(pin),
    )
}

#[test]
fn stock_clock_omission_preserves_exact_legacy_journal_bytes() -> TestResult {
    let snapshots = [
        Snapshot::initial(Stage::WorkshopOpening),
        Snapshot {
            stage: Stage::Workshop,
            checkpoint: workshop(),
            cancelled_from: None,
        },
    ];
    let (raw, ack, pin) = legacy_stores(&snapshots)?;
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    storage.replace(raw.clone());
    acknowledgements.replace(ack.clone());
    let journal = reopen(&storage, &acknowledgements, pin, Mode::Offline)?;
    assert!(journal.snapshot().checkpoint.stock_clock.is_none());
    assert!(
        serde_json::to_value(&journal.snapshot().checkpoint)?
            .get("stock_clock")
            .is_none()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), (raw, ack));
    assert_eq!(storage.state.borrow().writes, 0);
    assert_eq!(acknowledgements.state.borrow().writes, 0);
    Ok(())
}

#[test]
fn stock_clock_first_marker_requires_original_opt_in_and_control_authority() -> TestResult {
    let mut disabled = stock_original(false);
    disabled
        .as_object_mut()
        .ok_or("missing original")?
        .remove("stock_watch_clock");
    let (mut journal, storage, acknowledgements, original) = stock_seeded_with(disabled)?;
    let checkpoint = stock_marker(&journal, &original)?;
    let bytes = (storage.raw(), acknowledgements.raw());
    assert!(
        journal
            .advance(&context(), Stage::Production, checkpoint)
            .is_err()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);

    for policy in [Value::Null, json!(false), json!("true")] {
        let mut original = stock_original(false);
        original["stock_watch_clock"] = policy;
        let storage = Memory::default();
        let acknowledgements = Memory::default();
        assert!(
            Journal::open(
                storage.clone(),
                acknowledgements.clone(),
                &context(),
                Mode::Control,
                true,
                original,
                None,
            )
            .is_err()
        );
        assert!(storage.raw().is_empty());
        assert!(acknowledgements.raw().is_empty());
    }

    let (mut journal, storage, acknowledgements, original) =
        stock_seeded_with(stock_original(false))?;
    let checkpoint = stock_marker(&journal, &original)?;
    let bytes = (storage.raw(), acknowledgements.raw());
    let mut query_only = context();
    query_only
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    assert!(
        journal
            .advance(&query_only, Stage::Production, checkpoint.clone())
            .is_err()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    let pin = journal.pin();
    drop(journal);
    let mut recovered = reopen_stock(&storage, &acknowledgements, &original, pin, Mode::Recover)?;
    let bytes = (storage.raw(), acknowledgements.raw());
    let mut current = context();
    current.session_id = SessionId::new(7002);
    assert!(
        recovered
            .advance(&current, Stage::Production, checkpoint)
            .is_err()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    Ok(())
}

#[test]
fn stock_clock_first_marker_is_isolated_from_orders_and_requires_initial_custody() -> TestResult {
    for cause in 0..7 {
        let (mut journal, storage, acknowledgements, original) =
            stock_seeded_with(stock_original(false))?;
        let mut checkpoint = stock_marker(&journal, &original)?;
        match cause {
            0 => {
                checkpoint.production.as_mut().ok_or("missing production")?["observation_tick_floor"] =
                    json!(131);
            }
            1 => checkpoint.production_intent = Some(production_intent()),
            2 => {
                checkpoint.stock_clock.as_mut().ok_or("missing clock")?["head"] =
                    json!(Digest32::of_bytes(b"already running clock").to_hex());
            }
            3 => {
                checkpoint.stock_clock.as_mut().ok_or("missing clock")?["authority_tick_floor"] =
                    json!(131);
            }
            4 => {
                checkpoint.stock_clock.as_mut().ok_or("missing clock")?["high_source_sequence"] =
                    json!(1);
            }
            5 => {
                journal.advance(&context(), Stage::Production, intended_production())?;
                journal.advance(&context(), Stage::Production, published_production())?;
                checkpoint = stock_marker(&journal, &original)?;
            }
            _ => journal.cancel(&context()).map(|_| ())?,
        }
        let bytes = (storage.raw(), acknowledgements.raw());
        let next = if cause == 6 {
            Stage::Cancelled
        } else {
            Stage::Production
        };
        assert!(
            journal.advance(&context(), next, checkpoint).is_err(),
            "{cause}"
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), bytes, "{cause}");
    }
    Ok(())
}

#[test]
fn stock_clock_descendants_recover_and_drain_after_stop_without_restoring_proof() -> TestResult {
    for portfolio in [false, true] {
        for stopped in [false, true] {
            let (mut journal, storage, acknowledgements, original) =
                stock_seeded_with(stock_original(portfolio))?;
            let ancestor = journal.pin();
            let checkpoint = stock_marker(&journal, &original)?;
            let target = checkpoint.stock_clock.as_ref().ok_or("missing clock")?["target"].clone();
            journal.advance(&context(), Stage::Production, checkpoint)?;
            assert_eq!(journal.pin().authority_tick_floor, 130);
            if stopped {
                journal.cancel(&context())?;
            }
            let stage = if stopped {
                Stage::Cancelled
            } else {
                Stage::Production
            };
            let mut checkpoint = journal.snapshot().checkpoint.clone();
            stock_progress(&mut checkpoint, 170, 9)?;
            // The child is still at 130; the separate clock must raise the
            // parent's authority floor independently of that stale summary.
            journal.advance(&context(), stage, checkpoint.clone())?;
            assert_eq!(journal.pin().authority_tick_floor, 170);
            let expected = journal.pin();
            let bytes = (storage.raw(), acknowledgements.raw());
            let writes = (
                storage.state.borrow().writes,
                acknowledgements.state.borrow().writes,
            );
            drop(journal);
            let mut recovered = reopen_stock(
                &storage,
                &acknowledgements,
                &original,
                ancestor,
                Mode::Recover,
            )?;
            assert_eq!(recovered.pin(), expected);
            assert_eq!(recovered.snapshot().checkpoint, checkpoint);
            assert_eq!(recovered.snapshot().stage, stage);
            assert_eq!(
                recovered.snapshot().cancelled_from,
                stopped.then_some(Stage::Production)
            );
            assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
            assert_eq!(
                (
                    storage.state.borrow().writes,
                    acknowledgements.state.borrow().writes
                ),
                writes
            );
            // A retained safety/reconciliation frontier may exceed the goal
            // deadline. It neither renews that deadline nor restores its truth.
            let mut current = context();
            current.session_id = SessionId::new(7002);
            current
                .grants
                .retain(|grant| grant.capability == Capability::Query);
            stock_progress(&mut checkpoint, 1100, 10)?;
            recovered.advance(&current, stage, checkpoint.clone())?;
            assert_eq!(recovered.pin().authority_tick_floor, 1100);
            let clock = checkpoint.stock_clock.as_ref().ok_or("missing clock")?;
            assert_eq!(clock["target"], target);
            assert_eq!(clock["target"]["deadline_tick"], 1000);
            assert!(clock.get("current_goal_proven").is_none());
            assert!(clock.get("stable_samples").is_none());
            assert!(clock.get("review_seal").is_none());
            assert_eq!(
                checkpoint.production.as_ref().ok_or("missing production")?["original_goal_proven"],
                false
            );
        }
    }
    Ok(())
}

#[test]
fn stock_clock_binding_rejects_foreign_root_goal_source_and_fortress_before_first_marker()
-> TestResult {
    for portfolio in [false, true] {
        for cause in 0..6 {
            let (mut journal, storage, acknowledgements, original) =
                stock_seeded_with(stock_original(portfolio))?;
            let mut checkpoint = journal.snapshot().checkpoint.clone();
            let production = checkpoint.production.as_ref().ok_or("missing production")?;
            let mut goal_json = original["goal"].clone();
            let mut parent = parsed_digest(&journal.pin().journal_id)?;
            let mut original_hash = original_digest(&original)?;
            let mut source = production_source_digest(production)?;
            let mut origin = context().anchor;
            origin.tick = GameTick(130);
            match cause {
                0 => parent = Digest32::of_bytes(b"another resource project"),
                1 => original_hash = Digest32::of_bytes(b"another immutable policy"),
                2 => {
                    if portfolio {
                        goal_json["targets"][3]["minimum_stack_units"] = json!(3);
                    } else {
                        goal_json["minimum_stack_units"] = json!(3);
                    }
                }
                3 => source = Digest32::of_bytes(b"another native production owner"),
                4 => origin.fortress_id = FortressId::new(92),
                _ => origin.tick = GameTick(99),
            }
            let goal: ProjectStockGoal = serde_json::from_value(goal_json)?;
            let target = WatchClockTarget::new(parent, original_hash, &goal, source, origin)?;
            checkpoint.stock_clock = Some(
                ClockCheckpoint::prospective(
                    target,
                    Digest32::of_bytes(b"separate no-order stock clock journal"),
                )?
                .to_json(),
            );
            let bytes = (storage.raw(), acknowledgements.raw());
            assert!(
                journal
                    .advance(&context(), Stage::Production, checkpoint)
                    .is_err(),
                "{cause}"
            );
            assert_eq!((storage.raw(), acknowledgements.raw()), bytes, "{cause}");
        }
    }
    Ok(())
}

#[test]
fn settled_stock_clock_custody_survives_later_ordinary_production() -> TestResult {
    let (mut journal, storage, acknowledgements, original) =
        stock_seeded_with(stock_original(false))?;
    let mut checkpoint = stock_marker(&journal, &original)?;
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    stock_progress(&mut checkpoint, 170, 9)?;
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    let retained_clock = checkpoint.stock_clock.clone();
    checkpoint.production_intent = Some(production_intent());
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    checkpoint.production = Some(production_publication());
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    assert_eq!(checkpoint.stock_clock, retained_clock);
    let pin = journal.pin();
    drop(journal);
    let recovered = reopen_stock(&storage, &acknowledgements, &original, pin, Mode::Recover)?;
    assert_eq!(recovered.snapshot().checkpoint.stock_clock, retained_clock);
    assert_eq!(
        recovered.snapshot().checkpoint.production_intent,
        checkpoint.production_intent
    );
    Ok(())
}

#[test]
fn checksummed_replay_cannot_rebind_or_regress_stock_clock_beneath_a_higher_parent_floor()
-> TestResult {
    let (mut journal, storage, acknowledgements, original) =
        stock_seeded_with(stock_original(true))?;
    let mut checkpoint = stock_marker(&journal, &original)?;
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    stock_progress(&mut checkpoint, 170, 9)?;
    let mut c = context();
    c.anchor.tick = GameTick(400);
    journal.advance(&c, Stage::Production, checkpoint.clone())?;
    let pin = journal.pin();
    let ack_chain = journal.ack_chain;
    let base = (storage.raw(), acknowledgements.raw());
    drop(journal);
    for cause in 0..9 {
        storage.replace(base.0.clone());
        acknowledgements.replace(base.1.clone());
        let mut changed = checkpoint.clone();
        match cause {
            0 => changed.stock_clock = None,
            1 => {
                changed.stock_clock.as_mut().ok_or("missing clock")?["journal_id"] =
                    json!(Digest32::of_bytes(b"replacement clock journal").to_hex());
            }
            2 => {
                changed.stock_clock.as_mut().ok_or("missing clock")?["authority_tick_floor"] =
                    json!(169);
            }
            3 => {
                changed.stock_clock.as_mut().ok_or("missing clock")?["high_source_sequence"] =
                    json!(8);
            }
            4 => {
                changed.production.as_mut().ok_or("missing production")?["native_custody"]["order_journal_id"] =
                    json!(Digest32::of_bytes(b"replacement production journal").to_hex());
            }
            5 => {
                changed.stock_clock.as_mut().ok_or("missing clock")?["target"]["original_paired_seal"] =
                    json!(Digest32::of_bytes(b"replacement stock clock seal").to_hex());
            }
            6 => {
                changed.production.as_mut().ok_or("missing production")?["workshop_prerequisite"]
                    ["checkpoint"]["tick"] = json!(129);
            }
            7 => {
                changed.stock_clock.as_mut().ok_or("missing clock")?["current_goal_proven"] =
                    json!(true);
            }
            _ => {
                let clock = changed.stock_clock.as_mut().ok_or("missing clock")?;
                clock["head"] = clock["journal_id"].clone();
            }
        }
        let body = serde_json::to_vec(&Body {
            high_tick: 400,
            snapshot: Snapshot {
                stage: Stage::Production,
                checkpoint: changed,
                cancelled_from: None,
            },
        })?;
        append_stock_replay_frame(&storage, &acknowledgements, &pin, ack_chain, &body)?;
        let bytes = (storage.raw(), acknowledgements.raw());
        let writes = (
            storage.state.borrow().writes,
            acknowledgements.state.borrow().writes,
        );
        assert!(
            reopen_stock(
                &storage,
                &acknowledgements,
                &original,
                pin.clone(),
                Mode::Recover
            )
            .is_err(),
            "{cause}"
        );
        assert_eq!((storage.raw(), acknowledgements.raw()), bytes, "{cause}");
        assert_eq!(
            (
                storage.state.borrow().writes,
                acknowledgements.state.borrow().writes
            ),
            writes,
            "{cause}"
        );
    }
    assert_eq!(storage.state.borrow().truncates, 0);
    assert_eq!(acknowledgements.state.borrow().truncates, 0);
    Ok(())
}

#[test]
fn checksummed_replay_cannot_add_a_stock_clock_to_a_legacy_original() -> TestResult {
    let mut original = stock_original(false);
    original
        .as_object_mut()
        .ok_or("missing original")?
        .remove("stock_watch_clock");
    let (journal, storage, acknowledgements, original) = stock_seeded_with(original)?;
    let checkpoint = stock_marker(&journal, &original)?;
    let pin = journal.pin();
    let ack_chain = journal.ack_chain;
    drop(journal);
    let body = serde_json::to_vec(&Body {
        high_tick: 130,
        snapshot: Snapshot {
            stage: Stage::Production,
            checkpoint,
            cancelled_from: None,
        },
    })?;
    append_stock_replay_frame(&storage, &acknowledgements, &pin, ack_chain, &body)?;
    let bytes = (storage.raw(), acknowledgements.raw());
    assert!(reopen_stock(&storage, &acknowledgements, &original, pin, Mode::Recover).is_err());
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    Ok(())
}

#[test]
fn stock_clock_frontier_cannot_bypass_parent_authority_expiry() -> TestResult {
    let (mut journal, storage, acknowledgements, original) =
        stock_seeded_with(stock_original(false))?;
    let mut checkpoint = stock_marker(&journal, &original)?;
    journal.advance(&context(), Stage::Production, checkpoint.clone())?;
    let bytes = (storage.raw(), acknowledgements.raw());
    stock_progress(&mut checkpoint, 1201, 9)?;
    let mut query_only = context();
    query_only
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    assert!(
        journal
            .advance(&query_only, Stage::Production, checkpoint)
            .is_err()
    );
    assert_eq!(journal.pin().authority_tick_floor, 1201);
    assert!(journal.verify(&context()).is_err());
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    let pin = journal.pin();
    drop(journal);
    assert!(reopen_stock(&storage, &acknowledgements, &original, pin, Mode::Recover).is_err());
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    Ok(())
}

#[test]
fn stock_clock_marker_cannot_open_before_the_production_child() -> TestResult {
    let original = stock_original(false);
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    let mut journal = Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &context(),
        Mode::Control,
        true,
        original.clone(),
        None,
    )?;
    let goal: ProjectStockGoal = serde_json::from_value(original["goal"].clone())?;
    let source = preplan_production();
    let mut origin = context().anchor;
    origin.tick = GameTick(130);
    let target = WatchClockTarget::new(
        parsed_digest(&journal.pin().journal_id)?,
        original_digest(&original)?,
        &goal,
        production_source_digest(source.production.as_ref().ok_or("missing production")?)?,
        origin,
    )?;
    let mut checkpoint = workshop();
    checkpoint.stock_clock = Some(
        ClockCheckpoint::prospective(
            target,
            Digest32::of_bytes(b"separate no-order stock clock journal"),
        )?
        .to_json(),
    );
    let bytes = (storage.raw(), acknowledgements.raw());
    assert!(
        journal
            .advance(&context(), Stage::Workshop, checkpoint)
            .is_err()
    );
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    Ok(())
}
