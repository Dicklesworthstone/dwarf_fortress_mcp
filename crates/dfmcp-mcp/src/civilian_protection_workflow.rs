//! Closed protection requests and reference-world status projection. The server
//! owns Query/Plan/effect authority; the intent crate owns all protection rules.

use std::collections::BTreeSet;
use std::fmt;

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_intent::{CivilianProtectionPlan, RequestedAction, effects};
use dfmcp_world::{PredicateEvidence, PredicateTruth, WorldSnapshot};
use serde::{Deserialize, Deserializer, de::{SeqAccess, Visitor}};
use serde_json::{Value, json};

const MAX_CIVILIANS: usize = dfmcp_intent::civilian_protection::MAX_PROTECTED_CIVILIANS;
const WORKFLOW: &str = "civilian_protection_v1";
const QUERY: &str = "civilian_protection";

fn invalid() -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, "invalid closed civilian protection request")
}
fn budget() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "civilian protection request exceeds its byte limit")
}
fn entity(raw: &str) -> Result<EntityId> {
    if raw.is_empty() || raw.len() > 20 || raw.starts_with('0')
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    raw.parse::<u64>().map(EntityId::new).map_err(|_| invalid())
}

fn civilians<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<EntityId>, D::Error> {
    struct Roster;
    impl<'de> Visitor<'de> for Roster {
        type Value = Vec<EntityId>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("1..64 unique nonzero decimal citizen IDs")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> std::result::Result<Self::Value, A::Error> {
            let mut seen = BTreeSet::new();
            while let Some(raw) = input.next_element::<String>()? {
                if seen.len() == MAX_CIVILIANS {
                    return Err(serde::de::Error::custom("too many civilians"));
                }
                let id = entity(&raw).map_err(serde::de::Error::custom)?;
                if !seen.insert(id) { return Err(serde::de::Error::custom("duplicate civilian")); }
            }
            if seen.is_empty() { return Err(serde::de::Error::custom("empty civilian roster")); }
            Ok(seen.into_iter().collect())
        }
    }
    deserializer.deserialize_seq(Roster)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowRequest {
    workflow: String,
    burrow: String,
    #[serde(deserialize_with = "civilians")]
    civilians: Vec<EntityId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    mode: String,
    burrow: String,
    #[serde(deserialize_with = "civilians")]
    civilians: Vec<EntityId>,
}

pub(super) fn parse(raw: &str) -> Result<Vec<RequestedAction>> {
    if raw.len() > super::MAX_ACTIONS_JSON_BYTES { return Err(budget()); }
    let request: WorkflowRequest = serde_json::from_str(raw).map_err(|_| invalid())?;
    if request.workflow != WORKFLOW { return Err(invalid()); }
    Ok(CivilianProtectionPlan::compile(entity(&request.burrow)?, &request.civilians)?.into_requested_actions())
}

/// The existing laboratory query shell calls this only after its current Query
/// grant check, with an owned reference snapshot. No context or capability is
/// synthesized here. Live and imported snapshots must not enter this shell.
pub(super) fn query(snapshot: &WorldSnapshot, raw: &str) -> Result<Value> {
    if raw.len() > super::MAX_QUERY_JSON_BYTES { return Err(budget()); }
    let request: QueryRequest = serde_json::from_str(raw).map_err(|_| invalid())?;
    if request.mode != QUERY { return Err(invalid()); }
    let plan = CivilianProtectionPlan::compile(entity(&request.burrow)?, &request.civilians)?;
    let evidence = PredicateEvidence::laboratory(snapshot)?;
    let truth = evidence.evaluate(&plan.configuration_goal(snapshot.fortress_id))?;
    let mut assigned = 0usize;
    let mut unassigned = 0usize;
    let mut unknown = 0usize;
    // Reuse the reference membership predicates, not a second bit decoder or
    // a rule that treats omitted fields as false.
    let predicates = effects::default_postconditions(&plan.requested_actions()[1].action, "", snapshot.fortress_id);
    for predicate in &predicates {
        match evidence.evaluate(predicate)? {
            PredicateTruth::True => assigned += 1,
            PredicateTruth::False => unassigned += 1,
            PredicateTruth::Unknown => unknown += 1,
        }
    }
    Ok(json!({
        "mode": QUERY,
        "burrow": request.burrow,
        "requested_civilians": request.civilians.len(),
        "memberships": {"assigned": assigned, "unassigned": unassigned, "unknown": unknown},
        "configuration_truth": match truth { PredicateTruth::True => "true", PredicateTruth::False => "false", PredicateTruth::Unknown => "unknown" },
        "configuration_proven": truth == PredicateTruth::True,
        "physical_arrival_proven": false,
        "threat_clearance_proven": false,
        "native_evidence": false,
        "coverage": "exact named roster in one reference snapshot, not the whole population",
        "note": "current configuration is not evacuation or release authority; action receipts remain separate history",
    }))
}
