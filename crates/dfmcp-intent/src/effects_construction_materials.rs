//! Explicit finite input custody for the laboratory's closed wooden workshops.
//!
//! A dispatch removes one log from available inventory and records its held
//! custody on the original building. Completion consumes that held input;
//! unfinished cancellation returns it once. This is aggregate laboratory WOOD,
//! not a claim about an individual native item or a native construction recipe.

use super::*;
use crate::action::MaterialSelector;

pub const LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD: &str =
    "laboratory_construction_material_model";
pub const LABORATORY_CONSTRUCTION_MATERIAL_MODEL: &str = "wooden-workshops/1";
pub const CONSTRUCTION_MATERIAL_MODEL_FIELD: &str = "construction_material_model";
pub const CONSTRUCTION_MATERIAL_LEDGER_FIELD: &str = "construction_material_ledger";
pub const CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD: &str =
    "construction_material_ledger_generation";
pub const CONSTRUCTION_MATERIAL_WOOD_FIELD: &str = "construction_material_wood";
pub const CONSTRUCTION_MATERIAL_STATE_FIELD: &str = "construction_material_state";
pub const CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD: &str = "construction_material_forbidden_tokens";
pub const CONSTRUCTION_MATERIAL_NEAREST_FIELD: &str = "construction_material_prefer_nearest";
pub const CONSTRUCTION_MATERIAL_RESERVE_FIELD: &str = "construction_material_reserve_count";

const HELD: &str = "held";
const CONSUMED: &str = "consumed";
const RELEASED: &str = "released";
const MAX_FOOTPRINT_TILES: u64 = 64;
const MAX_SELECTOR_TOKENS: usize = 64;
const TASK_FIELDS: [&str; 8] = [
    CONSTRUCTION_MATERIAL_MODEL_FIELD,
    CONSTRUCTION_MATERIAL_LEDGER_FIELD,
    CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD,
    CONSTRUCTION_MATERIAL_WOOD_FIELD,
    CONSTRUCTION_MATERIAL_STATE_FIELD,
    CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
    CONSTRUCTION_MATERIAL_NEAREST_FIELD,
    CONSTRUCTION_MATERIAL_RESERVE_FIELD,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionMaterialAllocation {
    pub building: EntityId,
    pub building_kind: String,
    pub wood_units: u64,
    pub state: &'static str,
}

/// Current available inventory and exact retained allocation histories.
/// Released input is already back in available inventory, so these historical
/// categories must not be summed as an additional source of usable stock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionMaterialReport {
    pub model: &'static str,
    pub ledger: EntityId,
    pub ledger_generation: u64,
    pub available_wood: u64,
    pub held_wood: u64,
    pub consumed_wood: u64,
    pub released_wood: u64,
    pub allocations: Vec<ConstructionMaterialAllocation>,
}

pub(super) fn has_material_fields(record: &EntityRecord) -> bool {
    TASK_FIELDS
        .iter()
        .any(|field| record.fields.contains_key(*field))
}

fn recipe(label: &str) -> bool {
    matches!(
        label,
        "workshop:Still" | "workshop:Kitchen" | "workshop:Carpenters"
    )
}

fn require_value(
    record: &EntityRecord,
    field: &str,
    expected: &Value,
    tick: GameTick,
) -> Result<()> {
    if field_value(record, field, tick) != Some(expected) {
        return Err(precondition(format!(
            "construction material custody requires qualified {field} on entity {}",
            record.id.get(),
        )));
    }
    Ok(())
}

