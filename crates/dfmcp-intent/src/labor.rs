#![forbid(unsafe_code)]

//! WP-PLN-03: bounded labor proposals from a caller-declared workforce model.
//!
//! [`LaborAllocator::allocate_demands`] fills priority-ordered exclusive roles,
//! including roles for which no master exists. Alternating paths move a flexible
//! worker when that is necessary to staff a scarce specialty. The result is a
//! proposal, not observed staffing, a native skill mapping, or effect authority.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};

use crate::action::Action;

/// At or above this modeled stress, citizens receive no new staffing role.
pub const HIGH_STRESS_THRESHOLD: u32 = 800;

const HEAVY_LABORS: [&str; 5] = [
    "MINING",
    "MASONRY",
    "WEAPONSMITHING",
    "ARMORING",
    "CARPENTRY",
];
const MAX_PROFILE_TOKENS: usize = 256;
const MAX_TOKEN_BYTES: usize = 256;

/// Caller-declared citizen profile; this does not certify native observations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DwarfLaborProfile {
    pub id: EntityId,
    pub skills: BTreeMap<String, u32>,
    pub stress_level: u32,
    pub assigned_labors: BTreeSet<String>,
}

/// One exclusive role, with an explicit labor-to-skill mapping.
///
/// Smaller priorities are filled first; equal priorities use ascending labor
/// tokens. A zero minimum permits unskilled generalists. A zero worker target
/// explicitly retires that managed role. Duplicate labor targets are refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaborDemand {
    pub labor: String,
    pub skill: String,
    pub workers: u32,
    pub minimum_skill: u32,
    pub priority: u8,
}

/// Independent limits; exhaustion returns no partial allocation or actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaborAllocationLimits {
    pub max_roster: usize,
    pub max_demands: usize,
    pub max_positions: usize,
    pub max_actions: usize,
    pub max_work: u64,
}

impl Default for LaborAllocationLimits {
    fn default() -> Self {
        Self {
            max_roster: 512,
            max_demands: 64,
            max_positions: 512,
            max_actions: 64,
            max_work: 1_000_000,
        }
    }
}

/// One role's modeled coverage. Eligibility excludes high-stress citizens.
/// An eligible citizen may be needed by a higher-priority or scarcer role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaborStaffing {
    pub labor: String,
    pub skill: String,
    pub priority: u8,
    pub requested_workers: u32,
    pub eligible_workers: usize,
    pub assigned_units: Vec<EntityId>,
    pub missing_workers: u32,
}

/// A bounded, deterministic proposal. Unmentioned labors are left unchanged,
/// except for the existing high-stress heavy-labor relief rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaborAllocation {
    staffing: Vec<LaborStaffing>,
    actions: Vec<Action>,
    relieved_units: Vec<EntityId>,
    work_used: u64,
}

impl LaborAllocation {
    #[must_use]
    pub fn staffing(&self) -> &[LaborStaffing] {
        &self.staffing
    }

    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Citizens excluded from new staffing by the stress guard, even if they
    /// had no current assignment to disable.
    #[must_use]
    pub fn relieved_units(&self) -> &[EntityId] {
        &self.relieved_units
    }

    #[must_use]
    pub fn is_fully_staffed(&self) -> bool {
        self.staffing.iter().all(|role| role.missing_workers == 0)
    }

    /// Deterministic validation, candidate, traversal and output work units.
    #[must_use]
    pub const fn work_used(&self) -> u64 {
        self.work_used
    }

    /// Disable-before-enable, stable-ID grouped proposals. Ordinary observed
    /// preconditions, plan sealing, authority and commit revalidation still apply.
    #[must_use]
    pub fn into_actions(self) -> Vec<Action> {
        self.actions
    }
}

#[derive(Clone, Debug, Default)]
pub struct LaborAllocator;

