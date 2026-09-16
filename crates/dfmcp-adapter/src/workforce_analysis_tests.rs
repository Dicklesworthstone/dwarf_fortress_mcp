use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
use dfmcp_world::map_region::{MapRegion, Region, Shape, Tile};
use crate::live_map::LiveMapObservation;
use crate::live_operations::OperationsProfile;
use crate::live_spatial::{LiveSpatialObservation, citizens::LiveSpatialCitizenObservation};

fn put(out: &mut Vec<u8>, value: u32) { out.extend_from_slice(&value.to_be_bytes()); }
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes()); out.extend_from_slice(value.as_bytes());
}
fn floor() -> Tile {
    Tile { native_tiletype: 1, shape: Shape::Floor, liquid_depth: 0, magma: false,
        traffic: 0, dig_designation: 0, building_occupancy: 0, unit_occupancy: 0,
        walkable_region: 1, temperature_1: 10015, temperature_2: 10015 }
}
fn state(masks: &[u8]) -> Result<LiveSpatialCitizenState> {
    let hex = include_str!("../tests/fixtures/spatial_v1_6.hex").trim();
    let raw = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i+2], 16)
        .map_err(|_| invalid("test spatial fixture hex"))).collect::<Result<Vec<_>>>()?;
    let original = LiveSpatialObservation::decode_payload(&raw, 7, "df".into(), "dfhack".into())?;
    let mut operations = original.operations().clone();
    operations.jobs.jobs.clear(); operations.attachments.clear(); operations.items.clear(); operations.buildings.clear();
    let mut terrain: LiveMapObservation = original.terrain().clone();
    terrain.map = MapRegion { region: Region { origin: [0,0,5], size: [3,3,1] }, cells: vec![Cell::Visible(floor()); 9] };
    let mut spatial = b"DFMS1600".to_vec();
    for part in [operations.encode_profile(OperationsProfile::PagedV1_4)?, terrain.encode_payload()?] {
        put(&mut spatial, part.len() as u32); spatial.extend_from_slice(&part);
    }
    let mut citizens = b"DFMC1800".to_vec(); put(&mut citizens, masks.len() as u32);
    for (i, &mask) in masks.iter().enumerate() {
        put(&mut citizens, 10 + i as u32); text(&mut citizens, "Urist"); text(&mut citizens, "DWARF");
        put(&mut citizens, 0); // profession
        for n in [1, 0, 5] { put(&mut citizens, n); }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes()); // alive, sane, active, visible, strict adult
        put(&mut citizens, 6); citizens.extend_from_slice(&[1,1]); // stress and both availability policies
        citizens.extend_from_slice(&(mask.count_ones() as u16).to_be_bytes());
        for (bit, key) in ["CARPENTRY", "MINING", "MASONRY"].iter().enumerate() {
            if mask & (1u8 << bit) != 0 {
                put(&mut citizens, bit as u32); text(&mut citizens, key);
                for n in [5,5,1] { put(&mut citizens, n); }
            }
        }
    }
    let mut bytes = b"DFMS1800".to_vec();
    for part in [spatial, citizens] { put(&mut bytes, part.len() as u32); bytes.extend_from_slice(&part); }
    let source = LiveSpatialCitizenObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
    let mut state = LiveSpatialCitizenState::default(); state.publish(source)?; Ok(state)
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let anchor = state.snapshot().ok_or_else(|| invalid("fixture snapshot"))?.anchor();
    Ok(OperationContext { session_id: SessionId::new(171), request_id: RequestId::new(1), anchor,
        budget: WorkBudget { max_entities: 100_000, max_wall_millis: 60_000, max_bytes: 16*1024*1024,
            max_output_tokens: 65_536, ..WorkBudget::default() }, cancellation_requested: false,
        grants: vec![CapabilityGrant { capability: Capability::Query, max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope { fortress_id: Some(anchor.fortress_id), ..CapabilityScope::default() },
            expires_at_tick: None, remaining_uses: None }] })
}
fn demand(key: &str, skill: &str) -> WorkforceDemand {
    WorkforceDemand { key: key.into(), workers: 1, target: [0,0,5], skill_key: skill.into(),
        min_effective_skill: 1, preserve_social: true, adults_only: true }
}

