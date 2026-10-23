//! Exact-item reference placement. Custody is explicit and reciprocal; missing
//! historical custody never means that an item is free for another building.

use super::*;

pub const FURNITURE_INVENTORY_STATE_FIELD: &str = "inventory_state";
pub const FURNITURE_ALLOCATED_TO_FIELD: &str = "allocated_to";
pub const FURNITURE_PLACEMENT_MODEL_FIELD: &str = "placement_model";
pub const FURNITURE_PLACEMENT_MODEL: &str = "furniture/1";
pub const FURNITURE_SOURCE_ITEM_FIELD: &str = "source_item";
pub const FURNITURE_SOURCE_ITEM_GENERATION_FIELD: &str = "source_item_generation";
pub const FURNITURE_ATTACHMENT_STATE_FIELD: &str = "attachment_state";

const LOOSE: &str = "loose";
const ALLOCATED: &str = "allocated";
const INSTALLED: &str = "installed";
const ATTACHED: &str = "attached";
const RELEASED: &str = "released";
const MAX_FURNITURE_GOAL_BINDINGS: usize = 64;

/// Exact installed furniture identity. The descriptor is an expectation, not
/// authority; inspecting or proving it still requires current eligible state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FurnitureInstallation {
    pub building: EntityId,
    pub kind: FurnitureKind,
    pub item: EntityId,
    pub location: MapCoord,
}

#[derive(Clone, Copy)]
struct Placement {
    building: EntityId,
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
    progress: u64,
    phase: PlacementPhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlacementPhase {
    Active,
    Complete,
    Cancelled,
}

fn bounded_domain(snapshot: &WorldSnapshot) -> Result<()> {
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "furniture inventory exceeds the 65,536-entity inspection bound",
        ));
    }
    Ok(())
}

#[must_use]
pub const fn furniture_stock_field(kind: FurnitureKind) -> &'static str {
    match kind {
        FurnitureKind::Bed => STOCK_BED_FIELD,
        FurnitureKind::Door => STOCK_DOOR_FIELD,
        FurnitureKind::Table => STOCK_TABLE_FIELD,
        FurnitureKind::Chair => STOCK_CHAIR_FIELD,
    }
}

fn kind_label(kind: FurnitureKind) -> String {
    format!("furniture:{}", kind.as_str())
}

fn parse_kind(label: &str) -> Result<FurnitureKind> {
    match label {
        "furniture:Bed" => Ok(FurnitureKind::Bed),
        "furniture:Door" => Ok(FurnitureKind::Door),
        "furniture:Table" => Ok(FurnitureKind::Table),
        "furniture:Chair" => Ok(FurnitureKind::Chair),
        _ => Err(precondition("placement has no registered furniture kind")),
    }
}

fn require_value(
    record: &EntityRecord,
    name: &str,
    expected: &Value,
    tick: GameTick,
) -> Result<()> {
    if field_value(record, name, tick) != Some(expected) {
        return Err(precondition(format!(
            "furniture requires source-qualified {name} on entity {}",
            record.id.get()
        )));
    }
    Ok(())
}