impl LaborAllocator {
    /// Compatibility heuristic: relieve heavy labor and enable existing experts.
    /// Use `allocate_demands` for staffing targets and exclusive specializations.
    pub fn optimize_roster(&self, roster: &[DwarfLaborProfile]) -> Result<Vec<Action>> {
        let limits = LaborAllocationLimits::default();
        let mut work = Work::new(limits.max_work);
        let roster = validate_roster(roster, limits.max_roster, &mut work)?;
        let mut actions = Vec::new();
        for dwarf in roster {
            if dwarf.stress_level >= HIGH_STRESS_THRESHOLD {
                for labor in &dwarf.assigned_labors {
                    if HEAVY_LABORS.contains(&labor.as_str()) {
                        actions.push(Action::SetLabor {
                            units: vec![dwarf.id],
                            labor: labor.clone(),
                            enabled: false,
                        });
                    }
                }
            } else {
                for (skill, &rating) in &dwarf.skills {
                    if rating >= 10 && !dwarf.assigned_labors.contains(skill) {
                        actions.push(Action::SetLabor {
                            units: vec![dwarf.id],
                            labor: skill.clone(),
                            enabled: true,
                        });
                    }
                }
            }
        }
        Ok(actions)
    }

    /// Maximize coverage lexicographically by `(priority, labor)` with at most
    /// one demanded role per citizen. This is not a global skill-score optimizer.
    /// Among feasible augmenting paths, prefer fewer reassignments; candidate
    /// order is skill descending, existing assignment, stress ascending, then ID.
    /// Missing skills mean rating zero in this declared model, not known native
    /// absence. No proposal is applied and no input profile is mutated.
    pub fn allocate_demands(
        &self,
        roster: &[DwarfLaborProfile],
        demands: &[LaborDemand],
        limits: LaborAllocationLimits,
    ) -> Result<LaborAllocation> {
        validate_limits(limits)?;
        let mut work = Work::new(limits.max_work);
        let roster = validate_roster(roster, limits.max_roster, &mut work)?;
        let demands = validate_demands(demands, limits, &mut work)?;
        let mut candidates = Vec::with_capacity(demands.len());
        let mut positions = Vec::new();
        for (role, demand) in demands.iter().enumerate() {
            let mut eligible = Vec::new();
            for (worker, dwarf) in roster.iter().enumerate() {
                work.charge()?;
                if dwarf.stress_level < HIGH_STRESS_THRESHOLD
                    && rating(dwarf, &demand.skill) >= demand.minimum_skill
                {
                    eligible.push(worker);
                }
            }
            eligible.sort_by_key(|&worker| {
                let dwarf = roster[worker];
                (
                    std::cmp::Reverse(rating(dwarf, &demand.skill)),
                    !dwarf.assigned_labors.contains(&demand.labor),
                    dwarf.stress_level,
                    dwarf.id,
                )
            });
            candidates.push(eligible);
            for _ in 0..demand.workers {
                work.charge()?;
                positions.push(role);
            }
        }

        let mut position_workers = vec![None; positions.len()];
        let mut worker_positions = vec![None; roster.len()];
        for position in 0..positions.len() {
            augment(
                position,
                &positions,
                &candidates,
                &mut position_workers,
                &mut worker_positions,
                &mut work,
            )?;
        }

        let mut staffing = Vec::with_capacity(demands.len());
        for (role, demand) in demands.iter().enumerate() {
            let mut assigned_units = Vec::new();
            for (worker, position) in worker_positions.iter().enumerate() {
                work.charge()?;
                if position.is_some_and(|position| positions[position] == role) {
                    assigned_units.push(roster[worker].id);
                }
            }
            let assigned = u32::try_from(assigned_units.len()).map_err(|_| exhausted())?;
            staffing.push(LaborStaffing {
                labor: demand.labor.clone(),
                skill: demand.skill.clone(),
                priority: demand.priority,
                requested_workers: demand.workers,
                eligible_workers: candidates[role].len(),
                assigned_units,
                missing_workers: demand.workers.saturating_sub(assigned),
            });
        }

        // Group by (enabled, labor): all disables precede every enable.
        let mut changes = BTreeMap::<(bool, String), Vec<EntityId>>::new();
        let mut relieved_units = Vec::new();
        for (worker, dwarf) in roster.iter().enumerate() {
            let selected = worker_positions[worker].map(|position| positions[position]);
            for (role, demand) in demands.iter().enumerate() {
                work.charge()?;
                let enabled = selected == Some(role);
                if dwarf.assigned_labors.contains(&demand.labor) != enabled {
                    changes
                        .entry((enabled, demand.labor.clone()))
                        .or_default()
                        .push(dwarf.id);
                }
            }
            if dwarf.stress_level >= HIGH_STRESS_THRESHOLD {
                relieved_units.push(dwarf.id);
                for labor in HEAVY_LABORS {
                    work.charge()?;
                    if dwarf.assigned_labors.contains(labor)
                        && !demands.iter().any(|demand| demand.labor == labor)
                    {
                        changes
                            .entry((false, labor.to_owned()))
                            .or_default()
                            .push(dwarf.id);
                    }
                }
            }
        }
        if changes.len() > limits.max_actions {
            return Err(exhausted());
        }
        let actions = changes
            .into_iter()
            .map(|((enabled, labor), units)| Action::SetLabor {
                units,
                labor,
                enabled,
            })
            .collect();
        Ok(LaborAllocation {
            staffing,
            actions,
            relieved_units,
            work_used: work.used,
        })
    }
}

