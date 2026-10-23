//! Original room goals compiled into bounded procurement, excavation and exact
//! item-backed placement. This compiler is a laboratory generation; native
//! room assignment, item routing and live admission remain separate contracts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, IntentId, MapCoord, MapCuboid, Result, RiskTier,
    StepId,
};
use dfmcp_intent::blueprint::{BlueprintLayout, ExcavationRole};
use dfmcp_intent::{
    Action, BlueprintPlanner, BlueprintTemplate, Constraint, FurnitureKind, Intent,
    RequestedAction, StaticPlanner, derive_step_idempotency_key, effects,
};
use dfmcp_world::{EntityKind, Predicate, Value, WorldSnapshot, laboratory_fact_value};
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::lab_world::{ProductionRequest, parse_steps, spatial_index};

const PLANNER: &str = "provisioned_room_v1";
const RESIDUAL_PLANNER: &str = "provisioned_room_residual_v1";
const MAX_REQUEST_BYTES: usize = 64 * 1_024;
const MAX_SLOTS: usize = 16;
const MAX_STEPS: usize = 64;
const MAX_ENTITIES: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "template", rename_all = "snake_case", deny_unknown_fields)]
enum RoomLayout {
    BedroomCluster {
        origin: [i32; 3],
        rooms: u32,
        room_size: [u8; 2],
    },
    DiningHall {
        origin: [i32; 3],
        width: u8,
        height: u8,
    },
}

impl RoomLayout {
    fn geometry(&self) -> (MapCoord, BlueprintTemplate) {
        match self {
            Self::BedroomCluster {
                origin,
                rooms,
                room_size,
            } => (
                coordinate(*origin),
                BlueprintTemplate::BedroomCluster {
                    rooms_count: *rooms,
                    room_size: (room_size[0], room_size[1]),
                },
            ),
            Self::DiningHall {
                origin,
                width,
                height,
            } => (
                coordinate(*origin),
                BlueprintTemplate::DiningHall {
                    width: *width,
                    height: *height,
                },
            ),
        }
    }

    fn canonical_value(&self) -> Json {
        match self {
            Self::BedroomCluster {
                origin,
                rooms,
                room_size,
            } => json!({
                "template":"bedroom_cluster", "origin":origin,
                "rooms":rooms, "room_size":room_size,
            }),
            Self::DiningHall {
                origin,
                width,
                height,
            } => json!({
                "template":"dining_hall", "origin":origin,
                "width":width, "height":height,
            }),
        }
    }

    fn furniture(&self) -> &'static [FurnitureKind] {
        match self {
            Self::BedroomCluster { .. } => &[FurnitureKind::Bed],
            Self::DiningHall { .. } => &[FurnitureKind::Table, FurnitureKind::Chair],
        }
    }
}

/// Original geometry and optional production setup permission, independent of
/// which supplies and sites happen to be useful at the current source anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProvisionedRoomRequest {
    layout: RoomLayout,
    prerequisites: Option<Json>,
}

/// Optional work composed before exact furnishing step identities are derived.
/// Existing room compiler generations never install this hook.
pub(crate) struct RoomAccessComposition {
    pub excavations: Vec<MapCuboid>,
    pub terminal: Predicate,
    pub analysis: Json,
}

pub(crate) type RoomAccessBuilder<'a> =
    dyn Fn(&BlueprintLayout, &[MapCoord], &[RequestedAction]) -> Result<RoomAccessComposition> + 'a;

pub(crate) fn is_provisioned_room(raw: &str) -> bool {
    if raw.len() > MAX_REQUEST_BYTES {
        return false;
    }
    #[derive(Deserialize)]
    struct Header {
        template: String,
    }
    serde_json::from_str::<Header>(raw).is_ok_and(|header| header.template == "provisioned_room")
}

impl ProvisionedRoomRequest {
    /// New intake installs the compiler identity before retaining the source.
    pub(crate) fn parse_current(raw: &str) -> Result<Self> {
        Self::parse_inner(raw, true)
    }