fn reference_item(snapshot: &WorldSnapshot, id: EntityId, kind: FurnitureKind) -> Result<()> {
    let item = entity(snapshot, id)?;
    if item.kind != EntityKind::Item || item.generation != 1 {
        return Err(precondition(
            "furniture requires its original singleton item generation",
        ));
    }
    for (name, expected) in [
        ("item_token", Value::Text(kind.item_token().to_owned())),
        ("material_token", Value::Text("WOOD".to_owned())),
        (
            "material_tokens",
            Value::List(vec![Value::Text("WOOD".to_owned())]),
        ),
        ("stack_size", Value::U64(1)),
        (
            WORK_ORDER_PRODUCTION_MODEL_FIELD,
            Value::Text(CARPENTRY_PRODUCTION_MODEL.to_owned()),
        ),
    ] {
        require_value(item, name, &expected, snapshot.tick)?;
    }
    let Some(Value::Entity(order_id)) = field_value(item, "source_order", snapshot.tick) else {
        return Err(precondition("furniture has no qualified original producer"));
    };
    let ordinal = u32::try_from(field_u64(item, "production_ordinal", snapshot.tick)?)
        .ok()
        .filter(|ordinal| *ordinal != 0)
        .ok_or_else(|| precondition("furniture has no canonical production ordinal"))?;
    if produced_item_id(*order_id, ordinal) != id {
        return Err(precondition(
            "furniture identity does not belong to its original producer and ordinal",
        ));
    }
    let order = entity(snapshot, *order_id)?;
    if order.kind != EntityKind::WorkOrder || order.generation != 1 {
        return Err(precondition(
            "furniture producer generation is not established",
        ));
    }
    require_value(
        order,
        WORK_ORDER_PRODUCTION_MODEL_FIELD,
        &Value::Text(CARPENTRY_PRODUCTION_MODEL.to_owned()),
        snapshot.tick,
    )?;
    let job = field_text(order, "job_token", snapshot.tick)
        .ok_or_else(|| precondition("furniture producer has no qualified recipe"))?;
    let total = field_u64(order, "amount_total", snapshot.tick)?;
    let remaining = field_u64(order, AMOUNT_REMAINING_FIELD, snapshot.tick)?;
    if work_order_output_item(job) != Some(kind.item_token())
        || total == 0
        || total > u64::from(u32::MAX)
        || remaining > total
        || u64::from(ordinal) > total - remaining
        || !matches!(
            field_text(order, STATUS_FIELD, snapshot.tick),
            Some(STATUS_ACTIVE | STATUS_COMPLETE | STATUS_CANCELLED)
        )
    {
        return Err(precondition(
            "furniture is not an established completed unit of its original producer",
        ));
    }
    Ok(())
}

pub(super) fn has_placement_fields(record: &EntityRecord) -> bool {
    [
        FURNITURE_PLACEMENT_MODEL_FIELD,
        FURNITURE_SOURCE_ITEM_FIELD,
        FURNITURE_SOURCE_ITEM_GENERATION_FIELD,
        FURNITURE_ATTACHMENT_STATE_FIELD,
    ]
    .iter()
    .any(|field| record.fields.contains_key(*field))
}

fn placement(snapshot: &WorldSnapshot, id: EntityId) -> Result<Placement> {
    let building = entity(snapshot, id)?;
    if building.kind != EntityKind::Building || building.generation != 1 {
        return Err(precondition(
            "furniture building generation is not established",
        ));
    }
    require_value(
        building,
        FURNITURE_PLACEMENT_MODEL_FIELD,
        &Value::Text(FURNITURE_PLACEMENT_MODEL.to_owned()),
        snapshot.tick,
    )?;
    let kind = parse_kind(
        field_text(building, "building_kind", snapshot.tick)
            .ok_or_else(|| precondition("furniture building kind is not established"))?,
    )?;
    let Some(Value::Entity(item)) =
        field_value(building, FURNITURE_SOURCE_ITEM_FIELD, snapshot.tick)
    else {
        return Err(precondition("furniture building has no exact source item"));
    };
    require_value(
        building,
        FURNITURE_SOURCE_ITEM_GENERATION_FIELD,
        &Value::U64(1),
        snapshot.tick,
    )?;
    let location = coord_field(building, "position", snapshot.tick)
        .ok_or_else(|| precondition("furniture building position is not established"))?;
    for field in ["footprint_min", "footprint_max"] {
        require_value(building, field, &Value::Coord(location), snapshot.tick)?;
    }
    require_value(
        building,
        "required_ticks",
        &Value::U64(BUILD_TICKS),
        snapshot.tick,
    )?;
    let progress = field_u64(building, "progress_ticks", snapshot.tick)?;
    let phase = match field_text(building, CONSTRUCTION_STAGE_FIELD, snapshot.tick) {
        Some(STAGE_PLANNED) if progress == 0 => PlacementPhase::Active,
        Some(STAGE_UNDER_CONSTRUCTION) if progress > 0 && progress < BUILD_TICKS => {
            PlacementPhase::Active
        }
        Some(STAGE_COMPLETE) if progress == BUILD_TICKS => PlacementPhase::Complete,
        Some(STAGE_CANCELLED) if progress < BUILD_TICKS => PlacementPhase::Cancelled,
        _ => {
            return Err(precondition(
                "furniture has no canonical construction lifecycle",
            ));
        }
    };
    require_value(
        building,
        FURNITURE_ATTACHMENT_STATE_FIELD,
        &Value::Text(
            if phase == PlacementPhase::Cancelled {
                RELEASED
            } else {
                ATTACHED
            }
            .to_owned(),
        ),
        snapshot.tick,
    )?;
    Ok(Placement {
        building: id,
        kind,
        item: *item,
        location,
        progress,
        phase,
    })
}

