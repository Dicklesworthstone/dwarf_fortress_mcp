use super::super::super::archive::ArchivedProgress;
use super::super::{WatchState, tests as fixture};
use super::*;
use std::cell::RefCell;
use std::io::{Cursor, Read, Seek, Write};
use std::rc::Rc;

#[derive(Default)]
struct Faults {
    syncs: usize,
    fail_sync: Option<usize>,
    write_end: Option<u64>,
    custody_lost: bool,
}
#[derive(Clone, Default)]
struct Store {
    data: Rc<RefCell<Vec<u8>>>,
    faults: Rc<RefCell<Faults>>,
    position: u64,
}
impl Store {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            data: Rc::new(RefCell::new(bytes)),
            ..Self::default()
        }
    }
    fn bytes(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }
}
impl Read for Store {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let data = self.data.borrow();
        let mut cursor = Cursor::new(data.as_slice());
        cursor.set_position(self.position);
        let count = cursor.read(out)?;
        self.position = cursor.position();
        Ok(count)
    }
}
impl Write for Store {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self.faults.borrow().write_end;
        let n = end.map_or(bytes.len(), |n| {
            n.saturating_sub(self.position).min(bytes.len() as u64) as usize
        });
        if n == 0 {
            return Err(io::Error::other("injected torn write"));
        }
        let mut data = self.data.borrow_mut();
        let mut cursor = Cursor::new(&mut *data);
        cursor.set_position(self.position);
        let count = cursor.write(&bytes[..n])?;
        self.position = cursor.position();
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Store {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let data = self.data.borrow();
        let mut cursor = Cursor::new(data.as_slice());
        cursor.set_position(self.position);
        self.position = cursor.seek(from)?;
        Ok(self.position)
    }
}
impl JournalStorage for Store {
    fn sync(&mut self) -> io::Result<()> {
        let mut f = self.faults.borrow_mut();
        f.syncs += 1;
        if f.fail_sync == Some(f.syncs) {
            Err(io::Error::other("injected sync"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.faults.borrow().custody_lost {
            Err(io::Error::other("lost custody"))
        } else {
            Ok(())
        }
    }
}
fn ctx() -> Result<OperationContext> {
    fixture::context()
}
fn archive() -> Result<(ProgressArchive<Store>, Store)> {
    let s = Store::default();
    Ok((
        ProgressArchive::open(s.clone(), ArchiveMode::Live, true, &ctx()?)?,
        s,
    ))
}
fn append(
    a: &mut ProgressArchive<Store>,
    seq: u64,
    tick: u64,
    flags: u32,
) -> Result<ArchivedProgress> {
    let e = a.append(
        &fixture::manifest(),
        &fixture::observation(seq, tick, 3, flags)?,
        &ctx()?,
    )?;
    a.record(e.number, e.record_digest, &ctx()?)
}
fn spec(key: &str) -> Result<WatchSpec> {
    WatchSpec::new(key, 3, WatchGoal::Validated, 30, 1, 2)
}
fn book(a: &mut ProgressArchive<Store>) -> Result<(WatchBook<Store>, Store)> {
    let s = Store::default();
    Ok((
        WatchBook::open(s.clone(), ArchiveMode::Live, true, a, &ctx()?)?,
        s,
    ))
}
fn forged_event(store: &Store, event: Event) -> Result<()> {
    let bytes = store.bytes();
    let mut r = Reader {
        bytes: &bytes[48..80],
    };
    let id = Digest32::from_bytes(r.array()?);
    let mut offset = HEADER_BYTES as usize;
    let mut number = 1;
    while offset < bytes.len() {
        let size = u32::from_be_bytes(
            bytes[offset + 8..offset + 12]
                .try_into()
                .map_err(|_| corrupt("test frame"))?,
        ) as usize;
        offset += PREFIX + size + 40;
        number += 1;
    }
    let previous = if number == 1 {
        &bytes[80..112]
    } else {
        &bytes[bytes.len() - 40..bytes.len() - 8]
    };
    let body = encode_event(&event);
    let mut prefix = FRAME.to_vec();
    prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
    prefix.extend_from_slice(&(number as u64).to_be_bytes());
    prefix.extend_from_slice(previous);
    let hash = frame_hash(id, &prefix, &body);
    let mut frame = prefix;
    frame.extend_from_slice(&body);
    frame.extend_from_slice(hash.as_bytes());
    frame.extend_from_slice(FOOTER);
    store.data.borrow_mut().extend_from_slice(&frame);
    Ok(())
}
#[test]
fn registration_is_durable_discoverable_and_replay_never_renews() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    let one = b.register(spec("z")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    b.register(spec("a")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    let bytes = s.bytes();
    assert_eq!(
        b.register(spec("z")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?,
        one
    );
    assert_eq!(bytes, s.bytes());
    let changed = WatchSpec::new("z", 3, WatchGoal::Validated, 31, 1, 2)?;
    assert!(
        b.register(changed, WatchRecordRef::of(&origin), &mut a, &ctx()?)
            .is_err()
    );
    let mut reopened = WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    assert_eq!(
        reopened
            .summary(&mut a, &ctx()?)?
            .definitions
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(
        reopened.definition("z", one.definition.digest(), &mut a, &ctx()?)?,
        one
    );
    Ok(())
}
#[test]
fn appended_samples_survive_crash_before_any_watch_evaluation() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    let x = append(&mut a, 2, 11, 1)?;
    let y = append(&mut a, 3, 12, 1)?;
    drop(b);
    let before = s.bytes();
    let mut offline = WatchBook::open(s.clone(), ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    let result = offline.evaluate(&mut a, &ctx()?)?;
    assert_eq!(result.results[0].state(), WatchState::SatisfiedObservation);
    assert_eq!(
        result.results[0].samples(),
        [WatchRecordRef::of(&x), WatchRecordRef::of(&y)]
    );
    assert_eq!(s.bytes(), before);
    Ok(())
}
#[test]
fn cancellation_replays_in_order_and_cannot_rewrite_satisfaction() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    let d = b
        .register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?
        .definition;
    let x = append(&mut a, 2, 11, 1)?;
    assert!(
        b.cancel(
            "key",
            d.digest(),
            origin.entry.record_digest,
            &mut a,
            &ctx()?
        )
        .is_err()
    );
    b.cancel("key", d.digest(), x.entry.record_digest, &mut a, &ctx()?)?;
    let retained = s.bytes();
    b.cancel("key", d.digest(), Digest32::ZERO, &mut a, &ctx()?)?;
    assert_eq!(s.bytes(), retained);
    append(&mut a, 3, 12, 1)?;
    let mut offline = WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    assert_eq!(
        offline.evaluate(&mut a, &ctx()?)?.results[0].state(),
        WatchState::Cancelled
    );
    let origin = append(&mut a, 4, 13, 0)?;
    let (mut b, _) = book(&mut a)?;
    let d = b
        .register(spec("other")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?
        .definition;
    append(&mut a, 5, 14, 1)?;
    let end = append(&mut a, 6, 15, 1)?;
    assert!(
        b.cancel(
            "other",
            d.digest(),
            end.entry.record_digest,
            &mut a,
            &ctx()?
        )
        .is_err()
    );
    assert_eq!(
        b.evaluate(&mut a, &ctx()?)?.results[0].state(),
        WatchState::SatisfiedObservation
    );
    Ok(())
}
#[test]
fn rehashed_illegal_cancellations_and_duplicate_registrations_are_rejected() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    let d = b
        .register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?
        .definition;
    append(&mut a, 2, 11, 1)?;
    let end = append(&mut a, 3, 12, 1)?;
    for event in [
        Event::Cancel("key".into(), d.digest(), WatchRecordRef::of(&end)),
        Event::Register(spec("key")?, WatchRecordRef::of(&origin), d.digest()),
        Event::Cancel("unknown".into(), d.digest(), WatchRecordRef::of(&end)),
    ] {
        let s = Store::from_bytes(s.bytes());
        forged_event(&s, event)?;
        assert!(WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?).is_err());
    }
    Ok(())
}
#[test]
fn every_corruption_and_incomplete_prefix_is_refused_without_repair() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    let original = s.bytes();
    for i in 0..original.len() {
        let mut bytes = original.clone();
        bytes[i] ^= 1;
        let s = Store::from_bytes(bytes.clone());
        assert!(
            WatchBook::open(s.clone(), ArchiveMode::Offline, false, &mut a, &ctx()?).is_err(),
            "byte {i}"
        );
        assert_eq!(s.bytes(), bytes);
    }
    for end in 0..original.len() {
        if end == HEADER_BYTES as usize {
            continue;
        }
        let s = Store::from_bytes(original[..end].to_vec());
        assert!(
            WatchBook::open(s.clone(), ArchiveMode::Offline, false, &mut a, &ctx()?).is_err(),
            "prefix {end}"
        );
        assert_eq!(s.bytes(), original[..end]);
    }
    Ok(())
}
#[test]
fn failed_sync_fences_acknowledgement_but_complete_event_recovers() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    s.faults.borrow_mut().fail_sync = Some(2);
    assert!(
        b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)
            .is_err()
    );
    assert!(b.records.is_empty());
    assert!(b.summary(&mut a, &ctx()?).is_err());
    let mut recovered = WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    assert_eq!(recovered.summary(&mut a, &ctx()?)?.definitions.len(), 1);
    Ok(())
}
#[test]
fn torn_event_fences_owner_and_blocks_reopen() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    s.faults.borrow_mut().write_end = Some(HEADER_BYTES + 17);
    assert!(
        b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)
            .is_err()
    );
    assert!(b.records.is_empty());
    assert!(b.evaluate(&mut a, &ctx()?).is_err());
    let bytes = s.bytes();
    assert!(WatchBook::open(s.clone(), ArchiveMode::Offline, false, &mut a, &ctx()?).is_err());
    assert_eq!(s.bytes(), bytes);
    Ok(())
}
#[test]
fn reopened_archive_invalidates_pending_stability_not_historical_terminal() -> Result<()> {
    let (mut a, storage) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    b.register(
        spec("pending")?,
        WatchRecordRef::of(&origin),
        &mut a,
        &ctx()?,
    )?;
    b.register(
        WatchSpec::new("done", 3, WatchGoal::Validated, 30, 1, 1)?,
        WatchRecordRef::of(&origin),
        &mut a,
        &ctx()?,
    )?;
    append(&mut a, 2, 11, 1)?;
    drop(a);
    drop(b);
    let mut a = ProgressArchive::open(storage, ArchiveMode::Live, false, &ctx()?)?;
    append(&mut a, 3, 12, 1)?;
    let mut b = WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    let batch = b.evaluate(&mut a, &ctx()?)?;
    assert_eq!(batch.results[0].state(), WatchState::SatisfiedObservation);
    assert_eq!(batch.results[1].state(), WatchState::ContinuityLost);
    Ok(())
}
#[test]
fn modes_current_authority_horizon_and_pairing_never_widen() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    let mut low = ctx()?;
    low.budget.max_game_ticks = 1;
    assert!(
        b.register(spec("low")?, WatchRecordRef::of(&origin), &mut a, &low)
            .is_err()
    );
    let d = b
        .register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?
        .definition;
    let mut off = WatchBook::open(s, ArchiveMode::Offline, false, &mut a, &ctx()?)?;
    assert!(
        off.register(spec("extra")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)
            .is_err()
    );
    assert!(
        off.cancel(
            "key",
            d.digest(),
            origin.entry.record_digest,
            &mut a,
            &ctx()?
        )
        .is_err()
    );
    append(&mut a, 2, 20, 0)?;
    let mut expired = ctx()?;
    for g in &mut expired.grants {
        g.expires_at_tick = Some(GameTick(15));
    }
    assert!(off.summary(&mut a, &expired).is_err());
    let mut foreign = ctx()?;
    foreign.session_id = dfmcp_core::SessionId::new(99);
    let mut other = ProgressArchive::open(Store::default(), ArchiveMode::Live, true, &foreign)?;
    append(&mut other, 1, 10, 0)?;
    assert!(off.evaluate(&mut other, &ctx()?).is_err());
    Ok(())
}
#[test]
fn budget_refusal_and_custody_loss_never_return_partial_watch_sets() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, s) = book(&mut a)?;
    b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    let bytes = s.bytes();
    let mut low = ctx()?;
    low.budget.max_bytes = 1;
    assert!(b.evaluate(&mut a, &low).is_err());
    assert_eq!(s.bytes(), bytes);
    s.faults.borrow_mut().custody_lost = true;
    assert!(b.summary(&mut a, &ctx()?).is_err());
    s.faults.borrow_mut().custody_lost = false;
    assert!(b.summary(&mut a, &ctx()?).is_err());
    Ok(())
}
#[test]
fn all_watches_share_one_complete_archive_walk_and_retention_is_finite() -> Result<()> {
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    let (mut b, _) = book(&mut a)?;
    for i in 0..MAX_WATCHES {
        b.register(
            spec(&format!("w{i:02}"))?,
            WatchRecordRef::of(&origin),
            &mut a,
            &ctx()?,
        )?;
    }
    assert!(
        b.register(
            spec("overflow")?,
            WatchRecordRef::of(&origin),
            &mut a,
            &ctx()?
        )
        .is_err()
    );
    append(&mut a, 2, 11, 1)?;
    append(&mut a, 3, 12, 1)?;
    // Enough for three complete frames plus a metadata page, not 32 separate scans.
    let mut limited = ctx()?;
    limited.budget.max_bytes = 3 * MAX_FRAME_BYTES + 64 * 512 + 1;
    let batch = b.evaluate(&mut a, &limited)?;
    assert_eq!(batch.results.len(), 32);
    assert!(
        batch
            .results
            .iter()
            .all(|r| r.state() == WatchState::SatisfiedObservation)
    );
    Ok(())
}

