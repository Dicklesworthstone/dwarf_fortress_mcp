//! GA-FLOW / df-franken-storage-mvcc-54h.4: actual Rust model and independent cuts.
use std::collections::{BTreeSet, VecDeque};

use dfmcp_world::map_region::{Cell, MAX_ROUTE_WORK, MapError, MapRegion, Region, Shape, Tile};
use dfmcp_world::map_resilience::{
    CutElement, Disjointness, analyze, analyze_with_check, verify_certificate,
};

const MODES: [Disjointness; 2] = [Disjointness::Edges, Disjointness::InternalTiles];

fn floor() -> Cell {
    Cell::Visible(Tile {
        native_tiletype: 1,
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
    })
}
fn grid(size: [u32; 3]) -> Result<MapRegion, MapError> {
    let region = Region { origin: [0; 3], size };
    Ok(MapRegion { region, cells: vec![floor(); region.volume()?] })
}
fn pos(map: &MapRegion, i: usize) -> Result<[u32; 3], MapError> {
    map.region.position(i).ok_or(MapError::InvalidRegion)
}
fn run(map: &MapRegion, mode: Disjointness) -> Result<dfmcp_world::map_resilience::RouteResilience, MapError> {
    analyze(map, pos(map, 0)?, pos(map, map.cells.len() - 1)?, mode, MAX_ROUTE_WORK)
}

// An independent edge oracle uses the EXISTING public shortest-route policy.
// A shortest path of exactly two positions is a direct model edge. Only used
// for tiny fixtures: this is deliberately not an efficient graph constructor.
fn adjacency(map: &MapRegion) -> Result<Vec<Vec<usize>>, MapError> {
    let mut out = vec![Vec::new(); map.cells.len()];
    for (a, neighbors) in out.iter_mut().enumerate() {
        for b in 0..map.cells.len() {
            if a != b && map.route(pos(map, a)?, pos(map, b)?, MAX_ROUTE_WORK)?.path.len() == 2 {
                neighbors.push(b);
            }
        }
    }
    Ok(out)
}
fn reaches(adj: &[Vec<usize>], source: usize, target: usize, cut: &BTreeSet<CutElement>) -> bool {
    let mut seen = BTreeSet::from([source]);
    let mut pending = VecDeque::from([source]);
    while let Some(a) = pending.pop_front() {
        if a == target {
            return true;
        }
        for &b in &adj[a] {
            if !cut.contains(&CutElement::Tile(b))
                && !cut.contains(&CutElement::Edge([a.min(b), a.max(b)]))
                && seen.insert(b)
            {
                pending.push_back(b);
            }
        }
    }
    false
}
fn minimum_cut(adj: &[Vec<usize>], source: usize, target: usize, mode: Disjointness) -> usize {
    let mut elements = Vec::new();
    if mode == Disjointness::Edges {
        for (a, neighbors) in adj.iter().enumerate() {
            for &b in neighbors {
                if a < b {
                    elements.push(CutElement::Edge([a, b]));
                }
            }
        }
    } else {
        elements.extend((0..adj.len()).filter(|&i| i != source && i != target).map(CutElement::Tile));
        if adj[source].contains(&target) {
            elements.push(CutElement::Edge([source.min(target), source.max(target)]));
        }
    }
    let mut best = adj[source].len();
    for mask in 0..(1usize << elements.len()) {
        let count = mask.count_ones() as usize;
        if count > best {
            continue;
        }
        let cut = elements.iter().enumerate().filter(|(i, _)| mask & (1usize << *i) != 0)
            .map(|(_, &element)| element).collect();
        if !reaches(adj, source, target, &cut) {
            best = count;
        }
    }
    best
}