fn selector_tokens(record: &EntityRecord, field: &str, tick: GameTick) -> Result<BTreeSet<String>> {
    let Some(Value::List(values)) = field_value(record, field, tick) else {
        return Err(precondition(
            "construction material selector is not a qualified token list",
        ));
    };
    if values.len() > MAX_SELECTOR_TOKENS {
        return Err(advance_budget_error(
            "construction material selector exceeds its token bound",
        ));
    }
    let mut result = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for value in values {
        let Value::Text(token) = value else {
            return Err(precondition(
                "construction material selector contains a non-text token",
            ));
        };
        validate_condition_text(token)?;
        if previous.is_some_and(|prior| prior >= token.as_str()) {
            return Err(precondition(
                "construction material selector is not in unique canonical order",
            ));
        }
        previous = Some(token);
        result.insert(token.clone());
    }
    Ok(result)
}

fn validate_selector(material: &MaterialSelector) -> Result<()> {
    if material.required_tokens.len() > MAX_SELECTOR_TOKENS
        || material.forbidden_tokens.len() > MAX_SELECTOR_TOKENS
    {
        return Err(advance_budget_error(
            "construction material selector exceeds its token bound",
        ));
    }
    for token in material
        .required_tokens
        .iter()
        .chain(&material.forbidden_tokens)
    {
        validate_condition_text(token)?;
    }
    if material.required_tokens.iter().any(|token| token != "WOOD")
        || material.forbidden_tokens.contains("WOOD")
        || material.prefer_nearest
        || material.reserve_count != 0
    {
        return Err(precondition(
            "wooden-workshops/1 requires empty or WOOD-only required tokens, WOOD not forbidden, prefer_nearest=false and reserve_count=0",
        ));
    }
    Ok(())
}

fn validate_geometry(location: MapCoord, footprint: MapCuboid) -> Result<()> {
    let tiles = validate_region(footprint)?;
    if !footprint.contains(location)
        || footprint.min.z != footprint.max.z
        || tiles > MAX_FOOTPRINT_TILES
    {
        return Err(precondition(
            "wooden-workshops/1 requires a single-level footprint of at most 64 tiles containing its location",
        ));
    }
    Ok(())
}

fn validate_action(action: &Action) -> Result<()> {
    let Action::Build {
        kind,
        location,
        footprint,
        material,
    } = action
    else {
        return Err(precondition(
            "construction material custody requires a Build action",
        ));
    };
    if !recipe(&building_kind_label(kind)) {
        return Err(precondition(
            "wooden-workshops/1 supports only Still, Kitchen and Carpenters workshop Build actions; exact furniture uses PlaceFurniture",
        ));
    }
    validate_geometry(*location, *footprint)?;
    validate_selector(material)
}