#[test]
fn overlapping_demands_do_not_double_book_and_preserve_specialists() -> Result<()> {
    let state = state(&[3,1])?; let context = context(&state)?;
    let before = state.snapshot().cloned();
    let demands = [demand("a", "CARPENTRY"), demand("b", "MINING")];
    let result = plan(&state, &context, &demands, 1_000_000)?;
    assert_eq!(result.allocation.allocated_units, 2);
    assert_eq!(result.allocation.cut_capacity, 2);
    assert_eq!(result.allocation.assignments.iter().map(|a| (a.demand_index, a.supply_id, a.units)).collect::<Vec<_>>(),
        [(0, citizen_entity_id(11).get(), 1), (1, citizen_entity_id(10).get(), 1)]);
    assert_eq!(state.snapshot(), before.as_ref());
    assert_eq!(plan(&state, &context, &[demands[1].clone(), demands[0].clone()], 1_000_000)?, result);
    Ok(())
}

#[test]
fn shortage_certificate_counts_unique_people_not_candidate_rows() -> Result<()> {
    let state = state(&[3])?; let context = context(&state)?;
    let result = plan(&state, &context, &[demand("a", "CARPENTRY"), demand("b", "MINING")], 1_000_000)?;
    assert_eq!(result.analysis.candidates.iter().map(Vec::len).sum::<usize>(), 2);
    assert_eq!(result.allocation.allocated_units, 1);
    let shortage = result.allocation.shortage.ok_or_else(|| invariant("shortage absent"))?;
    assert_eq!((shortage.required_units, shortage.eligible_units, shortage.deficit), (2,1,1));
    Ok(())
}

fn oracle(masks: &[u8], worker: usize, used: u8) -> u64 {
    if worker == masks.len() { return 0; }
    let mut best = oracle(masks, worker + 1, used);
    for demand in 0..3 {
        let bit = 1 << demand;
        if masks[worker] & bit != 0 && used & bit == 0 {
            best = best.max(1 + oracle(masks, worker + 1, used | bit));
        }
    }
    best
}

#[test]
fn all_512_three_worker_skill_graphs_match_independent_exhaustive_assignments() -> Result<()> {
    let demands = [demand("a", "CARPENTRY"), demand("b", "MINING"), demand("c", "MASONRY")];
    for mask in 0..512u16 {
        let masks = [(mask & 7) as u8, ((mask >> 3) & 7) as u8, ((mask >> 6) & 7) as u8];
        let state = state(&masks)?;
        let result = plan(&state, &context(&state)?, &demands, 1_000_000)?;
        let expected = oracle(&masks, 0, 0);
        assert_eq!(result.allocation.allocated_units, expected, "graph={mask}");
        assert_eq!(result.allocation.cut_capacity, expected);
        let mut people = BTreeSet::new();
        for assignment in &result.allocation.assignments {
            assert!(people.insert(assignment.supply_id)); assert_eq!(assignment.units, 1);
            assert!(result.analysis.candidates[assignment.demand_index].iter().any(|c| c.entity_id == assignment.supply_id));
        }
        if let Some(shortage) = result.allocation.shortage {
            let neighbors: BTreeSet<_> = shortage.demand_indices.iter()
                .flat_map(|&i| result.analysis.candidates[i].iter().map(|c| c.entity_id)).collect();
            assert_eq!(shortage.eligible_units, neighbors.len() as u64);
            assert_eq!(shortage.required_units - shortage.eligible_units, 3 - expected);
        }
    }
    Ok(())
}

