//! Versioned original WOOD/PLANT stock goals for the reference laboratory.
//! The retained request is replayed against current qualified source custody;
//! pending work and discarded predictions never become available inventory.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, IntentId, Result, RiskTier};
use dfmcp_intent::{
    Constraint, HarvestResource, Intent, ResourceAcquisitionPlan, ResourceStockTarget,
    StaticPlanner, effects,
};
use dfmcp_world::{CompareOp, Predicate, PredicateEvidence, PredicateTruth, WorldSnapshot};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

pub(super) const PLANNER: &str = "resource_acquisition_v1";
pub(super) const MAX_REQUEST_BYTES: usize = 16 * 1024;

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn present<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

/// Detection never normalizes the request. Strict parsing below still consumes
/// its original bytes, so duplicate object keys cannot disappear in a round trip.
pub(super) fn is_request(raw: &str) -> bool {
    raw.len() <= MAX_REQUEST_BYTES
        && serde_json::from_str::<Value>(raw)
            .is_ok_and(|value| value["template"] == "resource_acquisition")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResourceAcquisitionRequest {
    quotas: BTreeMap<HarvestResource, u64>,
    assign_labor: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quota {
    resource: String,
    minimum: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    template: String,
    #[serde(default, deserialize_with = "present")]
    planner: Option<String>,
    quotas: Vec<Quota>,
    #[serde(default)]
    assign_labor: bool,
}

/// Nested retained sources must consume this typed grammar directly. A Value
/// round trip would erase duplicate keys before their original bytes are checked.
impl<'de> Deserialize<'de> for ResourceAcquisitionRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_request(Request::deserialize(deserializer)?, false)
            .map_err(serde::de::Error::custom)
    }
}

impl ResourceAcquisitionRequest {
    pub(super) fn parse_current(raw: &str) -> Result<Self> {
        Self::parse_generation(raw, true)
    }

    /// Saved sources must name the original algorithm. They never silently
    /// acquire a newer lowering generation when a process restarts.
    pub(super) fn parse(raw: &str) -> Result<Self> {
        Self::parse_generation(raw, false)
    }

    fn parse_generation(raw: &str, current: bool) -> Result<Self> {
        if raw.len() > MAX_REQUEST_BYTES || raw.contains('\0') {
            return Err(invalid(
                "resource acquisition exceeds its 16-KiB non-NUL request bound",
            ));
        }
        let request: Request = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid resource acquisition request: {error}")))?;
        Self::from_request(request, current)
    }

    fn from_request(request: Request, current: bool) -> Result<Self> {
        if request.template != "resource_acquisition" {
            return Err(invalid(
                "resource acquisition requires template=resource_acquisition",
            ));
        }
        match request.planner.as_deref() {
            Some(PLANNER) => {}
            None if current => {}
            _ => {
                return Err(invalid(
                    "resource acquisition requires the exact resource_acquisition_v1 planner",
                ));
            }
        }
        if !(1..=2).contains(&request.quotas.len()) {
            return Err(invalid(
                "resource acquisition requires one or two distinct WOOD/PLANT quotas",
            ));
        }
        let mut quotas = BTreeMap::new();
        for quota in request.quotas {
            let resource = match quota.resource.as_str() {
                "WOOD" => HarvestResource::Wood,
                "PLANT" => HarvestResource::Plant,
                _ => return Err(invalid("resource acquisition supports only WOOD and PLANT")),
            };
            if quota.minimum == 0 || quotas.insert(resource, quota.minimum).is_some() {
                return Err(invalid(
                    "resource acquisition quotas require positive minima and distinct resources",
                ));
            }
        }
        Ok(Self {
            quotas,
            assign_labor: request.assign_labor,
        })
    }

    pub(super) fn canonical_json(&self) -> String {
        json!({
            "template": "resource_acquisition",
            "planner": PLANNER,
            "quotas": self.quotas.iter().map(|(resource, minimum)| {
                json!({"resource": resource.as_str(), "minimum": minimum})
            }).collect::<Vec<_>>(),
            "assign_labor": self.assign_labor,
        })
        .to_string()
    }