fn allocation(
    snapshot: &WorldSnapshot,
    record: &EntityRecord,
    ledger: EntityId,
    ledger_generation: u64,
) -> Result<ConstructionMaterialAllocation> {
    if record.kind != EntityKind::Building
        || record.generation != 1
        || furniture::has_placement_fields(record)
    {
        return Err(precondition(
            "construction material custody requires its original workshop building generation",
        ));
    }
    let label = field_text(record, "building_kind", snapshot.tick)
        .filter(|label| recipe(label))
        .ok_or_else(|| {
            precondition("construction material custody has no qualified closed workshop recipe")
        })?;
    for (field, value) in [
        (
            CONSTRUCTION_MATERIAL_MODEL_FIELD,
            Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
        ),
        (CONSTRUCTION_MATERIAL_LEDGER_FIELD, Value::Entity(ledger)),
        (
            CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD,
            Value::U64(ledger_generation),
        ),
        (CONSTRUCTION_MATERIAL_WOOD_FIELD, Value::U64(1)),
        (CONSTRUCTION_MATERIAL_NEAREST_FIELD, Value::Bool(false)),
        (CONSTRUCTION_MATERIAL_RESERVE_FIELD, Value::U64(0)),
        (
            LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        ),
        ("required_ticks", Value::U64(BUILD_TICKS)),
    ] {
        require_value(record, field, &value, snapshot.tick)?;
    }
    let selector = MaterialSelector {
        required_tokens: selector_tokens(record, "material_tokens", snapshot.tick)?,
        forbidden_tokens: selector_tokens(
            record,
            CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
            snapshot.tick,
        )?,
        prefer_nearest: false,
        reserve_count: 0,
    };
    validate_selector(&selector)?;
    let location = coord_field(record, "position", snapshot.tick)
        .ok_or_else(|| precondition("construction material custody has no qualified location"))?;
    let min = coord_field(record, "footprint_min", snapshot.tick)
        .ok_or_else(|| precondition("construction material custody has no qualified footprint"))?;
    let max = coord_field(record, "footprint_max", snapshot.tick)
        .ok_or_else(|| precondition("construction material custody has no qualified footprint"))?;
    validate_geometry(location, MapCuboid::new(min, max)?)?;
    let progress = field_u64(record, "progress_ticks", snapshot.tick)?;
    let state = match field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick) {
        Some(STAGE_PLANNED) if progress == 0 => HELD,
        Some(STAGE_UNDER_CONSTRUCTION) if progress > 0 && progress < BUILD_TICKS => HELD,
        Some(STAGE_COMPLETE) if progress == BUILD_TICKS => CONSUMED,
        Some(STAGE_CANCELLED) if progress < BUILD_TICKS => RELEASED,
        _ => {
            return Err(precondition(
                "construction material custody has a noncanonical progress lifecycle",
            ));
        }
    };
    require_value(
        record,
        CONSTRUCTION_MATERIAL_STATE_FIELD,
        &Value::Text(state.to_owned()),
        snapshot.tick,
    )?;
    Ok(ConstructionMaterialAllocation {
        building: record.id,
        building_kind: label.to_owned(),
        wood_units: 1,
        state,
    })
}

fn capture(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<Option<ConstructionMaterialReport>> {
    let count = snapshot.graph.entities.len() as u64;
    budget.charge(
        count
            .saturating_mul(10)
            .saturating_add(snapshot.graph.edges.len() as u64 * 10)
            .saturating_add(1),
    )?;
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    let mut declared = false;
    let mut stamped = false;
    for record in snapshot.graph.edges.values() {
        if record
            .fields
            .contains_key(LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD)
            || TASK_FIELDS
                .iter()
                .any(|field| record.fields.contains_key(*field))
        {
            return Err(precondition(
                "construction material models and custody require canonical entity records",
            ));
        }
    }
    for (id, record) in &snapshot.graph.entities {
        if record
            .fields
            .contains_key(LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD)
        {
            if *id != settings
                || record.id != *id
                || record.generation != 1
                || record.kind != EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned())
                || field_text(
                    record,
                    LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                    snapshot.tick,
                ) != Some(LABORATORY_CONSTRUCTION_MATERIAL_MODEL)
            {
                return Err(precondition(
                    "construction material model requires a qualified supported declaration on the canonical fortress settings identity",
                ));
            }
            declared = true;
        }
        stamped |= has_material_fields(record);
    }
    if !declared {
        return if stamped {
            Err(precondition(
                "construction material model is missing while allocation history remains; downgrade is refused",
            ))
        } else {
            Ok(None)
        };
    }
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "construction material census exceeds its canonical entity bound",
        ));
    }
    budget.charge(count.saturating_mul(32).saturating_add(1))?;
    if laboratory_workforce_model(snapshot)? != Some(LABORATORY_WORKFORCE_MODEL) {
        return Err(precondition(
            "wooden-workshops/1 requires the qualified shared-workforce/1 model",
        ));
    }
    let mut ledgers = snapshot.graph.entities.iter().filter(
        |(_, record)| matches!(&record.kind, EntityKind::Other(kind) if kind == STOCK_LEDGER_KIND),
    );
    let (ledger, ledger_record) = ledgers.next().ok_or_else(|| {
        precondition("construction material inventory requires one qualified stock ledger")
    })?;
    if ledgers.next().is_some() || ledger_record.id != *ledger || ledger_record.generation != 1 {
        return Err(precondition(
            "construction material inventory requires one unambiguous original stock ledger identity",
        ));
    }
    let mut report = ConstructionMaterialReport {
        model: LABORATORY_CONSTRUCTION_MATERIAL_MODEL,
        ledger: *ledger,
        ledger_generation: u64::from(ledger_record.generation),
        available_wood: field_u64(ledger_record, STOCK_WOOD_FIELD, snapshot.tick)?,
        held_wood: 0,
        consumed_wood: 0,
        released_wood: 0,
        allocations: Vec::new(),
    };
    for (id, record) in &snapshot.graph.entities {
        if record.id != *id {
            return Err(precondition(
                "construction material source has a mismatched entity identity",
            ));
        }
        if has_material_fields(record) {
            let allocation = allocation(snapshot, record, report.ledger, report.ledger_generation)?;
            let sum = match allocation.state {
                HELD => &mut report.held_wood,
                CONSUMED => &mut report.consumed_wood,
                RELEASED => &mut report.released_wood,
                _ => {
                    return Err(DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "construction material lifecycle escaped its closed model",
                    ));
                }
            };
            *sum = sum.checked_add(allocation.wood_units).ok_or_else(|| {
                advance_budget_error("construction material accounting overflowed")
            })?;
            report.allocations.push(allocation);
        } else if record.kind == EntityKind::Building
            && active_building(snapshot, *id)?
            && !furniture::has_placement_fields(record)
        {
            return Err(precondition(
                "active construction lacks its explicit material custody; legacy work cannot acquire material history",
            ));
        }
    }
    Ok(Some(report))
}

