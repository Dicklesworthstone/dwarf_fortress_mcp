//! Actual association-store methods with counted mutable storage. No native or
//! production runtime substitute is involved in these custody regressions.
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId, StateAnchor,
    WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::rc::Rc;

#[derive(Clone, Copy)]
enum AfterRead {
    Append,
    ReplaceIdentity,
}

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    identity: u64,
    reads: usize,
    seeks: usize,
    identities: usize,
    writes: usize,
    flushes: usize,
    syncs: usize,
    truncates: usize,
    after_read: Option<AfterRead>,
}
impl Disk {
    fn calls(&self) -> [usize; 7] {
        [
            self.reads,
            self.seeks,
            self.identities,
            self.writes,
            self.flushes,
            self.syncs,
            self.truncates,
        ]
    }
    fn writes(&self) -> [usize; 4] {
        [self.writes, self.flushes, self.syncs, self.truncates]
    }
}

#[derive(Clone)]
struct Storage {
    disk: Rc<RefCell<Disk>>,
    identity: u64,
    position: usize,
}
impl Storage {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            disk: Rc::new(RefCell::new(Disk {
                bytes,
                identity: 1,
                ..Disk::default()
            })),
            identity: 1,
            position: 0,
        }
    }
}
impl Read for Storage {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.reads += 1;
        let start = self.position.min(disk.bytes.len());
        let length = out.len().min(disk.bytes.len() - start);
        out[..length].copy_from_slice(&disk.bytes[start..start + length]);
        self.position += length;
        match disk.after_read.take() {
            Some(AfterRead::Append) => disk.bytes.push(0x31),
            Some(AfterRead::ReplaceIdentity) => disk.identity += 1,
            None => {}
        }
        Ok(length)
    }
}
impl Seek for Storage {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let mut disk = self.disk.borrow_mut();
        disk.seeks += 1;
        let position = match offset {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(delta) => disk.bytes.len() as i128 + i128::from(delta),
            SeekFrom::Current(delta) => self.position as i128 + i128::from(delta),
        };
        self.position = usize::try_from(position).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Storage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.writes += 1;
        if self.position != disk.bytes.len() {
            return Err(io::Error::other("association must append to new custody"));
        }
        disk.bytes.extend_from_slice(bytes);
        self.position += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.disk.borrow_mut().flushes += 1;
        Ok(())
    }
}
impl EffectJournalStorage for Storage {
    fn sync(&mut self) -> io::Result<()> {
        self.disk.borrow_mut().syncs += 1;
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        self.disk.borrow_mut().truncates += 1;
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        let mut disk = self.disk.borrow_mut();
        disk.identities += 1;
        if disk.identity != self.identity {
            return Err(io::Error::other("private identity changed"));
        }
        Ok(())
    }
}

fn context(plan: bool) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"association test source"),
        },
        budget: WorkBudget {
            max_bytes: STORE_BYTES,
            max_game_ticks: 0,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Plan]
            .into_iter()
            .filter(|capability| plan || *capability == Capability::Query)
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}

fn bytes() -> Vec<u8> {
    // Enough to exercise more than one storage read/write chunk. These bytes
    // stand for the already-qualified canonical origin; the store compares the
    // entire expected value and never decodes or trusts fields from the file.
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(Digest32::of_bytes(b"original production plan").as_bytes());
    bytes.extend((0..9000).map(|index| (index % 251) as u8));
    bytes
}

#[test]
fn manager_association_exact_allowance_creates_then_query_only_reopens_without_writes() -> Result<()>
{
    let expected = bytes();
    let storage = Storage::new(Vec::new());
    let disk = Rc::clone(&storage.disk);
    let created = AssociationStore::create(storage.clone(), expected.clone(), &context(true))?;
    assert_eq!(created.seal, Digest32::of_bytes(&expected));
    assert_eq!(disk.borrow().bytes, expected);
    assert!(disk.borrow().writes > 1);
    assert_eq!(disk.borrow().syncs, 1);
    assert_eq!(disk.borrow().truncates, 0);
    let writes = disk.borrow().writes();
    drop(created);
    let mut reopened = AssociationStore::open(
        storage,
        expected,
        Digest32::of_bytes(&bytes()),
        &context(false),
    )?;
    let calls = disk.borrow().calls();
    reopened.verify(&context(false))?;
    assert!(disk.borrow().reads > calls[0]);
    assert_eq!(disk.borrow().writes(), writes);
    assert!(!reopened.fenced);
    Ok(())
}

#[test]
fn manager_association_rejects_external_seal_and_original_byte_substitution() {
    let expected = bytes();
    let seal = Digest32::of_bytes(&expected);
    for external in [Digest32::ZERO, Digest32::of_bytes(b"another association")] {
        let storage = Storage::new(expected.clone());
        let disk = Rc::clone(&storage.disk);
        assert!(
            AssociationStore::open(storage, expected.clone(), external, &context(false)).is_err()
        );
        assert_eq!(disk.borrow().calls(), [0; 7]);
    }
    let mut other_origin = expected.clone();
    other_origin[DOMAIN.len() + 40] ^= 1;
    for external in [seal, Digest32::of_bytes(&other_origin)] {
        let storage = Storage::new(expected.clone());
        let disk = Rc::clone(&storage.disk);
        assert!(
            AssociationStore::open(storage, other_origin.clone(), external, &context(false))
                .is_err()
        );
        assert_eq!(disk.borrow().writes(), [0; 4]);
        assert_eq!(disk.borrow().bytes, expected);
    }
}

