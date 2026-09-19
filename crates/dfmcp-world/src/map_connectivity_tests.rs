use super::*;
use crate::map_region::{Region, Tile, MAX_MAP_TILES};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn floor() -> Cell {
    Cell::Visible(Tile { native_tiletype: 1, shape: Shape::Floor, liquid_depth: 0, magma: false,
        traffic: 0, dig_designation: 0, building_occupancy: 0, unit_occupancy: 0,
        walkable_region: 1, temperature_1: 10015, temperature_2: 10015 })
}
fn grid(size: [u32; 3]) -> Result<MapRegion, MapError> {
    let region = Region { origin: [0, 0, 5], size };
    Ok(MapRegion { region, cells: vec![floor(); region.volume()?] })
}
fn groups(graph: &[Vec<usize>], members: &BTreeSet<usize>, removed: Option<usize>,
    edge: Option<[usize; 2]>) -> Vec<BTreeSet<usize>> {
    let mut remaining = members.clone();
    if let Some(removed) = removed { remaining.remove(&removed); }
    let mut out = Vec::new();
    while let Some(root) = remaining.pop_first() {
        let mut found = BTreeSet::from([root]); let mut queue = VecDeque::from([root]);
        while let Some(from) = queue.pop_front() {
            for &to in &graph[from] {
                if edge == Some([from.min(to), from.max(to)]) { continue; }
                if remaining.remove(&to) { found.insert(to); queue.push_back(to); }
            }
        }
        out.push(found);
    }
    out
}
fn flat_graph(map: &MapRegion) -> Result<Vec<Vec<usize>>, MapError> {
    let n = map.cells.len(); let mut graph = vec![Vec::new(); n];
    for (a, row) in graph.iter_mut().enumerate() {
        let p = map.region.position(a).ok_or(MapError::InvalidRegion)?;
        for b in 0..n {
            let q = map.region.position(b).ok_or(MapError::InvalidRegion)?;
            if map.candidate(a) && map.candidate(b) && p[2] == q[2]
                && p[0].abs_diff(q[0]) + p[1].abs_diff(q[1]) == 1 { row.push(b); }
        }
    }
    Ok(graph)
}