#[test]
fn corridor_has_one_route_and_a_checked_minimum_separator() -> Result<(), MapError> {
    let map = grid([5, 1, 1])?;
    for mode in MODES {
        let report = run(&map, mode)?;
        assert!(!report.endpoint_excluded);
        assert_eq!(report.paths, vec![vec![0, 1, 2, 3, 4]]);
        assert_eq!(report.candidate_tiles, 5);
        assert_eq!(report.candidate_edges, 4);
        assert_eq!(report.minimum_cut, vec![if mode == Disjointness::Edges {
            CutElement::Edge([0, 1])
        } else { CutElement::Tile(1) }]);
        verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
    }
    Ok(())
}

#[test]
fn two_edge_routes_may_still_share_one_failure_critical_tile() -> Result<(), MapError> {
    let mut map = grid([3, 3, 1])?;
    map.cells[2] = Cell::Hidden;
    map.cells[6] = Cell::Hidden;
    let edges = run(&map, Disjointness::Edges)?;
    let tiles = run(&map, Disjointness::InternalTiles)?;
    assert_eq!(edges.paths.len(), 2);
    assert!(edges.paths.iter().all(|path| path.contains(&4)));
    assert_eq!(tiles.paths.len(), 1);
    assert_eq!(tiles.minimum_cut, vec![CutElement::Tile(4)]);
    let mut forged = edges;
    forged.disjointness = Disjointness::InternalTiles;
    assert!(verify_certificate(&map, &forged, MAX_ROUTE_WORK).is_err());
    Ok(())
}

#[test]
fn adjacent_endpoints_do_not_invent_an_internal_tile_separator() -> Result<(), MapError> {
    let map = grid([2, 2, 1])?;
    let report = analyze(&map, [0, 0, 0], [1, 0, 0], Disjointness::InternalTiles, MAX_ROUTE_WORK)?;
    assert_eq!(report.paths.len(), 2);
    assert_eq!(report.minimum_cut, vec![CutElement::Tile(2), CutElement::Edge([0, 1])]);
    verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
    Ok(())
}

#[test]
fn excluded_endpoints_are_not_certified_zero_redundancy() -> Result<(), MapError> {
    for replacement in [Cell::Hidden, Cell::Unallocated] {
        let mut map = grid([3, 1, 1])?;
        map.cells[0] = replacement;
        for mode in MODES {
            let report = run(&map, mode)?;
            assert!(report.endpoint_excluded);
            assert!(report.paths.is_empty() && report.minimum_cut.is_empty());
            verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
        }
    }
    Ok(())
}

#[test]
fn disconnected_candidate_endpoints_have_a_zero_cut_only_in_this_model() -> Result<(), MapError> {
    let mut map = grid([3, 1, 1])?;
    map.cells[1] = Cell::Hidden;
    for mode in MODES {
        let report = run(&map, mode)?;
        assert!(!report.endpoint_excluded);
        assert!(report.paths.is_empty() && report.minimum_cut.is_empty());
        verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
    }
    Ok(())
}

#[test]
fn occupancy_liquid_unknown_walkability_and_unsupported_shapes_match_route_policy() -> Result<(), MapError> {
    for which in 0..6 {
        let mut map = grid([3, 1, 1])?;
        if let Cell::Visible(tile) = &mut map.cells[1] {
            match which {
                0 => tile.liquid_depth = 1,
                1 => tile.building_occupancy = 1,
                2 => tile.unit_occupancy = 2,
                3 => tile.walkable_region = 0,
                4 => tile.shape = Shape::Ramp,
                _ => tile.shape = Shape::Other,
            }
        }
        for mode in MODES {
            assert!(run(&map, mode)?.paths.is_empty());
        }
        assert!(map.route([0, 0, 0], [2, 0, 0], MAX_ROUTE_WORK)?.path.is_empty());
    }
    Ok(())
}