fn binding_values(snapshot: &WorldSnapshot, placement: Placement) -> Result<()> {
    if placement.phase == PlacementPhase::Cancelled {
        // Its released item may have been used by a later, independent action.
        return Ok(());
    }
    reference_item(snapshot, placement.item, placement.kind)?;
    let item = entity(snapshot, placement.item)?;
    require_value(
        item,
        FURNITURE_ALLOCATED_TO_FIELD,
        &Value::Entity(placement.building),
        snapshot.tick,
    )?;
    require_value(
        item,
        FURNITURE_INVENTORY_STATE_FIELD,
        &Value::Text(
            if placement.phase == PlacementPhase::Complete {
                INSTALLED
            } else {
                ALLOCATED
            }
            .to_owned(),
        ),
        snapshot.tick,
    )?;
    if placement.phase == PlacementPhase::Complete {
        require_value(
            item,
            "position",
            &Value::Coord(placement.location),
            snapshot.tick,
        )?;
    }
    Ok(())
}

fn attachments(snapshot: &WorldSnapshot) -> Result<BTreeMap<EntityId, EntityId>> {
    bounded_domain(snapshot)?;
    let mut attached = BTreeMap::new();
    for record in snapshot.graph.entities.values() {
        if record.kind != EntityKind::Building || !has_placement_fields(record) {
            continue;
        }
        let placement = placement(snapshot, record.id)?;
        binding_values(snapshot, placement)?;
        if placement.phase != PlacementPhase::Cancelled
            && attached
                .insert(placement.item, placement.building)
                .is_some()
        {
            return Err(precondition(
                "one furniture item is attached to several buildings",
            ));
        }
    }
    Ok(attached)
}

/// Verify the exact original placement parameters without substituting an item
/// or treating a matching lifecycle word as ownership.
pub fn validate_furniture_identity(
    snapshot: &WorldSnapshot,
    building: EntityId,
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
) -> Result<()> {
    bounded_domain(snapshot)?;
    let actual = placement(snapshot, building)?;
    if actual.kind != kind || actual.item != item || actual.location != location {
        return Err(precondition(
            "furniture parameters differ from their original action",
        ));
    }
    Ok(())
}

/// Validate current reciprocal custody. A cancelled record retains historical
/// identity but no claim on an item subsequently allocated by another action.
pub fn validate_furniture_binding(snapshot: &WorldSnapshot, building: EntityId) -> Result<()> {
    bounded_domain(snapshot)?;
    let actual = placement(snapshot, building)?;
    if actual.phase == PlacementPhase::Cancelled {
        return Ok(());
    }
    if attachments(snapshot)?.get(&actual.item) != Some(&building) {
        return Err(precondition("furniture attachment is not established"));
    }
    Ok(())
}

/// Verify complete, exact-item installation, including generation and reciprocal
/// custody evidence that ordinary field predicates cannot express.
pub fn validate_furniture_completion(
    snapshot: &WorldSnapshot,
    building: EntityId,
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
) -> Result<()> {
    validate_furniture_identity(snapshot, building, kind, item, location)?;
    if placement(snapshot, building)?.phase != PlacementPhase::Complete {
        return Err(precondition("furniture installation is not complete"));
    }
    validate_furniture_binding(snapshot, building)
}