#[test]
fn manager_association_corruption_or_identity_replacement_stays_fenced_after_restoration()
-> Result<()> {
    let expected = bytes();
    let seal = Digest32::of_bytes(&expected);
    for mutation in 0..4 {
        let storage = Storage::new(expected.clone());
        let disk = Rc::clone(&storage.disk);
        let mut store = AssociationStore::open(storage, expected.clone(), seal, &context(false))?;
        {
            let mut disk = disk.borrow_mut();
            match mutation {
                0 => disk.bytes[5000] ^= 1,
                1 => {
                    disk.bytes.pop();
                }
                2 => disk.bytes.push(0),
                _ => disk.identity += 1,
            }
        }
        assert_eq!(
            store.verify(&context(false)).err().map(|cause| cause.code),
            Some(ErrorCode::CorruptLedger)
        );
        assert!(store.fenced);
        {
            let mut disk = disk.borrow_mut();
            disk.bytes = expected.clone();
            disk.identity = 1;
        }
        let calls = disk.borrow().calls();
        assert!(store.verify(&context(false)).is_err());
        assert_eq!(
            disk.borrow().calls(),
            calls,
            "a repaired view must not revive its owner"
        );
        assert_eq!(disk.borrow().writes(), [0; 4]);
    }
    Ok(())
}

#[test]
fn manager_association_post_read_checks_catch_replacement_or_growth() {
    let expected = bytes();
    for mutation in [AfterRead::Append, AfterRead::ReplaceIdentity] {
        let storage = Storage::new(expected.clone());
        let disk = Rc::clone(&storage.disk);
        disk.borrow_mut().after_read = Some(mutation);
        assert!(
            AssociationStore::open(
                storage,
                expected.clone(),
                Digest32::of_bytes(&expected),
                &context(false)
            )
            .is_err()
        );
        assert!(disk.borrow().reads > 0);
        assert_eq!(disk.borrow().writes(), [0; 4]);
    }
}

#[test]
fn manager_association_refuses_authority_session_and_budget_failures_before_io() -> Result<()> {
    let expected = bytes();
    let seal = Digest32::of_bytes(&expected);
    for denied_context in [
        {
            let mut c = context(true);
            c.grants.clear();
            c
        },
        {
            let mut c = context(true);
            c.grants
                .retain(|grant| grant.capability == Capability::Plan);
            c
        },
        {
            let mut c = context(true);
            c.session_id = SessionId::NIL;
            c
        },
        {
            let mut c = context(true);
            c.budget.max_bytes = STORE_BYTES - 1;
            c
        },
        {
            let mut c = context(true);
            c.budget.max_wall_millis = 0;
            c
        },
        {
            let mut c = context(true);
            c.cancellation_requested = true;
            c
        },
        {
            let mut c = context(true);
            for grant in &mut c.grants {
                grant.expires_at_tick = Some(GameTick(99));
            }
            c
        },
    ] {
        for create in [false, true] {
            let storage = Storage::new(if create { Vec::new() } else { expected.clone() });
            let disk = Rc::clone(&storage.disk);
            let result = if create {
                AssociationStore::create(storage, expected.clone(), &denied_context)
            } else {
                AssociationStore::open(storage, expected.clone(), seal, &denied_context)
            };
            assert!(result.is_err());
            assert_eq!(disk.borrow().calls(), [0; 7]);
        }
    }
    let storage = Storage::new(expected.clone());
    let disk = Rc::clone(&storage.disk);
    let mut store = AssociationStore::open(storage, expected, seal, &context(false))?;
    for denied_context in [
        {
            let mut c = context(false);
            c.session_id = SessionId::new(2);
            c
        },
        {
            let mut c = context(false);
            c.grants.clear();
            c
        },
        {
            let mut c = context(false);
            c.cancellation_requested = true;
            c
        },
    ] {
        let calls = disk.borrow().calls();
        assert!(store.verify(&denied_context).is_err());
        assert_eq!(disk.borrow().calls(), calls);
    }
    let calls = disk.borrow().calls();
    let mut denied_context = context(false);
    denied_context.budget.max_bytes = STORE_BYTES - 1;
    assert!(store.verify(&denied_context).is_err());
    assert_eq!(disk.borrow().calls(), calls);
    store.verify(&context(false))?;
    Ok(())
}

#[test]
fn manager_association_bounds_and_existing_content_never_initialize_or_repair() {
    for invalid_bytes in [Vec::new(), vec![0; ASSOCIATION_MAX_BYTES + 1]] {
        let storage = Storage::new(Vec::new());
        let disk = Rc::clone(&storage.disk);
        assert!(
            AssociationStore::create(storage.clone(), invalid_bytes.clone(), &context(true))
                .is_err()
        );
        assert!(
            AssociationStore::open(
                storage,
                invalid_bytes.clone(),
                Digest32::of_bytes(&invalid_bytes),
                &context(false)
            )
            .is_err()
        );
        assert_eq!(disk.borrow().calls(), [0; 7]);
    }
    let expected = bytes();
    let storage = Storage::new(expected.clone());
    let disk = Rc::clone(&storage.disk);
    assert!(AssociationStore::create(storage, expected.clone(), &context(true)).is_err());
    assert_eq!(disk.borrow().writes(), [0; 4]);
    assert_eq!(disk.borrow().bytes, expected);
    let storage = Storage::new(Vec::new());
    let disk = Rc::clone(&storage.disk);
    assert!(AssociationStore::create(storage, expected, &context(false)).is_err());
    assert_eq!(disk.borrow().calls(), [0; 7]);
}