    /// Archived source must name its exact compiler. No old room or action
    /// source is silently upgraded to item-backed placement semantics.
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        Self::parse_inner(raw, false)
    }

    fn parse_inner(raw: &str, current: bool) -> Result<Self> {
        if raw.len() > MAX_REQUEST_BYTES {
            return Err(bounds("provisioned room request exceeds 65536 bytes"));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            template: String,
            planner: Option<String>,
            layout: RoomLayout,
            prerequisites: Option<Json>,
        }
        let input: Input = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid provisioned room request: {error}")))?;
        let supported_planner = match input.planner.as_deref() {
            Some(PLANNER) => true,
            None => current,
            _ => false,
        };
        if input.template != "provisioned_room" || !supported_planner {
            return Err(invalid(
                "provisioned_room requires the fixed provisioned_room_v1 planner generation",
            ));
        }
        let (origin, template) = input.layout.geometry();
        let layout = BlueprintPlanner.layout(origin, template)?;
        furniture_slots(&layout, input.layout.furniture())?;
        // Reuse the closed production setup grammar, including unused sites.
        // Positive dummy quotas establish only its relevant recipe families;
        // no snapshot is read and no candidate work is compiled here.
        let prerequisites = if let Some(options) = input.prerequisites {
            let production = ProductionRequest::parse_current(
                &json!({
                    "template":"production",
                    "quotas":input.layout.furniture().iter().map(|kind| json!({
                        "item":kind.item_token(), "minimum":1,
                    })).collect::<Vec<_>>(),
                    "prerequisites":options,
                })
                .to_string(),
            )?;
            let canonical: Json = serde_json::from_str(&production.canonical_json())
                .map_err(|_| invariant("canonical production request could not be decoded"))?;
            Some(canonical.get("prerequisites").cloned().ok_or_else(|| {
                invariant("production setup normalization lost the original options")
            })?)
        } else {
            None
        };
        Ok(Self {
            layout: input.layout,
            prerequisites,
        })
    }

    pub(crate) fn canonical_json(&self) -> String {
        let mut value = json!({
            "template":"provisioned_room", "planner":PLANNER,
            "layout":self.layout.canonical_value(),
        });
        if let Some(options) = &self.prerequisites {
            value["prerequisites"] = options.clone();
        }
        value.to_string()
    }

    pub(crate) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Json)> {
        self.compile_inner(id, snapshot, summary, false, None)
    }

    /// A separately versioned pursuit can retain exact completed installations.
    /// The caller owns current authority, original lineage and quiescence checks;
    /// pure archived reconstruction must not consult a mutable goal book.
    pub(crate) fn compile_residual(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Json)> {
        self.compile_inner(id, snapshot, summary, true, None)
    }

    pub(crate) fn access_geometry(&self) -> Result<(BlueprintLayout, Vec<MapCoord>)> {
        let (origin, template) = self.layout.geometry();
        let layout = BlueprintPlanner.layout(origin, template)?;
        let targets = furniture_slots(&layout, self.layout.furniture())?
            .into_iter()
            .map(|slot| slot.location)
            .collect();
        Ok((layout, targets))
    }

    pub(crate) fn compile_with_access(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        residual: bool,
        access: &RoomAccessBuilder<'_>,
    ) -> Result<(Intent, Json)> {
        self.compile_inner(id, snapshot, "", residual, Some(access))
    }

    fn compile_inner(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
        residual: bool,
        access_builder: Option<&RoomAccessBuilder<'_>>,
    ) -> Result<(Intent, Json)> {
        if snapshot.graph.entities.len() > MAX_ENTITIES {
            return Err(bounds(
                "room provisioning exceeds its complete entity scan bound",
            ));
        }
        let (origin, template) = self.layout.geometry();
        let layout = BlueprintPlanner.layout(origin, template.clone())?;
        let slots = furniture_slots(&layout, self.layout.furniture())?;
        let retained = if residual {
            retained_installations(snapshot, &layout, &slots)?
        } else {
            reject_occupied_geometry(snapshot, &layout)?;
            BTreeMap::new()
        };
        let excavations = BlueprintPlanner.compile_blueprint_intent(
            id,
            snapshot.anchor(),
            origin,
            template,
            &spatial_index(snapshot)?,
        )?;
        let mut needed = BTreeMap::<&'static str, usize>::new();
        for slot in &slots {
            *needed.entry(slot.kind.item_token()).or_default() += 1;
        }
        let mut supply = BTreeMap::<&'static str, VecDeque<(EntityId, bool)>>::new();
        let mut quotas = Vec::new();
        let mut inventory = Vec::new();
        for kind in self.layout.furniture() {
            let token = kind.item_token();
            let required = *needed
                .get(token)
                .ok_or_else(|| invariant("missing slot count"))?;
            let retained_count = retained.values().filter(|item| item.kind == *kind).count();
            let remaining = required.checked_sub(retained_count).ok_or_else(|| {
                invariant("retained furnishings exceed the complete original slot count")
            })?;
            if residual && remaining == 0 {
                supply.insert(token, VecDeque::new());
                inventory.push(json!({
                    "item":token, "required":required, "retained_installations":retained_count,
                    "remaining_furnishings":0, "observed_aggregate":null,
                    "inventory_inspected":false, "eligible_items_selected":0,
                    "new_items_required":0, "procurement_stock_target":null,
                    "legacy_or_unallocated_stock_is_not_item_credit":true,
                }));
                continue;
            }
            let available = effects::loose_furniture_items(snapshot, *kind)?;
            let selected = available.len().min(remaining);
            let missing = remaining - selected;
            let stock = observed_stock(snapshot, *kind)?;
            let minimum = if missing == 0 {
                None
            } else {
                let minimum =
                    stock
                        .checked_add(u64::try_from(missing).map_err(|_| {
                            bounds("room material deficit exceeds its representation")
                        })?)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| {
                            bounds("room procurement exceeds the finite production quota bound")
                        })?;
                quotas.push(json!({"item":token, "minimum":minimum}));
                Some(minimum)
            };
            supply.insert(
                token,
                available
                    .into_iter()
                    .take(selected)
                    .map(|id| (id, false))
                    .collect(),
            );
            let mut row = json!({
                "item":token, "required":required, "observed_aggregate":stock,
                "eligible_items_selected":selected, "new_items_required":missing,
                "procurement_stock_target":minimum,
                "legacy_or_unallocated_stock_is_not_item_credit":true,
            });
            if residual {
                row["retained_installations"] = json!(retained_count);
                row["remaining_furnishings"] = json!(remaining);
                row["inventory_inspected"] = json!(true);
            }
            inventory.push(row);
        }
        let mut requested_actions = Vec::new();
        let mut production_analysis = Json::Null;
        if !quotas.is_empty() {
            let mut source = json!({"template":"production", "quotas":quotas});
            if let Some(options) = &self.prerequisites {
                source["prerequisites"] = options.clone();
            }
            let compiled =
                ProductionRequest::parse_current(&source.to_string())?.compile(snapshot)?;
            let mut procurement = Intent {
                id,
                anchor: snapshot.anchor(),
                summary: "procure original room furnishings".to_owned(),
                terminal_condition: compiled.terminal.clone(),
                constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                requested_actions: parse_steps(&compiled.actions)?,
            };
            compiled.apply_capacity_horizon(&mut procurement)?;
            production_analysis = compiled.analysis;
            requested_actions = procurement.requested_actions;
        }
        let access = access_builder
            .map(|build| {
                let targets: Vec<_> = slots.iter().map(|slot| slot.location).collect();
                build(&layout, &targets, &requested_actions)
            })
            .transpose()?;
        let mut production_dependencies = Vec::new();
        for (index, requested) in requested_actions.iter().enumerate() {
            match &requested.action {
                Action::Build { footprint, .. } => {
                    if layout
                        .excavations()
                        .iter()
                        .any(|part| overlap(*footprint, part.area))
                    {
                        return Err(precondition(
                            "a planned production workshop overlaps the original room excavation",
                        ));
                    }
                }
                Action::CreateWorkOrder {
                    job_token, amount, ..
                } => {
                    let token = effects::work_order_output_item(job_token)
                        .ok_or_else(|| invariant("room procurement escaped its closed recipes"))?;
                    let outputs = supply.get_mut(token).ok_or_else(|| {
                        invariant("room procurement created an unrequested output")
                    })?;
                    let step = step_number(index)?;
                    production_dependencies.push(step);
                    let key = derive_step_idempotency_key(
                        id,
                        snapshot.anchor(),
                        StepId::new(step),
                        &requested.action.normalized(),
                    );
                    let order = effects::created_entity_id(&key, 0);
                    if usize::try_from(*amount)
                        .ok()
                        .is_none_or(|count| count > MAX_SLOTS)
                    {
                        return Err(bounds("room procurement exceeds sixteen new items"));
                    }
                    for ordinal in 1..=*amount {
                        let item = effects::produced_item_id(order, ordinal);
                        if snapshot.graph.entities.contains_key(&item) {
                            return Err(DfmcpError::new(
                                ErrorCode::Conflict,
                                "future room item identity already exists",
                            ));
                        }
                        outputs.push_back((item, true));
                    }
                }
                _ => {}
            }
        }
        let excavation_offset = step_number(requested_actions.len())?;
        let mut terminal = vec![excavations.terminal_condition];
        let mut excavation_steps: Vec<Vec<u32>> = Vec::new();
        for requested in excavations.requested_actions {
            let parts = if residual {
                residual_excavations(snapshot, requested, retained.values())?
            } else {
                vec![requested]
            };
            let mut steps = Vec::new();
            for mut requested in parts {
                // This compiler generation uses the reference model's work-scaled
                // dig horizon. StaticPlanner seals the default obligation and then
                // starts dependent placement horizons after that deadline. Keep
                // the legacy blueprint compiler's fixed horizon unchanged.
                if matches!(requested.action, Action::DesignateDig { .. }) {
                    requested.obligation = None;
                }
                if residual {
                    let mut dependencies = Vec::new();
                    for dependency in &requested.depends_on {
                        dependencies.extend_from_slice(
                            excavation_steps.get(*dependency as usize).ok_or_else(|| {
                                invariant("residual excavation dependency is not an earlier part")
                            })?,
                        );
                    }
                    dependencies.sort_unstable();
                    dependencies.dedup();
                    requested.depends_on = dependencies;
                } else {
                    for dependency in &mut requested.depends_on {
                        *dependency = dependency
                            .checked_add(excavation_offset)
                            .ok_or_else(|| bounds("room excavation dependency overflow"))?;
                    }
                }
                steps.push(step_number(requested_actions.len())?);
                requested_actions.push(requested);
            }
            excavation_steps.push(steps);
        }
        let mut access_steps = Vec::new();
        if let Some(access) = &access {
            for area in &access.excavations {
                let postcondition = Predicate::RegionTerrain {
                    area: *area,
                    tile_code: effects::dig_target_tile_code(dfmcp_intent::DigMode::Mine),
                };
                terminal.push(postcondition.clone());
                access_steps.push(step_number(requested_actions.len())?);
                requested_actions.push(RequestedAction {
                    action: Action::DesignateDig {
                        area: *area,
                        mode: dfmcp_intent::DigMode::Mine,
                    },
                    preconditions: Vec::new(),
                    postconditions: vec![postcondition],
                    compensation: None,
                    obligation: None,
                    depends_on: Vec::new(),
                });
            }
            terminal.push(access.terminal.clone());
        }
        let mut allocation = Vec::new();
        let mut allocated_ids = BTreeSet::new();
        for (slot_index, slot) in slots.into_iter().enumerate() {
            if let Some(installation) = retained.get(&slot_index) {
                if !allocated_ids.insert(installation.item) {
                    return Err(invariant(
                        "a retained item was allocated to two room targets",
                    ));
                }
                terminal.extend(effects::furniture_completion_predicates(installation));
                allocation.push(json!({
                    "kind":slot.kind.as_str(), "item_token":slot.kind.item_token(),
                    "item":installation.item.get().to_string(),
                    "building":installation.building.get().to_string(),
                    "location":[slot.location.x,slot.location.y,slot.location.z],
                    "supply":"retained_installation", "step":null, "excavation_steps":[],
                }));
                continue;
            }
            let (item, manufactured) = supply
                .get_mut(slot.kind.item_token())
                .and_then(VecDeque::pop_front)
                .ok_or_else(|| invariant("room procurement omitted an original furnishing"))?;
            if !allocated_ids.insert(item) {
                return Err(invariant("an exact item was allocated to two room targets"));
            }
            let action = Action::PlaceFurniture {
                kind: slot.kind,
                item,
                location: slot.location,
            };
            let step = step_number(requested_actions.len())?;
            let key = derive_step_idempotency_key(
                id,
                snapshot.anchor(),
                StepId::new(step),
                &action.normalized(),
            );
            let postconditions =
                effects::default_postconditions(&action, &key, snapshot.fortress_id);
            if postconditions.is_empty() {
                return Err(invariant(
                    "item-backed placement has no registered completion proof",
                ));
            }
            terminal.extend(postconditions.iter().cloned());
            let room_dependencies = excavation_steps
                .get(slot.excavation)
                .ok_or_else(|| invariant("room placement excavation is missing"))?;
            let mut depends_on = production_dependencies.clone();
            depends_on.extend_from_slice(room_dependencies);
            if access.is_some() {
                // Hauling approaches require the complete intended circulation,
                // including doorway/spine parts outside the furnishing's room.
                depends_on.extend(excavation_steps.iter().flatten().copied());
                depends_on.extend_from_slice(&access_steps);
            }
            depends_on.sort_unstable();
            depends_on.dedup();
            requested_actions.push(RequestedAction {
                action,
                preconditions: Vec::new(),
                postconditions,
                compensation: None,
                obligation: None,
                depends_on,
            });
            let mut row = json!({
                "kind":slot.kind.as_str(), "item_token":slot.kind.item_token(),
                "item":item.get().to_string(), "location":[slot.location.x,slot.location.y,slot.location.z],
                "supply":if manufactured { "planned_production" } else { "observed_loose_item" },
                "step":step, "excavation_step":room_dependencies.first(),
            });
            if residual {
                row["excavation_steps"] = json!(room_dependencies);
            }
            allocation.push(row);
        }
        if requested_actions.len() > MAX_STEPS {
            return Err(bounds(
                "provisioned room exceeds 64 combined production, excavation and placement steps",
            ));
        }
        if supply.values().any(|items| !items.is_empty()) {
            return Err(invariant(
                "room procurement produced excess unassigned items",
            ));
        }
        let mut source_bytes = if residual {
            b"dfmcp-provisioned-room-residual-source/1\0".to_vec()
        } else {
            b"dfmcp-provisioned-room-source/1\0".to_vec()
        };
        source_bytes.extend_from_slice(self.canonical_json().as_bytes());
        let digest = Digest32::of_bytes(&source_bytes);
        let label = if summary.is_empty() {
            "provision the complete original room"
        } else {
            summary
        };
        let sealed_summary = format!("{label} [provisioned room source {}]", digest.to_hex());
        if sealed_summary.len() > StaticPlanner::default().policy.max_string_bytes
            || sealed_summary.contains('\0')
        {
            return Err(invalid(
                "room summary plus source seal exceeds the planner bound; supply a shorter summary",
            ));
        }
        let terminal_condition = Predicate::All(terminal).normalized();
        terminal_condition.validate_shape()?;
        let intent = Intent {
            id,
            anchor: snapshot.anchor(),
            summary: sealed_summary,
            terminal_condition,
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions,
        };
        let mut analysis = json!({
            "template":"provisioned_room", "planner":if residual { RESIDUAL_PLANNER } else { PLANNER },
            "source_binding":{"room_source_digest":digest.to_hex(), "original_request_preserved":true},
            "layout":self.layout.canonical_value(), "inventory":inventory,
            "production":production_analysis, "allocation":allocation,
            "excavation":{"parts":layout.excavations().len(), "tiles":layout.tile_count()},
            "goal":"all original excavation regions and every exact installed furnishing",
            "intermediate_stock_quotas_are_final_goals":false,
            "room_assignment_created":false, "native_admission_changed":false,
        });
        if residual {
            analysis["source_planner"] = json!(PLANNER);
            analysis["retained_placements"] = json!(retained.len());
            analysis["new_placements"] = json!(allocated_ids.len() - retained.len());
            analysis["proves_old_exact_action_goal"] = json!(false);
            analysis["pursuit"] = json!("new reviewed pursuit of every original room target");
        }
        if let Some(access) = access {
            analysis["access"] = access.analysis;
        }
        Ok((intent, analysis))
    }
}

