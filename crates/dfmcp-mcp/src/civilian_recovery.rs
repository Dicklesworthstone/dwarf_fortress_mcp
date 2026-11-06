//! World-owned civilian clearance and separately reviewed laboratory release.
//!
//! Monitoring is process-local and effect-free. Only the owner can construct a
//! release source from accepted observations; decoding a historical source can
//! reproduce its seal, but cannot restore its clearance or dispatch authority.

use std::collections::BTreeMap;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, IntentId, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_intent::{
    AlertDecision, AlertObservation, AlertRecoveryPolicy, CivilianAlertFsm, CivilianReleasePlan,
    Constraint, Intent, StaticPlanner, ThreatAssessment, ThreatLevel,
};
use dfmcp_world::{
    EntityKind, LaboratoryCivilianCensus, LaboratoryEntityIdentity, LaboratoryThreatAssessment,
    PredicateTruth, WorldSnapshot, laboratory_civilian_census,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_BURROWS: usize = 16;
const MAX_CIVILIANS: usize = 64;
const SOURCE_SCHEMA: &str = "dfmcp.laboratory-civilian-release/1";
const MONITOR_SCHEMA: &str = "dfmcp.laboratory-civilian-recovery/1";

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn entity(raw: &str) -> Result<EntityId> {
    if raw.is_empty()
        || raw.len() > 20
        || raw.starts_with('0')
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            "civilian recovery requires a canonical nonzero decimal burrow ID",
        ));
    }
    raw.parse::<u64>()
        .map(EntityId::new)
        .map_err(|_| invalid("invalid burrow ID"))
}

fn truth(value: PredicateTruth) -> &'static str {
    match value {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

fn threat(value: LaboratoryThreatAssessment) -> &'static str {
    match value {
        LaboratoryThreatAssessment::Present => "present",
        LaboratoryThreatAssessment::Clear => "clear",
        LaboratoryThreatAssessment::Unknown => "unknown",
    }
}

fn observed_level(census: &LaboratoryCivilianCensus) -> ThreatLevel {
    if census.released == PredicateTruth::True {
        ThreatLevel::Peace
    } else {
        // Partial or unknown protection never establishes an observed release.
        ThreatLevel::EmergencyLockdown
    }
}

struct TrackedBurrow {
    fsm: CivilianAlertFsm,
    census: LaboratoryCivilianCensus,
    decision: Option<AlertDecision>,
    fault: Option<String>,
}

impl TrackedBurrow {
    fn ready(&self) -> bool {
        self.fault.is_none()
            && self.census.protected == PredicateTruth::True
            && self.census.complete_civilian_roster
            && (1..=MAX_CIVILIANS).contains(&self.census.civilians.len())
            && self.census.burrow.is_some()
            && self
                .decision
                .as_ref()
                .is_some_and(|decision| decision.release_ready)
    }

    fn projection(&self) -> Value {
        let policy = AlertRecoveryPolicy::default();
        json!({
            "burrow": self.census.burrow.map(identity_json),
            "source_anchor": anchor_json(self.census.anchor),
            "source_scope": self.census.source_scope.to_hex(),
            "threat": threat(self.census.threat),
            "complete_civilian_roster": self.census.complete_civilian_roster,
            "civilians": self.census.civilians.iter().copied().map(identity_json).collect::<Vec<_>>(),
            "protection_observed": truth(self.census.protected),
            "protection_complete": truth(self.census.protection_complete),
            "released_configuration": truth(self.census.released),
            "target_level": self.decision.as_ref().map(|decision| format!("{:?}", decision.target_level)),
            "clear_since_tick": self.decision.as_ref().and_then(|decision| decision.clear_since).map(|tick| tick.0),
            "clear_samples": self.decision.as_ref().map_or(0, |decision| decision.clear_samples),
            "release_ready": self.ready(),
            "policy": {
                "minimum_clear_ticks": policy.minimum_clear_ticks,
                "minimum_clear_samples": policy.minimum_clear_samples,
                "maximum_sample_gap_ticks": policy.maximum_sample_gap_ticks,
                "consecutive_cursors_required": true,
            },
            "fault": self.fault,
            "physical_arrival_proven": false,
            "native_evidence": false,
            "automatic_dispatch": false,
        })
    }
}