#[test]
fn corridor_reports_exact_vertex_and_edge_failure_impact() -> Result<(), MapError> {
    let map = grid([5, 1, 1])?; let before = map.clone();
    let report = analyze(&map, MAX_ROUTE_WORK)?;
    assert_eq!(map, before); assert_eq!(report.components.len(), 1);
    assert_eq!(report.candidate_tiles, 5); assert_eq!(report.edges, 4);
    assert_eq!(report.components[0].representative, 0);
    assert_eq!(report.components[0].min, [0, 0, 5]);
    assert_eq!(report.components[0].max, [4, 0, 5]);
    assert!(report.components[0].touches_region_boundary);
    assert_eq!(report.bottlenecks.iter().map(|b| b.tile).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(report.bottlenecks[1].partition_sizes, vec![2, 2]);
    assert_eq!(report.bottlenecks[1].separated_tile_pairs, 4);
    assert_eq!(report.bridges[0].side_sizes, [1, 4]);
    assert_eq!(report.bridges[3].side_sizes, [4, 1]);
    Ok(())
}

#[test]
fn disconnected_areas_isolated_tiles_and_cycles_are_not_conflated() -> Result<(), MapError> {
    let mut map = grid([7, 1, 1])?; map.cells[2] = Cell::Hidden; map.cells[5] = Cell::Unallocated;
    let report = analyze(&map, MAX_ROUTE_WORK)?;
    assert_eq!(report.components.iter().map(|c| (c.representative, c.tiles)).collect::<Vec<_>>(), vec![(0, 2), (3, 2), (6, 1)]);
    assert_eq!(report.component_of, vec![Some(0), Some(0), None, Some(1), Some(1), None, Some(2)]);
    assert!(report.bottlenecks.is_empty()); assert_eq!(report.bridges.len(), 2);
    let mut ring = grid([3, 3, 1])?; ring.cells[4] = Cell::Hidden;
    let ring = analyze(&ring, MAX_ROUTE_WORK)?;
    assert_eq!(ring.components.len(), 1); assert_eq!(ring.edges, 8);
    assert!(ring.bottlenecks.is_empty()); assert!(ring.bridges.is_empty());
    Ok(())
}

#[test]
fn every_small_map_matches_destructive_vertex_and_edge_oracles() -> Result<(), MapError> {
    for mask in 0..512u32 {
        let mut map = grid([3, 3, 1])?;
        for i in 0..9 { if mask & (1 << i) != 0 { map.cells[i] = Cell::Hidden; } }
        let graph = flat_graph(&map)?;
        let members: BTreeSet<_> = (0..9).filter(|&i| map.candidate(i)).collect();
        let expected = groups(&graph, &members, None, None);
        let report = analyze(&map, MAX_ROUTE_WORK)?;
        assert_eq!(report.components.len(), expected.len(), "mask={mask}");
        let mut cuts = BTreeMap::new(); let mut edges = BTreeMap::new();
        for (ordinal, component) in expected.iter().enumerate() {
            assert_eq!(report.components[ordinal].tiles as usize, component.len());
            assert_eq!(Some(&report.components[ordinal].representative), component.first());
            for &node in component {
                assert_eq!(report.component_of[node], Some(ordinal));
                let mut sizes: Vec<_> = groups(&graph, component, Some(node), None).iter().map(|s| s.len() as u32).collect();
                sizes.sort_unstable_by(|a, b| b.cmp(a));
                if sizes.len() > 1 { cuts.insert(node, sizes); }
                for &other in &graph[node] {
                    if node >= other { continue; }
                    let after = groups(&graph, component, None, Some([node, other]));
                    if after.len() > 1 {
                        let left = after.iter().find(|g| g.contains(&node)).ok_or(MapError::InvalidRegion)?.len() as u32;
                        edges.insert([node, other], [left, component.len() as u32 - left]);
                    }
                }
            }
        }
        assert_eq!(report.bottlenecks.iter().map(|b| (b.tile, b.partition_sizes.clone())).collect::<BTreeMap<_,_>>(), cuts);
        assert_eq!(report.bridges.iter().map(|b| (b.endpoints, b.side_sizes)).collect::<BTreeMap<_,_>>(), edges);
        for &a in &members {
            for &b in &members {
                let route = map.route(map.region.position(a).ok_or(MapError::InvalidRegion)?,
                    map.region.position(b).ok_or(MapError::InvalidRegion)?, MAX_ROUTE_WORK)?;
                assert_eq!(!route.path.is_empty(), report.component_of[a] == report.component_of[b]);
            }
        }
    }
    Ok(())
}

#[test]
fn all_vertical_shape_pairs_match_the_existing_route_policy() -> Result<(), MapError> {
    for lower in 0..=8 {
        for upper in 0..=8 {
            let mut map = grid([1, 1, 2])?;
            if let Cell::Visible(t) = &mut map.cells[0] { t.shape = Shape::from_tag(lower)?; }
            if let Cell::Visible(t) = &mut map.cells[1] { t.shape = Shape::from_tag(upper)?; }
            let report = analyze(&map, MAX_ROUTE_WORK)?;
            let route = map.route([0, 0, 5], [0, 0, 6], MAX_ROUTE_WORK)?;
            assert_eq!(report.edges == 1, !route.path.is_empty());
            assert_eq!(report.bridges.len(), report.edges as usize);
            assert!(report.bottlenecks.is_empty());
        }
    }
    Ok(())
}

#[test]
fn stacked_stair_corridor_has_correct_cuts_in_both_directions() -> Result<(), MapError> {
    let mut map = grid([1, 1, 4])?;
    for cell in &mut map.cells { if let Cell::Visible(t) = cell { t.shape = Shape::StairUpDown; } }
    let report = analyze(&map, MAX_ROUTE_WORK)?;
    assert_eq!(report.components[0].tiles, 4); assert_eq!(report.bottlenecks.len(), 2);
    assert_eq!(report.bridges.iter().map(|b| b.side_sizes).collect::<Vec<_>>(), vec![[1,3], [2,2], [3,1]]);
    Ok(())
}

#[test]
fn excluded_observations_never_become_candidate_connections() -> Result<(), MapError> {
    for case in 0..7 {
        let mut map = grid([3, 1, 1])?;
        match case {
            0 => map.cells[1] = Cell::Hidden,
            1 => map.cells[1] = Cell::Unallocated,
            _ => if let Cell::Visible(t) = &mut map.cells[1] {
                match case { 2 => t.shape = Shape::Ramp, 3 => t.liquid_depth = 1,
                    4 => t.building_occupancy = 1, 5 => t.unit_occupancy = 1, _ => t.walkable_region = 0 }
            },
        }
        assert_ne!(classify(map.cells[1]), Classification::Candidate);
        let report = analyze(&map, MAX_ROUTE_WORK)?;
        assert_eq!(report.components.len(), 2); assert_eq!(report.candidate_tiles, 2);
        assert!(report.bottlenecks.is_empty()); assert!(report.bridges.is_empty());
    }
    Ok(())
}

#[test]
fn maximum_map_and_long_snake_use_bounded_iterative_traversal() -> Result<(), MapError> {
    let map = grid([128, 128, 1])?;
    let report = analyze(&map, MAX_ROUTE_WORK)?;
    assert_eq!(report.candidate_tiles as usize, MAX_MAP_TILES);
    assert_eq!(report.components.len(), 1); assert!(report.bottlenecks.is_empty());
    assert!(report.bridges.is_empty()); assert!(report.work_units < MAX_ROUTE_WORK);
    let mut snake = map;
    for y in 0..128usize { for x in 0..128usize {
        if y % 2 == 1 && x != if y % 4 == 1 { 127 } else { 0 } { snake.cells[y * 128 + x] = Cell::Hidden; }
    } }
    let report = analyze(&snake, MAX_ROUTE_WORK)?;
    assert_eq!(report.components.len(), 1);
    assert_eq!(report.bridges.len() as u32 + 1, report.candidate_tiles);
    assert_eq!(report.bottlenecks.len() as u32 + 2, report.candidate_tiles);
    Ok(())
}

#[test]
fn cooperative_and_work_refusals_never_return_partial_components() -> Result<(), MapError> {
    let map = grid([8, 8, 1])?; let report = analyze(&map, MAX_ROUTE_WORK)?;
    assert_eq!(analyze(&map, report.work_units)?, report);
    for maximum in [0, 1, report.work_units - 1, MAX_ROUTE_WORK + 1] {
        assert_eq!(analyze(&map, maximum), Err(MapError::BudgetExceeded));
    }
    let mut checks = 0usize;
    assert_eq!(analyze_with_check(&map, MAX_ROUTE_WORK, || {
        checks += 1; if checks == 3 { Err(MapError::BudgetExceeded) } else { Ok(()) }
    }), Err(MapError::BudgetExceeded));
    assert_eq!(checks, 3);
    let mut malformed = map.clone(); malformed.cells.pop();
    assert_eq!(analyze(&malformed, MAX_ROUTE_WORK), Err(MapError::InvalidRegion));
    if let Cell::Visible(t) = &mut malformed.cells[0] { t.liquid_depth = 8; }
    malformed.cells.push(floor());
    assert_eq!(analyze(&malformed, MAX_ROUTE_WORK), Err(MapError::InvalidTile));
    Ok(())
}