struct Slot {
    kind: FurnitureKind,
    location: MapCoord,
    excavation: usize,
}

/// Retain only complete exact-item furniture at the original requested slots.
/// An occupied tile is never interpreted as useful inventory by its name alone.
fn retained_installations(
    snapshot: &WorldSnapshot,
    layout: &BlueprintLayout,
    slots: &[Slot],
) -> Result<BTreeMap<usize, effects::FurnitureInstallation>> {
    let mut positions = BTreeMap::new();
    for (index, slot) in slots.iter().enumerate() {
        if positions.insert(slot.location, index).is_some() {
            return Err(invariant(
                "the original room contains duplicate furnishing slots",
            ));
        }
    }
    let mut retained = BTreeMap::new();
    let mut items = BTreeSet::new();
    for building in snapshot
        .graph
        .entities
        .values()
        .filter(|record| record.kind == EntityKind::Building)
    {
        let Some(area) = effects::occupied_building_footprint(snapshot, building.id)? else {
            continue;
        };
        if !layout
            .excavations()
            .iter()
            .any(|part| overlap(area, part.area))
        {
            continue;
        }
        let slot_index = positions
            .get(&area.min)
            .copied()
            .filter(|_| area.min == area.max)
            .ok_or_else(|| {
                precondition(
                    "original room geometry is occupied outside an exact requested furnishing slot",
                )
            })?;
        let slot = &slots[slot_index];
        let installation = effects::inspect_complete_furniture(snapshot, building.id)?;
        if installation.kind != slot.kind || installation.location != slot.location {
            return Err(precondition(
                "occupied room slot does not contain its requested furniture kind and location",
            ));
        }
        let target = effects::dig_target_tile_code(layout.excavations()[slot.excavation].mode);
        if snapshot.tile_code_at(installation.location) != Some(target) {
            return Err(precondition(
                "a retained installation does not stand on its original observed room terrain",
            ));
        }
        if !items.insert(installation.item) || retained.insert(slot_index, installation).is_some() {
            return Err(precondition(
                "duplicate furniture occupancy cannot satisfy distinct original room slots",
            ));
        }
    }
    Ok(retained)
}

