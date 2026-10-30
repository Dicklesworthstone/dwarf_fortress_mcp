//! Joint source semantics and real durable monitor boundaries over native fixtures.
use super::store::{MonitorStore, Recovery};
use super::*;
use crate::build_placement::journal::private_file::PrivateFileIdentity;
use crate::build_placement::journal::{BuildJournal, BuildMode};
use crate::build_placement::{BuildBinding, BuildKind, BuildPlan, BuildRecord};
use crate::construction_plan::{Timing, fixtures};
use crate::control_effect_journal::EffectJournalStorage;
use crate::furniture_allocation::{Kind, Request, Slot};
use crate::furniture_batch::BatchDefinition;
use crate::furniture_handoff::{FurnitureRequest, Handoff};
use crate::live_operations::LiveOperationsState;
use crate::room_intent::TerrainRequirements;
use dfmcp_core::MapCoord;
use dfmcp_world::map_region::{MapRegion, Region, Tile};
use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
fn corrupt() -> DfmcpError {
    invalid("synthetic original fixture invalid")
}
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

fn fixture(count: usize) -> (Definition, OperationContext) {
    let originals = fixtures::goal(count);
    let first = originals.records()[0].plan().before();
    let endpoint = SocketAddr::from(([127, 0, 0, 1], 5000));
    let binding = BuildBinding::new(endpoint, "test-df", "test-dfhack", first).unwrap();
    let positions: Vec<_> = originals
        .records()
        .iter()
        .map(|record| record.plan().before().selection().target())
        .collect();
    let x = positions.iter().map(|p| p[0]).min().unwrap();
    let y = positions.iter().map(|p| p[1]).min().unwrap();
    let xmax = positions.iter().map(|p| p[0]).max().unwrap();
    let ymax = positions.iter().map(|p| p[1]).max().unwrap();
    let z = positions[0][2];
    let region = Region {
        origin: [x - 1, y - 1, z],
        size: [xmax - x + 3, ymax - y + 3, 1],
    };
    let mut floors = positions.clone();
    let extra = [x, ymax + 1, z];
    floors.push(extra);
    let wall = [x - 1, y - 1, z];
    let request = FurnitureRequest::new(
        first.folder().into(),
        first.site(),
        Request {
            slots: originals
                .records()
                .iter()
                .enumerate()
                .map(|(i, record)| {
                    let before = record.plan().before();
                    Slot {
                        name: format!("s{i:02}"),
                        kind: match before.selection().kind() {
                            BuildKind::Bed => Kind::Bed,
                            BuildKind::Chair => Kind::Chair,
                            BuildKind::Table => Kind::Table,
                        },
                        target: before.selection().target(),
                        after: Vec::new(),
                        material: None,
                        subtype: None,
                        max_distance: crate::furniture_allocation::MAX_DISTANCE,
                    }
                })
                .collect(),
            excluded_items: Vec::new(),
        },
    )
    .unwrap();
    let room = RoomIntent::new(
        request.clone(),
        TerrainRequirements::new(region, floors, vec![wall]).unwrap(),
    )
    .unwrap();
    let mut inventory = fixtures::observation(&originals, first.tick());
    inventory.jobs.next_job_id = first.next_job_id();
    inventory.next_building_id = first.next_building_id();
    inventory.buildings.clear();
    for item in &mut inventory.items {
        let before = originals
            .records()
            .iter()
            .find(|record| record.plan().before().selection().item_id() == item.native_id)
            .unwrap()
            .plan()
            .before();
        let p = before.selection().target();
        item.raw_position = MapCoord::new(p[0] as i32, p[1] as i32, p[2] as i32);
        item.flags = 64;
        item.holder_building_native_id = None;
    }
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(inventory).unwrap();
    let mut c = fixtures::context(&originals);
    c.anchor = state.snapshot().unwrap().anchor();
    c.budget.max_bytes = 1024 * 1024 * 1024;
    c.budget.max_entities = 100_000;
    let handoff = Handoff::allocate(
        &state,
        &c,
        endpoint,
        &request,
        crate::furniture_supply::MAX_WORK,
    )
    .unwrap()
    .handoff
    .unwrap();
    let mut raw = b"DFMBJ019".to_vec();
    field32(&mut raw, &binding.canonical_bytes());
    raw.extend_from_slice(&[1; 32]);
    let journal_id = domain_hash(b"dfmcp-build-journal/1", &raw);
    raw.extend_from_slice(journal_id.as_bytes());
    let batch =
        BatchDefinition::from_room_handoff(room, handoff, binding.clone(), journal_id).unwrap();
    let mut head = journal_id;
    let mut sequence = 0;
    let mut records = Vec::new();
    for (step, original) in batch.plan().ordered_steps().zip(originals.records()) {
        assert_eq!(step.selection, original.plan().before().selection());
        let key = batch.key(step);
        let prepared = rekey(original, &key, false).unwrap();
        let placed = rekey(original, &key, true).unwrap();
        for (tag, native) in [
            (0, None),
            (1, Some(&prepared)),
            (2, Some(&prepared)),
            (5, Some(&placed)),
        ] {
            append_original(
                &mut raw,
                &mut head,
                &mut sequence,
                placed.plan(),
                tag,
                native,
            );
        }
        records.push(placed);
    }
    let goal = Goal::new(
        records,
        Timing {
            deadline: fixtures::TICK + 1000,
            interval: 10,
            stable_samples: 2,
            stable_span: 10,
            max_gap: 1200,
            max_observations: 512,
        },
    )
    .unwrap();
    let mut original = BuildJournal::open(
        Memory::from_bytes(raw),
        &c,
        BuildMode::Recover,
        Some(binding.clone()),
        None,
    )
    .unwrap();
    let audit = original.inventory(&c).unwrap();
    let origin = Origin::from_batch(
        batch,
        &binding,
        &audit,
        identity("batch", 11),
        identity("placements", 12),
    )
    .unwrap();
    (
        Definition::new(MonitorDefinition::new(origin, goal).unwrap()).unwrap(),
        c,
    )
}