fn rating(dwarf: &DwarfLaborProfile, skill: &str) -> u32 {
    dwarf.skills.get(skill).copied().map_or(0, |value| value)
}

struct Work {
    used: u64,
    limit: u64,
}

impl Work {
    const fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }

    fn charge(&mut self) -> Result<()> {
        if self.used >= self.limit {
            return Err(exhausted());
        }
        self.used += 1;
        Ok(())
    }
}

fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "labor allocation budget exhausted")
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn validate_token(token: &str) -> Result<()> {
    if token.trim().is_empty() || token.len() > MAX_TOKEN_BYTES || token.contains('\0') {
        return Err(invalid(
            "labor tokens require 1..256 nonblank UTF-8 bytes without NUL",
        ));
    }
    Ok(())
}

fn validate_limits(limits: LaborAllocationLimits) -> Result<()> {
    if limits.max_roster > 512
        || limits.max_demands > 64
        || limits.max_positions > 512
        || limits.max_actions > 2_048
        || limits.max_work > 10_000_000
    {
        return Err(invalid("labor allocation limits exceed fixed model bounds"));
    }
    Ok(())
}

fn validate_roster<'a>(
    roster: &'a [DwarfLaborProfile],
    max_roster: usize,
    work: &mut Work,
) -> Result<Vec<&'a DwarfLaborProfile>> {
    if roster.len() > max_roster {
        return Err(exhausted());
    }
    let mut seen = BTreeSet::new();
    for dwarf in roster {
        work.charge()?;
        if dwarf.id == EntityId::NIL || !seen.insert(dwarf.id) || dwarf.stress_level > 1_000 {
            return Err(invalid(
                "labor roster contains a nil/duplicate ID or out-of-range stress",
            ));
        }
        if dwarf.skills.len() > MAX_PROFILE_TOKENS
            || dwarf.assigned_labors.len() > MAX_PROFILE_TOKENS
        {
            return Err(exhausted());
        }
        for (skill, rating) in &dwarf.skills {
            work.charge()?;
            validate_token(skill)?;
            if *rating > 20 {
                return Err(invalid("labor skill rating exceeds 20"));
            }
        }
        for labor in &dwarf.assigned_labors {
            work.charge()?;
            validate_token(labor)?;
        }
    }
    let mut sorted: Vec<_> = roster.iter().collect();
    sorted.sort_by_key(|dwarf| dwarf.id);
    Ok(sorted)
}