pub(super) fn validate_model(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<()> {
    capture(snapshot, budget).map(|_| ())
}

pub fn construction_material_model(snapshot: &WorldSnapshot) -> Result<Option<&'static str>> {
    Ok(construction_material_report(snapshot)?.map(|report| report.model))
}

pub fn construction_material_report(
    snapshot: &WorldSnapshot,
) -> Result<Option<ConstructionMaterialReport>> {
    let mut budget = EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?;
    capture(snapshot, &mut budget)
}

/// Validate a proposed build's closed recipe without allocating any input.
/// No-model worlds and actions other than Build retain their old interpretation.
pub fn construction_wood_cost(snapshot: &WorldSnapshot, action: &Action) -> Result<Option<u64>> {
    if construction_material_model(snapshot)?.is_none() || !matches!(action, Action::Build { .. }) {
        return Ok(None);
    }
    validate_action(action)?;
    Ok(Some(1))
}

fn tokens(values: &BTreeSet<String>) -> Value {
    Value::List(values.iter().cloned().map(Value::Text).collect())
}

/// Original-generation and full selector/geometry validation for physical work
/// inspection, refunds and current completion proof. No state is changed.
pub fn validate_construction_material_identity(
    snapshot: &WorldSnapshot,
    building: EntityId,
    action: &Action,
) -> Result<()> {
    if !matches!(action, Action::Build { .. }) {
        return Ok(());
    }
    let Some(report) = construction_material_report(snapshot)? else {
        return Ok(());
    };
    validate_action(action)?;
    let record = entity(snapshot, building)?;
    allocation(snapshot, record, report.ledger, report.ledger_generation)?;
    let Action::Build {
        kind,
        location,
        footprint,
        material,
    } = action
    else {
        return Err(precondition(
            "construction material identity requires a Build action",
        ));
    };
    for (field, value) in [
        ("building_kind", Value::Text(building_kind_label(kind))),
        ("position", Value::Coord(*location)),
        ("footprint_min", Value::Coord(footprint.min)),
        ("footprint_max", Value::Coord(footprint.max)),
        ("material_tokens", tokens(&material.required_tokens)),
        (
            CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
            tokens(&material.forbidden_tokens),
        ),
        (
            CONSTRUCTION_MATERIAL_NEAREST_FIELD,
            Value::Bool(material.prefer_nearest),
        ),
        (
            CONSTRUCTION_MATERIAL_RESERVE_FIELD,
            Value::U64(u64::from(material.reserve_count)),
        ),
    ] {
        require_value(record, field, &value, snapshot.tick)?;
    }
    Ok(())
}