fn hex(raw: &str) -> Result<Vec<u8>> {
    raw.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let t = std::str::from_utf8(p).map_err(|_| corrupt("test hex"))?;
            u8::from_str_radix(t, 16).map_err(|_| corrupt("test hex"))
        })
        .collect()
}
#[test]
fn independent_python_vectors_replay_through_real_rust_archive_and_book_codecs() -> Result<()> {
    let a = hex(include_str!(
        "../../tests/fixtures/progress_watch_archive_v1.hex"
    ))?;
    let b = hex(include_str!(
        "../../tests/fixtures/progress_watch_book_v1.hex"
    ))?;
    let mut archive =
        ProgressArchive::open(Store::from_bytes(a), ArchiveMode::Offline, false, &ctx()?)?;
    let mut book = WatchBook::open(
        Store::from_bytes(b),
        ArchiveMode::Offline,
        false,
        &mut archive,
        &ctx()?,
    )?;
    let batch = book.evaluate(&mut archive, &ctx()?)?;
    assert_eq!(batch.results.len(), 1);
    assert_eq!(batch.results[0].definition().spec().key(), "key");
    assert_eq!(batch.results[0].state(), WatchState::SatisfiedObservation);
    assert_eq!(
        batch.results[0]
            .samples()
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn private_book_lock_modes_replacement_and_offline_noncreation()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::fs;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let name = format!(
        "dfmcp-watch-book-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let parent = std::env::temp_dir().canonicalize()?.join(name);
    fs::DirBuilder::new().mode(0o700).create(&parent)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(parent.clone());
    let path = parent.join("watches.bin");
    let (mut a, _) = archive()?;
    let origin = append(&mut a, 1, 10, 0)?;
    assert!(open_watch_book(&path, ArchiveMode::Offline, &mut a, &ctx()?).is_err());
    assert!(!path.exists());
    let mut b = open_watch_book(&path, ArchiveMode::Live, &mut a, &ctx()?)?;
    b.register(spec("key")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)?;
    assert!(open_watch_book(&path, ArchiveMode::Live, &mut a, &ctx()?).is_err());
    drop(b);
    let before = fs::read(&path)?;
    let mut offline = open_watch_book(&path, ArchiveMode::Offline, &mut a, &ctx()?)?;
    assert!(
        offline
            .register(spec("new")?, WatchRecordRef::of(&origin), &mut a, &ctx()?)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, before);
    drop(offline);
    let mut b = open_watch_book(&path, ArchiveMode::Live, &mut a, &ctx()?)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    assert!(b.summary(&mut a, &ctx()?).is_err());
    drop(b);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let mut b = open_watch_book(&path, ArchiveMode::Live, &mut a, &ctx()?)?;
    fs::rename(&path, parent.join("retained.bin"))?;
    fs::write(&path, &before)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    assert!(b.summary(&mut a, &ctx()?).is_err());
    Ok(())
}