/// One owner per private/shared world, exchanged with its adapter under the
/// same lock. Reopening a durable world intentionally starts with no history.
#[derive(Default)]
pub(crate) struct CivilianRecovery {
    anchor: Option<StateAnchor>,
    paused: bool,
    burrows: BTreeMap<EntityId, TrackedBurrow>,
    fault: Option<String>,
}

impl CivilianRecovery {
    pub(crate) fn observe(&mut self, snapshot: &WorldSnapshot) {
        let anchor = snapshot.anchor();
        if self.anchor == Some(anchor) {
            return;
        }
        if self.anchor.is_some_and(|prior| {
            prior.fortress_id != anchor.fortress_id
                || prior.cursor.epoch != anchor.cursor.epoch
                || anchor.tick < prior.tick
                || anchor.cursor <= prior.cursor
        }) {
            // Restore/restart must rebuild from observed configuration. A saved
            // clear streak or another epoch never authorizes release.
            self.burrows.clear();
        }
        self.anchor = Some(anchor);
        self.paused = snapshot.paused;
        self.fault = None;
        let selected: Vec<_> = snapshot
            .graph
            .entities
            .values()
            .filter(|record| record.kind.as_str() == EntityKind::Burrow.as_str())
            .take(MAX_BURROWS + 1)
            .map(|record| record.id)
            .collect();
        if selected.len() > MAX_BURROWS {
            self.burrows.clear();
            self.fault =
                Some("civilian recovery exceeds the 16-burrow monitoring bound".to_owned());
            return;
        }
        self.burrows.retain(|id, _| selected.contains(id));
        for burrow in selected {
            let census = match laboratory_civilian_census(snapshot, burrow) {
                Ok(census) => census,
                Err(error) => {
                    self.burrows.remove(&burrow);
                    self.fault = Some(error.message);
                    continue;
                }
            };
            let level = observed_level(&census);
            let tracked = match self.burrows.entry(burrow) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let Ok(mut fsm) = CivilianAlertFsm::new(burrow, Vec::new()) else {
                        continue;
                    };
                    fsm.confirm_observed_level(level);
                    entry.insert(TrackedBurrow {
                        fsm,
                        census: census.clone(),
                        decision: None,
                        fault: None,
                    })
                }
            };
            if tracked.fsm.current_level() != level {
                tracked.fsm.confirm_observed_level(level);
            }
            let assessment = match census.threat {
                LaboratoryThreatAssessment::Present => ThreatAssessment::Present,
                LaboratoryThreatAssessment::Clear => ThreatAssessment::Clear,
                LaboratoryThreatAssessment::Unknown => ThreatAssessment::Unknown,
            };
            let observation = AlertObservation {
                anchor,
                source_scope: census.source_scope,
                threat: assessment,
                complete_civilian_roster: census.complete_civilian_roster,
            };
            let civilians: Vec<_> = census
                .civilians
                .iter()
                .map(|identity| identity.id)
                .collect();
            match tracked.fsm.plan_observation(observation, &civilians) {
                Ok(decision) => {
                    tracked.decision = Some(decision);
                    tracked.fault = None;
                }
                Err(error) => {
                    tracked.decision = None;
                    tracked.fault = Some(error.message);
                }
            }
            tracked.census = census;
        }
    }

    pub(crate) fn projection(&self) -> Value {
        json!({
            "schema": MONITOR_SCHEMA,
            "source_anchor": self.anchor.map(anchor_json),
            "burrows": self.burrows.values().map(TrackedBurrow::projection).collect::<Vec<_>>(),
            "fault": self.fault,
            "history": "process-local accepted observations; restore and restart reset clearance",
            "automatic_dispatch": false,
            "native_evidence": false,
        })
    }

    pub(crate) fn alerts(&self) -> Vec<Value> {
        self.burrows.iter().filter_map(|(burrow, tracked)| {
            let census = &tracked.census;
            if tracked.ready() {
                return Some(json!({
                    "domain": "civilian_safety", "alert": "all_clear", "severity": "low",
                    "finding": "the complete current laboratory census has remained clear through the required interval; review a guarded release plan",
                    "recovery": tracked.projection(),
                    "remedy": {"tool": "fortress.plan", "arguments": {"blueprint": json!({"template":"civilian_release", "burrow":burrow.to_string()}).to_string()}, "requires":"configure_logistics"},
                }));
            }
            if census.protected != PredicateTruth::True && census.threat == LaboratoryThreatAssessment::Clear {
                return None;
            }
            if census.protection_complete == PredicateTruth::True && census.threat != LaboratoryThreatAssessment::Clear {
                return Some(json!({
                    "domain":"civilian_safety", "alert":"civilian_protected", "severity":"low",
                    "finding":"all current civilians are assigned and the restriction is active; keep protection while threat clearance is unproven",
                    "recovery":tracked.projection(),
                }));
            }
            let mut alert = json!({
                "domain":"civilian_safety", "alert":"civilian_recovery", "severity":"medium",
                "finding":"retain civilian protection until complete, consecutive clear observations satisfy the recovery interval",
                "recovery":tracked.projection(),
            });
            if census.threat != LaboratoryThreatAssessment::Clear {
                alert["alert"] = json!("civilian_lockdown");
                alert["severity"] = json!("high");
                alert["finding"] = json!("a threat is present or its absence is unproven; retain or establish civilian protection");
                if census.complete_civilian_roster && (1..=MAX_CIVILIANS).contains(&census.civilians.len()) && census.burrow.is_some() {
                    alert["remedy"] = if self.paused {
                        json!({"tool":"fortress.plan", "arguments":{"actions":json!({
                            "workflow":"civilian_protection_v1", "burrow":burrow.to_string(),
                            "civilians":census.civilians.iter().map(|identity| identity.id.to_string()).collect::<Vec<_>>()
                        }).to_string()}, "requires":"configure_logistics"})
                    } else {
                        json!({"tool":"fortress.plan", "arguments":{"paused_target":true},
                            "requires":"control_clock", "why":"pause before preparing the guarded civilian protection workflow"})
                    };
                }
            }
            Some(alert)
        }).collect()
    }

    pub(crate) fn release_source(
        &self,
        snapshot: &WorldSnapshot,
        burrow: EntityId,
    ) -> Result<ReleaseSource> {
        if self.anchor != Some(snapshot.anchor()) || self.fault.is_some() {
            return Err(invalid(
                "civilian recovery has no complete current owned observation",
            ));
        }
        let tracked = self.burrows.get(&burrow).filter(|tracked| tracked.ready())
            .ok_or_else(|| invalid("civilian release requires observed protection and a stable complete clear interval"))?;
        let decision = tracked
            .decision
            .as_ref()
            .ok_or_else(|| invalid("missing clearance decision"))?;
        let source = ReleaseSource {
            schema: SOURCE_SCHEMA.to_owned(),
            burrow: Identity::from(
                tracked
                    .census
                    .burrow
                    .ok_or_else(|| invalid("missing burrow identity"))?,
            ),
            civilians: tracked
                .census
                .civilians
                .iter()
                .copied()
                .map(Identity::from)
                .collect(),
            anchor: Anchor::from(snapshot.anchor()),
            source_scope: tracked.census.source_scope.to_hex(),
            clear_since_tick: decision
                .clear_since
                .ok_or_else(|| invalid("missing clear interval"))?
                .0,
            clear_samples: decision.clear_samples,
        };
        source.validate_snapshot(snapshot)?;
        Ok(source)
    }

    /// Historical source decoding alone cannot pass this current-owner check.
    pub(crate) fn validate_release(&self, snapshot: &WorldSnapshot, raw: &str) -> Result<()> {
        let source = ReleaseSource::parse(raw)?;
        if source.anchor != Anchor::from(snapshot.anchor()) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "civilian release observation changed; request and review a fresh release plan",
            ));
        }
        let current = self.release_source(snapshot, source.burrow.identity()?.id)?;
        if source != current {
            return Err(invalid(
                "civilian release is not backed by this world's current owned clearance",
            ));
        }
        Ok(())
    }
}