pub fn validate_construction_material_completion(
    snapshot: &WorldSnapshot,
    building: EntityId,
    action: &Action,
) -> Result<()> {
    if !matches!(action, Action::Build { .. }) || construction_material_model(snapshot)?.is_none() {
        return Ok(());
    }
    validate_construction_material_identity(snapshot, building, action)?;
    let record = entity(snapshot, building)?;
    require_value(
        record,
        CONSTRUCTION_MATERIAL_STATE_FIELD,
        &Value::Text(CONSUMED.to_owned()),
        snapshot.tick,
    )
}

/// Additional proof for new-model builds; the legacy default proof and action
/// encoding remain unchanged. Generation and complete custody census validation
/// also run at proof publication, since field predicates cannot express them.
pub fn construction_completion_predicates(
    snapshot: &WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Vec<Predicate>> {
    if !matches!(action, Action::Build { .. }) {
        return Ok(Vec::new());
    }
    let Some(report) = construction_material_report(snapshot)? else {
        return Ok(Vec::new());
    };
    validate_action(action)?;
    let Action::Build {
        kind,
        location,
        footprint,
        material,
    } = action
    else {
        return Err(precondition(
            "construction material proof requires a Build action",
        ));
    };
    let building = created_entity_id(key, 0);
    let mut result = vec![Predicate::EntityKind {
        entity_id: building,
        kind: EntityKind::Building,
    }];
    for (field, value) in [
        ("building_kind", Value::Text(building_kind_label(kind))),
        ("position", Value::Coord(*location)),
        ("footprint_min", Value::Coord(footprint.min)),
        ("footprint_max", Value::Coord(footprint.max)),
        ("material_tokens", tokens(&material.required_tokens)),
        (
            CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
            tokens(&material.forbidden_tokens),
        ),
        (CONSTRUCTION_MATERIAL_NEAREST_FIELD, Value::Bool(false)),
        (CONSTRUCTION_MATERIAL_RESERVE_FIELD, Value::U64(0)),
        (
            CONSTRUCTION_MATERIAL_MODEL_FIELD,
            Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
        ),
        (
            CONSTRUCTION_MATERIAL_LEDGER_FIELD,
            Value::Entity(report.ledger),
        ),
        (
            CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD,
            Value::U64(report.ledger_generation),
        ),
        (CONSTRUCTION_MATERIAL_WOOD_FIELD, Value::U64(1)),
        (
            CONSTRUCTION_MATERIAL_STATE_FIELD,
            Value::Text(CONSUMED.to_owned()),
        ),
        (
            CONSTRUCTION_STAGE_FIELD,
            Value::Text(STAGE_COMPLETE.to_owned()),
        ),
        ("required_ticks", Value::U64(BUILD_TICKS)),
        ("progress_ticks", Value::U64(BUILD_TICKS)),
    ] {
        result.push(field_eq(building, field, value));
    }
    Ok(result)
}

fn location_available(
    snapshot: &WorldSnapshot,
    footprint: MapCuboid,
    report: &ConstructionMaterialReport,
) -> Result<()> {
    for coord in region_tiles(footprint) {
        if snapshot.tile_code_at(coord) != Some(tile_codes::FLOOR) {
            return Err(precondition(
                "wooden workshop footprint requires observed open floor",
            ));
        }
    }
    let released: BTreeSet<EntityId> = report
        .allocations
        .iter()
        .filter(|allocation| allocation.state == RELEASED)
        .map(|allocation| allocation.building)
        .collect();
    for record in snapshot.graph.entities.values() {
        if record.kind.as_str() != "building" || released.contains(&record.id) {
            continue;
        }
        let occupied = if record.kind == EntityKind::Building {
            occupied_building_footprint(snapshot, record.id)?
        } else {
            // Canonical kind aliases cannot hide physical occupancy. Their
            // qualified extent is sufficient to exclude a distant footprint;
            // absence of a registered lifecycle never certifies vacant ground.
            let min = coord_field(record, "footprint_min", snapshot.tick)
                .ok_or_else(|| precondition("aliased building has no qualified footprint"))?;
            let max = coord_field(record, "footprint_max", snapshot.tick)
                .ok_or_else(|| precondition("aliased building has no qualified footprint"))?;
            Some(MapCuboid::new(min, max)?)
        };
        let occupied = if let Some(area) = occupied {
            Some(area)
        } else if furniture::has_placement_fields(record) {
            validate_furniture_binding(snapshot, record.id)?;
            None
        } else {
            // Unmarked cancelled construction has no proved material release.
            // It remains conservatively occupied in this distinct new model.
            let min = coord_field(record, "footprint_min", snapshot.tick)
                .ok_or_else(|| precondition("existing construction has no qualified footprint"))?;
            let max = coord_field(record, "footprint_max", snapshot.tick)
                .ok_or_else(|| precondition("existing construction has no qualified footprint"))?;
            Some(MapCuboid::new(min, max)?)
        };
        if occupied.is_some_and(|area| {
            area.min.x <= footprint.max.x
                && footprint.min.x <= area.max.x
                && area.min.y <= footprint.max.y
                && footprint.min.y <= area.max.y
                && area.min.z <= footprint.max.z
                && footprint.min.z <= area.max.z
        }) {
            return Err(precondition(
                "wooden workshop footprint overlaps an occupied building",
            ));
        }
    }
    Ok(())
}

pub(super) fn build(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Option<bool>> {
    let Some(report) = construction_material_report(snapshot)? else {
        return Ok(None);
    };
    validate_action(action)?;
    let building = created_entity_id(key, 0);
    if snapshot.graph.entities.contains_key(&building) {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "the original material-backed workshop already exists",
        ));
    }
    if snapshot.graph.entities.len() >= MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "workshop allocation would exceed the canonical entity bound",
        ));
    }
    let available = report.available_wood.checked_sub(1).ok_or_else(|| {
        precondition("wooden workshop construction requires one available WOOD log")
    })?;
    let Action::Build {
        kind,
        location,
        footprint,
        material,
    } = action
    else {
        return Err(precondition(
            "construction material dispatch requires a Build action",
        ));
    };
    location_available(snapshot, *footprint, &report)?;
    let label = building_kind_label(kind);
    create_entity(
        snapshot,
        building,
        EntityKind::Building,
        label.clone(),
        vec![
            ("building_kind".to_owned(), Value::Text(label)),
            ("position".to_owned(), Value::Coord(*location)),
            ("footprint_min".to_owned(), Value::Coord(footprint.min)),
            ("footprint_max".to_owned(), Value::Coord(footprint.max)),
            (
                "material_tokens".to_owned(),
                tokens(&material.required_tokens),
            ),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_PLANNED.to_owned()),
            ),
            ("progress_ticks".to_owned(), Value::U64(0)),
            ("required_ticks".to_owned(), Value::U64(BUILD_TICKS)),
            (
                CONSTRUCTION_MATERIAL_MODEL_FIELD.to_owned(),
                Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
            ),
            (
                CONSTRUCTION_MATERIAL_LEDGER_FIELD.to_owned(),
                Value::Entity(report.ledger),
            ),
            (
                CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD.to_owned(),
                Value::U64(report.ledger_generation),
            ),
            (CONSTRUCTION_MATERIAL_WOOD_FIELD.to_owned(), Value::U64(1)),
            (
                CONSTRUCTION_MATERIAL_STATE_FIELD.to_owned(),
                Value::Text(HELD.to_owned()),
            ),
            (
                CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD.to_owned(),
                tokens(&material.forbidden_tokens),
            ),
            (
                CONSTRUCTION_MATERIAL_NEAREST_FIELD.to_owned(),
                Value::Bool(false),
            ),
            (
                CONSTRUCTION_MATERIAL_RESERVE_FIELD.to_owned(),
                Value::U64(0),
            ),
        ],
    )?;
    write_fields(
        snapshot,
        report.ledger,
        vec![(STOCK_WOOD_FIELD.to_owned(), Value::U64(available))],
    )?;
    Ok(Some(true))
}