fn complete_installation(
    snapshot: &WorldSnapshot,
    building: EntityId,
    attached: &BTreeMap<EntityId, EntityId>,
) -> Result<FurnitureInstallation> {
    let actual = placement(snapshot, building)?;
    if actual.phase != PlacementPhase::Complete {
        return Err(precondition("furniture installation is not complete"));
    }
    if attached.get(&actual.item) != Some(&building) {
        return Err(precondition("furniture attachment is not established"));
    }
    Ok(FurnitureInstallation {
        building,
        kind: actual.kind,
        item: actual.item,
        location: actual.location,
    })
}

/// Inspect one completed installation with the same original generation,
/// producer, material and reciprocal-custody requirements as action proof.
pub fn inspect_complete_furniture(
    snapshot: &WorldSnapshot,
    building: EntityId,
) -> Result<FurnitureInstallation> {
    complete_installation(snapshot, building, &attachments(snapshot)?)
}

/// Strengthen mandatory completed-furniture claims with the identity and
/// custody checks unavailable to field predicates. The caller first establishes
/// the complete terminal predicate as True using its ordinary evidence scope.
/// Only root/nested All claims are mandatory; Any and Not branches add none.
pub fn validate_furniture_goal(snapshot: &WorldSnapshot, terminal: &Predicate) -> Result<()> {
    terminal.validate_shape()?;
    let mut mandatory = BTreeMap::<EntityId, (bool, bool)>::new();
    let mut pending = vec![terminal];
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(children) => pending.extend(children.iter()),
            Predicate::FieldCompare {
                entity_id,
                field,
                op: CompareOp::Eq,
                value: Value::Text(value),
            } => {
                if field == FURNITURE_PLACEMENT_MODEL_FIELD && value == FURNITURE_PLACEMENT_MODEL {
                    mandatory.entry(*entity_id).or_default().0 = true;
                } else if field == CONSTRUCTION_STAGE_FIELD && value == STAGE_COMPLETE {
                    mandatory.entry(*entity_id).or_default().1 = true;
                }
            }
            _ => {}
        }
    }
    let completed = mandatory
        .values()
        .filter(|required| **required == (true, true))
        .count();
    if completed > MAX_FURNITURE_GOAL_BINDINGS {
        return Err(advance_budget_error(
            "furniture goal exceeds 64 mandatory completed installations",
        ));
    }
    if completed == 0 {
        return Ok(());
    }
    // One bounded global custody scan serves all required installations. A
    // large terminal cannot multiply complete-world scans by its claim count.
    let attached = attachments(snapshot)?;
    for (building, required) in mandatory {
        if required == (true, true) {
            complete_installation(snapshot, building, &attached)?;
        }
    }
    Ok(())
}

/// Read the complete eligible loose-item census for one closed kind in canonical
/// identity order. Legacy carpentry items with no custody fields are ineligible;
/// partial or unavailable custody is never interpreted as loose inventory.
pub fn loose_furniture_items(
    snapshot: &WorldSnapshot,
    kind: FurnitureKind,
) -> Result<Vec<EntityId>> {
    bounded_domain(snapshot)?;
    let attached = attachments(snapshot)?;
    let mut loose = Vec::new();
    for record in snapshot.graph.entities.values() {
        if record.kind != EntityKind::Item {
            continue;
        }
        let token = field_text(record, "item_token", snapshot.tick)
            .ok_or_else(|| precondition("furniture inventory contains an unknown item kind"))?;
        if token != kind.item_token() {
            continue;
        }
        reference_item(snapshot, record.id, kind)?;
        if !record.fields.contains_key(FURNITURE_INVENTORY_STATE_FIELD)
            && !record.fields.contains_key(FURNITURE_ALLOCATED_TO_FIELD)
        {
            continue;
        }
        match field_text(record, FURNITURE_INVENTORY_STATE_FIELD, snapshot.tick) {
            Some(LOOSE) => {
                require_value(
                    record,
                    FURNITURE_ALLOCATED_TO_FIELD,
                    &Value::Null,
                    snapshot.tick,
                )?;
                if attached.contains_key(&record.id) {
                    return Err(precondition(
                        "loose furniture still has a building attachment",
                    ));
                }
                loose.push(record.id);
            }
            Some(ALLOCATED | INSTALLED) => {
                let Some(Value::Entity(owner)) =
                    field_value(record, FURNITURE_ALLOCATED_TO_FIELD, snapshot.tick)
                else {
                    return Err(precondition("allocated furniture has no qualified owner"));
                };
                if attached.get(&record.id) != Some(owner) {
                    return Err(precondition("furniture owner has no reciprocal attachment"));
                }
            }
            _ => return Err(precondition("furniture custody is unknown or noncanonical")),
        }
    }
    let ledger = stock_ledger(snapshot)
        .ok_or_else(|| precondition("furniture stock ledger is not established"))?;
    let held = field_u64(
        entity(snapshot, ledger)?,
        furniture_stock_field(kind),
        snapshot.tick,
    )?;
    if held < loose.len() as u64 {
        return Err(precondition(
            "furniture stock is below its established loose-item census",
        ));
    }
    Ok(loose)
}

