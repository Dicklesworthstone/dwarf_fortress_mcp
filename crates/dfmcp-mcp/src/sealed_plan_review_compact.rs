//! Lossless table rows for complete readable proofs under output pressure.
//!
//! Only registered predicate positions interpret integer references. A field
//! value, source request, entity ID, coordinate or unknown response field is
//! literal data. Conversion restores the original shared projection before
//! the existing bounded graph validator or later-turn cache uses it.

use super::{MAX_EXPANDED_BYTES, MAX_NODES, MAX_STEPS, PREFIX, SCHEMA, bounded, invalid};
use dfmcp_core::Result;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

const COMPACT_SCHEMA: &str = "dfmcp.plan-predicate-references/2";
const REFERENCE: &str = "zero_based_predicate_index_or_boolean";
const ROW_FORMAT: &str = "kind_then_values_in_declared_column_order";

pub(super) fn is_compact(payload: &Value) -> bool {
    payload["predicate_encoding"]["schema"] == COMPACT_SCHEMA
}

/// Column names are part of a closed renderer contract, not instructions
/// supplied by a caller. Compound/scalar nodes keep one argument; object leaf
/// nodes flatten only their declared keys. Everything in a leaf stays literal.
fn columns(kind: &str) -> Result<&'static [&'static str]> {
    match kind {
        "all" | "any" => Ok(&["children"]),
        "not" => Ok(&["child"]),
        "entity_exists" => Ok(&["entity_id"]),
        "paused" => Ok(&["paused"]),
        "entity_kind" => Ok(&["entity_id", "kind"]),
        "field" => Ok(&["entity_id", "field", "op", "value"]),
        "entity_quantity" => Ok(&[
            "kind",
            "filter_field",
            "filter_value",
            "quantity_field",
            "op",
            "value",
        ]),
        "entity_quantity_increase" => Ok(&[
            "kind",
            "filter_field",
            "filter_value",
            "quantity_field",
            "baseline",
            "increase",
            "target",
            "baseline_anchor",
            "baseline_source_digest",
            "source_generation",
            "baseline_source_counters",
            "meaning",
        ]),
        "edge_exists" => Ok(&["edge_id", "kind"]),
        "region_terrain" => Ok(&["min", "max", "tile"]),
        "laboratory_room_access" | "laboratory_room_access_paired_stairs" => {
            Ok(&["policy", "region", "entry", "targets"])
        }
        "laboratory_civilian_release_safety" => Ok(&["policy", "burrow", "civilians"]),
        _ => Err(invalid(
            "compact proof contains an unregistered predicate kind",
        )),
    }
}

fn scalar(kind: &str) -> bool {
    matches!(kind, "entity_exists" | "paused")
}

fn reference(value: &Value, upper: usize, compress: bool) -> Result<Value> {
    if value.is_boolean() {
        return Ok(value.clone());
    }
    let index = if compress {
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or_else(|| invalid("compact proof requires a canonical predicate reference"))?;
        let raw = object
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.strip_prefix(PREFIX))
            .ok_or_else(|| {
                invalid("compact proof reference is not local to its predicate table")
            })?;
        raw.parse::<usize>()
            .ok()
            .filter(|index| index.to_string() == raw && *index < upper)
    } else {
        value
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < upper)
    }
    .ok_or_else(|| invalid("compact proof reference is noncanonical, dangling or cyclic"))?;
    Ok(if compress {
        json!(index)
    } else {
        json!({"$ref":format!("{PREFIX}{index}")})
    })
}

fn children(value: &Value, upper: usize, compress: bool) -> Result<Value> {
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAX_NODES)
        .ok_or_else(|| invalid("compact Boolean proof requires a bounded child array"))?;
    values
        .iter()
        .map(|value| reference(value, upper, compress))
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}

fn compact_row(value: &Value, upper: usize) -> Result<(String, Value)> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| invalid("compact proof requires one registered node per table row"))?;
    let (kind, value) = object
        .iter()
        .next()
        .ok_or_else(|| invalid("compact proof cannot encode an empty node"))?;
    let fields = columns(kind)?;
    let mut row = vec![json!(kind)];
    match kind.as_str() {
        "all" | "any" => row.push(children(value, upper, true)?),
        "not" => row.push(reference(value, upper, true)?),
        kind if scalar(kind) => row.push(value.clone()),
        _ => {
            let object = value
                .as_object()
                .filter(|object| {
                    object.len() == fields.len()
                        && fields.iter().all(|field| object.contains_key(*field))
                })
                .ok_or_else(|| invalid("compact proof leaf differs from its complete renderer"))?;
            row.extend(fields.iter().map(|field| object[*field].clone()));
        }
    }
    Ok((kind.clone(), Value::Array(row)))
}

fn readable_row(value: &Value, upper: usize) -> Result<(String, Value)> {
    let row = value
        .as_array()
        .ok_or_else(|| invalid("compact predicate table contains a non-row value"))?;
    let kind = row
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("compact predicate row is missing its kind"))?;
    let fields = columns(kind)?;
    if row.len() != fields.len() + 1 {
        return Err(invalid(
            "compact predicate row differs from its declared columns",
        ));
    }
    let value = match kind {
        "all" | "any" => children(&row[1], upper, false)?,
        "not" => reference(&row[1], upper, false)?,
        kind if scalar(kind) => row[1].clone(),
        _ => Value::Object(
            fields
                .iter()
                .zip(&row[1..])
                .map(|(field, value)| ((*field).to_owned(), value.clone()))
                .collect(),
        ),
    };
    Ok((kind.to_owned(), json!({kind:value})))
}

