use super::*;
use crate::tree_harvest::clock::Target;
use crate::tree_harvest::journal::journal_tests::{Directory, binding, context};
use crate::tree_harvest::tests as fixtures;
use std::net::SocketAddr;

type Fixture = (
    Directory,
    Journal<PrivateBuildFile>,
    WorkshopAcknowledgements<PrivateBuildFile>,
    OperationContext,
);

fn fixture() -> Result<Fixture> {
    let dir = Directory::new()?;
    let c = context()?;
    let b = binding(SocketAddr::from(([127, 0, 0, 1], 5000)))?;
    let (mut journal, mut ack) = open_private(
        &dir.0,
        &c,
        Mode::Control,
        &b,
        [15, 15, 2],
        true,
        None,
        [9; 32],
    )?;
    let original = Original::new(fixtures::plan()?, fixtures::capture()?.tick + 1000)?;
    journal.intent(original.clone(), &c, &mut ack)?;
    journal.prepared(fixtures::record("prepared")?, &c, &mut ack)?;
    journal.dispatch(original.seal(), &c, &mut ack, &mut |_| Ok(()), |_, _| {
        fixtures::record("designated")
    })?;
    Ok((dir, journal, ack, c))
}

#[test]
fn tree_clock_origin_uses_verified_bytes_instead_of_mutable_public_caches() -> Result<()> {
    let (dir, mut journal, mut ack, c) = fixture()?;
    let before = journal.clock_origin(&mut ack, &c)?;
    let journal_bytes = std::fs::read(dir.0.join("tree-harvest.journal")).map_err(|_| corrupt())?;
    let ack_bytes = std::fs::read(dir.0.join("tree-harvest.ack")).map_err(|_| corrupt())?;
    journal.original = None;
    journal.native = None;
    journal.latest = None;
    journal.high_tick = u64::MAX;
    journal.high_generation = u64::MAX;
    journal.high_sequence = u64::MAX;
    journal.high_item = u32::MAX;
    journal.samples = MAX_SAMPLES;
    journal.sample_pending = true;
    journal.continuity_lost = true;
    journal.achieved_tick = Some(0);
    let after = journal.clock_origin(&mut ack, &c)?;
    assert_eq!(before, after);
    assert_eq!(after.record().phase, 2);
    assert_eq!(after.samples(), 0);
    assert!(!after.pending_sample() && !after.continuity_lost());
    after.require_capacity(3)?;
    assert_eq!(
        std::fs::read(dir.0.join("tree-harvest.journal")).map_err(|_| corrupt())?,
        journal_bytes
    );
    assert_eq!(
        std::fs::read(dir.0.join("tree-harvest.ack")).map_err(|_| corrupt())?,
        ack_bytes
    );
    Ok(())
}

#[test]
fn tree_clock_origin_damage_to_either_custody_file_stickily_refuses_import() -> Result<()> {
    for file in ["tree-harvest.journal", "tree-harvest.ack"] {
        let (dir, mut journal, mut ack, c) = fixture()?;
        let path = dir.0.join(file);
        let original = std::fs::read(&path).map_err(|_| corrupt())?;
        let mut damaged = original.clone();
        damaged[0] ^= 1;
        std::fs::write(&path, damaged).map_err(|_| corrupt())?;
        assert!(journal.clock_origin(&mut ack, &c).is_err());
        assert!(journal.is_fenced());
        std::fs::write(&path, original).map_err(|_| corrupt())?;
        assert!(journal.clock_origin(&mut ack, &c).is_err());
    }
    Ok(())
}

#[test]
fn tree_clock_origin_preserves_stable_target_while_consumed_sample_floors_advance() -> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    let first = journal.clock_origin(&mut ack, &c)?;
    let first_target = Target::new(first.clone())?;
    journal.begin_sample(&c, &mut ack)?;
    journal.consume_uncredited(
        fixtures::progress(first.high_tick() + 10, first.high_generation())?,
        &c,
        &mut ack,
    )?;
    let next = journal.clock_origin(&mut ack, &c)?;
    assert_ne!(first.pin().head, next.pin().head);
    assert_eq!(next.high_tick(), first.high_tick() + 10);
    assert_eq!(next.high_item(), 1002);
    assert_eq!(next.samples(), 1);
    assert_eq!(next.achieved_tick(), None);
    assert_eq!(first_target.id(), Target::new(next)?.id());
    Ok(())
}

