//! Read current labor configuration without promoting staffing preferences into facts.
//!
//! A complete result covers only the named citizens and demanded labor bits.
//! Other enabled labors are retained only when positively source-qualified; no
//! absent field establishes a disabled labor or a complete native labor domain.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    Capability, DfmcpError, EntityId, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, PredicateTruth, Value};

use crate::{DwarfLaborProfile, LaborDemand};

pub const MAX_OBSERVED_LABOR_ROSTER: usize = 64;
pub const MAX_OBSERVED_LABOR_DEMANDS: usize = 16;
const MAX_TOKENS: usize = 256;
const MAX_FIELDS: usize = 512;
const MAX_BYTES: u64 = 256 * 1024;
const MAX_WORK: u64 = 1_000_000;

/// Allocation preferences, never observation evidence. There is deliberately no
/// caller-supplied initial labor matrix in this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaborPreference {
    pub id: EntityId,
    pub skills: BTreeMap<String, u32>,
    pub stress_level: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaborReadBudget {
    pub max_entities: usize,
    pub max_bytes: u64,
    pub max_work: u64,
}
impl Default for LaborReadBudget {
    fn default() -> Self {
        Self { max_entities: MAX_OBSERVED_LABOR_ROSTER, max_bytes: MAX_BYTES, max_work: MAX_WORK }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaborReadProblemKind {
    UnprovenUnit,
    NotAUnit,
    UnprovenLiveness,
    NotAlive,
    UnprovenLabor,
}
impl LaborReadProblemKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnprovenUnit => "unproven_unit",
            Self::NotAUnit => "not_a_unit",
            Self::UnprovenLiveness => "unproven_liveness",
            Self::NotAlive => "not_alive",
            Self::UnprovenLabor => "unproven_labor",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaborReadProblem {
    pub entity_id: EntityId,
    pub kind: LaborReadProblemKind,
    /// A requested field, not a disclosure of an unqualified source value.
    pub field: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedLaborRoster {
    anchor: StateAnchor,
    profiles: Vec<DwarfLaborProfile>,
    problems: Vec<LaborReadProblem>,
    known_enabled: usize,
    known_disabled: usize,
    work_used: u64,
    accounted_bytes: u64,
}
impl ObservedLaborRoster {
    #[must_use]
    pub const fn anchor(&self) -> StateAnchor { self.anchor }
    #[must_use]
    pub fn problems(&self) -> &[LaborReadProblem] { &self.problems }
    /// No partial profile list can escape when any required observation is absent.
    #[must_use]
    pub fn profiles(&self) -> Option<&[DwarfLaborProfile]> {
        self.problems.is_empty().then_some(self.profiles.as_slice())
    }
    #[must_use]
    pub const fn known_enabled(&self) -> usize { self.known_enabled }
    #[must_use]
    pub const fn known_disabled(&self) -> usize { self.known_disabled }
    #[must_use]
    pub const fn work_used(&self) -> u64 { self.work_used }
    #[must_use]
    pub const fn accounted_bytes(&self) -> u64 { self.accounted_bytes }

    /// Pure projection under an already issued source scope. The calling shell
    /// must authorize reading this roster. This method grants no capabilities.
    pub fn inspect(
        evidence: &PredicateEvidence<'_>,
        preferences: &[LaborPreference],
        demands: &[LaborDemand],
        budget: LaborReadBudget,
    ) -> Result<Self> {
        let mut work = ReadWork { budget, used: 0, bytes: 0 };
        validate(preferences, demands, &mut work)?;
        let mut sorted: Vec<_> = preferences.iter().collect();
        sorted.sort_by_key(|p| p.id);
        let managed: BTreeSet<_> = demands.iter().map(|d| d.labor.as_str()).collect();
        let mut out = Self {
            anchor: evidence.snapshot().anchor(), profiles: Vec::new(), problems: Vec::new(),
            known_enabled: 0, known_disabled: 0, work_used: 0, accounted_bytes: 0,
        };
        for preference in sorted {
            work.charge(1, 128)?;
            let id = preference.id;
            match evidence.evaluate(&Predicate::EntityKind { entity_id: id, kind: EntityKind::Unit })? {
                PredicateTruth::True => {}
                truth => {
                    out.problem(id, if truth == PredicateTruth::False {
                        LaborReadProblemKind::NotAUnit
                    } else { LaborReadProblemKind::UnprovenUnit }, None);
                    continue;
                }
            }
            work.charge(1, 0)?;
            match boolean(evidence, id, "alive")? {
                Some(true) => {}
                other => {
                    out.problem(id, if other == Some(false) {
                        LaborReadProblemKind::NotAlive
                    } else { LaborReadProblemKind::UnprovenLiveness }, Some("alive".to_owned()));
                    continue;
                }
            }
            let entity = evidence.snapshot().graph.entities.get(&id).ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation, "qualified citizen disappeared")
            })?;
            if entity.fields.len() > MAX_FIELDS { return Err(exhausted()); }
            let mut assigned_labors = BTreeSet::new();
            for name in &managed {
                let field = format!("{}{name}", crate::effects::LABOR_FIELD_PREFIX);
                work.charge(1, 128 + field.len() as u64)?;
                match boolean(evidence, id, &field)? {
                    Some(true) => { assigned_labors.insert((*name).to_owned()); out.known_enabled += 1; }
                    Some(false) => out.known_disabled += 1,
                    None => out.problem(id, LaborReadProblemKind::UnprovenLabor, Some(field)),
                }
            }
            // Keep eligible positive assignments for the allocator's existing
            // relief rule. Omitted/unqualified outside roles are not invented.
            for field in entity.fields.keys() {
                work.charge(1, 0)?;
                let Some(name) = field.strip_prefix(crate::effects::LABOR_FIELD_PREFIX) else { continue; };
                if managed.contains(name) || !valid_token(name, 128) { continue; }
                if boolean(evidence, id, field)? == Some(true) {
                    work.charge(1, 64 + name.len() as u64)?;
                    if assigned_labors.len() == MAX_TOKENS { return Err(exhausted()); }
                    assigned_labors.insert(name.to_owned());
                }
            }
            out.profiles.push(DwarfLaborProfile {
                id, skills: preference.skills.clone(), stress_level: preference.stress_level, assigned_labors,
            });
        }
        if !out.problems.is_empty() { out.profiles.clear(); }
        out.work_used = work.used;
        out.accounted_bytes = work.bytes;
        Ok(out)
    }