fn transform_roots(payload: &mut Value, count: usize, compress: bool) -> Result<()> {
    payload["terminal_condition"] = reference(&payload["terminal_condition"], count, compress)?;
    let steps = payload["steps"]
        .as_array_mut()
        .filter(|steps| steps.len() <= MAX_STEPS)
        .ok_or_else(|| invalid("compact proof requires all bounded plan steps"))?;
    for step in steps {
        step["postconditions"] = children(&step["postconditions"], count, compress)?;
        if !step["obligation"].is_null() {
            step["obligation"]["terminal"] =
                reference(&step["obligation"]["terminal"], count, compress)?;
        }
    }
    Ok(())
}

fn declared_columns(kinds: &BTreeSet<String>) -> Result<Value> {
    kinds
        .iter()
        .map(|kind| columns(kind).map(|fields| (kind.clone(), json!(fields))))
        .collect::<Result<Map<_, _>>>()
        .map(Value::Object)
}

fn table(payload: &Value) -> Result<&[Value]> {
    let values = payload["predicates"]
        .as_array()
        .filter(|values| values.len() <= MAX_NODES)
        .ok_or_else(|| invalid("compact proof requires a complete bounded predicate table"))?;
    if payload["predicates"].to_string().len() > MAX_EXPANDED_BYTES
        || payload["steps"].to_string().len() > MAX_EXPANDED_BYTES
    {
        return Err(bounded());
    }
    Ok(values)
}

/// Build separately, validate and compare the inverse before replacing any
/// field. Failure and non-saving conversion leave the original packet intact.
pub(super) fn apply(payload: &mut Value) -> Result<bool> {
    super::validate_v1(payload)?;
    let original = table(payload)?;
    let mut kinds = BTreeSet::new();
    let mut rows = Vec::with_capacity(original.len());
    for (index, predicate) in original.iter().enumerate() {
        let (kind, row) = compact_row(predicate, index)?;
        kinds.insert(kind);
        rows.push(row);
    }
    let mut proposed = payload.clone();
    proposed["predicates"] = Value::Array(rows);
    transform_roots(&mut proposed, original.len(), true)?;
    let metadata = proposed["predicate_encoding"]
        .as_object_mut()
        .ok_or_else(|| invalid("compact proof lost its complete encoding metadata"))?;
    for name in ["reference", "row_format", "columns", "leaf_values"] {
        if metadata.contains_key(name) {
            return Err(invalid(
                "compact proof metadata would overwrite an existing field",
            ));
        }
    }
    metadata.insert("schema".to_owned(), json!(COMPACT_SCHEMA));
    metadata.insert("reference".to_owned(), json!(REFERENCE));
    metadata.insert("row_format".to_owned(), json!(ROW_FORMAT));
    metadata.insert("columns".to_owned(), declared_columns(&kinds)?);
    metadata.insert("leaf_values".to_owned(), json!("literal_not_references"));
    let restored = readable(&proposed)?;
    super::validate_v1(&restored)?;
    if restored != *payload {
        return Err(invalid(
            "compact proof did not preserve its complete readable projection",
        ));
    }
    if proposed.to_string().len() >= payload.to_string().len() {
        return Ok(false);
    }
    *payload = proposed;
    Ok(true)
}

/// Restore a DAG, not an expanded predicate tree. The existing validator then
/// checks aggregate expansion size and depth before caching standalone steps.
pub(super) fn readable(payload: &Value) -> Result<Value> {
    let metadata = &payload["predicate_encoding"];
    if metadata["schema"] != COMPACT_SCHEMA
        || metadata["reference"] != REFERENCE
        || metadata["row_format"] != ROW_FORMAT
        || metadata["leaf_values"] != "literal_not_references"
    {
        return Err(invalid(
            "compact proof encoding metadata is incomplete or mismatched",
        ));
    }
    let original = table(payload)?;
    let mut rows = Vec::with_capacity(original.len());
    let mut kinds = BTreeSet::new();
    let mut bytes = 0usize;
    for (index, predicate) in original.iter().enumerate() {
        let (kind, row) = readable_row(predicate, index)?;
        bytes = bytes
            .checked_add(row.to_string().len())
            .ok_or_else(bounded)?;
        if bytes > MAX_EXPANDED_BYTES {
            return Err(bounded());
        }
        kinds.insert(kind);
        rows.push(row);
    }
    if metadata["columns"] != declared_columns(&kinds)? {
        return Err(invalid(
            "compact proof column declarations differ from the registered renderer",
        ));
    }
    let mut restored = payload.clone();
    restored["predicates"] = Value::Array(rows);
    transform_roots(&mut restored, original.len(), false)?;
    if restored["steps"].to_string().len() > MAX_EXPANDED_BYTES {
        return Err(bounded());
    }
    let metadata = restored["predicate_encoding"]
        .as_object_mut()
        .ok_or_else(|| invalid("compact proof lost its encoding object"))?;
    metadata.insert("schema".to_owned(), json!(SCHEMA));
    for name in ["reference", "row_format", "columns", "leaf_values"] {
        metadata.remove(name);
    }
    Ok(restored)
}