fn identity_json(identity: LaboratoryEntityIdentity) -> Value {
    json!({"id":identity.id.to_string(), "generation":identity.generation})
}

fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id":anchor.fortress_id.to_string(), "state_hash":anchor.state_hash.to_hex(),
        "game_tick":anchor.tick.0, "cursor":{"epoch":anchor.cursor.epoch,"sequence":anchor.cursor.sequence}})
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
    generation: u32,
}

impl From<LaboratoryEntityIdentity> for Identity {
    fn from(identity: LaboratoryEntityIdentity) -> Self {
        Self {
            id: identity.id.to_string(),
            generation: identity.generation,
        }
    }
}

impl Identity {
    fn identity(&self) -> Result<LaboratoryEntityIdentity> {
        if self.generation == 0 {
            return Err(invalid("civilian identity has no generation"));
        }
        Ok(LaboratoryEntityIdentity {
            id: entity(&self.id)?,
            generation: self.generation,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    fortress_id: String,
    state_hash: String,
    tick: u64,
    epoch: u64,
    sequence: u64,
}

impl From<StateAnchor> for Anchor {
    fn from(anchor: StateAnchor) -> Self {
        Self {
            fortress_id: anchor.fortress_id.to_string(),
            state_hash: anchor.state_hash.to_hex(),
            tick: anchor.tick.0,
            epoch: anchor.cursor.epoch,
            sequence: anchor.cursor.sequence,
        }
    }
}

/// Closed durable representation. Only `CivilianRecovery::release_source` admits
/// new work; `parse` and `compile` exist for exact historical reconstruction.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseSource {
    schema: String,
    burrow: Identity,
    civilians: Vec<Identity>,
    anchor: Anchor,
    source_scope: String,
    clear_since_tick: u64,
    clear_samples: u32,
}

impl ReleaseSource {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > crate::lab_world::MAX_ACTIONS_JSON_BYTES {
            return Err(invalid("civilian release source exceeds its byte bound"));
        }
        let source: Self = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid civilian release source: {error}")))?;
        let policy = AlertRecoveryPolicy::default();
        if source.schema != SOURCE_SCHEMA
            || !(1..=MAX_CIVILIANS).contains(&source.civilians.len())
            || source.clear_samples < policy.minimum_clear_samples
            || source
                .anchor
                .tick
                .checked_sub(source.clear_since_tick)
                .is_none_or(|span| span < policy.minimum_clear_ticks)
            || Digest32::from_hex(&source.source_scope)
                .is_none_or(|digest| digest == Digest32::ZERO)
        {
            return Err(invalid(
                "civilian release source has no bounded clearance identity",
            ));
        }
        let civilians = source
            .civilians
            .iter()
            .map(Identity::identity)
            .collect::<Result<Vec<_>>>()?;
        if !civilians.windows(2).all(|pair| pair[0].id < pair[1].id) {
            return Err(invalid(
                "civilian release source roster is not canonical and unique",
            ));
        }
        source.burrow.identity()?;
        Ok(source)
    }

    /// Repeat the complete-census and compiled-work allowance at both new-plan
    /// admission and commit; action-sized budgets alone cannot cover this read.
    pub(crate) fn validate_budget(
        &self,
        snapshot: &WorldSnapshot,
        context: &OperationContext,
    ) -> Result<()> {
        if snapshot.graph.entities.len() > context.budget.max_entities as usize
            || context.budget.max_actions < 2
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "civilian release census exceeds the current complete-domain allowance",
            ));
        }
        let civilians = self
            .civilians
            .iter()
            .map(Identity::identity)
            .collect::<Result<Vec<_>>>()?;
        let compiled = CivilianReleasePlan::compile(self.burrow.identity()?, &civilians)?;
        if compiled.encoded_bytes() as u64 > context.budget.max_bytes {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "civilian release compiled work exceeds the current byte allowance",
            ));
        }
        Ok(())
    }

    pub(crate) fn canonical_json(&self) -> String {
        // Only bounded strings, integer fields and lists: serialization cannot fail.
        json!(self).to_string()
    }

    fn validate_snapshot(&self, snapshot: &WorldSnapshot) -> Result<()> {
        let census = laboratory_civilian_census(snapshot, self.burrow.identity()?.id)?;
        if self.anchor != Anchor::from(snapshot.anchor())
            || census.burrow.map(Identity::from).as_ref() != Some(&self.burrow)
            || census
                .civilians
                .iter()
                .copied()
                .map(Identity::from)
                .collect::<Vec<_>>()
                != self.civilians
            || census.source_scope.to_hex() != self.source_scope
            || !census.complete_civilian_roster
            || census.threat != LaboratoryThreatAssessment::Clear
            || census.protected != PredicateTruth::True
        {
            return Err(invalid(
                "civilian release source does not match the complete clear protected census",
            ));
        }
        Ok(())
    }

    pub(crate) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Value)> {
        self.validate_snapshot(snapshot)?;
        let civilians = self
            .civilians
            .iter()
            .map(Identity::identity)
            .collect::<Result<Vec<_>>>()?;
        let plan = CivilianReleasePlan::compile(self.burrow.identity()?, &civilians)?;
        let mut bytes = b"dfmcp-laboratory-civilian-release-source/1\0".to_vec();
        bytes.extend_from_slice(self.canonical_json().as_bytes());
        let digest = Digest32::of_bytes(&bytes);
        let summary = format!("{summary} [civilian release source {}]", digest.to_hex());
        if summary.len() > StaticPlanner::default().policy.max_string_bytes
            || summary.contains('\0')
        {
            return Err(invalid(
                "civilian release summary plus its source seal exceeds the planner bound",
            ));
        }
        let risk = plan
            .requested_actions()
            .iter()
            .map(|step| step.action.risk())
            .max()
            .unwrap_or(RiskTier::ReadOnly);
        Ok((
            Intent {
                id,
                anchor: snapshot.anchor(),
                summary,
                terminal_condition: plan.configuration_goal(snapshot.fortress_id),
                constraints: vec![Constraint::MaxRisk(risk)],
                requested_actions: plan.requested_actions_for(snapshot.fortress_id),
            },
            json!({
                "schema":SOURCE_SCHEMA, "source_digest":digest.to_hex(),
                "burrow":self.burrow, "civilians":self.civilians,
                "clear_since_tick":self.clear_since_tick, "clear_samples":self.clear_samples,
                "fresh_census_checked_before_every_effect":true,
                "physical_arrival_proven":false, "native_evidence":false,
            }),
        ))
    }
}