/// Current occupied footprint, with an explicit cancelled lifecycle proving no
/// remaining occupancy. Released exact-item placements retain validated history.
pub fn occupied_building_footprint(
    snapshot: &WorldSnapshot,
    building: EntityId,
) -> Result<Option<MapCuboid>> {
    let record = entity(snapshot, building)?;
    if record.kind != EntityKind::Building {
        return Err(precondition("occupancy requires an observed building"));
    }
    if has_placement_fields(record) {
        let _identity = placement(snapshot, building)?;
    }
    match field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick) {
        Some(STAGE_CANCELLED) => return Ok(None),
        Some(STAGE_PLANNED | STAGE_UNDER_CONSTRUCTION | STAGE_COMPLETE) => {}
        _ => {
            return Err(precondition(
                "building occupancy has no qualified lifecycle",
            ));
        }
    }
    let (Some(min), Some(max)) = (
        coord_field(record, "footprint_min", snapshot.tick),
        coord_field(record, "footprint_max", snapshot.tick),
    ) else {
        return Err(precondition("building occupancy has an unknown footprint"));
    };
    Ok(Some(MapCuboid::new(min, max)?))
}

fn location_available(
    snapshot: &WorldSnapshot,
    location: MapCoord,
    own_building: Option<EntityId>,
) -> Result<()> {
    bounded_domain(snapshot)?;
    if snapshot.tile_code_at(location) != Some(tile_codes::FLOOR) {
        return Err(precondition(
            "furniture target is not established open floor",
        ));
    }
    for record in snapshot.graph.entities.values() {
        if record.kind != EntityKind::Building || Some(record.id) == own_building {
            continue;
        }
        if occupied_building_footprint(snapshot, record.id)?
            .is_some_and(|area| area.contains(location))
        {
            return Err(precondition(
                "furniture target is already occupied by a building",
            ));
        }
    }
    Ok(())
}

/// Establish the closed single-tile target and complete current building
/// occupancy. This is reference geometry, not native hauling or access proof.
pub fn validate_furniture_location(snapshot: &WorldSnapshot, location: MapCoord) -> Result<()> {
    location_available(snapshot, location, None)
}

pub(super) fn postconditions(
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
    key: &str,
) -> Vec<Predicate> {
    furniture_completion_predicates(&FurnitureInstallation {
        building: created_entity_id(key, 0),
        kind,
        item,
        location,
    })
}

