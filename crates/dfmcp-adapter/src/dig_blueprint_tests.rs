use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_designation::DigPlan;
use crate::dig_designation::journal::session::{COMPLETE_INVENTORY_BYTES, DigSession};
use crate::dig_designation::journal::{DigJournal, DigMode};
use crate::dig_designation::rpc::{DigManifest, DigPreparation, DigSource};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, MapCuboid, ObservationCursor,
    OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn part(origin: [u32; 3], size: [u32; 3]) -> BlueprintPart {
    BlueprintPart { origin, size }
}
fn layouts_parts() -> Vec<BlueprintPart> {
    vec![part([2, 3, 2], [10, 2, 1]), part([2, 7, 2], [2, 2, 2])]
}

#[test]
fn geometry_matches_independent_python_mask_and_layout_vectors() -> Result<()> {
    // Generated with the existing independent scripts/dig_blueprint.py compiler.
    let layout = Layout::new(layouts_parts())?;
    assert_eq!(
        layout.blueprint_digest().to_hex(),
        "734be33454864412fde1ec3659674922ba9babde4287bf9a1e30fdaf8606e40e"
    );
    assert_eq!(
        layout.digest().to_hex(),
        "441ee87463f1c807d0e12dea23802bfb5f23ac8b470b9d4afb13c70d95a90ab8"
    );
    assert_eq!(layout.canonical_bytes(), b"{\"parts\":[{\"region\":{\"origin\":[2,3,2],\"size\":[10,2,1]},\"shape\":\"floor\"},{\"region\":{\"origin\":[2,7,2],\"size\":[2,2,2]},\"shape\":\"floor\"}],\"schema\":\"dfmcp.excavation-blueprint/1\"}");
    assert_eq!(
        layout
            .regions()
            .iter()
            .map(|r| r.coordinates())
            .collect::<Vec<_>>(),
        [
            [2, 3, 2, 8, 2],
            [10, 3, 2, 2, 2],
            [2, 7, 2, 2, 2],
            [2, 7, 3, 2, 2]
        ]
    );
    assert_eq!(layout.targets().len(), 28);
    Ok(())
}

#[test]
fn all_small_sparse_masks_preserve_exact_targets_and_deterministic_identity() -> Result<()> {
    for mask in 1u32..4096 {
        let parts: Vec<_> = (0..12)
            .filter(|index| mask & (1 << index) != 0)
            .map(|index| part([2 + index % 4, 2 + index / 4, 2], [1, 1, 1]))
            .collect();
        let mut reversed = parts.clone();
        reversed.reverse();
        let layout = Layout::new(parts.clone())?;
        assert_eq!(layout, Layout::new(reversed)?);
        let mut covered = BTreeSet::new();
        for region in layout.regions() {
            let [x, y, z, w, h] = region.coordinates();
            for py in y..y + h {
                for px in x..x + w {
                    assert!(covered.insert([px, py, z]));
                }
            }
        }
        assert_eq!(covered, parts.into_iter().map(|p| p.origin).collect());
        assert_eq!(covered, layout.targets().iter().copied().collect());
    }
    Ok(())
}

#[test]
fn part_decomposition_changes_original_bytes_without_changing_compiled_mask() -> Result<()> {
    let whole = Layout::new(vec![part([2, 2, 2], [10, 2, 1])])?;
    let split = Layout::new(vec![part([2, 2, 2], [4, 2, 1]), part([6, 2, 2], [6, 2, 1])])?;
    assert_eq!(whole.digest(), split.digest());
    assert_eq!(whole.blueprint_digest(), split.blueprint_digest());
    assert_eq!(whole.regions(), split.regions());
    assert_ne!(whole.canonical_bytes(), split.canonical_bytes());
    Ok(())
}

