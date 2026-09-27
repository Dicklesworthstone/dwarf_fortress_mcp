//! Executable fault/replay tests for original-batch monitor custody.
//! Fixtures are synthetic native records; these tests do not qualify DFHack.
use super::*;
use crate::build_placement::journal::private_file::PrivateFileIdentity;
use crate::build_placement::journal::{BuildInventory, BuildJournal};
use crate::build_placement::{BuildBinding, BuildPlan, BuildRecord};
use crate::construction_plan::{Goal, Timing, fixtures};
use crate::furniture_batch::{BatchDefinition, FurniturePlan};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    writes: usize,
    syncs: usize,
    fail_sync: bool,
    fail_flush: bool,
    partial: Option<usize>,
    invalid: bool,
    mutate_on_sync: bool,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Memory {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            disk: Rc::new(RefCell::new(Disk {
                bytes,
                ..Disk::default()
            })),
            position: 0,
        }
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        let count = out
            .len()
            .min(disk.bytes.len().saturating_sub(self.position));
        out[..count].copy_from_slice(&disk.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(value) => self.disk.borrow().bytes.len() as i128 + i128::from(value),
            SeekFrom::Current(value) => self.position as i128 + i128::from(value),
        };
        self.position = usize::try_from(next).map_err(|_| io::Error::other("bad seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if self.position != disk.bytes.len() {
            return Err(io::Error::other("not append"));
        }
        let count = disk
            .partial
            .map_or(bytes.len(), |remaining| remaining.min(bytes.len()));
        if count == 0 {
            return Err(io::Error::other("injected torn append"));
        }
        disk.bytes.extend_from_slice(&bytes[..count]);
        disk.writes += 1;
        disk.partial = disk.partial.map(|remaining| remaining - count);
        self.position += count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.disk.borrow().fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut disk = self.disk.borrow_mut();
        disk.syncs += 1;
        if disk.mutate_on_sync && disk.bytes.len() > 80 {
            disk.bytes[80] ^= 1;
        }
        if disk.fail_sync {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair is forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.disk.borrow().invalid {
            Err(io::Error::other("changed inode"))
        } else {
            Ok(())
        }
    }
}
fn field32(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
fn field16(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn domain_hash(domain: &[u8], value: &[u8]) -> Digest32 {
    let mut bytes = domain.to_vec();
    bytes.push(0);
    bytes.extend_from_slice(value);
    Digest32::of_bytes(&bytes)
}
fn rekey(original: &BuildRecord, key: &str, placed: bool) -> Result<BuildRecord> {
    let plan = BuildPlan::new(key, original.plan().before().clone())?;
    let mut raw = b"DFMBR019".to_vec();
    field16(&mut raw, key.as_bytes());
    field16(&mut raw, plan.before().canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.extend_from_slice(if placed { &[2, 0, 1, 1] } else { &[0, 0, 0, 0] });
    if placed {
        field16(
            &mut raw,
            original.after().ok_or_else(corrupt)?.canonical_bytes(),
        );
        field16(
            &mut raw,
            original.insertion().ok_or_else(corrupt)?.canonical_bytes(),
        );
    }
    let digest = domain_hash(b"dfmcp-build-receipt/1", &raw);
    raw.extend_from_slice(digest.as_bytes());
    BuildRecord::decode(&raw)
}
fn append_original(
    raw: &mut Vec<u8>,
    head: &mut Digest32,
    sequence: &mut u64,
    plan: &BuildPlan,
    state: u8,
    native: Option<&BuildRecord>,
) {
    let mut body = vec![state, u8::from(matches!(state, 2 | 5)), 0];
    field32(&mut body, &plan.canonical_bytes());
    field32(&mut body, native.map_or(&[], BuildRecord::canonical_bytes));
    let mut frame = b"DFMBJF19".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    *sequence += 1;
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(&body);
    *head = domain_hash(b"dfmcp-build-journal-frame/1", &frame);
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(b"DFMBJEND");
    raw.extend_from_slice(&frame);
}
fn identity(name: &str, inode: u64) -> PrivateFileIdentity {
    PrivateFileIdentity {
        path: PathBuf::from(format!("/private/construction/{name}")),
        file_device: 7,
        file_inode: inode,
        file_owner: 1000,
        directory_device: 7,
        directory_inode: 10,
        directory_owner: 1000,
    }
}
struct Fixture {
    definition: MonitorDefinition,
    context: OperationContext,
    inventory: BuildInventory,
}
fn fixture() -> Result<Fixture> {
    let originals = fixtures::goal(2);
    let software = fixtures::sample(&originals, fixtures::TICK).before;
    let first = originals.records().first().ok_or_else(corrupt)?;
    let binding = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        &software.df_version,
        &software.dfhack_version,
        first.plan().before(),
    )?;
    let mut raw = b"DFMBJ019".to_vec();
    field32(&mut raw, &binding.canonical_bytes());
    raw.extend_from_slice(&[1; 32]);
    let journal_id = domain_hash(b"dfmcp-build-journal/1", &raw);
    raw.extend_from_slice(journal_id.as_bytes());
    let mut steps = Vec::new();
    for (index, record) in originals.records().iter().enumerate() {
        let selection = record.plan().before().selection();
        let dependency = if index == 0 {
            String::new()
        } else {
            format!(",\"after\":[\"s{:02}\"]", index - 1)
        };
        steps.push(format!("{{\"name\":\"s{index:02}\",\"kind\":\"{}\",\"item\":{},\"target\":[{},{},{}]{dependency}}}",
            selection.kind().as_str(), selection.item_id(), selection.target()[0], selection.target()[1], selection.target()[2]));
    }
    let json = format!(
        "{{\"schema\":\"dfmcp.furniture-plan/1\",\"steps\":[{}]}}",
        steps.join(",")
    );
    let batch = BatchDefinition::new(
        FurniturePlan::decode(json.as_bytes())?,
        binding.clone(),
        journal_id,
    )?;
    let mut head = journal_id;
    let mut sequence = 0;
    let mut receipts = Vec::new();
    for (step, original) in batch.plan().ordered_steps().zip(originals.records()) {
        let key = batch.key(step);
        let prepared = rekey(original, &key, false)?;
        let placed = rekey(original, &key, true)?;
        append_original(&mut raw, &mut head, &mut sequence, placed.plan(), 0, None);
        append_original(
            &mut raw,
            &mut head,
            &mut sequence,
            placed.plan(),
            1,
            Some(&prepared),
        );
        append_original(
            &mut raw,
            &mut head,
            &mut sequence,
            placed.plan(),
            2,
            Some(&prepared),
        );
        append_original(
            &mut raw,
            &mut head,
            &mut sequence,
            placed.plan(),
            5,
            Some(&placed),
        );
        receipts.push(placed);
    }
    let goal = Goal::new(
        receipts,
        Timing {
            deadline: fixtures::TICK + 1000,
            interval: 10,
            stable_samples: 2,
            stable_span: 10,
            max_gap: 1200,
            max_observations: 512,
        },
    )?;
    let mut context = fixtures::context(&goal);
    context.budget.max_bytes = 1024 * 1024 * 1024;
    context.budget.max_wall_millis = 60_000;
    let mut original = BuildJournal::open(
        Memory::from_bytes(raw),
        &context,
        BuildMode::Recover,
        Some(binding.clone()),
        None,
    )?;
    let inventory = original.inventory(&context)?;
    let origin = Origin::from_batch(
        batch,
        &binding,
        &inventory,
        identity("batch", 11),
        identity("placements", 12),
    )?;
    Ok(Fixture {
        definition: MonitorDefinition::new(origin, goal)?,
        context,
        inventory,
    })
}
fn admit(_: &MonitorDefinition, _: &Progress) -> Result<()> {
    Ok(())
}
fn guard(_: &Origin, _: &OperationContext) -> Result<()> {
    Ok(())
}
fn open(fixture: &Fixture, disk: Memory) -> Result<MonitorStore<Memory>> {
    MonitorStore::open(
        disk,
        &fixture.context,
        BuildMode::Recover,
        Some(fixture.definition.clone()),
        true,
    )
}
fn sample(store: &mut MonitorStore<Memory>, fixture: &Fixture, tick: u64) -> Result<()> {
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let evidence = fixtures::sample(store.definition().goal(), tick);
    store.publish_sample(&fixture.context, &evidence, &mut admit, &mut guard)
}

#[test]
fn original_batch_and_definition_roundtrip_keep_every_exact_step() -> Result<()> {
    let fixture = fixture()?;
    let definition = MonitorDefinition::decode(fixture.definition.canonical_bytes())?;
    assert_eq!(definition.id(), fixture.definition.id());
    assert_eq!(definition.origin().receipts().len(), 2);
    assert_eq!(definition.origin().definition().plan().steps().len(), 2);
    let origin = definition.origin();
    origin.verify_batch(
        origin.definition(),
        origin.definition().binding(),
        &fixture.inventory,
        origin.parent_identity(),
        origin.child_identity(),
    )?;
    Ok(())
}
#[test]
fn incomplete_selection_and_changed_original_custody_are_refused() -> Result<()> {
    let fixture = fixture()?;
    let origin = fixture.definition.origin();
    let omitted = Goal::new(
        vec![origin.receipts()[0].clone()],
        fixture.definition.goal().timing(),
    )?;
    assert!(MonitorDefinition::new(origin.clone(), omitted).is_err());
    let mut replaced = origin.child_identity().clone();
    replaced.file_inode += 1;
    assert!(
        origin
            .verify_batch(
                origin.definition(),
                origin.definition().binding(),
                &fixture.inventory,
                origin.parent_identity(),
                &replaced
            )
            .is_err()
    );
    let mut changed_head = fixture.inventory.clone();
    changed_head.head = Digest32::of_bytes(b"other original journal");
    assert!(
        origin
            .verify_batch(
                origin.definition(),
                origin.definition().binding(),
                &changed_head,
                origin.parent_identity(),
                origin.child_identity()
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn query_only_start_samples_and_cancel_never_need_construct() -> Result<()> {
    let mut fixture = fixture()?;
    fixture
        .context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut store = open(&fixture, Memory::default())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    assert_eq!(store.progress().observations, 1);
    store.cancel(&fixture.context, &mut admit)?;
    assert_eq!(store.progress().phase, "cancelled");
    Ok(())
}
#[test]
fn complete_plan_is_satisfied_then_replayed_offline_without_writes() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    assert_eq!(store.progress().streak, 1);
    sample(&mut store, &fixture, fixtures::TICK + 10)?;
    assert_eq!(store.progress().phase, "satisfied");
    let head = store.head();
    let raw = disk.disk.borrow().bytes.clone();
    let writes = disk.disk.borrow().writes;
    drop(store);
    let mut reopened = MonitorStore::open(
        disk.clone(),
        &fixture.context,
        BuildMode::Offline,
        None,
        false,
    )?;
    assert_eq!(reopened.head(), head);
    assert_eq!(reopened.progress().phase, "satisfied");
    reopened.begin_read(&fixture.context, &mut admit, &mut guard)?;
    reopened.cancel(&fixture.context, &mut admit)?;
    assert_eq!(disk.disk.borrow().writes, writes);
    assert_eq!(disk.disk.borrow().bytes, raw);
    Ok(())
}
#[test]
fn reopening_unknown_read_never_restores_publication_permission() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    assert!(store.read_owned());
    drop(store);
    let mut reopened = MonitorStore::open(disk, &fixture.context, BuildMode::Recover, None, false)?;
    assert!(reopened.progress().reading);
    assert!(!reopened.read_owned());
    let evidence = fixtures::sample(reopened.definition().goal(), fixtures::TICK + 10);
    assert!(
        reopened
            .publish_sample(&fixture.context, &evidence, &mut admit, &mut guard)
            .is_err()
    );
    reopened.begin_read(&fixture.context, &mut admit, &mut guard)?;
    assert_eq!(reopened.progress().interruptions, 1);
    assert_eq!(reopened.progress().streak, 0);
    reopened.publish_sample(&fixture.context, &evidence, &mut admit, &mut guard)?;
    assert_eq!(reopened.progress().phase, "candidate");
    assert_eq!(reopened.progress().streak, 1);
    Ok(())
}
#[test]
fn failed_acquisition_abandons_permit_and_restarts_shared_stability() -> Result<()> {
    let fixture = fixture()?;
    let mut store = open(&fixture, Memory::default())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    store.abandon_read();
    assert!(store.progress().reading);
    assert!(!store.read_owned());
    sample(&mut store, &fixture, fixtures::TICK + 10)?;
    assert_eq!(store.progress().phase, "candidate");
    assert_eq!(store.progress().interruptions, 1);
    assert_eq!(store.progress().streak, 1);
    Ok(())
}
#[test]
fn rendering_failure_retains_only_the_unknown_read_intent() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let raw = disk.disk.borrow().bytes.clone();
    let evidence = fixtures::sample(store.definition().goal(), fixtures::TICK);
    assert!(
        store
            .publish_sample(
                &fixture.context,
                &evidence,
                &mut |_, _| Err(bounded()),
                &mut guard
            )
            .is_err()
    );
    assert_eq!(disk.disk.borrow().bytes, raw);
    assert!(store.progress().reading);
    assert!(!store.read_owned());
    assert_eq!(store.progress().observations, 0);
    store.cancel(&fixture.context, &mut admit)?;
    assert_eq!(store.progress().phase, "cancelled");
    Ok(())
}
#[test]
fn original_loss_before_publication_allows_only_monitor_cancellation() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let raw = disk.disk.borrow().bytes.clone();
    let evidence = fixtures::sample(store.definition().goal(), fixtures::TICK);
    let mut calls = 0;
    assert!(
        store
            .publish_sample(&fixture.context, &evidence, &mut admit, &mut |_, _| {
                calls += 1;
                if calls == 2 { Err(corrupt()) } else { Ok(()) }
            })
            .is_err()
    );
    assert_eq!(disk.disk.borrow().bytes, raw);
    assert!(store.progress().reading);
    assert!(!store.is_fenced());
    store.cancel(&fixture.context, &mut admit)?;
    assert_eq!(store.progress().phase, "cancelled");
    Ok(())
}
#[test]
fn original_loss_after_sync_fences_before_acknowledging_a_sample() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let evidence = fixtures::sample(store.definition().goal(), fixtures::TICK);
    let mut calls = 0;
    assert!(
        store
            .publish_sample(&fixture.context, &evidence, &mut admit, &mut |_, _| {
                calls += 1;
                if calls == 3 { Err(corrupt()) } else { Ok(()) }
            })
            .is_err()
    );
    assert!(store.is_fenced());
    assert!(store.progress().reading);
    assert_eq!(store.progress().observations, 0);
    drop(store);
    let mut reopened = MonitorStore::open(disk, &fixture.context, BuildMode::Recover, None, false)?;
    assert_eq!(reopened.progress().observations, 1);
    assert!(
        reopened
            .verify_origin(&fixture.context, &mut |_, _| Err(corrupt()))
            .is_err()
    );
    reopened.cancel(&fixture.context, &mut admit)?;
    Ok(())
}
#[test]
fn every_ambiguous_write_boundary_fences_and_never_repairs_history() -> Result<()> {
    for fault in 0..4 {
        let fixture = fixture()?;
        let disk = Memory::default();
        let mut store = open(&fixture, disk.clone())?;
        match fault {
            0 => disk.disk.borrow_mut().fail_sync = true,
            1 => disk.disk.borrow_mut().fail_flush = true,
            2 => disk.disk.borrow_mut().partial = Some(17),
            _ => disk.disk.borrow_mut().mutate_on_sync = true,
        }
        assert!(
            store
                .begin_read(&fixture.context, &mut admit, &mut guard)
                .is_err()
        );
        assert!(store.is_fenced());
        assert!(!store.read_owned());
        let bytes = disk.disk.borrow().bytes.clone();
        assert!(store.cancel(&fixture.context, &mut admit).is_err());
        assert_eq!(disk.disk.borrow().bytes, bytes);
        drop(store);
        disk.disk.borrow_mut().fail_sync = false;
        disk.disk.borrow_mut().fail_flush = false;
        disk.disk.borrow_mut().partial = None;
        disk.disk.borrow_mut().mutate_on_sync = false;
        let replay = MonitorStore::open(
            disk.clone(),
            &fixture.context,
            BuildMode::Recover,
            None,
            false,
        );
        if fault < 2 {
            assert!(replay?.progress().reading);
        } else {
            assert!(replay.is_err());
        }
        assert_eq!(disk.disk.borrow().bytes, bytes);
    }
    Ok(())
}
#[test]
fn corruption_and_truncation_replay_refuse_unchanged_bytes() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    let raw = disk.disk.borrow().bytes.clone();
    drop(store);
    for offset in [8, 16, 80, raw.len() - 1] {
        let mut corrupted = raw.clone();
        corrupted[offset] ^= 1;
        let file = Memory::from_bytes(corrupted.clone());
        assert!(
            MonitorStore::open(
                file.clone(),
                &fixture.context,
                BuildMode::Recover,
                None,
                false
            )
            .is_err()
        );
        assert_eq!(file.disk.borrow().bytes, corrupted);
    }
    let shortened = raw[..raw.len() - 1].to_vec();
    let file = Memory::from_bytes(shortened.clone());
    assert!(
        MonitorStore::open(
            file.clone(),
            &fixture.context,
            BuildMode::Recover,
            None,
            false
        )
        .is_err()
    );
    assert_eq!(file.disk.borrow().bytes, shortened);
    Ok(())
}
#[test]
fn placement_time_software_is_required_on_first_sample_and_on_replay() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let mut evidence = fixtures::sample(store.definition().goal(), fixtures::TICK);
    evidence.before.df_version = "different-df".to_owned();
    evidence.after.df_version = "different-df".to_owned();
    evidence.operations.df_version = "different-df".to_owned();
    let payload = evidence.canonical_bytes()?;
    let forged = frame(store.frames(), store.head(), 3, &payload)?;
    let mut raw = disk.disk.borrow().bytes.clone();
    raw.extend_from_slice(&forged);
    assert!(
        store
            .publish_sample(&fixture.context, &evidence, &mut admit, &mut guard)
            .is_err()
    );
    let file = Memory::from_bytes(raw.clone());
    assert!(
        MonitorStore::open(
            file.clone(),
            &fixture.context,
            BuildMode::Recover,
            None,
            false
        )
        .is_err()
    );
    assert_eq!(file.disk.borrow().bytes, raw);
    Ok(())
}
#[test]
fn authority_and_insufficient_reservation_refuse_before_new_read_intent() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    let raw = disk.disk.borrow().bytes.clone();
    let mut denied = fixture.context.clone();
    denied.grants.clear();
    assert!(store.begin_read(&denied, &mut admit, &mut guard).is_err());
    let mut small = fixture.context.clone();
    small.budget.max_bytes = (store.byte_len() + 10) as u64;
    assert!(store.begin_read(&small, &mut admit, &mut guard).is_err());
    assert_eq!(disk.disk.borrow().bytes, raw);
    assert!(!store.read_owned());
    Ok(())
}
#[test]
fn terminal_admission_cannot_hide_changed_monitor_or_original_custody() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    sample(&mut store, &fixture, fixtures::TICK + 10)?;
    let mut calls = 0;
    assert!(
        store
            .begin_read(&fixture.context, &mut admit, &mut |_, _| {
                calls += 1;
                if calls == 2 { Err(corrupt()) } else { Ok(()) }
            })
            .is_err()
    );
    assert_eq!(store.progress().phase, "satisfied");
    assert!(
        store
            .begin_read(
                &fixture.context,
                &mut |_, _| {
                    disk.disk.borrow_mut().bytes[80] ^= 1;
                    Ok(())
                },
                &mut guard
            )
            .is_err()
    );
    assert!(store.is_fenced());
    Ok(())
}