/// Expected completion proof for an exact existing or prospective installation.
/// This preserves the original placement predicate order and canonical bytes;
/// constructing predicates does not establish that the installation exists.
#[must_use]
pub fn furniture_completion_predicates(installation: &FurnitureInstallation) -> Vec<Predicate> {
    let FurnitureInstallation {
        building,
        kind,
        item,
        location,
    } = *installation;
    let mut predicates = vec![
        Predicate::EntityKind {
            entity_id: building,
            kind: EntityKind::Building,
        },
        Predicate::EntityKind {
            entity_id: item,
            kind: EntityKind::Item,
        },
    ];
    for (id, field, value) in [
        (building, "building_kind", Value::Text(kind_label(kind))),
        (building, "position", Value::Coord(location)),
        (building, "footprint_min", Value::Coord(location)),
        (building, "footprint_max", Value::Coord(location)),
        (
            building,
            CONSTRUCTION_STAGE_FIELD,
            Value::Text(STAGE_COMPLETE.to_owned()),
        ),
        (
            building,
            FURNITURE_PLACEMENT_MODEL_FIELD,
            Value::Text(FURNITURE_PLACEMENT_MODEL.to_owned()),
        ),
        (building, FURNITURE_SOURCE_ITEM_FIELD, Value::Entity(item)),
        (
            building,
            FURNITURE_SOURCE_ITEM_GENERATION_FIELD,
            Value::U64(1),
        ),
        (
            building,
            FURNITURE_ATTACHMENT_STATE_FIELD,
            Value::Text(ATTACHED.to_owned()),
        ),
        (
            item,
            FURNITURE_INVENTORY_STATE_FIELD,
            Value::Text(INSTALLED.to_owned()),
        ),
        (item, FURNITURE_ALLOCATED_TO_FIELD, Value::Entity(building)),
        (item, "position", Value::Coord(location)),
        (
            item,
            "item_token",
            Value::Text(kind.item_token().to_owned()),
        ),
        (item, "material_token", Value::Text("WOOD".to_owned())),
        (
            item,
            "material_tokens",
            Value::List(vec![Value::Text("WOOD".to_owned())]),
        ),
        (item, "stack_size", Value::U64(1)),
        (
            item,
            WORK_ORDER_PRODUCTION_MODEL_FIELD,
            Value::Text(CARPENTRY_PRODUCTION_MODEL.to_owned()),
        ),
    ] {
        predicates.push(field_eq(id, field, value));
    }
    predicates
}

pub(super) fn place(
    snapshot: &mut WorldSnapshot,
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
    key: &str,
) -> Result<bool> {
    let building = created_entity_id(key, 0);
    if snapshot.graph.entities.contains_key(&building) {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "the original furniture building already exists",
        ));
    }
    if snapshot.graph.entities.len() >= MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "furniture placement exceeds the canonical entity bound",
        ));
    }
    if loose_furniture_items(snapshot, kind)?
        .binary_search(&item)
        .is_err()
    {
        return Err(precondition(
            "the exact furniture item is not established as loose inventory",
        ));
    }
    validate_furniture_location(snapshot, location)?;
    let ledger = stock_ledger(snapshot)
        .ok_or_else(|| precondition("furniture stock ledger is not established"))?;
    let stock_field = furniture_stock_field(kind);
    let held = field_u64(entity(snapshot, ledger)?, stock_field, snapshot.tick)?;
    let remaining = held
        .checked_sub(1)
        .ok_or_else(|| precondition("furniture placement has no available stock"))?;
    create_entity(
        snapshot,
        building,
        EntityKind::Building,
        kind_label(kind),
        vec![
            ("building_kind".to_owned(), Value::Text(kind_label(kind))),
            ("position".to_owned(), Value::Coord(location)),
            ("footprint_min".to_owned(), Value::Coord(location)),
            ("footprint_max".to_owned(), Value::Coord(location)),
            (
                "material_tokens".to_owned(),
                Value::List(vec![Value::Text("WOOD".to_owned())]),
            ),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_PLANNED.to_owned()),
            ),
            ("progress_ticks".to_owned(), Value::U64(0)),
            ("required_ticks".to_owned(), Value::U64(BUILD_TICKS)),
            (
                FURNITURE_PLACEMENT_MODEL_FIELD.to_owned(),
                Value::Text(FURNITURE_PLACEMENT_MODEL.to_owned()),
            ),
            (FURNITURE_SOURCE_ITEM_FIELD.to_owned(), Value::Entity(item)),
            (
                FURNITURE_SOURCE_ITEM_GENERATION_FIELD.to_owned(),
                Value::U64(1),
            ),
            (
                FURNITURE_ATTACHMENT_STATE_FIELD.to_owned(),
                Value::Text(ATTACHED.to_owned()),
            ),
        ],
    )?;
    write_fields(
        snapshot,
        item,
        vec![
            (
                FURNITURE_INVENTORY_STATE_FIELD.to_owned(),
                Value::Text(ALLOCATED.to_owned()),
            ),
            (
                FURNITURE_ALLOCATED_TO_FIELD.to_owned(),
                Value::Entity(building),
            ),
        ],
    )?;
    write_fields(
        snapshot,
        ledger,
        vec![(stock_field.to_owned(), Value::U64(remaining))],
    )?;
    Ok(true)
}