/// Exact retained tiles are holes in the new effect mask. The full original
/// terrain condition remains in the goal, including every hole and already-dug
/// part, but no residual designation can change terrain beneath an installation.
fn residual_excavations<'a>(
    snapshot: &WorldSnapshot,
    requested: RequestedAction,
    retained: impl Iterator<Item = &'a effects::FurnitureInstallation>,
) -> Result<Vec<RequestedAction>> {
    let Action::DesignateDig { area, mode } = requested.action else {
        return Err(invariant(
            "room excavation escaped its closed action family",
        ));
    };
    let mut holes: Vec<_> = retained
        .map(|installation| installation.location)
        .filter(|location| area.contains(*location))
        .collect();
    if holes.is_empty() {
        return Ok(vec![requested]);
    }
    if area.min.z != area.max.z {
        return Err(invariant(
            "residual room excavation must remain on one level",
        ));
    }
    holes.sort_by_key(|at| (at.z, at.y, at.x));
    holes.dedup();
    let mut pieces = vec![area];
    for hole in holes {
        let mut next = Vec::new();
        for piece in pieces {
            if !piece.contains(hole) {
                next.push(piece);
                continue;
            }
            if piece.min.y < hole.y {
                next.push(MapCuboid::new(
                    piece.min,
                    MapCoord::new(piece.max.x, hole.y - 1, hole.z),
                )?);
            }
            if hole.y < piece.max.y {
                next.push(MapCuboid::new(
                    MapCoord::new(piece.min.x, hole.y + 1, hole.z),
                    piece.max,
                )?);
            }
            if piece.min.x < hole.x {
                next.push(MapCuboid::new(
                    MapCoord::new(piece.min.x, hole.y, hole.z),
                    MapCoord::new(hole.x - 1, hole.y, hole.z),
                )?);
            }
            if hole.x < piece.max.x {
                next.push(MapCuboid::new(
                    MapCoord::new(hole.x + 1, hole.y, hole.z),
                    MapCoord::new(piece.max.x, hole.y, hole.z),
                )?);
            }
        }
        if next.len() > MAX_STEPS {
            return Err(bounds(
                "residual room excavation exceeds its bounded partition",
            ));
        }
        pieces = next;
    }
    pieces.sort_by_key(|part| {
        (
            part.min.z, part.min.y, part.min.x, part.max.z, part.max.y, part.max.x,
        )
    });
    let target = effects::dig_target_tile_code(mode);
    let mut result = Vec::new();
    for piece in pieces {
        if dfmcp_world::terrain::region_tiles(piece)
            .all(|at| snapshot.tile_code_at(at) == Some(target))
        {
            continue;
        }
        let mut part = requested.clone();
        part.action = Action::DesignateDig { area: piece, mode };
        part.postconditions = vec![Predicate::RegionTerrain {
            area: piece,
            tile_code: target,
        }];
        part.obligation = None;
        result.push(part);
    }
    Ok(result)
}

