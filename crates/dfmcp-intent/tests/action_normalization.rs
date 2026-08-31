#![forbid(unsafe_code)]

use dfmcp_core::{Capability, EntityId, MapCoord, MapCuboid, RiskTier};
use dfmcp_intent::{Action, BuildingKind, DigMode, MaterialSelector, WorkOrderCondition};
use std::collections::BTreeSet;
use std::error::Error;

#[test]
fn test_pause_normalization_and_metadata() {
    let unpause = Action::Pause { paused: false };
    assert_eq!(unpause.normalized(), unpause);
    assert_eq!(unpause.capability(), Capability::ControlClock);
    assert_eq!(unpause.risk(), RiskTier::Reversible);
    assert!(!unpause.naturally_temporal());

    let pause = Action::Pause { paused: true };
    assert_eq!(pause.normalized(), pause);
    assert_eq!(pause.capability(), Capability::ControlClock);
    assert_eq!(pause.risk(), RiskTier::Reversible);
    assert!(!pause.naturally_temporal());
}

#[test]
fn test_designate_dig_normalization_and_metadata() -> Result<(), Box<dyn Error>> {
    let min = MapCoord {
        x: 10,
        y: 20,
        z: 30,
    };
    let max = MapCoord {
        x: 15,
        y: 25,
        z: 30,
    };
    let cuboid_normal = MapCuboid::new(min, max)?;
    let cuboid_inverted = MapCuboid::from_corners(max, min);

    assert_eq!(cuboid_normal, cuboid_inverted);

    let dig1 = Action::DesignateDig {
        area: cuboid_normal,
        mode: DigMode::Mine,
    };
    let dig2 = Action::DesignateDig {
        area: cuboid_inverted,
        mode: DigMode::Mine,
    };

    assert_eq!(dig1.normalized(), dig2.normalized());
    assert_eq!(dig1.capability(), Capability::Designate);
    assert_eq!(dig1.risk(), RiskTier::Guarded);
    assert!(dig1.naturally_temporal());
    assert_eq!(dig1.canonical_bytes(), dig2.canonical_bytes());
    Ok(())
}

#[test]
fn test_build_normalization_and_metadata() -> Result<(), Box<dyn Error>> {
    let loc = MapCoord { x: 5, y: 5, z: 10 };
    let footprint = MapCuboid::new(loc, loc)?;
    let mut req_tokens = BTreeSet::new();
    req_tokens.insert("WOOD".to_string());
    req_tokens.insert("OAK".to_string());

    let build = Action::Build {
        kind: BuildingKind::Workshop("carpenter".to_string()),
        location: loc,
        footprint,
        material: MaterialSelector {
            required_tokens: req_tokens,
            forbidden_tokens: BTreeSet::new(),
            prefer_nearest: true,
            reserve_count: 1,
        },
    };

    assert_eq!(build.normalized(), build);
    assert_eq!(build.capability(), Capability::Construct);
    assert_eq!(build.risk(), RiskTier::Guarded);
    assert!(build.naturally_temporal());
    Ok(())
}

#[test]
fn test_set_labor_normalization_and_metadata() -> Result<(), Box<dyn Error>> {
    let unnormalized = Action::SetLabor {
        units: vec![
            EntityId::new(42),
            EntityId::new(10),
            EntityId::new(42),
            EntityId::new(5),
        ],
        labor: "MINE".to_string(),
        enabled: true,
    };

    let normalized = unnormalized.normalized();
    match normalized {
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => {
            assert_eq!(labor, "MINE");
            assert!(enabled);
            assert_eq!(
                units,
                vec![EntityId::new(5), EntityId::new(10), EntityId::new(42)]
            );
        }
        _ => return Err("unexpected action variant".into()),
    }
    assert_eq!(unnormalized.capability(), Capability::ConfigureLabor);
    assert_eq!(unnormalized.risk(), RiskTier::Reversible);
    assert!(!unnormalized.naturally_temporal());
    Ok(())
}

#[test]
fn test_configure_stockpile_normalization_and_metadata() {
    let mut accepts = BTreeSet::new();
    accepts.insert("BARS".to_string());
    accepts.insert("ORE".to_string());

    let stock = Action::ConfigureStockpile {
        stockpile: EntityId::new(100),
        accepts,
        max_bins: Some(5),
        max_barrels: None,
        max_wheelbarrows: Some(2),
    };

    assert_eq!(stock.normalized(), stock);
    assert_eq!(stock.capability(), Capability::ConfigureLogistics);
    assert_eq!(stock.risk(), RiskTier::Reversible);
    assert!(!stock.naturally_temporal());
}

#[test]
fn test_create_work_order_normalization_and_metadata() {
    let work_order = Action::CreateWorkOrder {
        name: "Make rock mugs".to_string(),
        job_token: "CRAFT_ROCK_MUG".to_string(),
        amount: 10,
        conditions: vec![
            WorkOrderCondition::ItemCountBelow {
                item_token: "MUG".to_string(),
                threshold: 5,
            },
            WorkOrderCondition::MaterialAvailable {
                material_token: "GRANITE".to_string(),
                minimum: 10,
            },
        ],
    };

    assert_eq!(work_order.normalized(), work_order);
    assert_eq!(work_order.capability(), Capability::ConfigureProduction);
    assert_eq!(work_order.risk(), RiskTier::Reversible);
    assert!(work_order.naturally_temporal());
}

#[test]
fn test_squad_and_burrow_normalization_and_metadata() -> Result<(), Box<dyn Error>> {
    let squad_action = Action::AssignSquad {
        units: vec![EntityId::new(99), EntityId::new(12), EntityId::new(99)],
        squad: EntityId::new(1),
    };
    match squad_action.normalized() {
        Action::AssignSquad { units, squad } => {
            assert_eq!(squad, EntityId::new(1));
            assert_eq!(units, vec![EntityId::new(12), EntityId::new(99)]);
        }
        _ => return Err("unexpected action variant".into()),
    }
    assert_eq!(squad_action.capability(), Capability::ConfigureMilitary);
    assert_eq!(squad_action.risk(), RiskTier::Guarded);

    let burrow_action = Action::SetBurrowMembership {
        units: vec![EntityId::new(88), EntityId::new(33), EntityId::new(88)],
        burrow: EntityId::new(5),
        assigned: true,
    };
    match burrow_action.normalized() {
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => {
            assert_eq!(burrow, EntityId::new(5));
            assert!(assigned);
            assert_eq!(units, vec![EntityId::new(33), EntityId::new(88)]);
        }
        _ => return Err("unexpected action variant".into()),
    }
    assert_eq!(burrow_action.capability(), Capability::ConfigureLogistics);
    assert_eq!(burrow_action.risk(), RiskTier::Reversible);
    Ok(())
}

#[test]
fn test_standing_order_and_extension_metadata() {
    let order = Action::SetStandingOrder {
        key: "gather_refuse_outside".to_string(),
        value: "true".to_string(),
    };
    assert_eq!(order.normalized(), order);
    assert_eq!(order.capability(), Capability::ConfigureLogistics);
    assert_eq!(order.risk(), RiskTier::Guarded);

    let ext = Action::Extension {
        namespace: "dfhack.plugins".to_string(),
        name: "autodump".to_string(),
        parameters: std::collections::BTreeMap::new(),
    };
    assert_eq!(ext.normalized(), ext);
    assert_eq!(ext.capability(), Capability::Extension);
    assert_eq!(ext.risk(), RiskTier::Guarded);
}
