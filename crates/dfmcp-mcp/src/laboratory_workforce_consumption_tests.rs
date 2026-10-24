//! Current reserve sizing includes construction competing for the same worker.

use super::*;

#[test]
fn one_worker_construction_queue_is_in_the_exact_consumption_and_sealed_horizon() -> Result<()> {
    let mut source = scenario_snapshot("starter_fortress_workforce", FortressId::new(988), false)?;
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        for labor in ["MINING", "CONSTRUCTION", "BREW", "COOK", "CARPENTRY"] {
            unit.fields.insert(
                format!("labor.{labor}"),
                lab_fact(Value::Bool(unit.id == EntityId::new(starter::FIRST_DWARF))),
            );
        }
    }
    source.refresh_hash();
    let quota = request(json!([
        {"item":"DRINK","minimum":60},
        {"item":"FOOD","minimum":65},
    ]))?;
    let uncontended = quota.compile(&source)?;
    let at = MapCoord::new(0, 0, starter::LEVEL_Z);
    effects::apply_effect(
        &mut source,
        &Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location: at,
            footprint: MapCuboid::new(at, at)?,
            material: MaterialSelector::default(),
        },
        "prior-construction-competes-for-the-worker",
    )?;
    source.refresh_hash();
    let original = source.clone();
    let report = effects::laboratory_workforce_report(&source)?
        .ok_or_else(|| invalid("queued workforce fixture is missing"))?;
    assert_eq!(report.remaining_service_ticks, effects::BUILD_TICKS);
    let compiled = quota.compile(&source)?;
    assert_eq!(source, original);
    let planning_ticks = compiled.analysis["consumption"]["planning_horizon_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("shared workforce horizon is missing"))?;
    let uncontended_ticks = uncontended.analysis["consumption"]["planning_horizon_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("uncontended workforce horizon is missing"))?;
    assert!(planning_ticks >= uncontended_ticks + 2 * effects::BUILD_TICKS);
    let plan = seal(&source, &compiled, 988)?;
    let sealed_horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .ok_or_else(|| invalid("sealed workforce deadline is missing"))?;
    assert_eq!(sealed_horizon, source.tick.0 + planning_ticks);
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(
        states.iter().all(|state| *state == CommitState::Verified),
        "{states:?}"
    );
    assert_through_horizon(adapter, &source, &compiled)
}
