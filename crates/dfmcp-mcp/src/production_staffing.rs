//! Bounded joint staffing for the closed laboratory production catalog.
//!
//! At most six jobs are compared. For each job, only its best `jobs.len()`
//! eligible citizens are needed: at most `jobs.len() - 1` identities can be
//! occupied by the other jobs. Replacing a discarded citizen with an unused,
//! no-worse shortlisted citizen cannot reduce diversity or worsen the score.
//! This keeps exact assignment independent of the size of the citizen roster.

use super::*;

const MAX_JOBS: usize = 6;
const MAX_SEARCH_NODES: usize = 56_000;

#[derive(Clone, Copy)]
struct Candidate<'a> {
    unit: &'a EntityRecord,
    enable: u8,
    military: u8,
}

impl Candidate<'_> {
    fn preference(&self) -> (u8, u8, EntityId) {
        (self.enable, self.military, self.unit.id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    reused: usize,
    enables: u8,
    military_sum: u8,
    military: Vec<u8>,
    identities: Vec<EntityId>,
}

fn score(assignment: &[Candidate<'_>]) -> Score {
    let identities: Vec<_> = assignment
        .iter()
        .map(|candidate| candidate.unit.id)
        .collect();
    let distinct: BTreeSet<_> = identities.iter().copied().collect();
    Score {
        reused: assignment.len() - distinct.len(),
        enables: assignment.iter().map(|candidate| candidate.enable).sum(),
        military_sum: assignment.iter().map(|candidate| candidate.military).sum(),
        military: assignment
            .iter()
            .map(|candidate| candidate.military)
            .collect(),
        identities,
    }
}

struct Search<'a> {
    nodes: usize,
    best: Option<(Score, Vec<&'a EntityRecord>)>,
}

impl<'a> Search<'a> {
    fn visit(
        &mut self,
        candidates: &[Vec<Candidate<'a>>],
        assignment: &mut Vec<Candidate<'a>>,
    ) -> Result<()> {
        self.nodes += 1;
        if self.nodes > MAX_SEARCH_NODES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "production staffing exceeds its bounded assignment search",
            ));
        }
        if assignment.len() == candidates.len() {
            let score = score(assignment);
            if self
                .best
                .as_ref()
                .is_none_or(|(previous, _)| score < *previous)
            {
                self.best = Some((
                    score,
                    assignment.iter().map(|candidate| candidate.unit).collect(),
                ));
            }
            return Ok(());
        }
        for candidate in &candidates[assignment.len()] {
            assignment.push(*candidate);
            self.visit(candidates, assignment)?;
            assignment.pop();
        }
        Ok(())
    }
}

/// Maximize distinct citizens, then minimize per-job labor enables, aggregate
/// military exposure, per-job military preference, and per-job canonical IDs.
/// This preserves the exact one/two-job policy used by retained food/drink
/// sources. Repeated labor enables are deduplicated by the action compiler.
pub(super) fn select<'a>(
    tick: GameTick,
    living: &[&'a EntityRecord],
    fields: &[&str],
    assign_labor: bool,
) -> Result<Vec<&'a EntityRecord>> {
    if fields.is_empty() || fields.len() > MAX_JOBS {
        return Err(invalid(
            "production staffing requires one to six closed jobs",
        ));
    }
    if living.len() > MAX_PRODUCTION_SETUP_ENTITIES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "production staffing exceeds its bounded citizen roster",
        ));
    }
    let mut candidates = Vec::with_capacity(fields.len());
    for field in fields {
        let mut best = Vec::<Candidate<'a>>::with_capacity(fields.len() + 1);
        for unit in living {
            let enable = match production_fact(unit, field, tick) {
                Some(Value::Bool(true)) => 0,
                Some(Value::Bool(false)) if assign_labor => 1,
                _ => continue,
            };
            let candidate = Candidate {
                unit,
                enable,
                military: production_military_rank(unit, tick),
            };
            let position =
                best.partition_point(|prior| prior.preference() < candidate.preference());
            if best
                .get(position)
                .is_some_and(|prior| prior.unit.id == unit.id)
            {
                continue;
            }
            if position < fields.len() {
                best.insert(position, candidate);
                best.truncate(fields.len());
            }
        }
        if best.is_empty() {
            return Err(production_setup_refusal(format!(
                "production requires a known living worker with {field}=true, or an explicitly false field and prerequisites.assign_labor=true; unavailable labor cannot establish staffing or compensation"
            )));
        }
        candidates.push(best);
    }
    let mut search = Search {
        nodes: 0,
        best: None,
    };
    search.visit(&candidates, &mut Vec::with_capacity(fields.len()))?;
    search
        .best
        .map(|(_, selected)| selected)
        .ok_or_else(|| invalid("production staffing found no bounded candidate assignment"))
}

#[cfg(test)]
#[path = "production_staffing_tests.rs"]
mod tests;
