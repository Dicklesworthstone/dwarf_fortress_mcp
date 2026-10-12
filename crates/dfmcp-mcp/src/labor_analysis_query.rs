//! Read-only observed staffing analysis and handoff to the existing guarded workflow.
//! Skills and stress remain declared preferences; current labor bits come only
//! from eligible snapshot facts. A query never prepares or dispatches an effect.

use std::collections::BTreeMap;
use std::fmt;

use dfmcp_core::{DfmcpError, Digest32, EntityId, ErrorCode, Result};
use dfmcp_intent::labor_observation::{LaborPreference, LaborReadBudget, ObservedLaborRoster};
use dfmcp_intent::{LaborAllocationLimits, LaborAllocator, LaborDemand};
use dfmcp_world::{PredicateEvidence, WorldSnapshot};
use serde::{Deserialize, Deserializer, de::{MapAccess, Visitor}};
use serde_json::{Value, json};

const MAX_WORK: u64 = 1_000_000;
const MAX_BYTES: u64 = 256 * 1024;
const MAX_REPLY_BYTES: usize = 32 * 1024;
const MAX_PROBLEMS: usize = 16;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode { LaborAllocation }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    mode: Mode,
    roster: Vec<Preference>,
    demands: Vec<Demand>,
    max_work: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    id: String,
    #[serde(deserialize_with = "unique_skills")]
    skills: BTreeMap<String, u32>,
    stress_level: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Demand { labor: String, skill: String, workers: u32, minimum_skill: u32, priority: u8 }

fn invalid(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, message) }
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "staffing read, allocation, compilation or complete handoff exceeds the aggregate allowance")
}
fn unique_skills<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<BTreeMap<String, u32>, D::Error> {
    struct Skills;
    impl<'de> Visitor<'de> for Skills {
        type Value = BTreeMap<String, u32>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("distinct skill keys and unsigned ratings") }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, u32>()? {
                if result.len() == 256 || result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate skill key or too many skills"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Skills)
}
fn entity(raw: &str) -> Result<EntityId> {
    if raw.is_empty() || raw.len() > 20 || raw.starts_with('0') || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("staffing IDs must be canonical nonzero decimal u64 strings"));
    }
    raw.parse::<u64>().map(EntityId::new).map_err(|_| invalid("staffing ID exceeds u64"))
}
fn limits(remaining: u64) -> LaborAllocationLimits {
    LaborAllocationLimits {
        max_roster: 64, max_demands: 16, max_positions: 64,
        max_actions: super::MAX_LAB_STEPS, max_work: remaining,
    }
}
fn remaining(maximum: u64, used: u64) -> Result<u64> {
    maximum.checked_sub(used).filter(|n| *n > 0).ok_or_else(exhausted)
}
fn finish(out: Value, bytes_used: u64) -> Result<Value> {
    let size = out.to_string().len();
    if size > MAX_REPLY_BYTES || bytes_used.checked_add(size as u64).is_none_or(|n| n > MAX_BYTES) {
        return Err(exhausted());
    }
    Ok(out)
}