fn evidence(definition: &Definition, tick: u64) -> Sample {
    let binding = definition.origin().definition().binding();
    let region = definition.room().terrain().region();
    let floor = Tile {
        native_tiletype: 219,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10015,
        temperature_2: 10015,
    };
    let mut map = LiveMapObservation {
        bridge_generation: 87,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
        year: (tick / 403200) as u32,
        year_tick: (tick % 403200) as u32,
        paused: true,
        site_id: binding.fortress().site(),
        world_folder: binding.fortress().folder().into(),
        map_dimensions: binding.dimensions(),
        map: MapRegion {
            region,
            cells: vec![Cell::Visible(floor); region.volume().unwrap()],
        },
    };
    for &wall in definition.room().terrain().walls() {
        let mut tile = floor;
        tile.shape = Shape::Wall;
        map.map.cells[region.index(wall).unwrap()] = Cell::Visible(tile);
    }
    for record in definition.goal().records() {
        let mut tile = floor;
        tile.building_occupancy = 1;
        map.map.cells[region
            .index(record.insertion().unwrap().position())
            .unwrap()] = Cell::Visible(tile);
    }
    let manifest = Manifest {
        generation: 87,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    };
    let raw = map.encode_payload().unwrap();
    Sample {
        before: manifest.clone(),
        before_capture: raw.clone(),
        furnishings: fixtures::sample(definition.goal(), tick),
        after: manifest,
        after_capture: raw,
    }
}
fn change_map(sample: &mut Sample, edit: impl FnOnce(&mut LiveMapObservation)) {
    let mut map = sample.map(true).unwrap();
    edit(&mut map);
    sample.before_capture = map.encode_payload().unwrap();
    sample.after_capture = sample.before_capture.clone();
}
fn step(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    c: &OperationContext,
) -> Progress {
    state
        .begin_read()
        .unwrap()
        .advance(definition, sample, c)
        .unwrap()
}
fn admit(_: &Definition, _: &Progress) -> Result<()> {
    Ok(())
}
fn guard(_: &Origin, _: &OperationContext) -> Result<()> {
    Ok(())
}
fn opened(definition: &Definition, c: &OperationContext, disk: Memory) -> MonitorStore<Memory> {
    MonitorStore::open(disk, c, BuildMode::Recover, Some(definition.clone()), true).unwrap()
}
fn publish(store: &mut MonitorStore<Memory>, c: &OperationContext, sample: &Sample) -> Result<()> {
    store.begin_read(c, &mut admit, &mut guard)?;
    store.publish_sample(c, sample, &mut admit, &mut guard)
}