#[test]
fn admission_checks_extents_overlap_and_native_retention_before_expansion() -> Result<()> {
    for parts in [
        Vec::new(),
        vec![part([1, 1, 1], [1, 1, 1]); 33],
        vec![part([0, 1, 1], [1, 1, 1])],
        vec![part([u32::MAX, 1, 1], [1, 1, 1])],
        vec![part([1, 1, 1], [0, 1, 1])],
        vec![part([1, 1, 1], [129, 1, 1])],
        vec![part([32766, 1, 1], [2, 1, 1])],
        vec![part([1, 1, 1], [128, 128, 128])],
        vec![part([1, 1, 1], [9, 8, 8])],
        vec![part([1, 1, 1], [1, 1, 1]), part([129, 1, 1], [1, 1, 1])],
        vec![part([1, 1, 1], [1, 1, 1]), part([12, 12, 12], [1, 1, 1])],
        vec![part([2, 2, 2], [2, 2, 1]), part([3, 3, 2], [1, 1, 1])],
    ] {
        assert!(Layout::new(parts).is_err());
    }
    assert_eq!(
        Layout::new(vec![part([1, 1, 1], [16, 16, 2])])?
            .targets()
            .len(),
        512
    );
    let mut maximum: Vec<_> = (0..32)
        .map(|n| part([1 + 2 * n, 1, 1], [1, 1, 4]))
        .collect();
    assert_eq!(Layout::new(maximum.clone())?.regions().len(), 128);
    maximum[0].size[2] = 5;
    assert!(Layout::new(maximum).is_err());
    assert!(Layout::new(vec![part([32766, 32766, 32766], [1, 1, 1])]).is_ok());
    Ok(())
}