#[test]
fn original_receipt_clock_floor_denies_expired_query_before_creation() -> Result<()> {
    let mut fixture = fixture()?;
    let floor = fixture
        .definition
        .origin()
        .receipts()
        .iter()
        .map(|record| record.plan().before().tick())
        .max()
        .ok_or_else(corrupt)?;
    fixture.context.anchor.tick = GameTick(0);
    for grant in &mut fixture.context.grants {
        grant.expires_at_tick = Some(GameTick(floor - 1));
    }
    let disk = Memory::default();
    assert!(open(&fixture, disk.clone()).is_err());
    assert!(disk.disk.borrow().bytes.is_empty());
    assert_eq!(disk.disk.borrow().writes, 0);
    Ok(())
}

#[test]
fn replayed_samples_share_the_same_semantic_work_allowance() -> Result<()> {
    let fixture = fixture()?;
    let sample = fixtures::sample(fixture.definition.goal(), fixtures::TICK);
    let payload = sample.canonical_bytes()?;
    let initial = Progress::new(fixture.definition.goal()).begin_read()?;
    let mut work = Work::new(&fixture.context)?;
    let candidate = transition(&fixture.definition, &initial, 3, &payload, &mut work)?;
    let first_cost = work.semantic_work;
    assert!(first_cost > 2);
    // Model earlier retained frames consuming most of the same replay budget.
    // Another individually valid sample must not receive a fresh 20M allowance.
    work.semantic_work = crate::construction_plan::MAX_WORK - first_cost / 2;
    let interrupted = candidate.begin_read()?;
    assert!(transition(&fixture.definition, &interrupted, 3, &payload, &mut work).is_err());
    assert!(work.semantic_work > crate::construction_plan::MAX_WORK);
    Ok(())
}
