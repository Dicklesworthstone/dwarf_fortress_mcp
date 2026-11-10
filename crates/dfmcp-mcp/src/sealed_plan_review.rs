//! Complete readable plan proofs with same-response predicate sharing.
//!
//! An installation's proof can occur in its step, temporal obligation and final
//! room goal. These are distinct requirements, but repeating their complete
//! JSON prevents ordinary plans from fitting the default response budget.
//! This projection retains every requirement and uses bounded local references
//! for identical readable predicate nodes. It never changes the sealed plan or
//! its lossless typed archive, and references are not executable authority.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::Predicate;
use serde_json::{Value, json};

const SCHEMA: &str = "dfmcp.plan-predicate-references/1";
const PREFIX: &str = "/predicates/";
const MAX_STEPS: usize = 64;
const MAX_NODES: usize = 16_384;
const MAX_DEPTH: usize = 64;
const MAX_EXPANDED_BYTES: usize = dfmcp_intent::plan_archive::MAX_ARCHIVE_BYTES;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "the complete readable plan proof exceeds its bounded predicate projection",
    )
}

#[derive(Default)]
struct Predicates {
    values: Vec<Value>,
    indices: BTreeMap<String, usize>,
}

impl Predicates {
    fn insert(&mut self, predicate: &Predicate) -> Value {
        let value = match predicate {
            Predicate::True => return json!(true),
            Predicate::False => return json!(false),
            Predicate::All(children) => json!({
                "all":children.iter().map(|child| self.insert(child)).collect::<Vec<_>>(),
            }),
            Predicate::Any(children) => json!({
                "any":children.iter().map(|child| self.insert(child)).collect::<Vec<_>>(),
            }),
            Predicate::Not(child) => json!({"not":self.insert(child)}),
            _ => crate::lab_world::predicate_json(predicate),
        };
        let key = value.to_string();
        let index = match self.indices.get(&key) {
            Some(index) => *index,
            None => {
                let index = self.values.len();
                self.values.push(value);
                self.indices.insert(key, index);
                index
            }
        };
        json!({"$ref":format!("{PREFIX}{index}")})
    }
}

/// Replace only the readable proof representation in an unpublished result.
/// The caller selects an explicit response generation before publication.
pub(crate) fn apply(payload: &mut Value, plan: &PreparedPlan) -> Result<()> {
    if !payload.is_object()
        || payload["plan_digest"] != plan.digest.to_string()
        || payload.get("predicate_encoding").is_some()
        || payload.get("predicates").is_some()
    {
        return Err(invalid(
            "shared plan proof differs from its unpublished plan",
        ));
    }
    if plan.steps.len() > MAX_STEPS {
        return Err(bounded());
    }
    let mut nodes = 0usize;
    let mut admit = |predicate: &Predicate| -> Result<()> {
        predicate.validate_shape()?;
        nodes = nodes
            .checked_add(predicate.complexity())
            .ok_or_else(bounded)?;
        if nodes > MAX_NODES {
            return Err(bounded());
        }
        Ok(())
    };
    admit(&plan.terminal_condition)?;
    for step in &plan.steps {
        for predicate in step.preconditions.iter().chain(&step.postconditions) {
            admit(predicate)?;
        }
        if let Some(obligation) = &step.obligation {
            admit(&obligation.terminal)?;
            if let Some(failure) = &obligation.failure {
                admit(failure)?;
            }
        }
    }
    plan.validate_structure()?;
    let mut steps = crate::lab_world::plan_steps_json(plan);
    if payload["steps"] != steps {
        return Err(invalid(
            "shared plan proof requires every original readable step",
        ));
    }
    let rows = steps
        .as_array_mut()
        .ok_or_else(|| invalid("the original plan step projection is not an array"))?;
    let mut predicates = Predicates::default();
    let terminal = predicates.insert(&plan.terminal_condition);
    for (row, step) in rows.iter_mut().zip(&plan.steps) {
        row["postconditions"] = json!(
            step.postconditions
                .iter()
                .map(|predicate| predicates.insert(predicate))
                .collect::<Vec<_>>()
        );
        if let Some(obligation) = &step.obligation {
            row["obligation"]["terminal"] = predicates.insert(&obligation.terminal);
        }
    }
    // Build and validate separately so a refusal cannot leave a partial proof.
    let mut proposed = payload.clone();
    proposed["terminal_condition"] = terminal;
    proposed["steps"] = steps;
    proposed["predicates"] = json!(predicates.values);
    proposed["predicate_encoding"] = json!({
        "schema":SCHEMA, "scope":"same_response", "plan_digest":plan.digest.to_string(),
        "values_pointer":"/predicates", "complete":true,
        "projection":"readable_predicate_json", "exact_typed_contract":"retained_plan_archive",
        "order":"terminal_then_steps_postconditions_then_obligation_children_first",
    });
    validate(&proposed)?;
    *payload = proposed;
    Ok(())
}