#[test]
fn original_room_roundtrip_and_legacy_refusal() {
    let (definition, _) = fixture(2);
    let decoded = Definition::decode(definition.canonical_bytes()).unwrap();
    assert_eq!(decoded.id(), definition.id());
    assert_eq!(decoded.room(), definition.room());
    let legacy = crate::construction_plan::store::tests::fixture().unwrap();
    assert!(Definition::new(legacy.definition).is_err());
    let sample = evidence(&definition, fixtures::TICK);
    assert_eq!(
        Sample::decode(&sample.canonical_bytes().unwrap()).unwrap(),
        sample
    );
    for length in [0, 7, 11, definition.canonical_bytes().len() - 1] {
        assert!(Definition::decode(&definition.canonical_bytes()[..length]).is_err());
    }
}

#[test]
fn whole_room_and_every_original_furnishing_share_one_streak() {
    for count in [2, 32] {
        let (definition, c) = fixture(count);
        let first = evidence(&definition, fixtures::TICK);
        let state = step(&Progress::new(&definition), &definition, &first, &c);
        assert_eq!(state.streak, 1);
        assert_eq!(state.terrain.furnishings.matched, count as u32);
        let mut blocked = evidence(&definition, fixtures::TICK + 10);
        let target = definition
            .goal()
            .records()
            .last()
            .unwrap()
            .insertion()
            .unwrap()
            .position();
        change_map(&mut blocked, |map| {
            let Cell::Visible(mut tile) = map.map.cells[map.map.region.index(target).unwrap()]
            else {
                panic!()
            };
            tile.building_occupancy = 0;
            map.map.cells[map.map.region.index(target).unwrap()] = Cell::Visible(tile);
        });
        let state = step(&state, &definition, &blocked, &c);
        assert_eq!(state.streak, 0);
        assert_eq!(state.terrain.furnishings.unoccupied, 1);
        let state = step(
            &state,
            &definition,
            &evidence(&definition, fixtures::TICK + 20),
            &c,
        );
        assert_eq!(state.streak, 1);
        let state = step(
            &state,
            &definition,
            &evidence(&definition, fixtures::TICK + 30),
            &c,
        );
        assert_eq!(state.phase, "satisfied");
    }
}

#[test]
fn wall_loss_and_alternating_furniture_terrain_truth_never_combine() {
    let (definition, c) = fixture(2);
    let mut furniture_pending = evidence(&definition, fixtures::TICK);
    let mut observed = furniture_pending.operations().unwrap();
    observed.buildings[1].build_stage = 0;
    furniture_pending.furnishings = fixtures::with_observation(definition.goal(), observed);
    let first = step(
        &Progress::new(&definition),
        &definition,
        &furniture_pending,
        &c,
    );
    assert_eq!(first.streak, 0);
    let mut terrain_failed = evidence(&definition, fixtures::TICK + 10);
    let wall = definition.room().terrain().walls()[0];
    change_map(&mut terrain_failed, |map| {
        let index = map.map.region.index(wall).unwrap();
        let Cell::Visible(mut tile) = map.map.cells[index] else {
            panic!()
        };
        tile.shape = Shape::Floor;
        map.map.cells[index] = Cell::Visible(tile);
    });
    let state = step(&first, &definition, &terrain_failed, &c);
    assert_eq!(state.construction.condition_met_count(), 2);
    assert_eq!(state.terrain.walls.wrong_shape, 1);
    assert_eq!(state.streak, 0);
    let state = step(
        &state,
        &definition,
        &evidence(&definition, fixtures::TICK + 20),
        &c,
    );
    assert_eq!(state.streak, 1);
}

#[test]
fn hidden_wet_designated_occupied_and_missing_requirements_withhold_proof() {
    let (definition, c) = fixture(2);
    let target = definition.room().terrain().floors()[0];
    for variant in 0..6 {
        let mut sample = evidence(&definition, fixtures::TICK);
        change_map(&mut sample, |map| {
            let index = map.map.region.index(target).unwrap();
            let Cell::Visible(mut tile) = map.map.cells[index] else {
                panic!()
            };
            map.map.cells[index] = match variant {
                0 => Cell::Hidden,
                1 => Cell::Unallocated,
                2 => {
                    tile.liquid_depth = 1;
                    Cell::Visible(tile)
                }
                3 => {
                    tile.magma = true;
                    Cell::Visible(tile)
                }
                4 => {
                    tile.dig_designation = 1;
                    Cell::Visible(tile)
                }
                _ => {
                    tile.shape = Shape::Ramp;
                    Cell::Visible(tile)
                }
            };
        });
        let progress = step(&Progress::new(&definition), &definition, &sample, &c);
        assert_eq!(progress.streak, 0);
        assert!(!progress.terrain.condition_met());
        assert!(progress.terrain.deficit_count() > 0);
    }
    let mut sample = evidence(&definition, fixtures::TICK);
    let wall = definition.room().terrain().walls()[0];
    change_map(&mut sample, |map| {
        let index = map.map.region.index(wall).unwrap();
        let Cell::Visible(mut tile) = map.map.cells[index] else {
            panic!()
        };
        tile.unit_occupancy = 1;
        map.map.cells[index] = Cell::Visible(tile);
    });
    assert_eq!(
        step(&Progress::new(&definition), &definition, &sample, &c)
            .terrain
            .walls
            .occupied,
        1
    );
}