#[test]
fn vertical_edges_require_complementary_observed_stairs_in_both_directions() -> Result<(), MapError> {
    let mut map = grid([1, 1, 2])?;
    for phase in 0..3 {
        if phase == 1 {
            if let Cell::Visible(tile) = &mut map.cells[0] { tile.shape = Shape::StairUp; }
        } else if phase == 2 {
            if let Cell::Visible(tile) = &mut map.cells[1] { tile.shape = Shape::StairDown; }
        }
        for (source, target) in [([0, 0, 0], [0, 0, 1]), ([0, 0, 1], [0, 0, 0])] {
            for mode in MODES {
                let report = analyze(&map, source, target, mode, MAX_ROUTE_WORK)?;
                assert_eq!(report.paths.len(), usize::from(phase == 2));
                verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
            }
        }
    }
    Ok(())
}

#[test]
fn every_three_by_three_obstacle_map_matches_independently_enumerated_cuts() -> Result<(), MapError> {
    for mask in 0..512u32 {
        let mut map = grid([3, 3, 1])?;
        for (i, cell) in map.cells.iter_mut().enumerate() {
            if mask & (1 << i) != 0 { *cell = Cell::Hidden; }
        }
        let excluded = !map.candidate(0) || !map.candidate(8);
        let adj = if excluded { Vec::new() } else { adjacency(&map)? };
        for mode in MODES {
            let report = run(&map, mode)?;
            assert_eq!(report.endpoint_excluded, excluded);
            if !excluded {
                assert_eq!(report.paths.len(), minimum_cut(&adj, 0, 8, mode), "mask={mask} mode={mode:?}");
                assert_eq!(report.paths.is_empty(), map.route([0, 0, 0], [2, 2, 0], MAX_ROUTE_WORK)?.path.is_empty());
            }
            verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
        }
    }
    Ok(())
}

#[test]
fn exact_work_boundary_is_inclusive_and_exhaustion_never_returns_a_partial_cut() -> Result<(), MapError> {
    let map = grid([5, 5, 1])?;
    for mode in MODES {
        let report = run(&map, mode)?;
        assert_eq!(analyze(&map, report.start, report.goal, mode, report.work_units)?, report);
        assert_eq!(analyze(&map, report.start, report.goal, mode, report.work_units - 1), Err(MapError::BudgetExceeded));
        assert_eq!(analyze(&map, report.start, report.goal, mode, 0), Err(MapError::BudgetExceeded));
        assert_eq!(analyze(&map, report.start, report.goal, mode, MAX_ROUTE_WORK + 1), Err(MapError::BudgetExceeded));
    }
    Ok(())
}

#[test]
fn every_cooperative_abort_including_final_publication_returns_no_result() -> Result<(), MapError> {
    let map = grid([5, 5, 1])?;
    for mode in MODES {
        let mut calls = 0;
        analyze_with_check(&map, [0, 0, 0], [4, 4, 0], mode, MAX_ROUTE_WORK, || {
            calls += 1;
            Ok(())
        })?;
        for reject_at in 1..=calls {
            let mut call = 0;
            assert_eq!(analyze_with_check(&map, [0, 0, 0], [4, 4, 0], mode, MAX_ROUTE_WORK, || {
                call += 1;
                if call == reject_at { Err(MapError::BudgetExceeded) } else { Ok(()) }
            }), Err(MapError::BudgetExceeded));
        }
    }
    Ok(())
}

#[test]
fn malformed_maps_and_endpoints_are_refused() -> Result<(), MapError> {
    let mut map = grid([3, 1, 1])?;
    assert!(analyze(&map, [0; 3], [0; 3], Disjointness::Edges, MAX_ROUTE_WORK).is_err());
    assert_eq!(analyze(&map, [0; 3], [3, 0, 0], Disjointness::Edges, MAX_ROUTE_WORK), Err(MapError::OutsideRegion));
    map.cells.pop();
    assert!(analyze(&map, [0; 3], [2, 0, 0], Disjointness::Edges, MAX_ROUTE_WORK).is_err());
    let mut map = grid([3, 1, 1])?;
    if let Cell::Visible(tile) = &mut map.cells[1] { tile.liquid_depth = 8; }
    assert_eq!(run(&map, Disjointness::Edges), Err(MapError::InvalidTile));
    Ok(())
}