pub(super) fn query(snapshot: &WorldSnapshot, raw: &str) -> Result<Value> {
    if raw.len() > super::MAX_QUERY_JSON_BYTES { return Err(exhausted()); }
    let request: Request = serde_json::from_str(raw).map_err(|_| invalid("invalid closed labor_allocation query"))?;
    let Mode::LaborAllocation = request.mode;
    let maximum = request.max_work.unwrap_or(MAX_WORK);
    if maximum == 0 || maximum > MAX_WORK { return Err(exhausted()); }
    let preferences: Vec<_> = request.roster.into_iter().map(|row| Ok(LaborPreference {
        id: entity(&row.id)?, skills: row.skills, stress_level: row.stress_level,
    })).collect::<Result<_>>()?;
    let mut demands: Vec<_> = request.demands.into_iter().map(|d| LaborDemand {
        labor: d.labor, skill: d.skill, workers: d.workers, minimum_skill: d.minimum_skill, priority: d.priority,
    }).collect();
    // Stable request bytes as well as stable allocation: caller ordering cannot
    // change the proposed workflow or its digest.
    demands.sort_by(|a, b| (a.priority, &a.labor).cmp(&(b.priority, &b.labor)));
    let evidence = PredicateEvidence::laboratory(snapshot)?;
    let observed = ObservedLaborRoster::inspect(&evidence, &preferences, &demands, LaborReadBudget {
        max_work: maximum, ..LaborReadBudget::default()
    })?;
    let mut used = observed.work_used();
    let mut bytes_used = observed.accounted_bytes();
    let anchor = observed.anchor();
    let mut out = json!({
        "mode": "labor_allocation",
        "source_anchor": {"fortress_id": anchor.fortress_id.to_string(), "epoch": anchor.cursor.epoch,
            "sequence": anchor.cursor.sequence, "game_tick": anchor.tick.0, "state_hash": anchor.state_hash.to_string()},
        "readiness": "missing_evidence", "model_feasible": null,
        "configuration_proven": false, "execution_ready": false, "native_evidence": false,
        "coverage": {"managed_matrix_complete": observed.profiles().is_some(),
            "known_enabled": observed.known_enabled(), "known_disabled": observed.known_disabled(),
            "unmanaged_labors": "qualified_positive_assignments_only", "skills_and_stress": "caller_declared_preferences"},
        "problems_total": observed.problems().len(), "problems_truncated": observed.problems().len() > MAX_PROBLEMS,
        "problems": observed.problems().iter().take(MAX_PROBLEMS).map(|p| json!({
            "entity_id": p.entity_id.get().to_string(), "kind": p.kind.as_str(), "field": p.field,
        })).collect::<Vec<_>>(),
        "staffing": [], "missing_workers": null, "proposed_workflow_json": null, "proposed_workflow_digest": null,
        "work_used": used, "maximum_work": maximum,
        "note": "analysis is not a plan or permission; review proposed_workflow_json through fortress.plan, then explicitly commit its sealed digest",
    });
    let Some(profiles) = observed.profiles() else { return finish(out, bytes_used); };
    let allocation = LaborAllocator.allocate_demands(profiles, &demands, limits(remaining(maximum, used)?))?;
    used = used.checked_add(allocation.work_used()).ok_or_else(exhausted)?;
    out["work_used"] = json!(used);
    out["model_feasible"] = json!(allocation.is_fully_staffed());
    out["missing_workers"] = json!(allocation.staffing().iter().map(|r| u64::from(r.missing_workers)).sum::<u64>());
    out["staffing"] = json!(allocation.staffing().iter().map(|r| json!({
        "labor": r.labor, "skill": r.skill, "priority": r.priority, "requested": r.requested_workers,
        "eligible": r.eligible_workers, "assigned": r.assigned_units.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
        "missing": r.missing_workers,
    })).collect::<Vec<_>>());
    out["readiness"] = json!("staffing_shortage");
    if !allocation.is_fully_staffed() { return finish(out, bytes_used); }
    out["readiness"] = json!("already_configured");
    if allocation.actions().is_empty() {
        out["configuration_proven"] = json!(true);
        return finish(out, bytes_used);
    }
    // Reuse the real compiler to ensure this complete proposal can be lowered.
    // Its second matching pass is charged to the same remaining work budget.
    let compiled = LaborAllocator.compile_execution(profiles, &demands, limits(remaining(maximum, used)?))?;
    used = used.checked_add(compiled.work_used()).ok_or_else(exhausted)?;
    bytes_used = bytes_used.checked_add(compiled.encoded_bytes() as u64).ok_or_else(exhausted)?;
    let workflow = json!({
        "workflow": "labor_allocation_v1",
        "roster": profiles.iter().map(|p| json!({"id":p.id.get().to_string(), "skills":p.skills,
            "stress_level":p.stress_level, "assigned_labors":p.assigned_labors})).collect::<Vec<_>>(),
        "demands": demands.iter().map(|d| json!({"labor":d.labor,"skill":d.skill,"workers":d.workers,
            "minimum_skill":d.minimum_skill,"priority":d.priority})).collect::<Vec<_>>(),
    }).to_string();
    if workflow.len() > super::MAX_ACTIONS_JSON_BYTES { return Err(exhausted()); }
    bytes_used = bytes_used.checked_add(workflow.len() as u64).ok_or_else(exhausted)?;
    out["readiness"] = json!("proposal_available");
    out["work_used"] = json!(used);
    out["proposed_workflow_digest"] = json!(Digest32::of_bytes(workflow.as_bytes()).to_string());
    // A scalar JSON string cannot be shortened by whole-row output fitting into
    // a different valid subset of executable actions. It may be omitted whole.
    out["proposed_workflow_json"] = json!(workflow);
    finish(out, bytes_used)
}