#[test]
fn same_tick_map_padding_change_and_unpaused_capture_reset_shared_streak() {
    let (definition, c) = fixture(2);
    let first = evidence(&definition, fixtures::TICK);
    let state = step(&Progress::new(&definition), &definition, &first, &c);
    let mut changed = first.clone();
    change_map(&mut changed, |map| {
        let index = map.map.cells.len() - 1;
        let Cell::Visible(mut tile) = map.map.cells[index] else {
            panic!()
        };
        tile.traffic = 1;
        map.map.cells[index] = Cell::Visible(tile);
    });
    let state = step(&state, &definition, &changed, &c);
    assert_eq!(state.streak, 0);
    let state = step(&state, &definition, &changed, &c);
    assert_eq!(state.streak, 0);
    let mut next = evidence(&definition, fixtures::TICK + 10);
    change_map(&mut next, |map| map.paused = false);
    assert_eq!(
        step(&state, &definition, &next, &c).reason,
        "room_capture_unpaused"
    );
}

#[test]
fn mismatched_brackets_and_region_refuse_while_source_and_floor_changes_invalidate() {
    let (definition, c) = fixture(2);
    let first = evidence(&definition, fixtures::TICK);
    let state = step(&Progress::new(&definition), &definition, &first, &c);
    let mut bad = evidence(&definition, fixtures::TICK + 10);
    bad.after_capture[20] ^= 1;
    assert!(
        state
            .begin_read()
            .unwrap()
            .advance(&definition, &bad, &c)
            .is_err()
    );
    let mut bad = evidence(&definition, fixtures::TICK + 10);
    bad.furnishings = fixtures::sample(definition.goal(), fixtures::TICK + 11);
    assert!(
        state
            .begin_read()
            .unwrap()
            .advance(&definition, &bad, &c)
            .is_err()
    );
    let mut changed = evidence(&definition, fixtures::TICK + 10);
    changed.before.generation += 1;
    changed.after.generation += 1;
    let state = step(&state, &definition, &changed, &c);
    assert_eq!(state.phase, "invalidated");
    assert_eq!(state.reason, "room_native_source_changed");
    assert_eq!(state.observed_tick_floor, fixtures::TICK + 10);
}

#[test]
fn original_lifetime_and_authority_budgets_remain_fixed() {
    let (definition, c) = fixture(2);
    let goal = Goal::new(
        definition.goal().records().to_vec(),
        Timing {
            max_observations: 2,
            ..definition.goal().timing()
        },
    )
    .unwrap();
    let definition =
        Definition::new(MonitorDefinition::new(definition.origin().clone(), goal).unwrap())
            .unwrap();
    let sample = evidence(&definition, fixtures::TICK);
    let state = step(&Progress::new(&definition), &definition, &sample, &c);
    let state = step(&state, &definition, &sample, &c);
    assert_eq!(state.phase, "expired");
    assert_eq!(state.reason, "sample_budget_exhausted");
    let mut denied = c.clone();
    denied.budget.max_bytes = 1;
    assert!(
        Progress::new(&definition)
            .begin_read()
            .unwrap()
            .advance(&definition, &sample, &denied)
            .is_err()
    );
    let mut denied = c.clone();
    denied.budget.max_entities = 1;
    assert!(
        Progress::new(&definition)
            .begin_read()
            .unwrap()
            .advance(&definition, &sample, &denied)
            .is_err()
    );
    let mut denied = c.clone();
    denied.grants[0].expires_at_tick = Some(GameTick(fixtures::TICK));
    let later = evidence(&definition, fixtures::TICK + 10);
    assert!(
        Progress::new(&definition)
            .begin_read()
            .unwrap()
            .advance(&definition, &later, &denied)
            .is_err()
    );
}

