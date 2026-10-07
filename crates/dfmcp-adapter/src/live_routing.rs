//! Translation of sealed semantic plans onto the live DFHack mutation families.
//!
//! The laboratory executes every semantic action against the reference model;
//! the live bridge exposes narrow, separately versioned development families
//! (control/1.7 pause, dig/1.16, build/1.19 furniture, work-orders/1.10,
//! workforce/1.17). This module is the single, pure, deterministic seam
//! between the two: for every step of a [`PreparedPlan`] it yields either the
//! bounded advisory request or an explicit refusal naming what the live surface
//! lacks. Canonical identities and semantic constraints survive translation.
//!
//! Routing is not authority. A route grants nothing, performs no I/O and never
//! weakens a family's own preconditions, journals or confirmation; every
//! family remains unadmitted development execution until the evidence chain in
//! `IMPLEMENTATION_STATUS.md` exists. Values that only a fresh live read can
//! supply (a concrete furniture item, a work-detail index) are left as named
//! resolution requirements rather than guessed.
//!
//! [`LiveRoutingEvidence`] binds resolution to an exact source-qualified canonical
//! snapshot and its typed live projection. Resolution produces a native plan for
//! review, never permission to prepare or dispatch it. Bead:
//! `df-dfhack-bridge-plane-c-pic.4`.

use std::collections::BTreeSet;

use dfmcp_core::{EntityId, MapCoord, MapCuboid, Result, StepId};
use dfmcp_intent::{Action, BuildingKind, DigMode, MaterialSelector, PlanStep, PreparedPlan};

use crate::build_placement::{BuildKind, BuildSelection};
use crate::dig_designation::DigRegion;
use crate::work_orders::{WorkOrderRecipe, WorkOrderSpec};

#[path = "live_routing_resolution.rs"]
mod resolution;
pub use resolution::{
    LiveIdentitySchema, LiveRoutingEvidence, NativeUnitIdentity, ResolvedFurnitureRoute,
    ResolvedWorkforceRoute, resolve_furniture_step, resolve_workforce_step,
};

/// Largest dig/1.16 rectangle edge.
pub const MAX_DIG_EDGE: i32 = 8;
/// Most dig/1.16 regions one semantic excavation may be tiled into.
pub const MAX_DIG_REGIONS_PER_STEP: usize = 64;
/// Most units one workforce/1.17 observation covers.
pub const MAX_WORKFORCE_UNITS: usize = 32;

/// One live development family and its exact protocol generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LiveFamily {
    ControlV1_7,
    DigV1_16,
    BuildV1_19,
    WorkOrdersV1_10,
    WorkforceV1_17,
}

impl LiveFamily {
    #[must_use]
    pub const fn protocol(self) -> &'static str {
        match self {
            Self::ControlV1_7 => "control/1.7",
            Self::DigV1_16 => "dig/1.16",
            Self::BuildV1_19 => "build/1.19",
            Self::WorkOrdersV1_10 => "work-orders/1.10",
            Self::WorkforceV1_17 => "workforce/1.17",
        }
    }

    /// The unadmitted development MCP server that executes this family.
    #[must_use]
    pub const fn dev_server(self) -> &'static str {
        match self {
            Self::ControlV1_7 => "dfmcp-live-control-dev-server",
            Self::DigV1_16 => "dfmcp-dig-control-dev-server",
            Self::BuildV1_19 => "dfmcp-build-placement-dev-server",
            Self::WorkOrdersV1_10 => "dfmcp-live-work-orders-dev-server",
            Self::WorkforceV1_17 => "dfmcp-live-workforce-dev-server",
        }
    }
}

/// A value the live family needs that only a fresh live read can supply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveResolution {
    /// An exact eligible item and evidence for every retained material rule.
    /// The current native capture cannot resolve token or allocation policies.
    FurnitureItem {
        kind: BuildKind,
        material: MaterialSelector,
    },
    /// An exact canonical-to-native identity resolution and a selected-only
    /// detail containing only this labor, with no overlapping permission on
    /// removal. Broad work-detail membership is not a single-labor action.
    WorkDetailForLabor { labor: String },
}

/// An advisory request, not a native wire request or dispatch permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveRequest {
    Pause {
        paused: bool,
    },
    /// Mining designation, tiled into dig/1.16 rectangles in z, y, x order.
    Dig {
        regions: Vec<DigRegion>,
    },
    Furniture {
        kind: BuildKind,
        target: [u32; 3],
        material: MaterialSelector,
    },
    WorkOrder {
        spec: WorkOrderSpec,
    },
    WorkDetail {
        /// Original canonical entities. These must never be cast to native IDs.
        units: Vec<EntityId>,
        labor: String,
        assigned: bool,
    },
}