    /// Embedding entry: current Query authority, exact anchor and the caller's
    /// entity/byte allowance are checked before the source is inspected.
    pub fn inspect_authorized(
        evidence: &PredicateEvidence<'_>,
        preferences: &[LaborPreference],
        demands: &[LaborDemand],
        mut budget: LaborReadBudget,
        context: &OperationContext,
    ) -> Result<Self> {
        if context.anchor != evidence.snapshot().anchor() {
            return Err(DfmcpError::new(ErrorCode::StaleAnchor, "labor read context names another world version"));
        }
        if preferences.len() > MAX_OBSERVED_LABOR_ROSTER { return Err(exhausted()); }
        let ids: Vec<_> = preferences.iter().map(|p| p.id).collect();
        context.authorize(Capability::Query, RiskTier::ReadOnly, &ids, None)?;
        budget.max_entities = budget.max_entities.min(context.budget.max_entities as usize);
        budget.max_bytes = budget.max_bytes.min(context.budget.max_bytes);
        Self::inspect(evidence, preferences, demands, budget)
    }

    fn problem(&mut self, entity_id: EntityId, kind: LaborReadProblemKind, field: Option<String>) {
        self.problems.push(LaborReadProblem { entity_id, kind, field });
    }
}

fn boolean(evidence: &PredicateEvidence<'_>, id: EntityId, name: &str) -> Result<Option<bool>> {
    let value = evidence.snapshot().graph.entities.get(&id)
        .and_then(|entity| entity.fields.get(name)).and_then(|fact| fact.known_value());
    let Some(Value::Bool(value)) = value else { return Ok(None); };
    let predicate = Predicate::FieldCompare {
        entity_id: id, field: name.to_owned(), op: CompareOp::Eq, value: Value::Bool(*value),
    };
    Ok(evidence.establishes(&predicate)?.then_some(*value))
}

fn invalid() -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, "invalid or ambiguous observed staffing request")
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "observed staffing exceeds its entity, byte or work allowance")
}
fn valid_token(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.trim().is_empty() && !value.chars().any(char::is_control)
}
struct ReadWork { budget: LaborReadBudget, used: u64, bytes: u64 }
impl ReadWork {
    fn charge(&mut self, work: u64, bytes: u64) -> Result<()> {
        self.used = self.used.checked_add(work).ok_or_else(exhausted)?;
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(exhausted)?;
        if self.used > self.budget.max_work.min(MAX_WORK) || self.bytes > self.budget.max_bytes.min(MAX_BYTES) {
            return Err(exhausted());
        }
        Ok(())
    }
}
fn validate(preferences: &[LaborPreference], demands: &[LaborDemand], work: &mut ReadWork) -> Result<()> {
    if preferences.is_empty() || demands.is_empty() { return Err(invalid()); }
    if preferences.len() > work.budget.max_entities.min(MAX_OBSERVED_LABOR_ROSTER)
        || demands.len() > MAX_OBSERVED_LABOR_DEMANDS { return Err(exhausted()); }
    let mut ids = BTreeSet::new();
    for profile in preferences {
        work.charge(1, 128)?;
        if profile.id == EntityId::NIL || !ids.insert(profile.id) || profile.stress_level > 1_000 {
            return Err(invalid());
        }
        if profile.skills.len() > MAX_TOKENS { return Err(exhausted()); }
        for (key, rating) in &profile.skills {
            work.charge(1, 128 + key.len() as u64)?;
            if !valid_token(key, 256) || *rating > 20 { return Err(invalid()); }
        }
    }
    let mut names = BTreeSet::new();
    let mut slots = 0u64;
    for demand in demands {
        work.charge(1, 128 + demand.labor.len() as u64 + demand.skill.len() as u64)?;
        if !valid_token(&demand.labor, 128) || !valid_token(&demand.skill, 256)
            || demand.minimum_skill > 20 || !names.insert(&demand.labor) { return Err(invalid()); }
        slots = slots.checked_add(u64::from(demand.workers)).ok_or_else(exhausted)?;
        if slots > 64 { return Err(exhausted()); }
    }
    Ok(())
}
