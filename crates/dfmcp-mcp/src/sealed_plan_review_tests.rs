use super::*;
use dfmcp_core::{
    Capability, Digest32, EntityId, FortressId, GameTick, IntentId, MapCoord, MapCuboid,
    ObservationCursor, PlanId, RiskTier, StateAnchor, StepId,
};
use dfmcp_intent::{
    Action, FurnitureKind, PlanStep, derive_step_idempotency_key, effects, plan_archive,
};
use dfmcp_world::{CompareOp, Value as WorldValue, tile_codes};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn plan() -> Result<PreparedPlan> {
    let intent = IntentId::new(14);
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 3,
        },
        tick: GameTick(100),
        state_hash: Digest32::of_bytes(b"bounded shared plan review fixture"),
    };
    let mut steps = Vec::new();
    let mut goal = vec![Predicate::RegionTerrain {
        area: MapCuboid::new(MapCoord::new(2, 6, 10), MapCoord::new(8, 8, 10))?,
        tile_code: tile_codes::FLOOR,
    }];
    for index in 0..2 {
        let id = StepId::new(index);
        let action = Action::PlaceFurniture {
            kind: FurnitureKind::Bed,
            item: EntityId::new(200 + u64::from(index)),
            location: MapCoord::new(3 + index as i32 * 4, 7, 10),
        };
        let key = derive_step_idempotency_key(intent, anchor, id, &action);
        let mut postconditions = effects::default_postconditions(&action, &key, anchor.fortress_id);
        postconditions.sort_by_key(Predicate::canonical_bytes);
        let mut obligation =
            effects::default_obligation(&action, &key, anchor.fortress_id, anchor.tick)?;
        if let Some(obligation) = &mut obligation {
            obligation.terminal = obligation.terminal.normalized();
        }
        goal.extend(postconditions.iter().cloned());
        steps.push(PlanStep {
            id,
            action,
            preconditions: Vec::new(),
            postconditions,
            obligation,
            compensation: None,
            depends_on: Vec::new(),
            risk: RiskTier::Guarded,
            required_capability: Capability::Construct,
            idempotency_key: key,
        });
    }
    // Exercise Boolean nodes and data that resembles a reference without
    // interpreting that data as part of the reference graph.
    goal.push(Predicate::Any(vec![
        Predicate::Paused(true),
        Predicate::Not(Box::new(Predicate::FieldCompare {
            entity_id: EntityId::new(9),
            field: "$ref".to_owned(),
            op: CompareOp::Eq,
            value: WorldValue::Text("/predicates/99999".to_owned()),
        })),
    ]));
    let plan = PreparedPlan::builder(
        intent,
        anchor,
        "complete original room",
        Predicate::All(goal),
    )
    .steps(steps)
    .max_risk(RiskTier::Guarded)
    .required_capabilities([Capability::Construct, Capability::Checkpoint].into())
    .requires_checkpoint(true)
    .expires_at_tick(GameTick(5_000))
    .build();
    plan.validate_structure()?;
    Ok(plan)
}

fn payload(plan: &PreparedPlan) -> Value {
    json!({
        "schema":"dfmcp.resource-room/2", "version":2,
        "plan_digest":plan.digest.to_string(),
        "terminal_condition":format!("{:?}",plan.terminal_condition),
        "steps":crate::lab_world::plan_steps_json(plan),
        "room":{"resource_workflow":{"original_request":{"rooms":2}}},
    })
}

fn expand(payload: &Value, value: &Value, depth: usize) -> TestResult<Value> {
    if depth > MAX_DEPTH * 2 {
        return Err("test expansion exceeded the validated predicate depth".into());
    }
    if let Some(reference) = value.get("$ref") {
        let pointer = reference.as_str().ok_or("reference is not a string")?;
        return expand(
            payload,
            payload.pointer(pointer).ok_or("reference is missing")?,
            depth + 1,
        );
    }
    for operator in ["all", "any"] {
        if let Some(children) = value.get(operator) {
            return Ok(
                json!({operator:children.as_array().ok_or("children are not an array")?
                .iter().map(|child| expand(payload,child,depth+1)).collect::<TestResult<Vec<_>>>()?}),
            );
        }
    }
    if let Some(child) = value.get("not") {
        return Ok(json!({"not":expand(payload,child,depth+1)?}));
    }
    Ok(value.clone())
}