/// Why a step has no live route today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveRefusal {
    pub reason: String,
}

/// The route for one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRoute {
    pub step: StepId,
    pub idempotency_key: String,
    pub outcome: std::result::Result<RoutedStep, LiveRefusal>,
}

/// A routable step: family, typed request, and live values still to resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedStep {
    pub family: LiveFamily,
    pub request: LiveRequest,
    pub requires: Vec<LiveResolution>,
    /// Live preconditions the family enforces beyond the semantic plan.
    pub live_preconditions: Vec<&'static str>,
}

/// The route of a whole sealed plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRoute {
    pub plan_digest: dfmcp_core::Digest32,
    pub steps: Vec<StepRoute>,
}

impl PlanRoute {
    /// Whether every step has an advisory family route. This does not establish
    /// that resolution, authority, compatibility, or native preparation can pass.
    #[must_use]
    pub fn fully_routable(&self) -> bool {
        self.steps.iter().all(|step| step.outcome.is_ok())
    }

    /// The distinct live families the routable steps need.
    #[must_use]
    pub fn families(&self) -> BTreeSet<LiveFamily> {
        self.steps
            .iter()
            .filter_map(|step| step.outcome.as_ref().ok().map(|routed| routed.family))
            .collect()
    }
}

fn refuse(reason: impl Into<String>) -> std::result::Result<RoutedStep, LiveRefusal> {
    Err(LiveRefusal {
        reason: reason.into(),
    })
}

fn coord_u32(value: i32, what: &str) -> std::result::Result<u32, LiveRefusal> {
    u32::try_from(value).map_err(|_| LiveRefusal {
        reason: format!("{what} coordinate {value} is negative; live coordinates are unsigned"),
    })
}

/// Tile `area` into dig/1.16 rectangles: one z level at a time, at most
/// 8x8, in canonical z, y, x order. The union is exactly `area`.
pub fn tile_dig_area(area: MapCuboid) -> std::result::Result<Vec<DigRegion>, LiveRefusal> {
    // Check the whole native geometry before subtracting signed coordinates or
    // iterating. Public MapCuboid fields can bypass its constructor.
    if area.min.x > area.max.x
        || area.min.y > area.max.y
        || area.min.z > area.max.z
        || !(1..=32766).contains(&area.min.x)
        || !(1..=32766).contains(&area.max.x)
        || !(1..=32766).contains(&area.min.y)
        || !(1..=32766).contains(&area.max.y)
        || !(1..=32766).contains(&area.min.z)
        || !(1..=32766).contains(&area.max.z)
    {
        return Err(LiveRefusal {
            reason: "dig/1.16 needs ordered native coordinates and a complete halo".to_owned(),
        });
    }
    let width = u64::from((area.max.x - area.min.x + 1) as u32);
    let height = u64::from((area.max.y - area.min.y + 1) as u32);
    let levels = u64::from((area.max.z - area.min.z + 1) as u32);
    let count = width.div_ceil(MAX_DIG_EDGE as u64) * height.div_ceil(MAX_DIG_EDGE as u64) * levels;
    if count > MAX_DIG_REGIONS_PER_STEP as u64 {
        return Err(LiveRefusal {
            reason: format!(
                "excavation needs more than {MAX_DIG_REGIONS_PER_STEP} dig/1.16 rectangles; split it into smaller steps"
            ),
        });
    }
    let mut regions = Vec::new();
    for z in area.min.z..=area.max.z {
        let mut y = area.min.y;
        while y <= area.max.y {
            let height = (area.max.y - y + 1).min(MAX_DIG_EDGE);
            let mut x = area.min.x;
            while x <= area.max.x {
                let width = (area.max.x - x + 1).min(MAX_DIG_EDGE);
                if regions.len() == MAX_DIG_REGIONS_PER_STEP {
                    return Err(LiveRefusal {
                        reason: format!(
                            "excavation needs more than {MAX_DIG_REGIONS_PER_STEP} dig/1.16 rectangles; split it into smaller steps"
                        ),
                    });
                }
                let region = DigRegion::new(
                    coord_u32(x, "x")?,
                    coord_u32(y, "y")?,
                    coord_u32(z, "z")?,
                    coord_u32(width, "width")?,
                    coord_u32(height, "height")?,
                )
                .map_err(|error| LiveRefusal {
                    reason: format!(
                        "dig/1.16 cannot designate the rectangle at ({x},{y},{z}): {}",
                        error.message
                    ),
                })?;
                regions.push(region);
                x += width;
            }
            y += height;
        }
    }
    Ok(regions)
}

