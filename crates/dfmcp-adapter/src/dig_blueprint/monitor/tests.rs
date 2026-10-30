use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_blueprint::{BlueprintPart, Layout};
use crate::dig_designation::journal::session::DigSession;
use crate::dig_designation::journal::{DigBinding, DigJournal, DigMode};
use crate::dig_designation::rpc::{DigManifest, DigPreparation, DigSource};
use crate::dig_designation::{DigObservation, DigPlan, DigRegion};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use dfmcp_world::map_region::{MapRegion, Tile};
use std::collections::BTreeSet;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

pub(crate) fn parts() -> Vec<BlueprintPart> {
    vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [1, 1, 1],
        },
        BlueprintPart {
            origin: [4, 2, 3],
            size: [1, 1, 1],
        },
    ]
}

fn dig_capture(
    region: DigRegion,
    tick: u64,
    sequence: u64,
    designated: &BTreeSet<[u32; 3]>,
) -> Result<DigObservation> {
    let mut raw = b"DFMDG016".to_vec();
    for n in [7u64, sequence, tick] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    for n in [1u32, 128, 128, 32] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    for n in region.coordinates() {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    raw.push(1);
    text(&mut raw, "region1");
    raw.extend_from_slice(&(region.halo_count() as u16).to_be_bytes());
    let [x, y, z, w, h] = region.coordinates();
    for pz in z - 1..=z + 1 {
        for py in y - 1..=y + h {
            for px in x - 1..=x + w {
                let selected = designated.contains(&[px, py, pz]);
                let dirty = designated
                    .iter()
                    .any(|p| p[2] == pz && p[0] / 16 == px / 16 && p[1] / 16 == py / 16);
                raw.push(2);
                for n in [200u32, 0, 0, if selected { 4000 } else { 0 }, 0, 0] {
                    raw.extend_from_slice(&n.to_be_bytes());
                }
                raw.extend_from_slice(&10000u16.to_be_bytes());
                raw.extend_from_slice(&10000u16.to_be_bytes());
                raw.extend_from_slice(&[u8::from(selected), 0, 1 | if dirty { 16 } else { 0 }]);
            }
        }
    }
    DigObservation::decode(&raw)
}
fn native(plan: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let before = plan.before();
    let mut raw = b"DFMDGE16".to_vec();
    for n in [before.generation(), before.sequence(), before.tick()] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    for n in before.region().coordinates() {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    raw.push(u8::from(plan.allow_hidden_neighbors()));
    raw.extend_from_slice(before.witness().as_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.push(match phase {
        DigPhase::Prepared => 0,
        DigPhase::Unknown => 1,
        DigPhase::Designated => 2,
        DigPhase::Refused => 4,
    });
    raw.push(if phase == DigPhase::Refused { 2 } else { 0 });
    raw.push(u8::from(phase == DigPhase::Designated));
    raw.extend_from_slice(
        &(if phase == DigPhase::Designated {
            before.region().target_count()
        } else {
            0
        })
        .to_be_bytes(),
    );
    let after = if phase == DigPhase::Designated {
        let mut bytes = before.canonical_bytes().to_vec();
        bytes[16..24].copy_from_slice(&(before.sequence() + 1).to_be_bytes());
        let start = bytes.len() - before.region().halo_count() * 32;
        let [x, y, z, w, h] = before.region().coordinates();
        for (index, (point, _)) in before.tiles().enumerate() {
            let at = start + index * 32;
            if point[2] == z && (x..x + w).contains(&point[0]) && (y..y + h).contains(&point[1]) {
                bytes[at + 13..at + 17].copy_from_slice(&4000u32.to_be_bytes());
                bytes[at + 29] = 1;
            }
            if before.region().write_area().contains(MapCoord::new(
                point[0] as i32,
                point[1] as i32,
                point[2] as i32,
            )) {
                bytes[at + 17..at + 21].fill(0);
                bytes[at + 31] |= 16;
            }
        }
        Digest32::of_bytes(&bytes)
    } else {
        Digest32::ZERO
    };
    raw.extend_from_slice(after.as_bytes());
    let mut proof = before.generation().to_be_bytes().to_vec();
    text(&mut proof, plan.key());
    proof.extend_from_slice(plan.digest().as_bytes());
    proof.extend_from_slice(plan.token());
    proof.extend_from_slice(&raw[133..172]);
    raw.extend_from_slice(
        if phase.terminal() {
            hash(b"dfmcp-dig-designation-receipt/1", &proof)
        } else {
            Digest32::ZERO
        }
        .as_bytes(),
    );
    text(&mut raw, plan.key());
    DigEffect::decode(&raw, plan)
}
fn frame(
    raw: &mut Vec<u8>,
    head: &mut Digest32,
    event: &mut u64,
    plan: &DigPlan,
    state: DigState,
    phase: Option<DigPhase>,
) -> Result<()> {
    let mut body = vec![state as u8];
    blob(&mut body, &plan.canonical_bytes());
    let effect = phase.map(|p| native(plan, p)).transpose()?;
    blob(
        &mut body,
        effect.as_ref().map_or(&[][..], DigEffect::canonical_bytes),
    );
    *event += 1;
    let mut entry = b"DFMDJR01".to_vec();
    entry.extend_from_slice(&(body.len() as u32).to_be_bytes());
    entry.extend_from_slice(&event.to_be_bytes());
    entry.extend_from_slice(head.as_bytes());
    entry.extend_from_slice(&body);
    *head = hash(b"dfmcp-dig-journal-frame/1", &entry);
    entry.extend_from_slice(head.as_bytes());
    entry.extend_from_slice(b"DFMDJEND");
    raw.extend_from_slice(&entry);
    Ok(())
}
struct Memory(Cursor<Vec<u8>>);
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.0.seek(at)
    }
}
impl Write for Memory {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("read-only native fixture"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("read-only native fixture"))
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Err(io::Error::other("read-only native fixture"))
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("read-only native fixture"))
    }
}
struct NoSource(DigManifest);
impl DigSource for NoSource {
    fn manifest(&self) -> &DigManifest {
        &self.0
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        None
    }
    fn observe(&mut self, _: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        Err(invalid("unexpected native fixture call"))
    }
    fn prepare(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        Err(invalid("unexpected native fixture call"))
    }
    fn commit(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        Err(invalid("unexpected native fixture call"))
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        Err(invalid("unexpected native fixture call"))
    }
    fn cancel(&mut self, _: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        Err(invalid("unexpected native fixture call"))
    }
}
pub(crate) fn context(spec: &MonitorSpec) -> OperationContext {
    context_for(spec.parent(), spec.start_tick())
}
fn context_for(parent: &BatchDefinition, tick: u64) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: parent.binding().fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(tick),
            state_hash: parent.id(),
        },
        budget: WorkBudget {
            max_wall_millis: 60000,
            max_game_ticks: 0,
            max_entities: 8192,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 65536,
            max_actions: 1,
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(parent.binding().fortress_id()),
                    map_area: Some(parent.binding().scope()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}
pub(crate) fn native_history(
    parts: Vec<BlueprintPart>,
) -> Result<(BatchDefinition, DigSessionView, Vec<DigRecord>)> {
    history(parts, false)
}
fn history(
    parts: Vec<BlueprintPart>,
    pending_last: bool,
) -> Result<(BatchDefinition, DigSessionView, Vec<DigRecord>)> {
    let layout = Layout::new(parts)?;
    let mut targets = BTreeSet::new();
    let bootstrap = dig_capture(layout.regions()[0], 10, 3, &targets)?;
    let binding = DigBinding::new(
        "127.0.0.1:5000"
            .parse()
            .map_err(|_| invalid("fixture endpoint"))?,
        DigManifest {
            generation: 7,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
        },
        &bootstrap,
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(127, 127, 31))?,
    )?;
    let encoded = binding.canonical_bytes();
    let mut raw = b"DFMDJ001".to_vec();
    raw.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
    raw.extend_from_slice(&encoded);
    raw.extend_from_slice(&[9; 32]);
    let id = hash(b"dfmcp-dig-journal/1", &raw);
    raw.extend_from_slice(id.as_bytes());
    let parent = BatchDefinition::new(layout, binding.clone(), id, bootstrap)?;
    let mut head = id;
    let mut event = 0;
    for (index, region) in parent.layout().regions().iter().copied().enumerate() {
        let plan = DigPlan::new(
            &parent.key(index)?,
            false,
            dig_capture(region, 20 + index as u64, 3 + index as u64, &targets)?,
        )?;
        frame(
            &mut raw,
            &mut head,
            &mut event,
            &plan,
            DigState::Intent,
            None,
        )?;
        frame(
            &mut raw,
            &mut head,
            &mut event,
            &plan,
            DigState::Prepared,
            Some(DigPhase::Prepared),
        )?;
        if !pending_last || index + 1 != parent.layout().regions().len() {
            frame(
                &mut raw,
                &mut head,
                &mut event,
                &plan,
                DigState::DispatchStarted,
                Some(DigPhase::Prepared),
            )?;
            frame(
                &mut raw,
                &mut head,
                &mut event,
                &plan,
                DigState::Terminal,
                Some(DigPhase::Designated),
            )?;
            let [x, y, z, w, h] = region.coordinates();
            for py in y..y + h {
                for px in x..x + w {
                    targets.insert([px, py, z]);
                }
            }
        }
    }
    let c = context_for(&parent, 1000);
    let journal = DigJournal::open(
        Memory(Cursor::new(raw)),
        &c,
        DigMode::Offline,
        Some(binding),
        None,
    )?;
    let mut session = DigSession::<_, NoSource>::new(journal, &c)?;
    let (view, records) = session.complete_inventory(&c)?;
    Ok((parent, view, records))
}
pub(crate) fn spec(parts: Vec<BlueprintPart>) -> Result<MonitorSpec> {
    let (parent, view, records) = native_history(parts)?;
    MonitorSpec::new(
        parent.clone(),
        &view,
        &records,
        SourcePin::new(
            parent.binding().endpoint(),
            29,
            "test-df".into(),
            "test-dfhack".into(),
        )?,
        Policy::new(2000, 10, 2, 1200, 128)?,
        1000,
    )
}
pub(crate) fn capture(spec: &MonitorSpec, tick: u64) -> Result<LiveMapObservation> {
    let value = LiveMapObservation {
        bridge_generation: spec.source().generation(),
        df_version: spec.source().df_version().into(),
        dfhack_version: spec.source().dfhack_version().into(),
        year: u32::try_from(tick / 403200).map_err(|_| invalid("fixture year"))?,
        year_tick: (tick % 403200) as u32,
        paused: true,
        site_id: spec.parent().binding().site(),
        world_folder: spec.parent().binding().folder().into(),
        map_dimensions: spec.parent().bootstrap().dimensions(),
        map: MapRegion {
            region: spec.region(),
            cells: vec![
                Cell::Visible(floor());
                spec.region().volume().map_err(crate::live_map::map_error)?
            ],
        },
    };
    value.validate()?;
    Ok(value)
}
fn floor() -> Tile {
    Tile {
        native_tiletype: 200,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10000,
        temperature_2: 10000,
    }
}
fn set(capture: &mut LiveMapObservation, position: [u32; 3], cell: Cell) -> Result<()> {
    let index = capture
        .map
        .region
        .index(position)
        .ok_or_else(|| invalid("fixture cell index"))?;
    let target = capture
        .map
        .cells
        .get_mut(index)
        .ok_or_else(|| invalid("fixture target cell"))?;
    *target = cell;
    Ok(())
}
fn with_policy(spec: &MonitorSpec, policy: Policy) -> Result<MonitorSpec> {
    let (parent, view, records) = native_history(spec.parent().layout().parts().to_vec())?;
    MonitorSpec::new(
        parent,
        &view,
        &records,
        spec.source().clone(),
        policy,
        spec.start_tick(),
    )
}