#[test]
fn multi_worker_demand_uses_each_citizen_once() -> Result<()> {
    let state = state(&[1,1,1])?; let mut request = demand("a", "CARPENTRY"); request.workers = 2;
    let result = plan(&state, &context(&state)?, &[request], 1_000_000)?;
    assert_eq!(result.allocation.allocated_by_demand, [2]);
    assert_eq!(result.allocation.assignments.len(), 2);
    Ok(())
}

#[test]
fn unknown_skill_cannot_recruit_every_novice_at_minimum_zero() -> Result<()> {
    let state = state(&[1,0])?; let context = context(&state)?;
    let mut requested = demand("a", "MISSPELLED"); requested.min_effective_skill = 0;
    let result = analyze(&state, &context, &[requested.clone()], 1_000_000)?;
    assert_eq!(result.skill_key_observed, [false]); assert!(result.candidates[0].is_empty());
    assert_eq!(result.classifications[0].get("skill_key_not_observed"), Some(&2));
    requested.skill_key = "CARPENTRY".into();
    assert_eq!(analyze(&state, &context, &[requested], 1_000_000)?.candidates[0].len(), 2);
    Ok(())
}

#[test]
fn occupied_endpoint_exception_never_bypasses_other_terrain_exclusions() -> Result<()> {
    for case in 0..8 {
        let mut tile = floor(); tile.unit_occupancy = 1;
        let cell = match case {
            0 => Cell::Hidden, 1 => Cell::Unallocated,
            2 => { tile.shape = Shape::Wall; Cell::Visible(tile) },
            3 => { tile.liquid_depth = 1; Cell::Visible(tile) },
            4 => { tile.building_occupancy = 1; Cell::Visible(tile) },
            5 => { tile.walkable_region = 0; Cell::Visible(tile) },
            6 => { tile.shape = Shape::Ramp; Cell::Visible(tile) },
            _ => Cell::Visible(tile),
        };
        let map = MapRegion { region: Region { origin: [0;3], size: [3,1,1] },
            cells: vec![Cell::Visible(floor()), cell, Cell::Visible(floor())] };
        let field = Reachability::compute(&map, &[[0;3]], 1000).map_err(map_error)?;
        assert_eq!(approach(&map, &field, [1,0,0]), if case == 7 { Some(([0;3],1,true)) } else { None }, "case={case}");
        assert_eq!(approach(&map, &field, [-1,0,0]), None);
        assert_eq!(approach(&map, &field, [3,0,0]), None);
    }
    Ok(())
}

#[test]
fn denied_stale_and_exhausted_requests_do_not_return_feasibility_claims() -> Result<()> {
    let state = state(&[3,1])?; let context = context(&state)?;
    let requested = [demand("a", "CARPENTRY")];
    for case in 0..4 {
        let mut context = context.clone();
        let expected = match case {
            0 => { context.grants.clear(); ErrorCode::CapabilityDenied },
            1 => { context.cancellation_requested = true; ErrorCode::CancellationRequested },
            2 => { context.anchor.cursor.sequence += 1; ErrorCode::StaleAnchor },
            _ => { context.budget.max_entities = 1; ErrorCode::BudgetExceeded },
        };
        assert!(matches!(plan(&state, &context, &requested, 1_000_000), Err(e) if e.code == expected));
    }
    assert!(matches!(plan(&state, &context, &requested, 1), Err(e) if e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn request_shape_and_target_bounds_fail_before_allocation() -> Result<()> {
    let state = state(&[1])?; let context = context(&state)?; let request = demand("a", "CARPENTRY");
    assert!(plan(&state, &context, &[], 1_000_000).is_err());
    assert!(plan(&state, &context, &[request.clone(), request.clone()], 1_000_000).is_err());
    for case in 0..5 {
        let mut request = request.clone();
        match case { 0 => request.workers = 0, 1 => request.workers = 129,
            2 => request.target = [3,0,5], 3 => request.min_effective_skill = -1,
            _ => request.skill_key = "x\n".into() }
        assert!(plan(&state, &context, &[request], 1_000_000).is_err());
    }
    Ok(())
}
