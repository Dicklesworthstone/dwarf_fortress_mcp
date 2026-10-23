//! Closed material-backed reference carpentry. These are laboratory inventory
//! transitions, not native item IDs, material proofs, or DFHack throughput.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct Recipe {
    pub(super) item_token: &'static str,
    pub(super) stock_field: &'static str,
}

pub(super) fn recipe(job: &str) -> Option<Recipe> {
    let (item_token, stock_field) = match job {
        "CONSTRUCT_BED" | "wooden_bed" => ("BED", STOCK_BED_FIELD),
        "CONSTRUCT_DOOR" | "wooden_door" => ("DOOR", STOCK_DOOR_FIELD),
        "CONSTRUCT_TABLE" | "wooden_table" => ("TABLE", STOCK_TABLE_FIELD),
        "CONSTRUCT_THRONE" | "CONSTRUCT_CHAIR" | "wooden_chair" => ("CHAIR", STOCK_CHAIR_FIELD),
        _ => return None,
    };
    Some(Recipe {
        item_token,
        stock_field,
    })
}

/// Missing inventory is unknown even when the intended baseline would be zero.
/// Require the producer to establish both the input and output counts, and the
/// original order to opt into the material-backed generation explicitly.
pub(super) fn blocker(snapshot: &WorldSnapshot, order: &TimelineOrder) -> Result<Option<String>> {
    let Some(recipe) = recipe(&order.job) else {
        return Ok(None);
    };
    let record = entity(snapshot, order.id)?;
    if field_text(record, WORK_ORDER_PRODUCTION_MODEL_FIELD, snapshot.tick)
        != Some(CARPENTRY_PRODUCTION_MODEL)
    {
        return Ok(Some(
            "carpentry production model is not established; legacy abstract orders cannot create inventory history"
                .to_owned(),
        ));
    }
    let total = field_u64(record, "amount_total", snapshot.tick)?;
    let remaining = field_u64(record, AMOUNT_REMAINING_FIELD, snapshot.tick)?;
    if record.generation != 1 || total == 0 || total > u64::from(u32::MAX) || remaining > total {
        return Err(precondition(
            "carpentry requires its original order generation and canonical finite total",
        ));
    }
    let Some(ledger) = stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id))
    else {
        return Ok(Some(
            "carpentry input and output stock ledger is not established".to_owned(),
        ));
    };
    for field in [STOCK_WOOD_FIELD, recipe.stock_field] {
        if !matches!(
            field_value(ledger, field, snapshot.tick),
            Some(Value::U64(_))
        ) {
            return Ok(Some(format!(
                "carpentry inventory count is not established in {field}"
            )));
        }
    }
    if field_value(ledger, STOCK_WOOD_FIELD, snapshot.tick) == Some(&Value::U64(0)) {
        return Ok(Some(
            "carpentry requires one available WOOD log per completed furniture item".to_owned(),
        ));
    }
    Ok(None)
}

/// Consume one log and publish one exact new item. The timeline separately
/// increments the eligible aggregate output stock and finishes the order unit;
/// all three changes belong to its caller's single transaction shadow.
pub(super) fn complete(
    snapshot: &mut WorldSnapshot,
    order: &TimelineOrder,
    remaining_after: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    let recipe = recipe(&order.job).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "unregistered carpentry output",
        )
    })?;
    if snapshot.graph.entities.len() >= MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "carpentry output would exceed the bounded canonical production entity domain",
        ));
    }
    let depth = usize::BITS - snapshot.graph.entities.len().leading_zeros();
    budget.charge(snapshot.graph.entities.len() as u64 + 64 + u64::from(depth) * 4)?;
    let record = entity(snapshot, order.id)?;
    let total = field_u64(record, "amount_total", snapshot.tick)?;
    let ordinal = total
        .checked_sub(remaining_after)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or_else(|| precondition("carpentry output ordinal is not established"))?;
    let output = produced_item_id(order.id, ordinal);
    if snapshot.graph.entities.contains_key(&output) {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "carpentry output identity already exists; refuse to recreate or consume material twice",
        ));
    }
    let ledger = stock_ledger(snapshot).ok_or_else(|| {
        precondition("carpentry input and output stock ledger is not established")
    })?;
    let logs = field_u64(entity(snapshot, ledger)?, STOCK_WOOD_FIELD, snapshot.tick)?;
    let logs_after = logs
        .checked_sub(1)
        .ok_or_else(|| precondition("carpentry completion has no available WOOD log"))?;
    // Refuse an impossible output before consuming input or creating its item.
    field_u64(entity(snapshot, ledger)?, recipe.stock_field, snapshot.tick)?
        .checked_add(1)
        .ok_or_else(|| advance_budget_error("carpentry would overflow its output stock counter"))?;
    write_fields(
        snapshot,
        ledger,
        vec![(STOCK_WOOD_FIELD.to_owned(), Value::U64(logs_after))],
    )?;
    create_entity(
        snapshot,
        output,
        EntityKind::Item,
        format!("wooden {}", recipe.item_token.to_ascii_lowercase()),
        vec![
            (
                "item_token".to_owned(),
                Value::Text(recipe.item_token.to_owned()),
            ),
            ("material_token".to_owned(), Value::Text("WOOD".to_owned())),
            (
                "material_tokens".to_owned(),
                Value::List(vec![Value::Text("WOOD".to_owned())]),
            ),
            ("stack_size".to_owned(), Value::U64(1)),
            ("source_order".to_owned(), Value::Entity(order.id)),
            (
                "production_ordinal".to_owned(),
                Value::U64(u64::from(ordinal)),
            ),
            (
                WORK_ORDER_PRODUCTION_MODEL_FIELD.to_owned(),
                Value::Text(CARPENTRY_PRODUCTION_MODEL.to_owned()),
            ),
        ],
    )?;
    Ok(true)
}