#[test]
fn tree_clock_origin_admits_complete_three_sample_capacity_and_refuses_the_next_interval()
-> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    let start = fixtures::capture()?;
    for number in 1..=14 {
        journal.begin_sample(&c, &mut ack)?;
        journal.consume_uncredited(
            fixtures::progress(start.tick + u64::from(number), start.generation)?,
            &c,
            &mut ack,
        )?;
        let origin = journal.clock_origin(&mut ack, &c)?;
        assert_eq!(origin.samples(), number);
        if number == 13 {
            origin.require_capacity(3)?;
        }
        if number == 14 {
            assert!(origin.require_capacity(3).is_err());
            origin.require_capacity(2)?;
        }
    }
    Ok(())
}

#[test]
fn tree_clock_origin_capacity_also_accounts_for_whole_frames_and_bytes() -> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    let mut origin = journal.clock_origin(&mut ack, &c)?;
    origin.pin.frames = MAX_FRAMES - 2;
    origin.require_capacity(1)?;
    assert!(origin.require_capacity(2).is_err());
    origin.pin.frames = 4;
    origin.retained_bytes = MAX_BYTES - 2 * (MAX_BODY + 84);
    origin.require_capacity(1)?;
    assert!(origin.require_capacity(2).is_err());
    assert!(origin.require_capacity(0).is_err());
    assert!(origin.require_capacity(u32::MAX).is_err());
    Ok(())
}

#[test]
fn tree_clock_origin_keeps_pending_publication_and_continuity_loss_visible() -> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    journal.begin_sample(&c, &mut ack)?;
    let pending = journal.clock_origin(&mut ack, &c)?;
    assert!(pending.pending_sample());
    assert!(pending.require_capacity(3).is_err());
    let later = fixtures::progress(pending.high_tick() + 5, pending.high_generation() + 1)?;
    journal.consume_uncredited(later, &c, &mut ack)?;
    journal.continuity_lost = false;
    journal.high_generation = 7;
    let changed = journal.clock_origin(&mut ack, &c)?;
    assert!(!changed.pending_sample());
    assert!(changed.continuity_lost());
    assert_eq!(changed.high_generation(), 8);
    assert_eq!(changed.achieved_tick(), None);
    Ok(())
}

#[test]
fn tree_clock_origin_cannot_erase_historical_supply_or_renew_its_deadline() -> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    let before = journal.clock_origin(&mut ack, &c)?;
    journal.begin_sample(&c, &mut ack)?;
    journal.sample(
        fixtures::progress(before.high_tick() + 5, before.high_generation())?,
        &c,
        &mut ack,
    )?;
    journal.achieved_tick = None;
    let after = journal.clock_origin(&mut ack, &c)?;
    assert_eq!(after.achieved_tick(), Some(before.high_tick() + 5));
    assert_eq!(after.original(), before.original());
    assert_eq!(
        Target::new(after)?.deadline_tick(),
        before.original().deadline_tick
    );
    Ok(())
}

#[test]
fn tree_clock_origin_requires_current_scoped_authority_and_complete_budget() -> Result<()> {
    let (_dir, mut journal, mut ack, c) = fixture()?;
    for case in 0..5 {
        let mut limited = c.clone();
        match case {
            0 => limited.budget.max_bytes = CLOCK_ORIGIN_BYTES - 1,
            1 => limited.grants.retain(|g| g.capability != Capability::Query),
            2 => limited.cancellation_requested = true,
            3 => {
                for grant in &mut limited.grants {
                    grant.expires_at_tick = Some(GameTick(c.anchor.tick.get() - 1));
                }
            }
            _ => {
                for grant in &mut limited.grants {
                    grant.scope.map_area = Some(journal.binding.scope);
                }
            }
        }
        assert!(
            journal.clock_origin(&mut ack, &limited).is_err(),
            "case {case}"
        );
    }
    journal.clock_origin(&mut ack, &c)?;
    Ok(())
}