#[test]
fn durable_replay_resets_unfinished_streak_and_retains_first_independent_sources() {
    let (definition, c) = fixture(2);
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone());
    let initial = store.recovery();
    assert_eq!(store::initial_recovery(&definition).unwrap(), initial);
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)).unwrap();
    assert_eq!(store.progress().streak, 1);
    let acknowledged = store.recovery();
    let mut reopened = MonitorStore::open(disk, &c, BuildMode::Recover, None, false).unwrap();
    reopened.verify_recovery(&c, acknowledged).unwrap();
    reopened.verify_recovery(&c, initial).unwrap();
    assert_eq!(reopened.progress().streak, 0);
    assert!(!reopened.read_owned());
    assert!(
        reopened
            .publish_sample(
                &c,
                &evidence(&definition, fixtures::TICK + 10),
                &mut admit,
                &mut guard
            )
            .is_err()
    );
    publish(
        &mut reopened,
        &c,
        &evidence(&definition, fixtures::TICK + 10),
    )
    .unwrap();
    assert_eq!(reopened.progress().streak, 1);
    let first_source = reopened.first_source().unwrap().clone();
    let mut changed = evidence(&definition, fixtures::TICK + 20);
    changed.before.generation += 1;
    changed.after.generation += 1;
    publish(&mut reopened, &c, &changed).unwrap();
    assert_eq!(reopened.progress().phase, "invalidated");
    assert_eq!(reopened.first_source(), Some(&first_source));
    assert_ne!(reopened.progress().source.as_ref(), Some(&first_source));
}

#[test]
fn read_intent_sync_failure_fences_and_complete_lost_ack_descendant_recovers() {
    let (definition, c) = fixture(2);
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone());
    let pin = store.recovery();
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    disk.disk.borrow_mut().fail_sync = true;
    assert!(
        store
            .publish_sample(
                &c,
                &evidence(&definition, fixtures::TICK),
                &mut admit,
                &mut guard
            )
            .is_err()
    );
    assert!(store.is_fenced());
    assert!(!store.read_owned());
    disk.disk.borrow_mut().fail_sync = false;
    let mut reopened =
        MonitorStore::open(disk.clone(), &c, BuildMode::Recover, None, false).unwrap();
    reopened.verify_recovery(&c, pin).unwrap();
    assert_eq!(reopened.progress().observations, 1);
    assert_eq!(reopened.progress().streak, 0);
    let tail = disk.disk.borrow().bytes.clone();
    for cut in [tail.len() - 1, tail.len() - 31] {
        assert!(
            MonitorStore::open(
                Memory::from_bytes(tail[..cut].to_vec()),
                &c,
                BuildMode::Recover,
                None,
                false
            )
            .is_err()
        );
    }
}

#[test]
fn failed_guard_retains_raw_authority_floor_and_cancel_needs_no_original_files() {
    let (definition, c) = fixture(2);
    let mut store = opened(&definition, &c, Memory::default());
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)).unwrap();
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    let sample = evidence(&definition, fixtures::TICK + 70);
    assert!(
        store
            .publish_sample(&c, &sample, &mut admit, &mut |_, _| Err(corrupt()))
            .is_err()
    );
    assert_eq!(store.authority_tick_floor(), fixtures::TICK + 70);
    assert_eq!(store.progress().streak, 0);
    let mut old = c.clone();
    old.grants[0].expires_at_tick = Some(GameTick(fixtures::TICK + 30));
    assert!(store.verify(&old).is_err());
    store.cancel(&c, &mut admit).unwrap();
    assert_eq!(store.progress().phase, "cancelled");
    assert!(!store.read_owned());
}

#[test]
fn partial_decode_clock_floor_survives_next_intent_and_refuses_regression_after_restart() {
    let (definition, c) = fixture(2);
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone());
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)).unwrap();
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    let mut incomplete = evidence(&definition, fixtures::TICK + 70);
    incomplete.after_capture.truncate(25);
    assert!(
        store
            .publish_sample(&c, &incomplete, &mut admit, &mut guard)
            .is_err()
    );
    assert_eq!(store.authority_tick_floor(), fixtures::TICK + 70);
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    let mut empty_tail = evidence(&definition, fixtures::TICK + 80);
    empty_tail.after_capture.clear();
    assert!(
        store
            .publish_sample(&c, &empty_tail, &mut admit, &mut guard)
            .is_err()
    );
    assert_eq!(store.authority_tick_floor(), fixtures::TICK + 80);
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    let pin = store.recovery();
    assert_eq!(store.progress().observed_tick_floor, fixtures::TICK + 80);
    drop(store);
    let mut reopened = MonitorStore::open(disk, &c, BuildMode::Recover, None, false).unwrap();
    reopened.verify_recovery(&c, pin).unwrap();
    assert_eq!(reopened.authority_tick_floor(), fixtures::TICK + 80);
    publish(
        &mut reopened,
        &c,
        &evidence(&definition, fixtures::TICK + 10),
    )
    .unwrap();
    assert_eq!(reopened.progress().phase, "invalidated");
    assert_eq!(reopened.progress().reason, "room_game_clock_regressed");
    assert_eq!(reopened.progress().streak, 0);
    assert_eq!(reopened.progress().observed_tick_floor, fixtures::TICK + 80);
}

