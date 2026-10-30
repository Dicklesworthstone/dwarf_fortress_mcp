use super::*;
use crate::build_placement::BuildCapture;
use crate::semantic_furniture::store::Association;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, IntentId, MapCoord, MapCuboid, RequestId, StateAnchor,
    StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, BuildingKind, MaterialSelector, PlanStep, PreparedPlan, derive_step_idempotency_key,
    effects,
};
use dfmcp_world::Predicate;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{Cursor, Read, Seek, Write};
use std::rc::Rc;

fn invalid(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::InternalInvariantViolation, message)
}
fn hex(raw: &str) -> Result<Vec<u8>> {
    if !raw.is_ascii() || !raw.len().is_multiple_of(2) {
        return Err(invalid("fixture hex extent"));
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| invalid("fixture hex text"))?;
            u8::from_str_radix(text, 16).map_err(|_| invalid("fixture hex byte"))
        })
        .collect()
}
fn golden(key: &str) -> Result<Vec<u8>> {
    let fixture =
        include_str!("../../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let prefix = format!("\"{key}\": \"");
    let start = fixture
        .find(&prefix)
        .ok_or_else(|| invalid("fixture key"))?
        + prefix.len();
    let raw = fixture[start..]
        .split('"')
        .next()
        .ok_or_else(|| invalid("fixture value"))?;
    hex(raw)
}
fn fixture() -> Result<(SemanticFurnitureReview, Digest32, OperationContext)> {
    let capture = BuildCapture::decode(&golden("capture")?)?;
    let anchor = StateAnchor {
        fortress_id: capture.fortress_id(),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(capture.tick()),
        state_hash: capture.witness(),
    };
    let context = OperationContext {
        session_id: SessionId::new(741),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_entities: 8192,
            max_bytes: 512 * 1024 * 1024,
            max_actions: 1,
            max_output_tokens: 4096,
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(anchor.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    let [x, y, z] = capture.selection().target();
    let location = MapCoord::new(x as i32, y as i32, z as i32);
    let action = Action::Build {
        kind: BuildingKind::Furniture("bed".to_owned()),
        location,
        footprint: MapCuboid {
            min: location,
            max: location,
        },
        material: MaterialSelector::default(),
    }
    .normalized();
    let intent = IntentId::new(741);
    let key = derive_step_idempotency_key(intent, anchor, StepId::ZERO, &action);
    let postconditions = effects::default_postconditions(&action, &key, anchor.fortress_id);
    let obligation = effects::default_obligation(&action, &key, anchor.fortress_id, anchor.tick)?;
    let deadline = obligation
        .as_ref()
        .ok_or_else(|| invalid("fixture construction obligation"))?
        .deadline_tick;
    let original = PreparedPlan::builder(
        intent,
        anchor,
        "Original furniture history",
        Predicate::All(postconditions.clone()).normalized(),
    )
    .max_risk(action.risk())
    .required_capabilities(BTreeSet::from([action.capability()]))
    .expires_at_tick(deadline)
    .steps(vec![PlanStep {
        id: StepId::ZERO,
        idempotency_key: key.clone(),
        risk: action.risk(),
        required_capability: action.capability(),
        action,
        preconditions: vec![Predicate::Paused(true)],
        postconditions,
        depends_on: Vec::new(),
        compensation: None,
        obligation,
    }])
    .build();
    let object = crate::semantic_furniture::original(&original, &Call::new(&context)?)?;
    let native = BuildPlan::new(&key, capture)?;
    let association = Association {
        key,
        semantic: original.digest,
        original_object: object,
        step: StepId::ZERO,
        anchor,
        source: Digest32::of_bytes(b"history test original source"),
        native: native.digest(),
        witness: native.before().witness(),
    };
    let journal = Digest32::of_bytes(b"history test native journal");
    let seal = association.seal(journal);
    Ok((
        SemanticFurnitureReview {
            original,
            native,
            association,
            seal,
        },
        journal,
        context,
    ))
}

#[derive(Default)]
struct Disk {
    cursor: Cursor<Vec<u8>>,
    synced: usize,
    fail_sync: bool,
    write_left: Option<usize>,
    identity_lost: bool,
}
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Disk>>);
impl Memory {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Rc::new(RefCell::new(Disk {
            cursor: Cursor::new(bytes),
            ..Disk::default()
        })))
    }
    fn stored_bytes(&self) -> Vec<u8> {
        self.0.borrow().cursor.get_ref().clone()
    }
}
impl Read for Memory {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().cursor.read(output)
    }
}
impl Write for Memory {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let mut disk = self.0.borrow_mut();
        let length = match disk.write_left {
            Some(0) => return Err(io::Error::other("injected torn append")),
            Some(left) => {
                let count = left.min(input.len());
                disk.write_left = Some(left - count);
                count
            }
            None => input.len(),
        };
        disk.cursor.write(&input[..length])
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().cursor.seek(position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut disk = self.0.borrow_mut();
        if disk.fail_sync {
            disk.fail_sync = false;
            return Err(io::Error::other("injected sync failure"));
        }
        disk.synced += 1;
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.0.borrow().identity_lost {
            Err(io::Error::other("injected custody loss"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn history_is_exactly_bound_and_reopen_does_not_restore_write_permission() -> Result<()> {
    let (review, native, c) = fixture()?;
    let memory = Memory::default();
    let mut history = GoalHistoryStore::open(memory.clone(), native, &review, true, false, &c)?;
    let tick = GameTick(c.anchor.tick.get() + 1);
    history.append(
        Event::Interrupted {
            tick,
            fenced: false,
        },
        &c,
    )?;
    let head = history.head();
    let bytes = memory.stored_bytes();
    let writes = memory.0.borrow().synced;
    let mut reopened = c.clone();
    reopened.session_id = SessionId::new(742);
    let mut offline =
        GoalHistoryStore::open(memory.clone(), native, &review, false, true, &reopened)?;
    assert_eq!(offline.head(), head);
    assert_eq!(offline.high_tick(), tick);
    assert_eq!(offline.event_count(), 1);
    assert!(
        offline
            .append(
                Event::Interrupted {
                    tick,
                    fenced: false
                },
                &reopened
            )
            .is_err()
    );
    assert_eq!(memory.stored_bytes(), bytes);
    assert_eq!(memory.0.borrow().synced, writes);
    assert!(
        GoalHistoryStore::open(
            Memory::from_bytes(bytes.clone()),
            Digest32::of_bytes(b"another journal"),
            &review,
            false,
            true,
            &c
        )
        .is_err()
    );
    let mut changed = review.clone();
    changed.original.summary.push('!');
    assert!(
        GoalHistoryStore::open(Memory::from_bytes(bytes), native, &changed, false, true, &c)
            .is_err()
    );
    Ok(())
}

#[test]
fn every_incomplete_frame_and_same_extent_byte_substitution_is_refused() -> Result<()> {
    let (review, native, c) = fixture()?;
    let memory = Memory::default();
    let mut history = GoalHistoryStore::open(memory.clone(), native, &review, true, false, &c)?;
    history.append(
        Event::Interrupted {
            tick: c.anchor.tick,
            fenced: false,
        },
        &c,
    )?;
    let bytes = memory.stored_bytes();
    for end in HEADER_BYTES + 1..bytes.len() {
        assert!(
            GoalHistoryStore::open(
                Memory::from_bytes(bytes[..end].to_vec()),
                native,
                &review,
                false,
                true,
                &c
            )
            .is_err(),
            "prefix {end}"
        );
    }
    for offset in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        assert!(
            GoalHistoryStore::open(
                Memory::from_bytes(changed),
                native,
                &review,
                false,
                true,
                &c
            )
            .is_err(),
            "byte {offset}"
        );
    }
    Ok(())
}

#[test]
fn failed_sync_fences_until_verified_reopen_but_torn_append_never_repairs() -> Result<()> {
    for torn in [false, true] {
        let (review, native, c) = fixture()?;
        let memory = Memory::default();
        let mut history = GoalHistoryStore::open(memory.clone(), native, &review, true, false, &c)?;
        if torn {
            memory.0.borrow_mut().write_left = Some(13);
        } else {
            memory.0.borrow_mut().fail_sync = true;
        }
        assert!(
            history
                .append(
                    Event::Interrupted {
                        tick: c.anchor.tick,
                        fenced: false
                    },
                    &c
                )
                .is_err()
        );
        assert!(history.is_fenced());
        assert_eq!(history.event_count(), 0);
        assert!(history.verify(&c).is_err());
        let surviving = memory.stored_bytes();
        let reopened = GoalHistoryStore::open(
            Memory::from_bytes(surviving.clone()),
            native,
            &review,
            false,
            false,
            &c,
        );
        if torn {
            assert!(reopened.is_err());
        } else {
            let reopened = reopened?;
            assert_eq!(reopened.event_count(), 1);
            assert!(!reopened.is_fenced());
        }
        assert_eq!(memory.stored_bytes(), surviving);
    }
    Ok(())
}

#[test]
fn live_identity_or_byte_loss_fences_cached_history() -> Result<()> {
    for identity in [true, false] {
        let (review, native, c) = fixture()?;
        let memory = Memory::default();
        let mut history = GoalHistoryStore::open(memory.clone(), native, &review, true, false, &c)?;
        if identity {
            memory.0.borrow_mut().identity_lost = true;
        } else if let Some(first) = memory.0.borrow_mut().cursor.get_mut().first_mut() {
            *first ^= 1;
        }
        assert!(history.verify(&c).is_err());
        assert!(history.is_fenced());
        assert!(
            history
                .append(
                    Event::Interrupted {
                        tick: c.anchor.tick,
                        fenced: false
                    },
                    &c
                )
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn published_receipts_are_original_placed_bytes_and_never_reconstructed_policy() -> Result<()> {
    let placed = BuildRecord::decode(&golden("placed")?)?;
    let event = Event::Published {
        fingerprint: Digest32::of_bytes(b"independently issued policy fingerprint"),
        receipt: Box::new(placed.clone()),
        dispatch_started: true,
        cancel_requested: false,
    };
    let encoded = event.encode()?;
    let mut expected = vec![2];
    expected.extend_from_slice(
        Digest32::of_bytes(b"independently issued policy fingerprint").as_bytes(),
    );
    expected.extend_from_slice(&[1, 0]);
    expected.extend_from_slice(&(placed.canonical_bytes().len() as u32).to_be_bytes());
    expected.extend_from_slice(placed.canonical_bytes());
    assert_eq!(encoded, expected);
    let decoded = Event::decode(&encoded, placed.plan())?;
    assert!(matches!(decoded, Event::Published { receipt, .. } if *receipt == placed));
    let different = BuildPlan::new("different-original-key", placed.plan().before().clone())?;
    assert!(Event::decode(&encoded, &different).is_err());
    let prepared = BuildRecord::decode(&golden("prepared")?)?;
    let unverified = Event::Published {
        fingerprint: Digest32::of_bytes(b"independently issued policy fingerprint"),
        receipt: Box::new(prepared.clone()),
        dispatch_started: false,
        cancel_requested: false,
    }
    .encode()?;
    assert!(Event::decode(&unverified, prepared.plan()).is_err());
    Ok(())
}

#[test]
fn contradictory_raw_capture_remains_negative_custody_and_cannot_publish() -> Result<()> {
    let raw = include_str!("../../construction_plan/testdata/python_sample_32.hex").trim();
    let mut sample = LinkedSample::decode(&hex(raw)?)?;
    sample.capture = vec![0];
    let mut frontier = Frontier {
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(0),
        phase: 0,
        source_fenced: false,
    };
    let cursor = ObservationCursor {
        epoch: frontier.cursor.epoch,
        sequence: frontier.cursor.sequence + 1,
    };
    frontier.apply(&Event::Started {
        cursor,
        tick: GameTick(0),
        interrupted: false,
    })?;
    let canonical = sample.canonical_bytes()?;
    let event = Event::Captured(Box::new(sample));
    assert_eq!(&event.encode()?[1..], canonical);
    let placed = BuildRecord::decode(&golden("placed")?)?;
    Event::decode(&event.encode()?, placed.plan())?;
    frontier.apply(&event)?;
    assert!(frontier.source_fenced);
    assert!(
        frontier
            .apply(&Event::Published {
                fingerprint: Digest32::of_bytes(b"policy"),
                receipt: Box::new(placed),
                dispatch_started: true,
                cancel_requested: false,
            })
            .is_err()
    );
    frontier.apply(&Event::Interrupted {
        tick: frontier.tick,
        fenced: false,
    })?;
    assert!(frontier.source_fenced);
    assert!(
        frontier
            .apply(&Event::Started {
                cursor: ObservationCursor {
                    epoch: cursor.epoch,
                    sequence: cursor.sequence + 1
                },
                tick: frontier.tick,
                interrupted: true,
            })
            .is_err()
    );
    Ok(())
}