    pub(super) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Value)> {
        self.compile_sealed(
            id,
            snapshot,
            summary,
            self.source_digest(),
            "resource acquisition source",
        )
    }

    pub(super) fn source_digest(&self) -> Digest32 {
        let mut source_bytes = b"dfmcp-resource-acquisition-source/1\0".to_vec();
        source_bytes.extend_from_slice(self.canonical_json().as_bytes());
        Digest32::of_bytes(&source_bytes)
    }

    /// Current semantic stock truth is independent of an earlier pursuit's
    /// exact task completion. The existing harvest census validates the ledger
    /// and complete original custody before any quota can be reported true.
    pub(super) fn stock_goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        use dfmcp_intent::resource_acquisition::MAX_RESOURCE_ACQUISITION_ENTITIES;

        if snapshot.graph.entities.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.edges.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.chunks.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.events.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "resource goal truth exceeds its bounded complete-source inspection",
            ));
        }
        let evidence = PredicateEvidence::laboratory(snapshot)?;
        let report = effects::harvest_report(evidence.snapshot())?.ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "resource goal truth requires the original qualified finite-harvest ledger",
            )
        })?;
        evidence.evaluate(&Predicate::All(
            self.quotas
                .iter()
                .map(|(resource, minimum)| Predicate::FieldCompare {
                    entity_id: report.ledger,
                    field: match resource {
                        HarvestResource::Wood => effects::STOCK_WOOD_FIELD,
                        HarvestResource::Plant => effects::STOCK_PLANT_FIELD,
                    }
                    .to_owned(),
                    op: CompareOp::Ge,
                    value: dfmcp_world::Value::U64(*minimum),
                })
                .collect(),
        ))
    }

    /// Both root and continuation sources use the same lowering. Their complete
    /// source digests select a single seal, never a nested pair of summary seals.
    pub(super) fn compile_sealed(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
        source_digest: Digest32,
        seal_label: &str,
    ) -> Result<(Intent, Value)> {
        let targets = self
            .quotas
            .iter()
            .map(|(resource, minimum)| ResourceStockTarget {
                resource: *resource,
                minimum: *minimum,
            })
            .collect::<Vec<_>>();
        let compiled = ResourceAcquisitionPlan::compile(snapshot, &targets, self.assign_labor)?;
        if compiled.requested_actions.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "the original resource stock targets are already satisfied; no acquisition plan or effect is needed",
            ));
        }
        let summary = if summary.is_empty() {
            "acquire original resource stock minima"
        } else {
            summary
        };
        let sealed_summary = format!("{summary} [{seal_label} {}]", source_digest.to_hex());
        if sealed_summary.len() > StaticPlanner::default().policy.max_string_bytes
            || sealed_summary.chars().any(char::is_control)
        {
            return Err(invalid(
                "resource acquisition summary plus its source seal exceeds the planner text bound",
            ));
        }
        let analysis = json!({
            "schema": "dfmcp.resource-acquisition/1",
            "planner": PLANNER,
            "epistemic_state": "certified_derived",
            "evidence_scope": "laboratory_reference_world",
            "source_anchor": crate::observation_projection::anchor_json(&compiled.source_anchor),
            "source_digest": source_digest.to_hex(),
            "original_request_preserved": true,
            "original_request": serde_json::from_str::<Value>(&self.canonical_json())
                .map_err(|error| invalid(error.to_string()))?,
            "ledger_entity_id": compiled.ledger.to_string(),
            "ledger_generation": compiled.ledger_generation,
            "requirements": compiled.requirements.iter().map(|row| json!({
                "resource": row.resource.as_str(),
                "minimum": row.minimum,
                "observed": row.observed,
                "deficit": row.deficit,
                "available_source_units": row.available_source_units,
            })).collect::<Vec<_>>(),
            "allocations": compiled.allocations.iter().map(|row| json!({
                "resource": row.resource.as_str(),
                "source_entity_id": row.source.to_string(),
                "source_generation": row.source_generation,
                "amount": row.amount,
                "action_index": row.action_index,
            })).collect::<Vec<_>>(),
            "staffing": compiled.staffing.iter().map(|row| json!({
                "resource": row.resource.as_str(),
                "worker_entity_id": row.worker.to_string(),
                "worker_generation": row.generation,
                "previously_enabled": row.previously_enabled,
                "labor_enable_action_index": row.action_index,
            })).collect::<Vec<_>>(),
            "queued_activities": compiled.queued_activities,
            "queued_service_ticks": compiled.queued_service_ticks,
            "requested_service_ticks": compiled.requested_service_ticks,
            "deadline_tick": compiled.deadline_tick.map(|tick| tick.0),
            "pending_harvest_credited_as_inventory": false,
            "source_units_reserved": false,
            "credit_on_completion_only": true,
            "native_execution_supported": false,
            "next_stage": "observe completed inputs, then prepare production or construction against that new anchor",
        });
        Ok((
            Intent {
                id,
                anchor: snapshot.anchor(),
                summary: sealed_summary,
                terminal_condition: compiled.terminal_condition,
                constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                requested_actions: compiled.requested_actions,
            },
            analysis,
        ))
    }
}

#[cfg(test)]
#[path = "resource_acquisition_tests.rs"]
mod tests;