fn validate_demands<'a>(
    demands: &'a [LaborDemand],
    limits: LaborAllocationLimits,
    work: &mut Work,
) -> Result<Vec<&'a LaborDemand>> {
    if demands.len() > limits.max_demands {
        return Err(exhausted());
    }
    let mut seen = BTreeSet::new();
    let mut positions = 0_usize;
    for demand in demands {
        work.charge()?;
        validate_token(&demand.labor)?;
        validate_token(&demand.skill)?;
        if !seen.insert(&demand.labor) || demand.minimum_skill > 20 {
            return Err(invalid(
                "labor demands contain a duplicate role or invalid minimum skill",
            ));
        }
        positions = positions
            .checked_add(usize::try_from(demand.workers).map_err(|_| exhausted())?)
            .ok_or_else(exhausted)?;
        if positions > limits.max_positions {
            return Err(exhausted());
        }
    }
    let mut sorted: Vec<_> = demands.iter().collect();
    sorted.sort_by(|left, right| (left.priority, &left.labor).cmp(&(right.priority, &right.labor)));
    Ok(sorted)
}

/// Iterative alternating-path search: no recursion and no dropped earlier seat.
fn augment(
    root: usize,
    positions: &[usize],
    candidates: &[Vec<usize>],
    position_workers: &mut [Option<usize>],
    worker_positions: &mut [Option<usize>],
    work: &mut Work,
) -> Result<()> {
    work.charge()?;
    let mut parents = vec![None; worker_positions.len()];
    let mut visited = vec![false; positions.len()];
    let mut queue = VecDeque::from([root]);
    visited[root] = true;
    while let Some(position) = queue.pop_front() {
        for &worker in &candidates[positions[position]] {
            work.charge()?;
            if parents[worker].is_some() {
                continue;
            }
            parents[worker] = Some(position);
            if let Some(occupied) = worker_positions[worker] {
                if !visited[occupied] {
                    visited[occupied] = true;
                    queue.push_back(occupied);
                }
                continue;
            }
            let mut free_worker = worker;
            loop {
                work.charge()?;
                let target = parents[free_worker].ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "broken labor augmenting path",
                    )
                })?;
                let previous = position_workers[target].replace(free_worker);
                worker_positions[free_worker] = Some(target);
                match previous {
                    Some(displaced) => {
                        worker_positions[displaced] = None;
                        free_worker = displaced;
                    }
                    None => return Ok(()),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dwarf(id: u64, skills: &[(&str, u32)]) -> DwarfLaborProfile {
        DwarfLaborProfile {
            id: EntityId::new(id),
            skills: skills
                .iter()
                .map(|(skill, rating)| ((*skill).to_owned(), *rating))
                .collect(),
            stress_level: 0,
            assigned_labors: BTreeSet::new(),
        }
    }

    fn demand(labor: &str, workers: u32, minimum_skill: u32, priority: u8) -> LaborDemand {
        LaborDemand {
            labor: labor.to_owned(),
            skill: labor.to_owned(),
            workers,
            minimum_skill,
            priority,
        }
    }

    fn allocate(roster: &[DwarfLaborProfile], demands: &[LaborDemand]) -> Result<LaborAllocation> {
        LaborAllocator.allocate_demands(roster, demands, LaborAllocationLimits::default())
    }

    #[test]
    fn test_high_stress_dwarf_relieved_from_heavy_labor() -> Result<()> {
        let mut citizen = dwarf(1, &[]);
        citizen.stress_level = 850;
        citizen.assigned_labors = BTreeSet::from(["MINING".to_owned(), "HAULING".to_owned()]);
        let actions = LaborAllocator.optimize_roster(&[citizen])?;
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            Action::SetLabor { units, labor, enabled: false }
                if units == &vec![EntityId::new(1)] && labor == "MINING"
        ));
        Ok(())
    }

    #[test]
    fn test_master_craftsdwarf_assigned_labor() -> Result<()> {
        let actions = LaborAllocator.optimize_roster(&[dwarf(2, &[("WEAPONSMITHING", 15)])])?;
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            Action::SetLabor { labor, enabled: true, .. } if labor == "WEAPONSMITHING"
        ));
        Ok(())
    }

    #[test]
    fn reassigns_flexible_worker_instead_of_stranding_specialty() -> Result<()> {
        let roster = [
            dwarf(1, &[("BREWING", 15), ("MINING", 15)]),
            dwarf(2, &[("BREWING", 5)]),
        ];
        let demands = [demand("BREWING", 1, 1, 0), demand("MINING", 1, 1, 1)];
        let plan = allocate(&roster, &demands)?;
        assert!(plan.is_fully_staffed());
        assert_eq!(plan.staffing()[0].assigned_units, vec![EntityId::new(2)]);
        assert_eq!(plan.staffing()[1].assigned_units, vec![EntityId::new(1)]);
        Ok(())
    }

    #[test]
    fn fills_generalist_roles_without_fabricating_skills() -> Result<()> {
        let demands = [demand("HAULING", 1, 0, 0), demand("MINING", 1, 1, 1)];
        let plan = allocate(&[dwarf(1, &[])], &demands)?;
        assert!(!plan.is_fully_staffed());
        assert_eq!(plan.staffing()[0].missing_workers, 0);
        assert_eq!(plan.staffing()[1].eligible_workers, 0);
        assert_eq!(plan.staffing()[1].missing_workers, 1);
        Ok(())
    }

    #[test]
    fn priority_coverage_never_counts_one_worker_twice() -> Result<()> {
        let roster = [dwarf(1, &[("BREWING", 20), ("MINING", 20)])];
        let demands = [demand("BREWING", 2, 0, 0), demand("MINING", 1, 0, 1)];
        let plan = allocate(&roster, &demands)?;
        assert_eq!(plan.staffing()[0].assigned_units.len(), 1);
        assert_eq!(plan.staffing()[0].missing_workers, 1);
        assert!(plan.staffing()[1].assigned_units.is_empty());
        assert_eq!(plan.staffing()[1].eligible_workers, 1);
        Ok(())
    }

    #[test]
    fn threshold_relief_replaces_expert_and_preserves_unmanaged_labor() -> Result<()> {
        let mut expert = dwarf(1, &[("MINING", 20)]);
        expert.stress_level = HIGH_STRESS_THRESHOLD;
        expert.assigned_labors = BTreeSet::from(["MINING".to_owned(), "HAULING".to_owned()]);
        let plan = allocate(
            &[expert, dwarf(2, &[("MINING", 1)])],
            &[demand("MINING", 1, 1, 0)],
        )?;
        assert_eq!(plan.relieved_units(), &[EntityId::new(1)]);
        assert_eq!(plan.actions().len(), 2);
        assert!(matches!(
            &plan.actions()[0],
            Action::SetLabor { labor, enabled: false, .. } if labor == "MINING"
        ));
        assert!(matches!(
            &plan.actions()[1],
            Action::SetLabor { units, enabled: true, .. } if units == &vec![EntityId::new(2)]
        ));
        Ok(())
    }

    #[test]
    fn explicit_skill_mapping_and_zero_target_retirement() -> Result<()> {
        let mut citizen = dwarf(1, &[("CARVING", 12)]);
        citizen.assigned_labors.insert("MINING".to_owned());
        let mut role = demand("MASONRY", 1, 10, 0);
        role.skill = "CARVING".to_owned();
        let plan = allocate(&[citizen], &[role, demand("MINING", 0, 0, 1)])?;
        assert!(plan.is_fully_staffed());
        assert_eq!(plan.actions().len(), 2);
        assert_eq!(plan.staffing()[0].assigned_units, vec![EntityId::new(1)]);
        Ok(())
    }

    #[test]
    fn roster_and_demand_order_do_not_change_plan_or_work() -> Result<()> {
        let roster = vec![
            dwarf(3, &[("A", 10)]),
            dwarf(1, &[("A", 10), ("B", 5)]),
            dwarf(2, &[("B", 5)]),
        ];
        let demands = vec![demand("B", 1, 1, 1), demand("A", 2, 1, 0)];
        let expected = allocate(&roster, &demands)?;
        let mut reversed = roster.clone();
        reversed.reverse();
        let mut reversed_demands = demands.clone();
        reversed_demands.reverse();
        assert_eq!(expected, allocate(&reversed, &reversed_demands)?);
        assert_eq!(roster[0].assigned_labors, BTreeSet::new());
        Ok(())
    }

    #[test]
    fn prefers_existing_equal_skill_assignment_and_groups_units() -> Result<()> {
        let mut incumbent = dwarf(3, &[("A", 10)]);
        incumbent.assigned_labors.insert("A".to_owned());
        let roster = [
            dwarf(1, &[("A", 10)]),
            dwarf(2, &[("A", 10)]),
            incumbent,
        ];
        let plan = allocate(&roster, &[demand("A", 2, 1, 0)])?;
        assert_eq!(
            plan.staffing()[0].assigned_units,
            vec![EntityId::new(1), EntityId::new(3)]
        );
        assert_eq!(plan.actions().len(), 1);
        assert!(matches!(
            &plan.actions()[0],
            Action::SetLabor { units, enabled: true, .. } if units == &vec![EntityId::new(1)]
        ));
        let grouped = allocate(&roster[..2], &[demand("A", 2, 1, 0)])?;
        assert_eq!(grouped.actions().len(), 1);
        assert!(matches!(
            &grouped.actions()[0], Action::SetLabor { units, .. } if units.len() == 2
        ));
        Ok(())
    }

    #[test]
    fn refuses_invalid_models_and_independent_budgets() {
        let roster = [dwarf(1, &[("A", 1)])];
        let demands = [demand("A", 1, 1, 0)];
        assert!(allocate(&[roster[0].clone(), roster[0].clone()], &demands).is_err());
        assert!(allocate(&roster, &[demands[0].clone(), demands[0].clone()]).is_err());
        for limits in [
            LaborAllocationLimits {
                max_roster: 0,
                ..LaborAllocationLimits::default()
            },
            LaborAllocationLimits {
                max_demands: 0,
                ..LaborAllocationLimits::default()
            },
            LaborAllocationLimits {
                max_positions: 0,
                ..LaborAllocationLimits::default()
            },
            LaborAllocationLimits {
                max_actions: 0,
                ..LaborAllocationLimits::default()
            },
            LaborAllocationLimits {
                max_work: 0,
                ..LaborAllocationLimits::default()
            },
        ] {
            assert!(LaborAllocator.allocate_demands(&roster, &demands, limits).is_err());
        }
        assert!(allocate(&roster, &[demand("A", u32::MAX, 1, 0)]).is_err());
        assert!(allocate(&roster, &[demand("A\0", 1, 1, 0)]).is_err());
        assert!(allocate(&roster, &[demand("A", 1, 21, 0)]).is_err());
    }

    #[test]
    fn empty_roster_reports_all_shortages() -> Result<()> {
        let plan = allocate(&[], &[demand("A", 2, 0, 0), demand("B", 1, 0, 1)])?;
        assert_eq!(
            plan.staffing()
                .iter()
                .map(|role| role.missing_workers)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert!(plan.actions().is_empty());
        Ok(())
    }

    #[test]
    fn exhaustive_three_by_three_matches_priority_coverage_oracle() -> Result<()> {
        let names = ["A", "B", "C"];
        let demands: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(role, name)| demand(name, 1, 1, role as u8))
            .collect();
        for mask in 0_u32..512 {
            let roster: Vec<_> = (0..3)
                .map(|worker| {
                    let skills: Vec<_> = (0..3)
                        .filter(|role| mask & (1 << (worker * 3 + role)) != 0)
                        .map(|role| (names[role], 1))
                        .collect();
                    dwarf(worker as u64 + 1, &skills)
                })
                .collect();
            let actual: Vec<_> = allocate(&roster, &demands)?
                .staffing()
                .iter()
                .map(|role| role.assigned_units.len())
                .collect();
            let mut best = vec![0; 3];
            // Four choices per citizen: one of three roles or unassigned.
            for assignment in 0_usize..64 {
                let mut remaining = assignment;
                let mut counts = vec![0; 3];
                let mut valid = true;
                for worker in 0..3 {
                    let role = remaining % 4;
                    remaining /= 4;
                    if role < 3 {
                        if mask & (1 << (worker * 3 + role)) == 0 || counts[role] != 0 {
                            valid = false;
                            break;
                        }
                        counts[role] += 1;
                    }
                }
                if valid && counts > best {
                    best = counts;
                }
            }
            assert_eq!(actual, best, "eligibility mask {mask}");
        }
        Ok(())
    }
}
