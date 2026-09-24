#![forbid(unsafe_code)]

use std::collections::{BTreeSet, VecDeque};

use dfmcp_core::{ErrorCode, MapCoord, MapCuboid, Result};
use dfmcp_intent::DigMode;
use dfmcp_intent::blueprint::{
    BlueprintLayout, BlueprintPlanner, BlueprintTemplate, ExcavationRole,
};

fn origin() -> MapCoord {
    MapCoord {
        x: -7,
        y: -5,
        z: 10,
    }
}

fn tiles(layout: &BlueprintLayout) -> BTreeSet<[i32; 3]> {
    let mut result = BTreeSet::new();
    for part in layout.excavations() {
        for z in part.area.min.z..=part.area.max.z {
            for y in part.area.min.y..=part.area.max.y {
                for x in part.area.min.x..=part.area.max.x {
                    assert!(result.insert([x, y, z]), "overlapping excavation parts");
                }
            }
        }
    }
    assert_eq!(result.len() as u64, layout.tile_count());
    result
}

#[test]
fn moat_matches_exact_perimeter_and_reserved_crossing_exhaustively() -> Result<()> {
    for width in 3_i32..=16 {
        for height in 3_i32..=16 {
            for gap in 0..=(width - 2) as u8 {
                let low = origin();
                let high = MapCoord {
                    x: low.x + width - 1,
                    y: low.y + height - 1,
                    z: low.z,
                };
                let layout = BlueprintPlanner.layout(
                    low,
                    BlueprintTemplate::DefensiveMoat {
                        perimeter_cuboid: MapCuboid::new(low, high)?,
                        drawbridge_span: gap,
                    },
                )?;
                let actual = tiles(&layout);
                let start = low.x + (width - i32::from(gap)) / 2;
                let expected: BTreeSet<_> = (low.y..=high.y)
                    .flat_map(|y| {
                        (low.x..=high.x).filter_map(move |x| {
                            let boundary = x == low.x || x == high.x || y == low.y || y == high.y;
                            let crossing =
                                gap > 0 && y == low.y && x >= start && x < start + i32::from(gap);
                            (boundary && !crossing).then_some([x, y, low.z])
                        })
                    })
                    .collect();
                assert_eq!(actual, expected);
                assert_eq!(layout.excavations().len(), if gap == 0 { 4 } else { 5 });
                assert!(
                    layout
                        .excavations()
                        .iter()
                        .all(|p| p.mode == DigMode::Channel && p.role == ExcavationRole::Perimeter)
                );
                assert_eq!(layout.reserved_crossing().is_some(), gap > 0);
                if let Some(crossing) = layout.reserved_crossing() {
                    assert_eq!(crossing.min.x, start);
                    assert_eq!(crossing.max.x, start + i32::from(gap) - 1);
                    assert_eq!(crossing.min.y, low.y);
                    assert_eq!(crossing.max.y, low.y);
                }
            }
        }
    }
    Ok(())
}

