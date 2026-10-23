//! Joint staffing regressions for df-lab-conditional-production-c4p.

use super::*;

fn worker(id: u64, enabled: &[&str], disabled: &[&str], military: u8) -> EntityRecord {
    let mut fields = vec![("alive", Value::Bool(true))];
    fields.extend(enabled.iter().map(|field| (*field, Value::Bool(true))));
    fields.extend(disabled.iter().map(|field| (*field, Value::Bool(false))));
    if military == 0 {
        fields.push((effects::SQUAD_FIELD, Value::Null));
    } else if military == 2 {
        fields.push((effects::SQUAD_FIELD, Value::Entity(EntityId::new(90))));
    }
    record(
        EntityId::new(id),
        EntityKind::Unit,
        "staffing fixture",
        fields,
    )
}

fn ids(selected: &[&EntityRecord]) -> Vec<u64> {
    selected.iter().map(|unit| unit.id.get()).collect()
}

#[test]
fn three_roles_preserve_the_only_carpenter_instead_of_greedily_using_them() -> Result<()> {
    let units = [
        worker(1, &["labor.BREW", "labor.COOK", "labor.CARPENTRY"], &[], 0),
        worker(2, &["labor.BREW"], &[], 0),
        worker(3, &["labor.COOK"], &[], 0),
    ];
    let living: Vec<_> = units.iter().collect();
    let selected = select(
        GameTick(1),
        &living,
        &["labor.BREW", "labor.COOK", "labor.CARPENTRY"],
        false,
    )?;
    assert_eq!(ids(&selected), vec![2, 3, 1]);
    Ok(())
}

#[test]
fn six_outputs_use_six_distinct_workers_with_canonical_ties() -> Result<()> {
    let fields = [
        "labor.BREW",
        "labor.COOK",
        "labor.CARPENTRY",
        "labor.CARPENTRY",
        "labor.CARPENTRY",
        "labor.CARPENTRY",
    ];
    let units: Vec<_> = (1..=8)
        .map(|id| worker(id, &["labor.BREW", "labor.COOK", "labor.CARPENTRY"], &[], 0))
        .collect();
    let living: Vec<_> = units.iter().collect();
    let forward = select(GameTick(1), &living, &fields, false)?;
    assert_eq!(ids(&forward), vec![1, 2, 3, 4, 5, 6]);
    let reverse: Vec<_> = units.iter().rev().collect();
    assert_eq!(
        ids(&select(GameTick(1), &reverse, &fields, false)?),
        ids(&forward)
    );
    Ok(())
}

#[test]
fn scarcity_reuses_workers_without_claiming_simultaneous_capacity() -> Result<()> {
    let units = [worker(1, &["labor.CARPENTRY"], &[], 0)];
    let fields = ["labor.CARPENTRY"; 4];
    assert_eq!(
        ids(&select(GameTick(1), &[&units[0]], &fields, false)?),
        vec![1, 1, 1, 1],
    );
    Ok(())
}

#[test]
fn explicit_disablement_is_required_before_proposing_a_labor_enable() -> Result<()> {
    let units = [
        worker(1, &[], &[], 0),
        worker(2, &[], &["labor.CARPENTRY"], 0),
    ];
    let living: Vec<_> = units.iter().collect();
    let fields = ["labor.CARPENTRY"];
    assert!(select(GameTick(1), &living, &fields, false).is_err());
    assert_eq!(ids(&select(GameTick(1), &living, &fields, true)?), vec![2]);
    assert!(select(GameTick(1), &[&units[0]], &fields, true).is_err());
    Ok(())
}

#[test]
fn current_evidence_and_job_bounds_are_required() -> Result<()> {
    let unit = worker(1, &["labor.CARPENTRY"], &[], 0);
    // Scenario facts are observed at tick one; future facts cannot staff tick zero.
    assert!(select(GameTick(0), &[&unit], &["labor.CARPENTRY"], false).is_err());
    assert!(select(GameTick(1), &[&unit], &[], false).is_err());
    assert!(select(GameTick(1), &[&unit], &["labor.CARPENTRY"; 7], false).is_err());
    assert!(select(GameTick(1), &[], &["labor.CARPENTRY"], true).is_err());
    Ok(())
}

#[test]
fn established_skills_precede_military_preference_then_known_civilians_win() -> Result<()> {
    let units = [
        worker(1, &["labor.BREW", "labor.COOK"], &[], 2),
        worker(2, &["labor.BREW", "labor.COOK"], &[], 1),
        worker(3, &["labor.BREW", "labor.COOK"], &[], 0),
        worker(4, &[], &["labor.BREW", "labor.COOK"], 0),
    ];
    let living: Vec<_> = units.iter().collect();
    assert_eq!(
        ids(&select(
            GameTick(1),
            &living,
            &["labor.BREW", "labor.COOK"],
            true
        )?),
        vec![3, 2],
    );
    Ok(())
}

#[test]
fn all_three_worker_three_role_eligibility_graphs_match_exhaustive_oracle() -> Result<()> {
    let fields = ["labor.BREW", "labor.COOK", "labor.CARPENTRY"];
    for mask in 0u16..512 {
        let units: Vec<_> = (0..3)
            .map(|worker_index| {
                let enabled: Vec<_> = fields
                    .iter()
                    .enumerate()
                    .filter_map(|(role, field)| {
                        ((mask & (1 << (worker_index * 3 + role))) != 0).then_some(*field)
                    })
                    .collect();
                worker((worker_index + 1) as u64, &enabled, &[], worker_index as u8)
            })
            .collect();
        let mut expected = None;
        for brewer in 0..3 {
            for cook in 0..3 {
                for carpenter in 0..3 {
                    let assignment = [brewer, cook, carpenter];
                    if assignment
                        .iter()
                        .enumerate()
                        .any(|(role, unit)| mask & (1 << (unit * 3 + role)) == 0)
                    {
                        continue;
                    }
                    let distinct = assignment.into_iter().collect::<BTreeSet<_>>().len();
                    let ranks = assignment.map(|index| index as u8);
                    let key = (
                        3 - distinct,
                        ranks.into_iter().sum::<u8>(),
                        ranks,
                        assignment.map(|index| (index + 1) as u64),
                    );
                    if expected.as_ref().is_none_or(|prior| key < *prior) {
                        expected = Some(key);
                    }
                }
            }
        }
        let living: Vec<_> = units.iter().collect();
        match (expected, select(GameTick(1), &living, &fields, false)) {
            (Some((_, _, _, expected)), Ok(actual)) => {
                assert_eq!(ids(&actual), expected, "eligibility graph {mask}");
            }
            (None, Err(_)) => {}
            (expected, actual) => {
                return Err(invalid(format!(
                    "eligibility graph {mask}: expected {expected:?}, got {actual:?}"
                )));
            }
        }
    }
    Ok(())
}