fn furniture_kind(name: &str) -> Option<BuildKind> {
    match name {
        "Bed" | "bed" => Some(BuildKind::Bed),
        "Chair" | "chair" | "Throne" | "throne" => Some(BuildKind::Chair),
        "Table" | "table" => Some(BuildKind::Table),
        _ => None,
    }
}

/// The closed work-orders/1.10 recipe for a semantic job token.
#[must_use]
pub fn work_order_recipe(job_token: &str) -> Option<WorkOrderRecipe> {
    match job_token {
        "CONSTRUCT_BED" | "wooden_bed" => Some(WorkOrderRecipe::WoodenBed),
        "CONSTRUCT_DOOR" | "wooden_door" => Some(WorkOrderRecipe::WoodenDoor),
        "CONSTRUCT_TABLE" | "wooden_table" => Some(WorkOrderRecipe::WoodenTable),
        "CONSTRUCT_THRONE" | "CONSTRUCT_CHAIR" | "wooden_chair" => {
            Some(WorkOrderRecipe::WoodenChair)
        }
        _ => None,
    }
}

fn unit_ids(units: &[EntityId]) -> std::result::Result<Vec<EntityId>, LiveRefusal> {
    if units.is_empty() || units.len() > MAX_WORKFORCE_UNITS || units.contains(&EntityId::NIL) {
        return Err(LiveRefusal {
            reason: format!(
                "workforce/1.17 needs 1..={MAX_WORKFORCE_UNITS} nonzero canonical unit identities"
            ),
        });
    }
    let mut ids = units.to_vec();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn single_tile(footprint: MapCuboid, location: MapCoord) -> bool {
    footprint.min == footprint.max && footprint.min == location
}

fn route_shape(action: &Action) -> std::result::Result<(), LiveRefusal> {
    let valid_token = |token: &str, maximum: usize| {
        !token.is_empty() && token.len() <= maximum && !token.chars().any(char::is_control)
    };
    match action {
        Action::SetLabor { labor, .. } if !valid_token(labor, 64) => Err(LiveRefusal {
            reason: "workforce/1.17 labor keys require 1..=64 non-control UTF-8 bytes".to_owned(),
        }),
        Action::Build { material, .. }
            if material.required_tokens.len() > 64
                || material.forbidden_tokens.len() > 64
                || material.required_tokens.iter().chain(&material.forbidden_tokens)
                    .any(|token| !valid_token(token, 128))
                || !material.required_tokens.is_disjoint(&material.forbidden_tokens) =>
        {
            Err(LiveRefusal {
                reason: "material selectors require disjoint bounded token sets (at most 64 per set, 128 bytes per token)".to_owned(),
            })
        }
        _ => Ok(()),
    }
}

/// Route one sealed step onto its live family, or explain why it cannot be.
#[must_use]
pub fn route_step(step: &PlanStep) -> StepRoute {
    if let Err(refusal) = route_shape(&step.action) {
        return StepRoute {
            step: step.id,
            idempotency_key: step.idempotency_key.clone(),
            outcome: Err(refusal),
        };
    }
    let outcome = match &step.action {
        Action::Pause { paused } => Ok(RoutedStep {
            family: LiveFamily::ControlV1_7,
            request: LiveRequest::Pause { paused: *paused },
            requires: Vec::new(),
            live_preconditions: vec!["observed pause state differs from the target"],
        }),
        Action::DesignateDig { area, mode } => {
            if *mode == DigMode::Mine {
                tile_dig_area(*area).map(|regions| RoutedStep {
                    family: LiveFamily::DigV1_16,
                    request: LiveRequest::Dig { regions },
                    requires: Vec::new(),
                    live_preconditions: vec![
                        "every target and one-tile halo tile is observed and revealed",
                        "no magma, water, aquifer or occupied tile in the halo",
                        "the operator dig policy admits the region",
                    ],
                })
            } else {
                refuse(format!(
                    "dig/1.16 designates mining only; {mode:?} has no live family"
                ))
            }
        }
        Action::Build {
            kind,
            location,
            footprint,
            material,
        } => match kind {
            BuildingKind::Furniture(name) => match furniture_kind(name) {
                Some(kind) if single_tile(*footprint, *location) => {
                    match (
                        coord_u32(location.x, "x"),
                        coord_u32(location.y, "y"),
                        coord_u32(location.z, "z"),
                    ) {
                        (Ok(x), Ok(y), Ok(z)) => BuildSelection::new(kind, 0, [x, y, z])
                        .map_err(|error| LiveRefusal { reason: error.message })
                        .map(|selection| RoutedStep {
                            family: LiveFamily::BuildV1_19,
                            request: LiveRequest::Furniture {
                                kind,
                                target: selection.target(),
                                material: material.clone(),
                            },
                            requires: vec![LiveResolution::FurnitureItem {
                                kind,
                                material: material.clone(),
                            }],
                            live_preconditions: vec![
                                "the target is an observed, unoccupied floor tile with a complete same-level halo",
                                "an exact eligible item of the kind is selected from one live capture",
                                "every material, nearest-item and reservation constraint must be established; unsupported constraints are refused",
                            ],
                        }),
                        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
                    }
                }
                Some(_) => refuse("build/1.19 places single-tile furniture at its own location"),
                None => refuse(format!(
                    "build/1.19 places beds, chairs and tables only; furniture {name:?} has no live family"
                )),
            },
            other => refuse(format!(
                "build/1.19 places furniture only; {other:?} has no live family"
            )),
        },
        Action::CreateWorkOrder {
            job_token,
            amount,
            conditions,
            ..
        } => match work_order_recipe(job_token) {
            None => refuse(format!(
                "work-orders/1.10 has a closed catalog (wooden bed, door, table, chair); {job_token:?} is outside it"
            )),
            Some(_) if !conditions.is_empty() => {
                refuse("work-orders/1.10 cannot attach order conditions")
            }
            Some(recipe) => match WorkOrderSpec::new(recipe, *amount) {
                Ok(spec) => Ok(RoutedStep {
                    family: LiveFamily::WorkOrdersV1_10,
                    request: LiveRequest::WorkOrder { spec },
                    requires: Vec::new(),
                    live_preconditions: vec!["the material is wood (the live recipe is fixed)"],
                }),
                Err(error) => refuse(format!("work-orders/1.10: {}", error.message)),
            },
        },
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => unit_ids(units).map(|units| RoutedStep {
            family: LiveFamily::WorkforceV1_17,
            request: LiveRequest::WorkDetail {
                units,
                labor: labor.clone(),
                assigned: *enabled,
            },
            requires: vec![LiveResolution::WorkDetailForLabor {
                labor: labor.clone(),
            }],
            live_preconditions: vec![
                "the game is paused while the work-detail membership changes",
                "canonical unit identities are resolved from the exact source schema, snapshot and native capture",
                "the resolved detail is automatic, selected-only and contains only the requested labor",
                "removal has no other granting detail and readback changes no other labor",
            ],
        }),
        Action::ConfigureStockpile { .. } => {
            refuse("no live stockpile configuration family exists")
        }
        Action::AssignSquad { .. } => refuse("no live squad assignment family exists"),
        Action::SetBurrowMembership { .. } => refuse("no live burrow membership family exists"),
        Action::SetStandingOrder { .. } => refuse("no live standing-order family exists"),
        Action::Extension {
            namespace, name, ..
        } => refuse(format!(
            "extension {namespace}.{name} has no reviewed live family"
        )),
    };
    StepRoute {
        step: step.id,
        idempotency_key: step.idempotency_key.clone(),
        outcome,
    }
}

/// Route every step of a sealed plan, in step order.
pub fn route_plan(plan: &PreparedPlan) -> Result<PlanRoute> {
    plan.validate_structure()?;
    Ok(PlanRoute {
        plan_digest: plan.digest,
        steps: plan.steps.iter().map(route_step).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Capability, RiskTier};
    use dfmcp_intent::MaterialSelector;

    fn step(action: Action) -> PlanStep {
        PlanStep {
            id: StepId::new(1),
            required_capability: Capability::Designate,
            risk: RiskTier::Guarded,
            action,
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
            idempotency_key: "k".to_owned(),
        }
    }

    fn cuboid(min: (i32, i32, i32), max: (i32, i32, i32)) -> Result<MapCuboid> {
        MapCuboid::new(
            MapCoord::new(min.0, min.1, min.2),
            MapCoord::new(max.0, max.1, max.2),
        )
    }

    #[test]
    fn dig_tiles_cover_the_area_exactly_in_canonical_order() -> Result<()> {
        let area = cuboid((1, 1, 10), (18, 9, 11))?;
        let regions = tile_dig_area(area).map_err(|r| {
            dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InvalidRequest, r.reason)
        })?;
        let mut covered = BTreeSet::new();
        for region in &regions {
            let [x, y, z, w, h] = region.coordinates();
            assert!(w <= 8 && h <= 8);
            for dy in 0..h {
                for dx in 0..w {
                    assert!(covered.insert((z, y + dy, x + dx)), "overlap");
                }
            }
        }
        assert_eq!(covered.len(), 18 * 9 * 2);
        let order: Vec<_> = regions
            .iter()
            .map(|r| {
                let [x, y, z, ..] = r.coordinates();
                (z, y, x)
            })
            .collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
        assert_eq!(regions.len(), 3 * 2 * 2);
        Ok(())
    }

    #[test]
    fn dig_refusals_are_explicit() -> Result<()> {
        let channel = route_step(&step(Action::DesignateDig {
            area: cuboid((1, 1, 1), (2, 2, 1))?,
            mode: DigMode::Channel,
        }));
        assert!(channel.outcome.is_err());
        let edge = route_step(&step(Action::DesignateDig {
            area: cuboid((0, 1, 1), (2, 2, 1))?,
            mode: DigMode::Mine,
        }));
        assert!(edge.outcome.is_err(), "x=0 has no complete halo");
        let huge = route_step(&step(Action::DesignateDig {
            area: cuboid((1, 1, 1), (100, 100, 1))?,
            mode: DigMode::Mine,
        }));
        assert!(huge.outcome.is_err());
        Ok(())
    }

    #[test]
    fn furniture_work_orders_and_labor_route_with_named_resolutions() -> Result<()> {
        let bed = route_step(&step(Action::Build {
            kind: BuildingKind::Furniture("Bed".to_owned()),
            location: MapCoord::new(5, 6, 10),
            footprint: cuboid((5, 6, 10), (5, 6, 10))?,
            material: MaterialSelector::default(),
        }));
        let routed = bed.outcome.map_err(|r| {
            dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InvalidRequest, r.reason)
        })?;
        assert_eq!(routed.family, LiveFamily::BuildV1_19);
        assert_eq!(
            routed.request,
            LiveRequest::Furniture {
                kind: BuildKind::Bed,
                target: [5, 6, 10],
                material: MaterialSelector::default(),
            }
        );
        assert_eq!(
            routed.requires,
            vec![LiveResolution::FurnitureItem {
                kind: BuildKind::Bed,
                material: MaterialSelector::default(),
            }]
        );
        let still = route_step(&step(Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location: MapCoord::new(5, 6, 10),
            footprint: cuboid((4, 5, 10), (6, 7, 10))?,
            material: MaterialSelector::default(),
        }));
        assert!(still.outcome.is_err());

        let order = route_step(&step(Action::CreateWorkOrder {
            name: "beds".to_owned(),
            job_token: "CONSTRUCT_BED".to_owned(),
            amount: 4,
            conditions: Vec::new(),
        }));
        assert!(matches!(
            order.outcome,
            Ok(RoutedStep {
                family: LiveFamily::WorkOrdersV1_10,
                ..
            })
        ));
        let brew = route_step(&step(Action::CreateWorkOrder {
            name: "brew".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 4,
            conditions: Vec::new(),
        }));
        assert!(brew.outcome.is_err());
        let too_many = route_step(&step(Action::CreateWorkOrder {
            name: "beds".to_owned(),
            job_token: "CONSTRUCT_BED".to_owned(),
            amount: 101,
            conditions: Vec::new(),
        }));
        assert!(too_many.outcome.is_err());

        let labor = route_step(&step(Action::SetLabor {
            units: vec![EntityId::new(9), EntityId::new(3), EntityId::new(9)],
            labor: "MINE".to_owned(),
            enabled: true,
        }));
        match labor.outcome {
            Ok(RoutedStep {
                request:
                    LiveRequest::WorkDetail {
                        units,
                        assigned,
                        labor,
                    },
                requires,
                ..
            }) => {
                assert_eq!(units, vec![EntityId::new(3), EntityId::new(9)]);
                assert_eq!(labor, "MINE");
                assert!(assigned);
                assert_eq!(
                    requires,
                    vec![LiveResolution::WorkDetailForLabor {
                        labor: "MINE".to_owned()
                    }]
                );
            }
            other => {
                return Err(dfmcp_core::DfmcpError::new(
                    dfmcp_core::ErrorCode::InvalidRequest,
                    format!("unexpected labor route {other:?}"),
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn families_without_a_live_surface_are_refused() {
        let squad = route_step(&step(Action::AssignSquad {
            units: vec![EntityId::new(1)],
            squad: EntityId::new(2),
        }));
        assert!(squad.outcome.is_err());
        let pause = route_step(&step(Action::Pause { paused: true }));
        assert!(matches!(
            pause.outcome,
            Ok(RoutedStep {
                family: LiveFamily::ControlV1_7,
                ..
            })
        ));
    }
}