#[test]
fn certificate_rejects_forged_packings_cuts_counts_and_changed_maps() -> Result<(), MapError> {
    let map = grid([3, 3, 1])?;
    let report = run(&map, Disjointness::InternalTiles)?;
    for which in 0..7 {
        let mut forged = report.clone();
        match which {
            0 => forged.paths[0][1] = 8,
            1 => forged.paths[1] = forged.paths[0].clone(),
            2 => forged.minimum_cut[0] = CutElement::Tile(0),
            3 => forged.minimum_cut[0] = CutElement::Tile(8),
            4 => forged.candidate_tiles += 1,
            5 => forged.paths[0] = vec![0; map.cells.len() + 1],
            _ => forged.minimum_cut.clear(),
        }
        assert!(verify_certificate(&map, &forged, MAX_ROUTE_WORK).is_err(), "mutation {which}");
    }
    let mut changed = map;
    changed.cells[report.paths[0][1]] = Cell::Hidden;
    assert!(verify_certificate(&changed, &report, MAX_ROUTE_WORK).is_err());
    Ok(())
}

#[test]
fn maximum_size_snaking_corridor_is_iterative_and_remains_budgeted() -> Result<(), MapError> {
    let mut map = grid([128, 128, 1])?;
    map.cells.fill(Cell::Hidden);
    for y in (0usize..128).step_by(2) {
        for x in 0..128 { map.cells[y * 128 + x] = floor(); }
        if y < 126 {
            let x = if (y / 2) % 2 == 0 { 127 } else { 0 };
            map.cells[(y + 1) * 128 + x] = floor();
        }
    }
    for mode in MODES {
        let report = analyze(&map, [0, 0, 0], [0, 126, 0], mode, MAX_ROUTE_WORK)?;
        assert_eq!(report.paths.len(), 1);
        assert_eq!(report.paths[0].len(), 8255);
        assert!(report.work_units <= MAX_ROUTE_WORK);
        verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
    }
    Ok(())
}

#[test]
fn six_independent_stair_lattice_routes_reach_the_endpoint_degree_bound() -> Result<(), MapError> {
    let mut map = grid([4, 4, 4])?;
    for cell in &mut map.cells {
        if let Cell::Visible(tile) = cell { tile.shape = Shape::StairUpDown; }
    }
    for mode in MODES {
        let report = analyze(&map, [1, 1, 1], [2, 2, 2], mode, MAX_ROUTE_WORK)?;
        assert_eq!(report.paths.len(), 6);
        assert_eq!(report.minimum_cut.len(), 6);
        verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
        assert_eq!(analyze(&map, report.start, report.goal, mode, MAX_ROUTE_WORK)?, report);
    }
    Ok(())
}

#[test]
fn residual_rerouting_finds_a_second_corridor_that_greedy_path_deletion_loses() -> Result<(), MapError> {
    let mut map = grid([5, 5, 1])?;
    for i in [9, 12, 13, 15, 19, 20, 21, 22] { map.cells[i] = Cell::Hidden; }
    let start = pos(&map, 11)?;
    let goal = pos(&map, 3)?;
    let first = map.route(start, goal, MAX_ROUTE_WORK)?;
    let mut greedy = map.clone();
    for &position in &first.path[1..first.path.len() - 1] {
        let i = map.region.index(position).ok_or(MapError::InvalidRegion)?;
        greedy.cells[i] = Cell::Hidden;
    }
    assert!(greedy.route(start, goal, MAX_ROUTE_WORK)?.path.is_empty());
    let report = analyze(&map, start, goal, Disjointness::InternalTiles, MAX_ROUTE_WORK)?;
    assert_eq!(report.paths.len(), 2);
    verify_certificate(&map, &report, MAX_ROUTE_WORK)?;
    Ok(())
}