fn assert_connected(layout: &BlueprintLayout) {
    let expected = tiles(layout);
    let entrance = match layout.access_point() {
        Some(point) => [point.x, point.y, point.z],
        None => panic!("connected layout has no corridor endpoint"),
    };
    assert!(expected.contains(&entrance));
    let mut seen = BTreeSet::from([entrance]);
    let mut queue = VecDeque::from([entrance]);
    while let Some([x, y, z]) = queue.pop_front() {
        for next in [[x - 1, y, z], [x + 1, y, z], [x, y - 1, z], [x, y + 1, z]] {
            if expected.contains(&next) && seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    assert_eq!(
        seen, expected,
        "an excavated room cannot reach the entrance"
    );
}

#[test]
fn all_bedroom_counts_have_disjoint_connected_rooms_doors_and_corridors() -> Result<()> {
    for count in 1..=24 {
        for width in 1..=7 {
            for height in 1..=7 {
                let template = BlueprintTemplate::BedroomCluster {
                    rooms_count: count,
                    room_size: (width, height),
                };
                let layout = BlueprintPlanner.layout(origin(), template.clone())?;
                assert_eq!(layout, BlueprintPlanner.layout(origin(), template)?);
                assert_connected(&layout);
                assert_eq!(
                    layout
                        .excavations()
                        .iter()
                        .filter(|p| p.role == ExcavationRole::Room)
                        .count(),
                    count as usize
                );
                assert_eq!(
                    layout
                        .excavations()
                        .iter()
                        .filter(|p| p.role == ExcavationRole::Doorway)
                        .count(),
                    count as usize
                );
                assert!(layout.excavations().len() <= 64);
                let set = tiles(&layout);
                // Every room has one southern doorway; other southern wall tiles survive.
                for part in layout
                    .excavations()
                    .iter()
                    .filter(|p| p.role == ExcavationRole::Room)
                {
                    let doorway_x = part.area.min.x + (i32::from(width) - 1) / 2;
                    for x in part.area.min.x..=part.area.max.x {
                        assert_eq!(
                            set.contains(&[x, part.area.max.y + 1, part.area.min.z]),
                            x == doorway_x
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn workshop_bays_share_a_corridor_without_erasing_separating_walls() -> Result<()> {
    for count in 1..=24 {
        let layout = BlueprintPlanner.layout(
            origin(),
            BlueprintTemplate::WorkshopHub { bays_count: count },
        )?;
        assert_connected(&layout);
        let set = tiles(&layout);
        for boundary in 1..count {
            let x = origin().x + boundary as i32 * 6 - 1;
            for y in origin().y..origin().y + 5 {
                assert!(!set.contains(&[x, y, origin().z]));
            }
        }
    }
    Ok(())
}

#[test]
fn invalid_and_extreme_geometry_is_refused_without_expansion() -> Result<()> {
    for count in [0, 25, u32::MAX] {
        assert_eq!(
            BlueprintPlanner
                .layout(
                    origin(),
                    BlueprintTemplate::WorkshopHub { bays_count: count }
                )
                .expect_err("unbounded bays accepted")
                .code,
            ErrorCode::BudgetExceeded
        );
        assert!(
            BlueprintPlanner
                .layout(
                    origin(),
                    BlueprintTemplate::BedroomCluster {
                        rooms_count: count,
                        room_size: (3, 3)
                    }
                )
                .is_err()
        );
    }
    for span in [(2, 3, 0), (3, 2, 0), (5, 5, 4)] {
        assert!(
            BlueprintPlanner
                .layout(
                    origin(),
                    BlueprintTemplate::DefensiveMoat {
                        perimeter_cuboid: MapCuboid::new(
                            origin(),
                            MapCoord {
                                x: origin().x + span.0 - 1,
                                y: origin().y + span.1 - 1,
                                z: origin().z
                            }
                        )?,
                        drawbridge_span: span.2,
                    }
                )
                .is_err()
        );
    }
    assert!(
        BlueprintPlanner
            .layout(
                origin(),
                BlueprintTemplate::DefensiveMoat {
                    perimeter_cuboid: MapCuboid::new(
                        origin(),
                        MapCoord {
                            x: 10,
                            y: 10,
                            z: 11
                        }
                    )?,
                    drawbridge_span: 0,
                }
            )
            .is_err()
    );
    assert!(
        BlueprintPlanner
            .layout(
                origin(),
                BlueprintTemplate::DefensiveMoat {
                    perimeter_cuboid: MapCuboid::new(
                        MapCoord {
                            x: i32::MIN,
                            y: 0,
                            z: 0
                        },
                        MapCoord {
                            x: i32::MAX,
                            y: 2,
                            z: 0
                        }
                    )?,
                    drawbridge_span: 0,
                }
            )
            .is_err()
    );
    assert!(
        BlueprintPlanner
            .layout(
                MapCoord {
                    x: i32::MIN,
                    ..origin()
                },
                BlueprintTemplate::BedroomCluster {
                    rooms_count: 1,
                    room_size: (3, 3)
                }
            )
            .is_err()
    );
    assert!(
        BlueprintPlanner
            .layout(
                MapCoord {
                    x: i32::MAX,
                    ..origin()
                },
                BlueprintTemplate::DiningHall {
                    width: 2,
                    height: 2
                }
            )
            .is_err()
    );
    assert!(
        BlueprintPlanner
            .layout(
                origin(),
                BlueprintTemplate::DiningHall {
                    width: 255,
                    height: 255
                }
            )
            .is_err()
    );
    assert!(
        BlueprintPlanner
            .layout(
                origin(),
                BlueprintTemplate::StockpileVault {
                    width: 1,
                    height: 1,
                    category: "x".repeat(129)
                }
            )
            .is_err()
    );
    Ok(())
}
