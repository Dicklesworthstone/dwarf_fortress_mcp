use super::*;
use dfmcp_core::{ObservationCursor, Result};
use dfmcp_world::{FactPresence, PredicateEvidence, PredicateTruth, WorldGraph};

const STOCKPILE: EntityId = EntityId::new(2001);

fn snapshot() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        STOCKPILE,
        EntityRecord {
            id: STOCKPILE,
            generation: 1,
            revision: 1,
            kind: EntityKind::Stockpile,
            label: "stockpile".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn action() -> Action {
    Action::ConfigureStockpile {
        stockpile: STOCKPILE,
        accepts: ["FOOD".to_owned()].into_iter().collect(),
        max_bins: Some(0),
        max_barrels: Some(7),
        max_wheelbarrows: Some(2),
    }
}

#[test]
fn stockpile_complete_proof_requires_every_requested_capacity() -> Result<()> {
    let mut world = snapshot();
    let action = action();
    let full = complete_stockpile_postconditions(&action).ok_or_else(|| precondition("proof"))?;
    assert_eq!(full.len(), 5);
    let predicate = Predicate::All(full);
    assert_ne!(
        PredicateEvidence::laboratory(&world)?.evaluate(&predicate)?,
        PredicateTruth::True
    );
    apply_effect(&mut world, &action, "configure")?;
    world.refresh_hash();
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&predicate)?,
        PredicateTruth::True
    );
    for field in [
        STOCKPILE_MAX_BINS_FIELD,
        STOCKPILE_MAX_BARRELS_FIELD,
        STOCKPILE_MAX_WHEELBARROWS_FIELD,
    ] {
        let mut changed = world.clone();
        changed
            .graph
            .entities
            .get_mut(&STOCKPILE)
            .ok_or_else(|| precondition("stockpile"))?
            .fields
            .insert(field.to_owned(), known(Value::U64(99), changed.tick));
        changed.refresh_hash();
        assert_eq!(
            PredicateEvidence::laboratory(&changed)?.evaluate(&predicate)?,
            PredicateTruth::False,
            "{field}"
        );
    }
    Ok(())
}

#[test]
fn stockpile_missing_unknown_or_wrong_kind_cannot_satisfy_full_goal() -> Result<()> {
    let mut world = snapshot();
    apply_effect(&mut world, &action(), "configure")?;
    world.refresh_hash();
    let predicate = Predicate::All(
        complete_stockpile_postconditions(&action()).ok_or_else(|| precondition("proof"))?,
    );
    for variant in 0..4 {
        let mut changed = world.clone();
        let record = changed
            .graph
            .entities
            .get_mut(&STOCKPILE)
            .ok_or_else(|| precondition("stockpile"))?;
        if variant == 0 {
            record.fields.remove(STOCKPILE_MAX_BINS_FIELD);
        } else if variant == 1 {
            record.kind = EntityKind::Building;
        } else {
            let fact = record
                .fields
                .get_mut(STOCKPILE_MAX_BINS_FIELD)
                .ok_or_else(|| precondition("capacity"))?;
            if variant == 2 {
                fact.presence = Some(FactPresence::Unknown("unavailable".to_owned()));
            } else {
                fact.observed_at = GameTick(11);
            }
        }
        changed.refresh_hash();
        assert_ne!(
            PredicateEvidence::laboratory(&changed)?.evaluate(&predicate)?,
            PredicateTruth::True
        );
    }
    Ok(())
}

#[test]
fn stockpile_omitted_capacities_and_historical_proof_remain_distinct() -> Result<()> {
    let mut world = snapshot();
    apply_effect(&mut world, &action(), "initial")?;
    let mut request = action();
    if let Action::ConfigureStockpile {
        accepts,
        max_bins,
        max_barrels,
        max_wheelbarrows,
        ..
    } = &mut request
    {
        *accepts = ["WOOD".to_owned()].into_iter().collect();
        *max_bins = None;
        *max_barrels = None;
        *max_wheelbarrows = None;
    }
    let full = complete_stockpile_postconditions(&request).ok_or_else(|| precondition("proof"))?;
    assert_eq!(full.len(), 2);
    apply_effect(&mut world, &request, "filters")?;
    world.refresh_hash();
    assert_eq!(
        world.graph.entities[&STOCKPILE].fields[STOCKPILE_MAX_BARRELS_FIELD].known_value(),
        Some(&Value::U64(7))
    );
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&Predicate::All(full))?,
        PredicateTruth::True
    );
    let historical = default_postconditions(&action(), "original", world.fortress_id);
    assert_eq!(historical.len(), 1);
    assert!(
        matches!(&historical[0], Predicate::FieldCompare {field, ..} if field == STOCKPILE_ACCEPTS_FIELD)
    );
    assert!(complete_stockpile_postconditions(&Action::Pause { paused: true }).is_none());
    Ok(())
}