#[test]
fn complete_original_receipts_are_required_and_reverified() -> Result<()> {
    let original = spec(parts())?;
    let (parent, view, records) = native_history(parts())?;
    original.verify_original(&parent, &view, &records)?;
    let (pending, partial, entries) = history(parts(), true)?;
    assert!(
        MonitorSpec::new(
            pending,
            &partial,
            &entries,
            original.source().clone(),
            *original.policy(),
            1000
        )
        .is_err()
    );
    let mut short = view.clone();
    short.total_records -= 1;
    assert!(
        MonitorSpec::new(
            parent.clone(),
            &short,
            &records[..records.len() - 1],
            original.source().clone(),
            *original.policy(),
            1000
        )
        .is_err()
    );
    assert!(
        original
            .verify_original(&parent, &view, &records[..records.len() - 1])
            .is_err()
    );
    let mut changed = view.clone();
    changed.head = Digest32::of_bytes(b"another complete native head");
    assert!(
        original
            .verify_original(&parent, &changed, &records)
            .is_err()
    );
    let mut reordered = records.clone();
    reordered.reverse();
    original.verify_original(&parent, &view, &reordered)?;
    let split = spec(vec![BlueprintPart {
        origin: [2, 2, 2],
        size: [1, 1, 1],
    }])?;
    assert!(
        original
            .verify_original(split.parent(), &view, &records)
            .is_err()
    );
    assert_eq!(
        parent.audit(&view, &records, true)?.designated,
        records.len()
    );
    original.verify_original(&parent, &view, &records)?; // A parent stop does not alter this link.
    Ok(())
}