#[test]
fn native_failed_read_floor_latch_changes_no_evidence_and_cancel_preserves_it_on_reopen() {
    let (definition, c) = fixture(2);
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone());
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    let pin = store.recovery();
    let progress = store.progress().clone();
    let bytes = disk.disk.borrow().bytes.clone();
    store.retain_observed_tick_floor(fixtures::TICK + 90);
    store.retain_observed_tick_floor(fixtures::TICK + 20);
    assert_eq!(store.authority_tick_floor(), fixtures::TICK + 90);
    assert_eq!(store.progress(), &progress);
    assert_eq!(store.recovery(), pin);
    assert_eq!(disk.disk.borrow().bytes, bytes);
    assert!(store.read_owned());
    assert!(store.first_source().is_none());
    store.abandon_read();
    assert!(!store.read_owned());
    let mut expired = c.clone();
    expired.grants[0].expires_at_tick = Some(GameTick(fixtures::TICK + 80));
    assert!(store.cancel(&expired, &mut admit).is_err());
    store.cancel(&c, &mut admit).unwrap();
    let cancelled = store.recovery();
    drop(store);
    let mut reopened = MonitorStore::open(disk, &c, BuildMode::Recover, None, false).unwrap();
    reopened.verify_recovery(&c, cancelled).unwrap();
    assert_eq!(reopened.progress().phase, "cancelled");
    assert_eq!(reopened.progress().observations, 0);
    assert_eq!(reopened.progress().streak, 0);
    assert_eq!(reopened.progress().observed_tick_floor, fixtures::TICK + 90);
    assert_eq!(reopened.authority_tick_floor(), fixtures::TICK + 90);
    assert!(reopened.progress().source.is_none());
    assert!(reopened.first_source().is_none());
    assert!(!reopened.read_owned());
}

#[test]
fn exact_recovery_refuses_rollback_foreign_goal_and_current_byte_substitution() {
    let (definition, c) = fixture(2);
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone());
    let old_bytes = disk.disk.borrow().bytes.clone();
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)).unwrap();
    let newer = store.recovery();
    let mut old = MonitorStore::open(
        Memory::from_bytes(old_bytes),
        &c,
        BuildMode::Recover,
        None,
        false,
    )
    .unwrap();
    assert!(old.verify_recovery(&c, newer).is_err());
    assert!(
        store
            .verify_recovery(
                &c,
                Recovery {
                    monitor_id: Digest32::of_bytes(b"different"),
                    head: newer.head
                }
            )
            .is_err()
    );
    disk.disk.borrow_mut().bytes[80] ^= 1;
    assert!(store.verify(&c).is_err());
    assert!(store.is_fenced());
}

#[cfg(target_os = "linux")]
#[test]
fn private_monitor_create_lock_reopen_and_substitution_preserve_custody() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let root = std::env::temp_dir().join(format!(
        "dfmcp-joint-room-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.join("condition");
    let (definition, c) = fixture(2);
    let mut store =
        store::open_private_monitor(&path, &c, BuildMode::Recover, Some(definition.clone()))
            .unwrap();
    let pin = store.recovery();
    assert!(store::open_private_monitor(&path, &c, BuildMode::Recover, None).is_err());
    store.begin_read(&c, &mut admit, &mut guard).unwrap();
    store
        .publish_sample(
            &c,
            &evidence(&definition, fixtures::TICK),
            &mut admit,
            &mut guard,
        )
        .unwrap();
    drop(store);
    let mut store = store::open_private_monitor(&path, &c, BuildMode::Recover, None).unwrap();
    store.verify_recovery(&c, pin).unwrap();
    assert_eq!(store.progress().streak, 0);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, root.join("original")).unwrap();
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.verify(&c).is_err());
    assert!(store.is_fenced());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
