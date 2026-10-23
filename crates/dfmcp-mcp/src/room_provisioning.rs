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
        if snapshot.graph.entities.len() > MAX_ENTITIES {
            return Err(bounds(
                "room provisioning exceeds its complete entity scan bound",
            ));
        }
        let (origin, template) = self.layout.geometry();
        let layout = BlueprintPlanner.layout(origin, template.clone())?;
        let slots = furniture_slots(&layout, self.layout.furniture())?;
        reject_occupied_geometry(snapshot, &layout)?;
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
            let available = effects::loose_furniture_items(snapshot, *kind)?;
            let required = *needed
                .get(token)
                .ok_or_else(|| invariant("missing slot count"))?;
            let selected = available.len().min(required);
            let missing = required - selected;
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
            inventory.push(json!({
                "item":token, "required":required, "observed_aggregate":stock,
                "eligible_items_selected":selected, "new_items_required":missing,
                "procurement_stock_target":minimum,
                "legacy_or_unallocated_stock_is_not_item_credit":true,
            }));
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
        for mut requested in excavations.requested_actions {
            // This compiler generation uses the reference model's work-scaled
            // dig horizon. StaticPlanner seals the default obligation and then
            // starts dependent placement horizons after that deadline. Keep
            // the legacy blueprint compiler's fixed horizon unchanged.
            if matches!(requested.action, Action::DesignateDig { .. }) {
                requested.obligation = None;
            }
            for dependency in &mut requested.depends_on {
                *dependency = dependency
                    .checked_add(excavation_offset)
                    .ok_or_else(|| bounds("room excavation dependency overflow"))?;
            }
            requested_actions.push(requested);
        }
        let mut allocation = Vec::new();
        let mut allocated_ids = BTreeSet::new();
        for slot in slots {
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
            let room_dependency = step_number(slot.excavation)?
                .checked_add(excavation_offset)
                .ok_or_else(|| bounds("room placement dependency overflow"))?;
            let mut depends_on = production_dependencies.clone();
            depends_on.push(room_dependency);
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
            allocation.push(json!({
                "kind":slot.kind.as_str(), "item_token":slot.kind.item_token(),
                "item":item.get().to_string(), "location":[slot.location.x,slot.location.y,slot.location.z],
                "supply":if manufactured { "planned_production" } else { "observed_loose_item" },
                "step":step, "excavation_step":room_dependency,
            }));
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
        let mut source_bytes = b"dfmcp-provisioned-room-source/1\0".to_vec();
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
        Ok((
            intent,
            json!({
                "template":"provisioned_room", "planner":PLANNER,
                "source_binding":{"room_source_digest":digest.to_hex(), "original_request_preserved":true},
                "layout":self.layout.canonical_value(), "inventory":inventory,
                "production":production_analysis, "allocation":allocation,
                "excavation":{"parts":layout.excavations().len(), "tiles":layout.tile_count()},
                "goal":"all original excavation regions and every exact installed furnishing",
                "intermediate_stock_quotas_are_final_goals":false,
                "room_assignment_created":false, "native_admission_changed":false,
            }),
        ))
    }
}

struct Slot {
    kind: FurnitureKind,
    location: MapCoord,
    excavation: usize,
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