pub(crate) fn is_release_request(raw: &str) -> bool {
    raw.len() <= crate::lab_world::MAX_ACTIONS_JSON_BYTES
        && serde_json::from_str::<Value>(raw)
            .is_ok_and(|value| value["template"] == "civilian_release")
}

pub(crate) fn parse_release_request(raw: &str) -> Result<EntityId> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        template: String,
        burrow: String,
    }
    if raw.len() > crate::lab_world::MAX_ACTIONS_JSON_BYTES {
        return Err(invalid("civilian release request exceeds its byte bound"));
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid civilian release request: {error}")))?;
    if request.template != "civilian_release" {
        return Err(invalid("unknown civilian release template"));
    }
    entity(&request.burrow)
}

/// The live owner is the only source for recovery history. There is deliberately
/// no historical `at` selector and no caller-supplied threat or stability claim.
pub(crate) fn is_recovery_query(raw: &str) -> Result<bool> {
    if raw == "civilian_recovery" {
        return Ok(true);
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(false);
    }
    let selected =
        serde_json::from_str::<Value>(raw).is_ok_and(|value| value["mode"] == "civilian_recovery");
    if !selected {
        return Ok(false);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Query {
        mode: String,
    }
    let query: Query = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid civilian recovery query: {error}")))?;
    Ok(query.mode == "civilian_recovery")
}

#[cfg(test)]
#[path = "civilian_recovery_tests.rs"]
mod tests;