fn capture(region: DigRegion, tick: u64, sequence: u64) -> Result<DigObservation> {
    let mut raw = b"DFMDG016".to_vec();
    for value in [7u64, sequence, tick] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    for value in [1u32, 128, 128, 8] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    for value in region.coordinates() {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.push(1);
    text(&mut raw, "region1");
    raw.extend_from_slice(&(region.halo_count() as u16).to_be_bytes());
    for _ in 0..region.halo_count() {
        raw.push(2);
        for value in [200u32, 0, 0, 0, 0, 0] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.extend_from_slice(&10000u16.to_be_bytes());
        raw.extend_from_slice(&10000u16.to_be_bytes());
        raw.extend_from_slice(&[0, 0, 1]);
    }
    DigObservation::decode(&raw)
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn binding(capture: &DigObservation) -> Result<DigBinding> {
    DigBinding::new(
        "127.0.0.1:5000"
            .parse()
            .map_err(|_| invalid("fixture endpoint"))?,
        DigManifest {
            generation: 7,
            df_version: "test-df".to_owned(),
            dfhack_version: "test-dfhack".to_owned(),
        },
        capture,
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(127, 127, 7))?,
    )
}
fn header(binding: &DigBinding) -> Vec<u8> {
    let raw = binding.canonical_bytes();
    let mut out = b"DFMDJ001".to_vec();
    out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
    out.extend_from_slice(&raw);
    out.extend_from_slice(&[9; 32]);
    let id = hash(b"dfmcp-dig-journal/1", &out);
    out.extend_from_slice(id.as_bytes());
    out
}
fn definition(parts: Vec<BlueprintPart>) -> Result<BatchDefinition> {
    let layout = Layout::new(parts)?;
    let bootstrap = capture(layout.regions()[0], 10, 3)?;
    let binding = binding(&bootstrap)?;
    let raw = header(&binding);
    let id = Digest32::from_bytes(
        raw[raw.len() - 32..]
            .try_into()
            .map_err(|_| invalid("fixture id"))?,
    );
    BatchDefinition::new(layout, binding, id, bootstrap)
}
fn context(definition: &BatchDefinition) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: definition.binding().fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(10),
            state_hash: definition.id(),
        },
        budget: WorkBudget {
            max_wall_millis: 60000,
            max_game_ticks: 0,
            max_entities: 8192,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 65536,
            max_actions: 1,
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(definition.binding().fortress_id()),
                map_area: Some(definition.binding().scope()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    read_bytes: usize,
}
#[derive(Clone)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    at: usize,
}
impl Memory {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            disk: Rc::new(RefCell::new(Disk {
                bytes,
                read_bytes: 0,
            })),
            at: 0,
        }
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        let n = out.len().min(disk.bytes.len().saturating_sub(self.at));
        out[..n].copy_from_slice(&disk.bytes[self.at..self.at + n]);
        self.at += n;
        disk.read_bytes += n;
        Ok(n)
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let n = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.at as i128 + i128::from(n),
        };
        self.at = usize::try_from(n).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.at as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("read-only fixture"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("read-only fixture"))
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Err(io::Error::other("read-only fixture"))
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("read-only fixture"))
    }
}
struct NoSource {
    manifest: DigManifest,
}
impl DigSource for NoSource {
    fn manifest(&self) -> &DigManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        None
    }
    fn observe(&mut self, _: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        Err(invalid("unexpected source call"))
    }
    fn prepare(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        Err(invalid("unexpected source call"))
    }
    fn commit(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        Err(invalid("unexpected source call"))
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        Err(invalid("unexpected source call"))
    }
    fn cancel(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        Err(invalid("unexpected source call"))
    }
}
fn native(plan: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let mut out = b"DFMDGE16".to_vec();
    let before = plan.before();
    for value in [before.generation(), before.sequence(), before.tick()] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    for value in before.region().coordinates() {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.push(u8::from(plan.allow_hidden_neighbors()));
    out.extend_from_slice(before.witness().as_bytes());
    out.extend_from_slice(plan.digest().as_bytes());
    out.extend_from_slice(plan.token());
    out.push(match phase {
        DigPhase::Prepared => 0,
        DigPhase::Unknown => 1,
        DigPhase::Designated => 2,
        DigPhase::Refused => 4,
    });
    out.push(if phase == DigPhase::Refused { 2 } else { 0 });
    out.push(u8::from(phase == DigPhase::Designated));
    out.extend_from_slice(
        &(if phase == DigPhase::Designated {
            before.region().target_count()
        } else {
            0
        })
        .to_be_bytes(),
    );
    let after = if phase == DigPhase::Designated {
        let mut raw = before.canonical_bytes().to_vec();
        raw[16..24].copy_from_slice(&(before.sequence() + 1).to_be_bytes());
        let start = raw.len() - before.region().halo_count() * 32;
        let [x, y, z, w, h] = before.region().coordinates();
        for (index, (point, _)) in before.tiles().enumerate() {
            let offset = start + 32 * index;
            if point[2] == z
                && point[0] >= x
                && point[0] < x + w
                && point[1] >= y
                && point[1] < y + h
            {
                raw[offset + 13..offset + 17].copy_from_slice(&4000u32.to_be_bytes());
                raw[offset + 29] = 1;
            }
            if before.region().write_area().contains(MapCoord::new(
                point[0] as i32,
                point[1] as i32,
                point[2] as i32,
            )) {
                raw[offset + 17..offset + 21].fill(0);
                raw[offset + 31] |= 16;
            }
        }
        Digest32::of_bytes(&raw)
    } else {
        Digest32::ZERO
    };
    out.extend_from_slice(after.as_bytes());
    let mut proof = before.generation().to_be_bytes().to_vec();
    text(&mut proof, plan.key());
    proof.extend_from_slice(plan.digest().as_bytes());
    proof.extend_from_slice(plan.token());
    proof.extend_from_slice(&out[133..172]);
    out.extend_from_slice(
        if phase.terminal() {
            hash(b"dfmcp-dig-designation-receipt/1", &proof)
        } else {
            Digest32::ZERO
        }
        .as_bytes(),
    );
    text(&mut out, plan.key());
    DigEffect::decode(&out, plan)
}
#[derive(Clone, Copy)]
enum Child {
    Intent,
    Prepared,
    Dispatched,
    Unknown,
    Designated,
    Refused,
    CancelRequested,
}
fn journal_bytes(definition: &BatchDefinition, entries: &[(DigPlan, Child)]) -> Result<Vec<u8>> {
    let mut raw = header(definition.binding());
    let mut head = definition.journal_id();
    let mut sequence = 0u64;
    for (plan, child) in entries {
        let mut states = vec![(DigState::Intent, None)];
        match child {
            Child::Intent => {}
            Child::Refused => states.push((DigState::Terminal, Some(DigPhase::Refused))),
            Child::CancelRequested => states.push((DigState::CancelRequested, None)),
            _ => {
                states.push((DigState::Prepared, Some(DigPhase::Prepared)));
                if !matches!(child, Child::Prepared) {
                    states.push((DigState::DispatchStarted, Some(DigPhase::Prepared)));
                }
                match child {
                    Child::Unknown => states.push((DigState::Tracking, Some(DigPhase::Unknown))),
                    Child::Designated => {
                        states.push((DigState::Terminal, Some(DigPhase::Designated)))
                    }
                    _ => {}
                }
            }
        }
        for (state, phase) in states {
            let bytes = plan.canonical_bytes();
            let effect = phase.map(|phase| native(plan, phase)).transpose()?;
            let mut body = vec![state as u8];
            body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            body.extend_from_slice(&bytes);
            let bytes = effect.as_ref().map_or(&[][..], DigEffect::canonical_bytes);
            body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            body.extend_from_slice(bytes);
            sequence += 1;
            let mut frame = b"DFMDJR01".to_vec();
            frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
            frame.extend_from_slice(&sequence.to_be_bytes());
            frame.extend_from_slice(head.as_bytes());
            frame.extend_from_slice(&body);
            head = hash(b"dfmcp-dig-journal-frame/1", &frame);
            frame.extend_from_slice(head.as_bytes());
            frame.extend_from_slice(b"DFMDJEND");
            raw.extend_from_slice(&frame);
        }
    }
    Ok(raw)
}
fn session(definition: &BatchDefinition, memory: Memory) -> Result<DigSession<Memory, NoSource>> {
    let context = context(definition);
    let journal = DigJournal::open(
        memory,
        &context,
        DigMode::Offline,
        Some(definition.binding().clone()),
        None,
    )?;
    DigSession::new(journal, &context)
}
fn inventory(
    definition: &BatchDefinition,
    entries: &[(DigPlan, Child)],
) -> Result<(DigSessionView, Vec<DigRecord>)> {
    session(definition, Memory::new(journal_bytes(definition, entries)?))?
        .complete_inventory(&context(definition))
}
fn child(definition: &BatchDefinition, index: usize) -> Result<DigPlan> {
    DigPlan::new(
        &definition.key(index)?,
        false,
        capture(
            definition.layout().regions()[index],
            10 + index as u64,
            3 + index as u64,
        )?,
    )
}

#[test]
fn parent_roundtrip_seals_original_parts_capture_binding_and_journal() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let raw = definition.canonical_bytes();
    assert_eq!(BatchDefinition::decode(&raw)?, definition);
    for length in 0..raw.len() {
        assert!(BatchDefinition::decode(&raw[..length]).is_err());
    }
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(BatchDefinition::decode(&trailing).is_err());
    let mut order = raw.clone();
    let first: Vec<_> = order[10..34].to_vec();
    order.copy_within(34..58, 10);
    order[34..58].copy_from_slice(&first);
    assert!(BatchDefinition::decode(&order).is_err());
    let newer = BatchDefinition::new(
        definition.layout().clone(),
        definition.binding().clone(),
        definition.journal_id(),
        capture(definition.layout().regions()[0], 11, 3)?,
    )?;
    assert_ne!(newer.id(), definition.id());
    assert_ne!(newer.key(0)?, definition.key(0)?);
    assert!(definition.key(128).is_err());
    assert!(raw.len() <= MAX_DEFINITION_BYTES);
    Ok(())
}