pub(super) fn cancel(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Option<bool>> {
    let Some(report) = construction_material_report(snapshot)? else {
        return Ok(None);
    };
    let building = created_entity_id(key, 0);
    if !snapshot.graph.entities.contains_key(&building) {
        return Ok(Some(false));
    }
    validate_construction_material_identity(snapshot, building, action)?;
    let record = entity(snapshot, building)?;
    let allocation = allocation(snapshot, record, report.ledger, report.ledger_generation)?;
    if allocation.state != HELD {
        return Ok(Some(false));
    }
    let refunded = report.available_wood.checked_add(1).ok_or_else(|| {
        advance_budget_error("workshop cancellation would overflow available WOOD")
    })?;
    write_fields(
        snapshot,
        building,
        vec![
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_CANCELLED.to_owned()),
            ),
            (
                CONSTRUCTION_MATERIAL_STATE_FIELD.to_owned(),
                Value::Text(RELEASED.to_owned()),
            ),
            (BLOCKED_BY_FIELD.to_owned(), Value::Null),
        ],
    )?;
    write_fields(
        snapshot,
        report.ledger,
        vec![(STOCK_WOOD_FIELD.to_owned(), Value::U64(refunded))],
    )?;
    Ok(Some(true))
}

pub(super) fn advance(
    snapshot: &mut WorldSnapshot,
    building: EntityId,
    elapsed: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    budget.charge(32)?;
    let record = entity(snapshot, building)?;
    let Some(Value::Entity(ledger)) =
        field_value(record, CONSTRUCTION_MATERIAL_LEDGER_FIELD, snapshot.tick)
    else {
        return Err(precondition(
            "workshop progress has no qualified material ledger identity",
        ));
    };
    let ledger_record = entity(snapshot, *ledger)?;
    if !matches!(&ledger_record.kind, EntityKind::Other(kind) if kind == STOCK_LEDGER_KIND) {
        return Err(precondition(
            "workshop progress has an incompatible material ledger",
        ));
    }
    field_u64(ledger_record, STOCK_WOOD_FIELD, snapshot.tick)?;
    let allocation = allocation(
        snapshot,
        record,
        *ledger,
        u64::from(ledger_record.generation),
    )?;
    if allocation.state != HELD {
        return Err(precondition(
            "workshop progress requires held construction material",
        ));
    }
    let progress = field_u64(record, "progress_ticks", snapshot.tick)?
        .checked_add(elapsed)
        .filter(|progress| *progress <= BUILD_TICKS)
        .ok_or_else(|| precondition("workshop progress crossed its completion boundary"))?;
    write_fields(
        snapshot,
        building,
        vec![
            ("progress_ticks".to_owned(), Value::U64(progress)),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(
                    if progress == BUILD_TICKS {
                        STAGE_COMPLETE
                    } else {
                        STAGE_UNDER_CONSTRUCTION
                    }
                    .to_owned(),
                ),
            ),
            (
                CONSTRUCTION_MATERIAL_STATE_FIELD.to_owned(),
                Value::Text(
                    if progress == BUILD_TICKS {
                        CONSUMED
                    } else {
                        HELD
                    }
                    .to_owned(),
                ),
            ),
        ],
    )
}