/// Validate every local reference before publishing an otherwise complete
/// packet. Table children point strictly backwards, so cycles and references to
/// a future node cannot hide behind a small serialized proof. Values inside a
/// leaf predicate are data: only predicate positions interpret `$ref`.
pub(crate) fn validate(payload: &Value) -> Result<()> {
    let encoding = &payload["predicate_encoding"];
    if encoding["schema"] != SCHEMA
        || encoding["scope"] != "same_response"
        || encoding["plan_digest"] != payload["plan_digest"]
        || payload["plan_digest"].as_str().is_none_or(|digest| {
            Digest32::from_hex(digest).is_none_or(|value| value.to_string() != digest)
        })
        || encoding["values_pointer"] != "/predicates"
        || encoding["complete"] != true
        || encoding["projection"] != "readable_predicate_json"
        || encoding["exact_typed_contract"] != "retained_plan_archive"
        || encoding["order"] != "terminal_then_steps_postconditions_then_obligation_children_first"
    {
        return Err(invalid(
            "shared plan predicate metadata is incomplete or mismatched",
        ));
    }
    let predicates = payload["predicates"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_NODES)
        .ok_or_else(|| invalid("shared plan predicates require a bounded complete table"))?;
    let steps = payload["steps"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_STEPS)
        .ok_or_else(|| invalid("shared plan predicates require bounded complete steps"))?;
    let mut work = 0usize;
    let mut shapes = Vec::with_capacity(predicates.len());
    for (index, predicate) in predicates.iter().enumerate() {
        if predicate.is_boolean() || predicate.get("$ref").is_some() {
            return Err(invalid(
                "shared predicate table rows must define a nonboolean node",
            ));
        }
        shapes.push(validate_node(predicate, index, &shapes, 0, &mut work)?);
    }
    let terminal = validate_node(
        &payload["terminal_condition"],
        predicates.len(),
        &shapes,
        0,
        &mut work,
    )?;
    let mut expanded = terminal.nodes;
    let mut bytes = terminal.bytes;
    for step in steps {
        let postconditions = step["postconditions"]
            .as_array()
            .ok_or_else(|| invalid("a shared plan step lost its complete postconditions"))?;
        for predicate in postconditions {
            let shape = validate_node(predicate, predicates.len(), &shapes, 0, &mut work)?;
            expanded = expanded.checked_add(shape.nodes).ok_or_else(bounded)?;
            bytes = bytes.checked_add(shape.bytes).ok_or_else(bounded)?;
        }
        if !step["obligation"].is_null() {
            let shape = validate_node(
                &step["obligation"]["terminal"],
                predicates.len(),
                &shapes,
                0,
                &mut work,
            )?;
            expanded = expanded.checked_add(shape.nodes).ok_or_else(bounded)?;
            bytes = bytes.checked_add(shape.bytes).ok_or_else(bounded)?;
        }
        if expanded > MAX_NODES || bytes > MAX_EXPANDED_BYTES {
            return Err(bounded());
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Shape {
    nodes: usize,
    depth: usize,
    bytes: usize,
}

fn validate_node(
    value: &Value,
    upper: usize,
    shapes: &[Shape],
    depth: usize,
    work: &mut usize,
) -> Result<Shape> {
    *work = work.checked_add(1).ok_or_else(bounded)?;
    if *work > MAX_NODES * 3 || depth >= MAX_DEPTH {
        return Err(bounded());
    }
    if value.is_boolean() {
        return Ok(Shape {
            nodes: 1,
            depth: 1,
            bytes: if value == true { 4 } else { 5 },
        });
    }
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| invalid("a shared plan proof contains an invalid predicate node"))?;
    let mut shape = Shape {
        nodes: 1,
        depth: 1,
        bytes: 0,
    };
    if let Some(reference) = object.get("$ref") {
        let raw = reference
            .as_str()
            .and_then(|reference| reference.strip_prefix(PREFIX))
            .ok_or_else(|| {
                invalid("plan proof references must stay in this response's predicate table")
            })?;
        let index = raw
            .parse::<usize>()
            .ok()
            .filter(|index| index.to_string() == raw && *index < upper)
            .ok_or_else(|| invalid("plan proof reference is noncanonical, dangling or cyclic"))?;
        // Reuse its checked shape without expanding the tree. This bounds even
        // acyclic DAGs that would otherwise expand exponentially.
        shape = *shapes
            .get(index)
            .ok_or_else(|| invalid("plan proof lost a validated predecessor"))?;
    } else if let Some(children) = object.get("all").or_else(|| object.get("any")) {
        let children = children
            .as_array()
            .ok_or_else(|| invalid("shared Boolean predicates require an intact child array"))?;
        shape.bytes = 10 + children.len().saturating_sub(1);
        for child in children {
            let child = validate_node(child, upper, shapes, depth + 1, work)?;
            shape.nodes = shape.nodes.checked_add(child.nodes).ok_or_else(bounded)?;
            shape.depth = shape.depth.max(child.depth.saturating_add(1));
            shape.bytes = shape.bytes.checked_add(child.bytes).ok_or_else(bounded)?;
        }
    } else if let Some(child) = object.get("not") {
        let child = validate_node(child, upper, shapes, depth + 1, work)?;
        shape.nodes = shape.nodes.checked_add(child.nodes).ok_or_else(bounded)?;
        shape.depth = child.depth.saturating_add(1);
        shape.bytes = child.bytes.checked_add(8).ok_or_else(bounded)?;
    } else if !object.keys().all(|kind| {
        matches!(
            kind.as_str(),
            "entity_exists"
                | "entity_kind"
                | "field"
                | "entity_quantity"
                | "entity_quantity_increase"
                | "edge_exists"
                | "paused"
                | "region_terrain"
                | "laboratory_room_access"
                | "laboratory_room_access_paired_stairs"
                | "laboratory_civilian_release_safety"
        )
    }) {
        return Err(invalid(
            "shared plan proof contains an unregistered predicate projection",
        ));
    } else {
        shape.bytes = value.to_string().len();
    }
    if shape.nodes > MAX_NODES || shape.depth > MAX_DEPTH || shape.bytes > MAX_EXPANDED_BYTES {
        return Err(bounded());
    }
    Ok(shape)
}

/// Cache standalone readable steps for later turns. Same-response references
/// are valid only in their original packet and must not outlive its table.
pub(crate) fn expanded_steps(payload: &Value) -> Result<Vec<Value>> {
    validate(payload)?;
    if payload["steps"].to_string().len() > MAX_EXPANDED_BYTES {
        return Err(bounded());
    }
    let mut steps = payload["steps"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("shared plan steps are missing"))?;
    let mut work = 0usize;
    for step in &mut steps {
        let postconditions = step["postconditions"]
            .as_array_mut()
            .ok_or_else(|| invalid("shared step postconditions are missing"))?;
        for predicate in postconditions {
            *predicate = expand_node(payload, predicate, 0, &mut work)?;
        }
        if !step["obligation"].is_null() {
            step["obligation"]["terminal"] =
                expand_node(payload, &step["obligation"]["terminal"], 0, &mut work)?;
        }
    }
    if serde_json::to_vec(&steps)
        .map_err(|_| invalid("expanded plan steps are not serializable"))?
        .len()
        > MAX_EXPANDED_BYTES
    {
        return Err(bounded());
    }
    Ok(steps)
}

fn expand_node(payload: &Value, value: &Value, depth: usize, work: &mut usize) -> Result<Value> {
    *work = work.checked_add(1).ok_or_else(bounded)?;
    if depth > MAX_DEPTH * 2 || *work > MAX_NODES * 2 {
        return Err(bounded());
    }
    if let Some(reference) = value.get("$ref") {
        let target = reference
            .as_str()
            .and_then(|pointer| payload.pointer(pointer))
            .ok_or_else(|| invalid("a validated plan proof lost its reference target"))?;
        return expand_node(payload, target, depth + 1, work);
    }
    for operator in ["all", "any"] {
        if let Some(children) = value.get(operator) {
            let children = children
                .as_array()
                .ok_or_else(|| invalid("a validated Boolean proof lost its children"))?;
            return Ok(
                json!({operator:children.iter().map(|child| expand_node(payload,child,depth+1,work)).collect::<Result<Vec<_>>>()?}),
            );
        }
    }
    if let Some(child) = value.get("not") {
        return Ok(json!({"not":expand_node(payload,child,depth+1,work)?}));
    }
    Ok(value.clone())
}

#[cfg(test)]
#[path = "sealed_plan_review_tests.rs"]
mod tests;