pub(super) fn cancel(
    snapshot: &mut WorldSnapshot,
    kind: FurnitureKind,
    item: EntityId,
    location: MapCoord,
    key: &str,
) -> Result<bool> {
    let building = created_entity_id(key, 0);
    if !snapshot.graph.entities.contains_key(&building) {
        return Ok(false);
    }
    validate_furniture_identity(snapshot, building, kind, item, location)?;
    let actual = placement(snapshot, building)?;
    if actual.phase == PlacementPhase::Cancelled {
        return Ok(false);
    }
    validate_furniture_binding(snapshot, building)?;
    if actual.phase == PlacementPhase::Complete {
        return Ok(false);
    }
    // The exact attachment is still allocated. Refuse stock contradiction or
    // overflow before returning this same item to the available inventory.
    let _loose = loose_furniture_items(snapshot, kind)?;
    let ledger = stock_ledger(snapshot)
        .ok_or_else(|| precondition("furniture stock ledger is not established"))?;
    let stock_field = furniture_stock_field(kind);
    let refunded = field_u64(entity(snapshot, ledger)?, stock_field, snapshot.tick)?
        .checked_add(1)
        .ok_or_else(|| advance_budget_error("furniture cancellation would overflow stock"))?;
    write_fields(
        snapshot,
        item,
        vec![
            (
                FURNITURE_INVENTORY_STATE_FIELD.to_owned(),
                Value::Text(LOOSE.to_owned()),
            ),
            (FURNITURE_ALLOCATED_TO_FIELD.to_owned(), Value::Null),
        ],
    )?;
    write_fields(
        snapshot,
        building,
        vec![
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_CANCELLED.to_owned()),
            ),
            (
                FURNITURE_ATTACHMENT_STATE_FIELD.to_owned(),
                Value::Text(RELEASED.to_owned()),
            ),
        ],
    )?;
    write_fields(
        snapshot,
        ledger,
        vec![(stock_field.to_owned(), Value::U64(refunded))],
    )?;
    Ok(true)
}

pub(super) fn advance(
    snapshot: &mut WorldSnapshot,
    building: EntityId,
    elapsed: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    bounded_domain(snapshot)?;
    let count = snapshot.graph.entities.len() as u64;
    let depth = usize::BITS - snapshot.graph.entities.len().leading_zeros();
    // Global attachment and occupancy scans include tree lookups of each
    // referenced original order/item. Reserve their work before either scan.
    budget.charge(
        count
            .saturating_mul(24 + u64::from(depth) * 8)
            .saturating_add(128),
    )?;
    let actual = placement(snapshot, building)?;
    if actual.phase != PlacementPhase::Active {
        return Ok(false);
    }
    validate_furniture_binding(snapshot, building)?;
    location_available(snapshot, actual.location, Some(building))?;
    let progress = actual
        .progress
        .checked_add(elapsed)
        .ok_or_else(|| advance_budget_error("furniture progress overflowed"))?;
    if progress > BUILD_TICKS {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "furniture interval crossed completion",
        ));
    }
    let complete = progress == BUILD_TICKS;
    let mut changed = false;
    if complete {
        changed |= write_fields(
            snapshot,
            actual.item,
            vec![
                (
                    FURNITURE_INVENTORY_STATE_FIELD.to_owned(),
                    Value::Text(INSTALLED.to_owned()),
                ),
                ("position".to_owned(), Value::Coord(actual.location)),
            ],
        )?;
    }
    changed |= write_fields(
        snapshot,
        building,
        vec![
            ("progress_ticks".to_owned(), Value::U64(progress)),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(
                    if complete {
                        STAGE_COMPLETE
                    } else {
                        STAGE_UNDER_CONSTRUCTION
                    }
                    .to_owned(),
                ),
            ),
        ],
    )?;
    Ok(changed)
}