#[test]
fn source_pin_and_spec_codecs_preserve_exact_identity_without_restoring_evidence() -> Result<()> {
    let spec = spec(parts())?;
    let source = spec.source().canonical_bytes();
    assert_eq!(SourcePin::decode(&source)?, *spec.source());
    for end in 0..source.len() {
        assert!(SourcePin::decode(&source[..end]).is_err());
    }
    let mut extra = source;
    extra.push(0);
    assert!(SourcePin::decode(&extra).is_err());
    let bytes = spec.canonical_bytes();
    assert_eq!(MonitorSpec::decode(&bytes)?, spec);
    assert!(bytes.len() <= MAX_SPEC_BYTES);
    for end in 0..bytes.len() {
        assert!(MonitorSpec::decode(&bytes[..end]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(MonitorSpec::decode(&extra).is_err());
    let mut substituted = bytes.clone();
    let end = substituted.len();
    substituted[end - 1] ^= 1;
    let substituted = MonitorSpec::decode(&substituted)?;
    assert_ne!(substituted.id(), spec.id());
    let (parent, view, records) = native_history(parts())?;
    assert!(
        substituted
            .verify_original(&parent, &view, &records)
            .is_err()
    );
    let mut missing_head = bytes;
    missing_head[end - 64..end - 32].fill(0);
    assert!(MonitorSpec::decode(&missing_head).is_err());
    Ok(())
}

#[test]
fn independent_generations_are_never_equated_and_selectors_cannot_change() -> Result<()> {
    let base = spec(parts())?;
    assert_ne!(
        base.source().generation(),
        base.parent().binding().manifest().generation
    );
    base.validate_capture(&capture(&base, 1000)?)?;
    let (parent, view, records) = native_history(parts())?;
    let equal = MonitorSpec::new(
        parent.clone(),
        &view,
        &records,
        SourcePin::new(
            parent.binding().endpoint(),
            7,
            "test-df".into(),
            "test-dfhack".into(),
        )?,
        *base.policy(),
        1000,
    )?;
    equal.validate_capture(&capture(&equal, 1000)?)?;
    assert_ne!(equal.id(), base.id()); // Neither equality nor inequality grants cross-plugin identity.
    for source in [
        SourcePin::new(
            "127.0.0.1:5001"
                .parse()
                .map_err(|_| invalid("fixture endpoint"))?,
            29,
            "test-df".into(),
            "test-dfhack".into(),
        )?,
        SourcePin::new(
            parent.binding().endpoint(),
            29,
            "other-df".into(),
            "test-dfhack".into(),
        )?,
    ] {
        assert!(
            MonitorSpec::new(
                parent.clone(),
                &view,
                &records,
                source,
                *base.policy(),
                1000
            )
            .is_err()
        );
    }
    for generation in [0, u64::MAX] {
        assert!(
            SourcePin::new(
                parent.binding().endpoint(),
                generation,
                "df".into(),
                "dfhack".into()
            )
            .is_err()
        );
    }
    assert!(
        SourcePin::new(
            "192.0.2.1:5000"
                .parse()
                .map_err(|_| invalid("fixture endpoint"))?,
            1,
            "df".into(),
            "dfhack".into()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn sparse_multilevel_targets_match_together_and_ignore_unselected_hidden_holes() -> Result<()> {
    let spec = spec(parts())?;
    assert_eq!(
        spec.region(),
        Region {
            origin: [2, 2, 2],
            size: [3, 1, 2]
        }
    );
    let mut a = capture(&spec, 1000)?;
    let targets = spec.parent().layout().targets();
    for index in 0..a.map.cells.len() {
        let position = a
            .map
            .region
            .position(index)
            .ok_or_else(|| invalid("fixture position"))?;
        if !targets.contains(&position) {
            a.map.cells[index] = Cell::Hidden;
        }
    }
    assert_eq!(
        spec.classify(&a)?,
        Counts {
            matched: 2,
            ..Counts::default()
        }
    );
    let state = MonitorState::new(spec.clone()).sample(a)?;
    assert_eq!(state.status(), Status::Stabilizing);
    let b = capture(&spec, 1010)?;
    let complete = state.sample(b)?;
    assert_eq!(complete.status(), Status::Satisfied);
    assert_eq!(
        complete.first_satisfied().map(|s| s.tick().get()),
        Some(1010)
    );

    let mut first = capture(&spec, 1000)?;
    set(
        &mut first,
        targets[1],
        Cell::Visible(Tile {
            shape: Shape::Wall,
            ..floor()
        }),
    )?;
    let state = MonitorState::new(spec.clone()).sample(first)?;
    assert_eq!(state.status(), Status::Pending);
    let mut second = capture(&spec, 1010)?;
    set(
        &mut second,
        targets[0],
        Cell::Visible(Tile {
            shape: Shape::Wall,
            ..floor()
        }),
    )?;
    let state = state.sample(second)?;
    assert_eq!(state.status(), Status::Pending);
    assert_eq!(state.streak(), 0);
    assert_eq!(state.counts()?.map(|c| c.matched), Some(1));
    let state = state.sample(capture(&spec, 1020)?)?;
    assert_eq!(state.streak(), 1);
    assert_eq!(
        state.sample(capture(&spec, 1030)?)?.status(),
        Status::Satisfied
    );
    Ok(())
}

#[test]
fn all_576_shape_liquid_designation_combinations_and_magma_are_exact() -> Result<()> {
    let spec = spec(vec![BlueprintPart {
        origin: [2, 2, 2],
        size: [1, 1, 1],
    }])?;
    for tag in 0..=8 {
        let shape = Shape::from_tag(tag).map_err(crate::live_map::map_error)?;
        for liquid_depth in 0..=7 {
            for dig_designation in 0..=7 {
                let mut raw = capture(&spec, 1000)?;
                raw.map.cells[0] = Cell::Visible(Tile {
                    shape,
                    liquid_depth,
                    dig_designation,
                    ..floor()
                });
                let counts = spec.classify(&raw)?;
                assert_eq!(
                    counts.matched,
                    u32::from(shape == Shape::Floor && liquid_depth == 0 && dig_designation == 0)
                );
                assert_eq!(counts.total(), 1);
                assert_eq!(counts.wrong_shape, u32::from(shape != Shape::Floor));
                assert_eq!(counts.wet, u32::from(liquid_depth != 0));
                assert_eq!(counts.designated, u32::from(dig_designation != 0));
            }
        }
    }
    let mut raw = capture(&spec, 1000)?;
    raw.map.cells[0] = Cell::Visible(Tile {
        magma: true,
        ..floor()
    });
    assert_eq!(spec.classify(&raw)?.wet, 1);
    assert_eq!(spec.classify(&raw)?.matched, 0);
    raw.map.cells[0] = Cell::Visible(Tile {
        building_occupancy: 7,
        unit_occupancy: 3,
        walkable_region: 0,
        temperature_1: 65535,
        ..floor()
    });
    assert_eq!(spec.classify(&raw)?.matched, 1); // No access, occupancy or safety claim.
    Ok(())
}

#[test]
fn hidden_missing_and_contradictions_reset_the_complete_mask_streak() -> Result<()> {
    let spec = spec(parts())?;
    for bad in [
        Cell::Hidden,
        Cell::Unallocated,
        Cell::Visible(Tile {
            dig_designation: 1,
            ..floor()
        }),
    ] {
        let state = MonitorState::new(spec.clone()).sample(capture(&spec, 1000)?)?;
        let mut raw = capture(&spec, 1005)?;
        set(&mut raw, spec.parent().layout().targets()[1], bad)?;
        let state = state.sample(raw)?;
        assert_eq!(state.streak(), 0);
        assert_eq!(state.since_tick(), None);
        assert!(matches!(state.status(), Status::Unknown | Status::Pending));
        let state = state.sample(capture(&spec, 1010)?)?;
        assert_eq!(state.status(), Status::Stabilizing);
        assert_eq!(state.streak(), 1);
        assert_eq!(state.since_tick(), Some(1010));
    }
    Ok(())
}

#[test]
fn equal_ticks_failed_reads_and_reopen_do_not_inflate_sampled_stability() -> Result<()> {
    let spec = spec(parts())?;
    let state = MonitorState::new(spec.clone()).sample(capture(&spec, 1000)?)?;
    let state = state.sample(capture(&spec, 1000)?)?;
    assert_eq!(state.streak(), 1);
    assert_eq!(state.attempts(), 2);
    let pending = state.begin_attempt()?;
    assert!(pending.pending());
    assert_eq!(pending.attempts(), 3);
    assert!(pending.begin_attempt().is_err());
    for reset in [pending.interrupt()?, pending.reopened()?, state.reopened()?] {
        assert_eq!(reset.status(), Status::Unknown);
        assert_eq!(reset.streak(), 0);
        assert_eq!(reset.latest(), state.latest());
        let next = reset.sample(capture(&spec, 1010)?)?;
        assert_eq!(next.streak(), 1);
        assert_eq!(next.since_tick(), Some(1010));
    }
    let completed = state.sample(capture(&spec, 1010)?)?;
    assert_eq!(completed.reopened()?, completed); // Historical only; no current-proof flag exists.
    assert!(completed.sample(capture(&spec, 1020)?).is_err());
    assert!(completed.interrupt().is_err());
    assert!(completed.cancel().is_err());
    Ok(())
}

#[test]
fn changed_same_tick_captures_reset_stability_until_an_advancing_sample() -> Result<()> {
    let spec = spec(parts())?;
    for unselected in [false, true] {
        let state = MonitorState::new(spec.clone()).sample(capture(&spec, 1000)?)?;
        let mut changed = capture(&spec, 1000)?;
        if unselected {
            changed.map.cells[1] = Cell::Hidden;
        } else {
            set(
                &mut changed,
                spec.parent().layout().targets()[0],
                Cell::Visible(Tile {
                    temperature_1: 10001,
                    ..floor()
                }),
            )?;
        }
        assert_eq!(spec.classify(&changed)?.matched, 2);
        let state = state.sample(changed.clone())?;
        assert_eq!(state.streak(), 0);
        assert_eq!(state.since_tick(), None);
        assert_eq!(state.status(), Status::Stabilizing);
        let state = state.sample(changed)?;
        assert_eq!(state.streak(), 0);
        let state = state.sample(capture(&spec, 1005)?)?;
        assert_eq!(state.streak(), 1);
        assert_eq!(state.since_tick(), Some(1005));
        let state = state.sample(capture(&spec, 1010)?)?;
        assert_eq!(state.status(), Status::Stabilizing);
        let state = state.sample(capture(&spec, 1015)?)?;
        assert_eq!(state.status(), Status::Satisfied);
        assert_eq!(
            state.first_satisfied().map(|raw| raw.tick().get()),
            Some(1015)
        );
    }
    Ok(())
}

#[test]
fn equal_tick_corrections_and_restarts_cannot_seed_single_sample_success() -> Result<()> {
    let base = spec(parts())?;
    let spec = with_policy(&base, Policy::new(2000, 0, 1, 1200, 128)?)?;
    let mut mismatch = capture(&spec, 1000)?;
    mismatch.map.cells[0] = Cell::Visible(Tile {
        shape: Shape::Wall,
        ..floor()
    });
    let state = MonitorState::new(spec.clone()).sample(mismatch)?;
    let state = state.sample(capture(&spec, 1000)?)?;
    assert_eq!(state.status(), Status::Stabilizing);
    assert_eq!(state.streak(), 0);
    let state = state.sample(capture(&spec, 1000)?)?;
    assert_eq!(state.streak(), 0);
    assert_eq!(
        state.sample(capture(&spec, 1001)?)?.status(),
        Status::Satisfied
    );

    let state = MonitorState::new(base.clone()).sample(capture(&base, 1000)?)?;
    let state = state.reopened()?.sample(capture(&base, 1000)?)?;
    assert_eq!(state.streak(), 0);
    assert_eq!(state.since_tick(), None);
    assert_eq!(state.status(), Status::Stabilizing);
    Ok(())
}

#[test]
fn excessive_gaps_restart_stability_and_deadline_is_fixed_inclusive() -> Result<()> {
    let base = spec(parts())?;
    let spec = with_policy(&base, Policy::new(1030, 10, 2, 10, 128)?)?;
    let state = MonitorState::new(spec.clone()).sample(capture(&spec, 1000)?)?;
    let state = state.sample(capture(&spec, 1011)?)?;
    assert_eq!(state.streak(), 1);
    assert_eq!(state.since_tick(), Some(1011));
    let state = state.sample(capture(&spec, 1020)?)?;
    assert_eq!(state.status(), Status::Stabilizing);
    let state = state.sample(capture(&spec, 1030)?)?;
    assert_eq!(state.status(), Status::Satisfied);
    assert_eq!(state.first_satisfied().map(|s| s.tick().get()), Some(1030));
    let late = MonitorState::new(spec.clone()).sample(capture(&spec, 1031)?)?;
    assert_eq!(late.status(), Status::Expired);
    assert_eq!(late.latest().map(|s| s.tick().get()), Some(1031));
    assert!(late.sample(capture(&spec, 1032)?).is_err());
    assert_eq!(late.spec().policy().deadline_tick(), 1030);
    assert_eq!(late.reopened()?.status(), Status::Expired);
    Ok(())
}

#[test]
fn source_changes_and_clock_regression_invalidate_without_replacing_accepted_sample() -> Result<()>
{
    let spec = spec(parts())?;
    let state = MonitorState::new(spec.clone()).sample(capture(&spec, 1010)?)?;
    let baseline = state.latest().cloned();
    let mutations: Vec<Box<dyn Fn(&mut LiveMapObservation)>> = vec![
        Box::new(|r| r.bridge_generation += 1),
        Box::new(|r| r.df_version.push('x')),
        Box::new(|r| r.dfhack_version.push('x')),
        Box::new(|r| r.world_folder.push('x')),
        Box::new(|r| r.site_id += 1),
        Box::new(|r| r.map_dimensions[0] += 1),
        Box::new(|r| r.paused = false),
        Box::new(|r| r.map.region.origin[0] += 1),
    ];
    for mutate in mutations {
        let mut raw = capture(&spec, 1020)?;
        mutate(&mut raw);
        raw.validate()?;
        assert!(spec.validate_capture(&raw).is_err());
        let next = state.sample(raw)?;
        assert_eq!(next.status(), Status::Invalidated);
        assert_eq!(next.latest().cloned(), baseline);
        assert_eq!(next.high_tick(), 1020);
        assert!(next.sample(capture(&spec, 1030)?).is_err());
    }
    let next = state.sample(capture(&spec, 1009)?)?;
    assert_eq!(next.status(), Status::Invalidated);
    assert_eq!(next.high_tick(), 1010);
    assert_eq!(next.latest().cloned(), baseline);
    let before_start = MonitorState::new(spec.clone()).sample(capture(&spec, 999)?)?;
    assert_eq!(before_start.status(), Status::Invalidated);
    assert_eq!(before_start.latest(), None);
    Ok(())
}

#[test]
fn all_128_attempts_include_failed_reads_and_keep_local_cancellation_available() -> Result<()> {
    let spec = spec(parts())?;
    let mut state = MonitorState::new(spec.clone());
    for attempt in 1..=128 {
        state = state.begin_attempt()?.interrupt()?;
        assert_eq!(state.attempts(), attempt);
        assert_eq!(state.status(), Status::Unknown);
    }
    assert!(state.begin_attempt().is_err());
    assert!(state.sample(capture(&spec, 1000)?).is_err());
    assert_eq!(state.cancel()?.status(), Status::Cancelled);
    assert_eq!(state.cancel()?.attempts(), 128);
    let pending = MonitorState::new(spec).begin_attempt()?;
    assert_eq!(pending.invalidate()?.status(), Status::Invalidated);
    Ok(())
}

#[test]
fn policy_bounds_and_feasibility_are_sealed_before_any_read() -> Result<()> {
    for policy in [
        Policy::new(MAX_NATIVE_TICK + 1, 0, 1, 1, 1),
        Policy::new(2000, 403201, 1, 1, 128),
        Policy::new(2000, 0, 0, 1, 128),
        Policy::new(2000, 0, 65, 1, 128),
        Policy::new(2000, 0, 1, 0, 128),
        Policy::new(2000, 0, 1, 403201, 128),
        Policy::new(2000, 0, 1, 1, 0),
        Policy::new(2000, 0, 1, 1, 129),
        Policy::new(2000, 0, 3, 1, 2),
    ] {
        assert!(policy.is_err());
    }
    let base = spec(parts())?;
    for policy in [
        Policy::new(999, 0, 1, 1, 128)?,
        Policy::new(1001, 2, 1, 1200, 128)?,
        Policy::new(1001, 0, 3, 1200, 128)?,
        Policy::new(2000, 10, 2, 4, 3)?,
    ] {
        assert!(with_policy(&base, policy).is_err());
    }
    let immediate = with_policy(&base, Policy::new(1000, 0, 1, 1, 1)?)?;
    assert_eq!(
        MonitorState::new(immediate.clone())
            .sample(capture(&immediate, 1000)?)?
            .status(),
        Status::Satisfied
    );
    assert_ne!(immediate.id(), base.id());
    Ok(())
}

#[test]
fn scoped_read_authority_never_comes_from_stored_source_pins() -> Result<()> {
    let spec = spec(parts())?;
    let c = context(&spec);
    spec.authorize(&c, true)?;
    assert!(
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
            .is_err()
    );
    let mut query_only = c.clone();
    query_only
        .grants
        .retain(|g| g.capability == Capability::Query);
    spec.authorize(&query_only, false)?;
    assert!(spec.authorize(&query_only, true).is_err());
    let mut exact_observe = c.clone();
    for grant in &mut exact_observe.grants {
        if grant.capability == Capability::Observe {
            grant.scope.map_area = Some(cuboid(spec.region())?);
        }
    }
    spec.authorize(&exact_observe, true)?;
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(spec.authorize(&denied, false).is_err());
    let mut denied = c.clone();
    denied.anchor.fortress_id = FortressId::new(c.anchor.fortress_id.get() + 2);
    assert!(spec.authorize(&denied, false).is_err());
    let mut denied = c.clone();
    denied.cancellation_requested = true;
    assert!(spec.authorize(&denied, false).is_err());
    let mut denied = c.clone();
    denied.budget.max_entities = 1;
    assert!(spec.authorize(&denied, true).is_err());
    let mut expired = c;
    expired.anchor.tick = GameTick(0);
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(999));
    }
    assert!(spec.authorize(&expired, false).is_err());
    Ok(())
}

#[test]
fn map_capsules_redecode_exact_source_and_reject_truncation_and_derived_forgery() -> Result<()> {
    let spec = spec(parts())?;
    let sample = capture(&spec, 1000)?;
    let raw = encode_capture(&sample)?;
    assert_eq!(decode_capture(&raw)?, sample);
    for end in 0..raw.len() {
        assert!(decode_capture(&raw[..end]).is_err());
    }
    let mut extra = raw;
    extra.push(0);
    assert!(decode_capture(&extra).is_err());
    let mut malformed = sample.clone();
    malformed.map.cells.pop();
    assert!(encode_capture(&malformed).is_err());
    assert!(MonitorState::new(spec.clone()).sample(malformed).is_err());
    let mut changed = sample.clone();
    changed.bridge_generation += 1;
    assert_ne!(capture_digest(&sample)?, capture_digest(&changed)?);
    assert_eq!(
        MonitorState::new(spec)
            .sample(decode_capture(&encode_capture(&changed)?)?)?
            .status(),
        Status::Invalidated
    );
    Ok(())
}

#[test]
fn maximum_sparse_mask_and_128_receipts_keep_complete_bounded_evidence() -> Result<()> {
    let maximum = spec(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [8, 8, 4],
        },
        BlueprintPart {
            origin: [2, 2, 14],
            size: [8, 8, 4],
        },
    ])?;
    assert_eq!(maximum.parent().layout().targets().len(), 512);
    assert_eq!(
        maximum
            .region()
            .volume()
            .map_err(crate::live_map::map_error)?,
        1024
    );
    let sample = capture(&maximum, 1000)?;
    let raw = encode_capture(&sample)?;
    assert!(raw.len() <= MAX_CAPTURE_BYTES);
    assert_eq!(decode_capture(&raw)?, sample);
    assert_eq!(maximum.classify(&sample)?.matched, 512);
    let state = MonitorState::new(maximum.clone()).sample(sample)?;
    assert_eq!(
        state.sample(capture(&maximum, 1010)?)?.status(),
        Status::Satisfied
    );
    let many = spec(
        (0..32)
            .map(|n| BlueprintPart {
                origin: [1 + 2 * n, 1, 1],
                size: [1, 1, 4],
            })
            .collect(),
    )?;
    assert_eq!(many.parent().layout().regions().len(), 128);
    let raw = many.canonical_bytes();
    assert!(raw.len() <= MAX_SPEC_BYTES);
    assert_eq!(MonitorSpec::decode(&raw)?, many);
    assert_eq!(many.classify(&capture(&many, 1000)?)?.matched, 128);
    Ok(())
}