#[test]
fn shared_plan_review_reconstructs_every_original_proof_and_preserves_exact_archive() -> TestResult
{
    let plan = plan()?;
    let original_plan = plan.clone();
    let original_archive = plan_archive::encode(&plan, plan_archive::ArchiveLimits::default())?;
    let original = payload(&plan);
    let mut shared = original.clone();
    apply(&mut shared, &plan)?;
    validate(&shared)?;
    assert_eq!(
        expand(&shared, &shared["terminal_condition"], 0)?,
        crate::lab_world::predicate_json(&plan.terminal_condition)
    );
    let mut restored = shared["steps"].clone();
    for row in restored.as_array_mut().ok_or("steps are not an array")? {
        for predicate in row["postconditions"]
            .as_array_mut()
            .ok_or("postconditions missing")?
        {
            *predicate = expand(&shared, predicate, 0)?;
        }
        if !row["obligation"].is_null() {
            row["obligation"]["terminal"] = expand(&shared, &row["obligation"]["terminal"], 0)?;
        }
    }
    assert_eq!(restored, original["steps"]);
    assert_eq!(json!(expanded_steps(&shared)?), original["steps"]);
    assert_eq!(shared["room"], original["room"]);
    assert_eq!(plan, original_plan);
    assert_eq!(
        plan_archive::encode(&plan, plan_archive::ArchiveLimits::default())?,
        original_archive
    );
    assert!(
        shared.to_string().len() < original.to_string().len(),
        "shared={} original={}",
        shared.to_string().len(),
        original.to_string().len()
    );
    let mut repeated = original;
    apply(&mut repeated, &plan)?;
    assert_eq!(
        shared, repeated,
        "predicate ordering and references must be deterministic"
    );
    Ok(())
}

#[test]
fn shared_plan_review_refuses_invalid_seals_and_step_substitution_without_partial_output()
-> TestResult {
    let original_plan = plan()?;
    for altered in [false, true] {
        let mut plan = original_plan.clone();
        if altered {
            plan.digest = Digest32::of_bytes(b"a different seal");
        } else {
            plan.id = PlanId::NIL;
        }
        let mut request = payload(&plan);
        let before = request.clone();
        assert!(apply(&mut request, &plan).is_err());
        assert_eq!(request, before);
    }
    let mut request = payload(&original_plan);
    request["steps"][0]["obligation"]["deadline_tick"] = json!(u64::MAX);
    let before = request.clone();
    assert!(apply(&mut request, &original_plan).is_err());
    assert_eq!(request, before);
    Ok(())
}

#[test]
fn shared_plan_review_rejects_missing_foreign_noncanonical_and_cyclic_references() -> TestResult {
    let plan = plan()?;
    let mut shared = payload(&plan);
    apply(&mut shared, &plan)?;
    for reference in [
        "/steps/0/postconditions/0",
        "https://example.test/proof",
        "/predicates/01",
        "/predicates/-1",
        "/predicates/+1",
        "/predicates/999999",
    ] {
        let mut changed = shared.clone();
        changed["terminal_condition"] = json!({"$ref":reference});
        assert!(validate(&changed).is_err(), "{reference}");
        assert!(expanded_steps(&changed).is_err(), "{reference}");
    }
    for changed_value in [
        json!({"$ref":"/predicates/0"}),
        json!({"all":[{"$ref":"/predicates/1"}]}),
        json!({"all":null}),
        json!({"$ref":"/predicates/0","field":{}}),
        Value::Null,
    ] {
        let mut changed = shared.clone();
        changed["predicates"][0] = changed_value;
        assert!(validate(&changed).is_err(), "{}", changed["predicates"][0]);
    }
    for path in [
        "/steps/0/postconditions",
        "/steps/0/obligation/terminal",
        "/predicate_encoding/complete",
        "/predicate_encoding/plan_digest",
    ] {
        let mut changed = shared.clone();
        *changed
            .pointer_mut(path)
            .ok_or("test mutation path missing")? = Value::Null;
        assert!(validate(&changed).is_err(), "{path}");
    }
    Ok(())
}

#[test]
fn shared_plan_review_bounds_expanded_bytes_and_rejects_alias_only_table_rows() -> TestResult {
    let plan = plan()?;
    let mut shared = payload(&plan);
    apply(&mut shared, &plan)?;
    shared["predicates"] = json!([
        {"paused":true},
        {"$ref":"/predicates/0"},
    ]);
    shared["terminal_condition"] = json!({"$ref":"/predicates/1"});
    assert!(validate(&shared).is_err());
    assert!(expanded_steps(&shared).is_err());

    // Five uses of one literal are a small graph, but expansion must refuse
    // before copying more than the complete typed archive's byte bound.
    shared["predicates"] = json!([
        {"field":{"entity_id":"9", "field":"large", "op":"eq",
            "value":"x".repeat(MAX_EXPANDED_BYTES / 4)}},
        {"all":vec![json!({"$ref":"/predicates/0"});5]},
    ]);
    assert_eq!(
        validate(&shared).err().map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        expanded_steps(&shared).err().map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn shared_plan_review_bounds_expanded_depth_and_exponential_reference_graphs() -> TestResult {
    let plan = plan()?;
    for deep in [true, false] {
        let mut shared = payload(&plan);
        apply(&mut shared, &plan)?;
        let mut table = vec![json!({"paused":true})];
        for index in 1..if deep { MAX_DEPTH + 1 } else { 20 } {
            let reference = json!({"$ref":format!("{PREFIX}{}",index - 1)});
            table.push(if deep {
                json!({"not":reference})
            } else {
                json!({"all":[reference.clone(),reference]})
            });
        }
        shared["terminal_condition"] = json!({"$ref":format!("{PREFIX}{}",table.len() - 1)});
        shared["predicates"] = json!(table);
        assert_eq!(
            validate(&shared).err().map(|error| error.code),
            Some(ErrorCode::BudgetExceeded)
        );
    }
    Ok(())
}