#[test]
fn parent_creation_preflights_every_future_halo_and_shared_block() -> Result<()> {
    let good = definition(layouts_parts())?;
    assert!(
        BatchDefinition::new(
            good.layout().clone(),
            good.binding().clone(),
            Digest32::ZERO,
            good.bootstrap().clone()
        )
        .is_err()
    );
    let outside = Layout::new(vec![part([2, 3, 2], [1, 1, 1]), part([2, 3, 7], [1, 1, 1])])?;
    assert!(
        BatchDefinition::new(
            outside,
            good.binding().clone(),
            good.journal_id(),
            capture(DigRegion::new(2, 3, 2, 1, 1)?, 10, 3)?
        )
        .is_err()
    );
    // The first rectangle's halo AND shared block fit this binding. The
    // later rectangle's halo fits too, but its shared block extends to x=31.
    // DigBinding::new validates the bootstrap; the parent must reject that
    // future write footprint before admitting any child preparation.
    let later_block = Layout::new(vec![
        part([2, 3, 2], [8, 2, 1]),
        part([17, 3, 2], [1, 1, 1]),
    ])?;
    assert_eq!(later_block.regions()[0], good.bootstrap().region());
    let narrow = DigBinding::new(
        good.binding().endpoint(),
        good.binding().manifest().clone(),
        good.bootstrap(),
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(18, 15, 7))?,
    )?;
    assert!(
        BatchDefinition::new(
            later_block,
            narrow,
            good.journal_id(),
            good.bootstrap().clone()
        )
        .is_err()
    );
    let mut raw = good.bootstrap().canonical_bytes().to_vec();
    raw[68] = 0;
    assert!(
        BatchDefinition::new(
            good.layout().clone(),
            good.binding().clone(),
            good.journal_id(),
            DigObservation::decode(&raw)?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn every_original_prefix_advances_only_after_durable_designation() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let mut entries = Vec::new();
    for index in 0..=definition.layout().regions().len() {
        let (view, records) = inventory(&definition, &entries)?;
        let progress = definition.audit(&view, &records, false)?;
        assert_eq!(progress.designated, index);
        assert_eq!(progress.rows.len(), 4);
        assert_eq!(progress.next_index, (index < 4).then_some(index));
        assert_eq!(progress.pending_index, None);
        assert_eq!(
            progress.status,
            if index == 4 {
                "all_designated"
            } else {
                "ready"
            }
        );
        if index < 4 {
            entries.push((child(&definition, index)?, Child::Designated));
        }
    }
    Ok(())
}

#[test]
fn maximum_native_prefix_retains_all_128_original_receipts() -> Result<()> {
    let definition = definition(
        (0..32)
            .map(|n| part([1 + 2 * n, 1, 1], [1, 1, 4]))
            .collect(),
    )?;
    let entries = (0..128)
        .map(|index| Ok((child(&definition, index)?, Child::Designated)))
        .collect::<Result<Vec<_>>>()?;
    let (view, records) = inventory(&definition, &entries)?;
    let progress = definition.audit(&view, &records, false)?;
    assert_eq!(progress.status, "all_designated");
    assert_eq!(progress.rows.len(), 128);
    assert_eq!(progress.designated, 128);
    assert!(progress.rows.iter().all(|row| row.receipt.is_some()));
    Ok(())
}

#[test]
fn pending_unknown_refused_and_stopped_parent_preserve_exact_handoff() -> Result<()> {
    let definition = definition(layouts_parts())?;
    for (child_state, status) in [
        (Child::Intent, "pending_recovery"),
        (Child::Prepared, "pending_recovery"),
        (Child::Dispatched, "pending_recovery"),
        (Child::CancelRequested, "pending_recovery"),
        (Child::Unknown, "halted_unknown"),
        (Child::Refused, "halted_cancelled"),
    ] {
        let entries = vec![
            (child(&definition, 0)?, Child::Designated),
            (child(&definition, 1)?, child_state),
        ];
        let (view, records) = inventory(&definition, &entries)?;
        for stopped in [false, true] {
            let progress = definition.audit(&view, &records, stopped)?;
            assert_eq!(progress.status, status);
            assert_eq!(progress.next_index, None);
            assert_eq!(progress.designated, 1);
            assert_eq!(
                progress.pending_index,
                if matches!(child_state, Child::Refused) {
                    None
                } else {
                    Some(1)
                }
            );
            assert_eq!(progress.rows[1].key, definition.key(1)?);
        }
        if matches!(
            child_state,
            Child::Unknown | Child::Refused | Child::CancelRequested
        ) {
            assert!(
                definition
                    .validate_next(
                        &view,
                        &records,
                        false,
                        &definition.key(1)?,
                        definition.layout().regions()[1],
                        None
                    )
                    .is_err()
            );
        }
    }
    let (view, records) = inventory(&definition, &[])?;
    let stopped = definition.audit(&view, &records, true)?;
    assert_eq!(stopped.status, "stopped");
    assert_eq!(stopped.next_index, None);
    assert!(
        definition
            .validate_next(
                &view,
                &records,
                true,
                &definition.key(0)?,
                definition.layout().regions()[0],
                None
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn audit_refuses_foreign_duplicate_nonprefix_and_incomplete_inventories() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let (view, records) = inventory(&definition, &[(child(&definition, 1)?, Child::Designated)])?;
    assert!(definition.audit(&view, &records, false).is_err());
    let foreign = DigPlan::new(
        "foreign",
        false,
        capture(definition.layout().regions()[0], 10, 3)?,
    )?;
    let (view, records) = inventory(&definition, &[(foreign, Child::Designated)])?;
    assert!(definition.audit(&view, &records, false).is_err());
    let (mut view, records) =
        inventory(&definition, &[(child(&definition, 0)?, Child::Designated)])?;
    let duplicate = vec![records[0].clone(), records[0].clone()];
    view.total_records = 2;
    assert!(definition.audit(&view, &duplicate, false).is_err());
    view.total_records = 1;
    assert!(definition.audit(&view, &[], false).is_err());
    view.journal_id = Digest32::of_bytes(b"other");
    assert!(definition.audit(&view, &records, false).is_err());
    let (mut view, records) = inventory(&definition, &[(child(&definition, 0)?, Child::Prepared)])?;
    view.pending = None;
    assert!(definition.audit(&view, &records, false).is_err());
    Ok(())
}

#[test]
fn audit_rejects_hidden_relaxation_wrong_region_and_regressed_original_clock() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let first = child(&definition, 0)?;
    for changed in [
        DigPlan::new(&definition.key(0)?, true, first.before().clone())?,
        DigPlan::new(
            &definition.key(0)?,
            false,
            capture(definition.layout().regions()[1], 10, 3)?,
        )?,
        DigPlan::new(
            &definition.key(0)?,
            false,
            capture(definition.layout().regions()[0], 9, 3)?,
        )?,
        DigPlan::new(
            &definition.key(0)?,
            false,
            capture(definition.layout().regions()[0], 10, 2)?,
        )?,
    ] {
        let (view, records) = inventory(&definition, &[(changed, Child::Designated)])?;
        assert!(definition.audit(&view, &records, false).is_err());
    }
    // The native journal itself refuses regressed retained sequences on reopen.
    let regressed = DigPlan::new(
        &definition.key(1)?,
        false,
        capture(definition.layout().regions()[1], 11, 3)?,
    )?;
    assert!(
        inventory(
            &definition,
            &[(first, Child::Designated), (regressed, Child::Designated)]
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn next_guard_preserves_bootstrap_dimensions_floors_and_exact_prepared_capture() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let (view, records) = inventory(&definition, &[(child(&definition, 0)?, Child::Designated)])?;
    let plan = child(&definition, 1)?;
    definition.validate_next(
        &view,
        &records,
        false,
        plan.key(),
        plan.before().region(),
        Some(plan.before()),
    )?;
    assert!(
        definition
            .validate_next(
                &view,
                &records,
                false,
                &definition.key(2)?,
                plan.before().region(),
                Some(plan.before())
            )
            .is_err()
    );
    let mut raw = plan.before().canonical_bytes().to_vec();
    raw[36..40].copy_from_slice(&127u32.to_be_bytes());
    assert!(
        definition
            .validate_next(
                &view,
                &records,
                false,
                plan.key(),
                plan.before().region(),
                Some(&DigObservation::decode(&raw)?)
            )
            .is_err()
    );
    let old = capture(plan.before().region(), 10, 3)?;
    assert!(
        definition
            .validate_next(
                &view,
                &records,
                false,
                plan.key(),
                plan.before().region(),
                Some(&old)
            )
            .is_err()
    );
    let (view, records) = inventory(
        &definition,
        &[
            (child(&definition, 0)?, Child::Designated),
            (plan.clone(), Child::Prepared),
        ],
    )?;
    definition.validate_next(
        &view,
        &records,
        false,
        plan.key(),
        plan.before().region(),
        Some(plan.before()),
    )?;
    let newer = capture(plan.before().region(), 12, 4)?;
    assert!(
        definition
            .validate_next(
                &view,
                &records,
                false,
                plan.key(),
                plan.before().region(),
                Some(&newer)
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn complete_inventory_reads_one_journal_and_never_restores_native_permission() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let memory = Memory::new(journal_bytes(
        &definition,
        &[
            (child(&definition, 0)?, Child::Designated),
            (child(&definition, 1)?, Child::Prepared),
        ],
    )?);
    let mut session = session(&definition, memory.clone())?;
    memory.disk.borrow_mut().read_bytes = 0;
    let mut context = context(&definition);
    context.budget.max_bytes = COMPLETE_INVENTORY_BYTES;
    let (view, records) = session.complete_inventory(&context)?;
    assert_eq!(view.total_records, 2);
    assert_eq!(records.len(), 2);
    assert_eq!(memory.disk.borrow().read_bytes, view.byte_len);
    assert!(
        view.pending
            .as_ref()
            .is_some_and(|pending| !pending.dispatchable)
    );
    assert!(!session.has_preparation_connection());
    assert_eq!(
        definition.audit(&view, &records, false)?.status,
        "pending_recovery"
    );
    Ok(())
}

#[test]
fn complete_inventory_rechecks_authority_budget_and_exact_custody() -> Result<()> {
    let definition = definition(layouts_parts())?;
    let memory = Memory::new(journal_bytes(
        &definition,
        &[(child(&definition, 0)?, Child::Designated)],
    )?);
    let mut session = session(&definition, memory.clone())?;
    for change in 0..4 {
        let mut c = context(&definition);
        match change {
            0 => c.grants.clear(),
            1 => c.budget.max_bytes = 1,
            2 => c.budget.max_entities = 0,
            _ => c.cancellation_requested = true,
        };
        assert!(session.complete_inventory(&c).is_err());
    }
    memory.disk.borrow_mut().bytes[0] ^= 1;
    assert!(session.complete_inventory(&context(&definition)).is_err());
    Ok(())
}