fn furniture_slots(layout: &BlueprintLayout, kinds: &[FurnitureKind]) -> Result<Vec<Slot>> {
    let mut slots = Vec::new();
    for (excavation, part) in layout.excavations().iter().enumerate() {
        if part.role != ExcavationRole::Room {
            continue;
        }
        let centre = MapCoord::new(
            part.area.min.x + (part.area.max.x - part.area.min.x) / 2,
            part.area.min.y + (part.area.max.y - part.area.min.y) / 2,
            part.area.min.z,
        );
        for (index, kind) in kinds.iter().enumerate() {
            let location = if index == 0 {
                centre
            } else {
                // A dining chair uses the first interior cardinal neighbour.
                // This also handles one-tile-wide rooms without escaping them.
                [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .into_iter()
                    .find_map(|(dx, dy)| {
                        Some(MapCoord::new(
                            centre.x.checked_add(dx)?,
                            centre.y.checked_add(dy)?,
                            centre.z,
                        ))
                        .filter(|location| part.area.contains(*location))
                    })
                    .ok_or_else(|| {
                        invalid("a dining hall needs two distinct interior furniture tiles")
                    })?
            };
            if slots.len() == MAX_SLOTS {
                return Err(bounds(
                    "provisioned room supports at most sixteen furnishing slots",
                ));
            }
            slots.push(Slot {
                kind: *kind,
                location,
                excavation,
            });
        }
    }
    if slots.is_empty() {
        return Err(invalid("provisioned room requires at least one furnishing"));
    }
    Ok(slots)
}

fn observed_stock(snapshot: &WorldSnapshot, kind: FurnitureKind) -> Result<u64> {
    let field = effects::furniture_stock_field(kind);
    effects::stock_ledger(snapshot)
        .and_then(|id| snapshot.graph.entities.get(&id))
        .and_then(|ledger| ledger.fields.get(field))
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        .and_then(|value| match value {
            Value::U64(count) => Some(*count),
            _ => None,
        })
        .ok_or_else(|| {
            precondition("room provisioning requires its qualified furniture stock count")
        })
}

fn reject_occupied_geometry(snapshot: &WorldSnapshot, layout: &BlueprintLayout) -> Result<()> {
    for building in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Building)
    {
        let Some(area) = effects::occupied_building_footprint(snapshot, building.id)? else {
            continue;
        };
        if layout
            .excavations()
            .iter()
            .any(|part| overlap(area, part.area))
        {
            return Err(precondition(
                "the original room excavation overlaps an existing building",
            ));
        }
    }
    Ok(())
}

fn overlap(left: MapCuboid, right: MapCuboid) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
        && left.min.z <= right.max.z
        && left.max.z >= right.min.z
}

fn coordinate(values: [i32; 3]) -> MapCoord {
    MapCoord::new(values[0], values[1], values[2])
}
fn step_number(index: usize) -> Result<u32> {
    u32::try_from(index).map_err(|_| bounds("room step index overflow"))
}
fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn precondition(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}
fn bounds(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}
fn invariant(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

#[cfg(test)]
#[path = "room_provisioning_tests.rs"]
mod tests;
