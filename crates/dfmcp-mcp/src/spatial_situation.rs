//! Deterministic orientation over a spatial/1.8 canonical snapshot. These are
//! observed signs worth inspecting, not diagnoses, safety proofs or authority.
//! Missing, stale, redacted, contradictory and non-native facts remain unknown.

use std::collections::BTreeMap;
use std::time::Instant;
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result,
    RiskTier, SessionId, StateAnchor};
use dfmcp_world::{EntityKind, EntityRecord, FactPresence, FactSource, Value as WorldValue, WorldSnapshot};
use serde_json::{Value, json};
use super::anchor_json;

pub(super) const POLICY: &str = "dfmcp.spatial-situation/1";
pub(super) const ATTENTION_LIMIT: usize = 2;
const CHANGE_LIMIT: usize = 4;
const MAX_ENTITIES: usize = 131_072;

struct Rule {
    code: &'static str,
    severity: &'static str,
    finding: &'static str,
    limitation: &'static str,
    fields: &'static [&'static str],
}
// Order is the complete inspection-priority policy, not a probability or a
// causal score. Equal inputs always retain the smallest matching entity ID.
const RULES: [Rule; 7] = [
    Rule { code: "citizen_not_alive", severity: "high", finding: "Strict citizens observed not alive",
        limitation: "Does not establish when or why a death occurred.", fields: &["alive", "position"] },
    Rule { code: "living_citizen_not_sane", severity: "high", finding: "Living strict citizens observed not sane",
        limitation: "Does not establish diagnosis, cause or imminent violence.", fields: &["alive", "sane", "position"] },
    Rule { code: "visible_magma", severity: "medium", finding: "Magma observed in the requested terrain region",
        limitation: "Magma may be contained; exposure, flooding and danger are not proved.", fields: &["visibility", "magma", "liquid_depth", "position"] },
    Rule { code: "suspended_jobs", severity: "medium", finding: "Current jobs observed suspended",
        limitation: "Suspension does not establish cause, age or criticality.", fields: &["type_key", "suspended", "blocking_reason"] },
    Rule { code: "unassigned_unsuspended_jobs", severity: "low", finding: "Unsuspended current jobs have no assigned worker",
        limitation: "May be normal queueing; labor shortage or blockage is not proved.", fields: &["type_key", "suspended", "worker_assigned", "blocking_reason"] },
    Rule { code: "incomplete_buildings", severity: "low", finding: "Buildings observed below their maximum build stage",
        limitation: "Does not prove stalled construction or missing materials.", fields: &["build_stage", "max_build_stage"] },
    Rule { code: "retained_rotten_items", severity: "low", finding: "Non-removed items observed rotten",
        limitation: "These are item counts, not edible stock or a food-shortage diagnosis.", fields: &["type_key", "rotten", "removed"] },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Subject { id: String, generation: u32, revision: u64 }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Signal { examined: u64, matches: u64, unknown: u64, example: Option<Subject> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Report {
    anchor: StateAnchor,
    source: Digest32,
    paused: bool,
    counts: BTreeMap<&'static str, u64>,
    signals: Vec<Signal>,
}
fn failure(code: ErrorCode, message: &str) -> DfmcpError { DfmcpError::new(code, message) }

fn known<'a>(entity: &'a EntityRecord, field: &str, anchor: StateAnchor, source: Digest32) -> Option<&'a WorldValue> {
    let fact = entity.fields.get(field)?;
    if fact.observed_at != anchor.tick || fact.source_digest != source
        || !matches!(&fact.source, FactSource::DfhackField(_)) { return None; }
    match &fact.presence {
        None => Some(&fact.value),
        Some(FactPresence::Known(value)) if value == &fact.value => Some(value),
        _ => None,
    }
}
fn boolean(entity: &EntityRecord, field: &str, anchor: StateAnchor, source: Digest32) -> Option<bool> {
    match known(entity, field, anchor, source) { Some(WorldValue::Bool(value)) => Some(*value), _ => None }
}
fn integer(entity: &EntityRecord, field: &str, anchor: StateAnchor, source: Digest32) -> Option<i64> {
    match known(entity, field, anchor, source) { Some(WorldValue::I64(value)) => Some(*value), _ => None }
}
fn both(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) { (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true), _ => None }
}
fn applies(index: usize, entity: &EntityRecord) -> bool {
    matches!((index, &entity.kind), (0 | 1, EntityKind::Unit) | (2, EntityKind::TileFeature)
        | (3 | 4, EntityKind::Job) | (5, EntityKind::Building) | (6, EntityKind::Item))
}
fn evaluate(index: usize, entity: &EntityRecord, anchor: StateAnchor, source: Digest32) -> Option<bool> {
    let flag = |name| boolean(entity, name, anchor, source);
    match index {
        0 => flag("alive").map(|value| !value),
        1 => both(flag("alive"), flag("sane").map(|value| !value)),
        2 => {
            // Hidden/unallocated terrain is outside the observed liquid scope.
            // Never consult plausible-looking residual fields behind redaction.
            if !matches!(known(entity,"visibility",anchor,source), Some(WorldValue::Text(text)) if text=="visible") { return None; }
            let depth = match known(entity,"liquid_depth",anchor,source) {
                Some(WorldValue::U64(n)) if *n<=7 => Some(*n>0), _ => None };
            both(flag("magma"), depth)
        }
        3 => flag("suspended"),
        4 => both(flag("suspended").map(|value| !value), flag("worker_assigned").map(|value| !value)),
        5 => match (integer(entity,"build_stage",anchor,source), integer(entity,"max_build_stage",anchor,source)) {
            (Some(stage), Some(maximum)) if stage>=0 && maximum>=stage => Some(stage<maximum), _ => None },
        6 => both(flag("removed").map(|value| !value), flag("rotten")),
        _ => None,
    }
}

/// Whole-projection Query authority is checked before any canonical facts are
/// inspected. Work is one ordered entity pass with a fixed seven-rule ceiling.
pub(super) fn build(snapshot: &WorldSnapshot, context: &OperationContext, source: Digest32) -> Result<Report> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor() { return Err(failure(ErrorCode::StaleAnchor,"situation context names another snapshot")); }
    if snapshot.graph.entities.len()>context.budget.max_entities as usize || snapshot.graph.entities.len()>MAX_ENTITIES {
        return Err(failure(ErrorCode::BudgetExceeded,"situation exceeds its entity-scan bound"));
    }
    let started=Instant::now();
    let check=|| -> Result<()> {
        context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
        if started.elapsed().as_millis()>=u128::from(context.budget.max_wall_millis) {
            return Err(failure(ErrorCode::BudgetExceeded,"situation exceeded its cooperative wall-time allowance"));
        }
        Ok(())
    };
    check()?;
    if !snapshot.hash_is_valid() { return Err(failure(ErrorCode::InternalInvariantViolation,"situation snapshot hash is invalid")); }
    let mut counts=BTreeMap::from([("citizens",0), ("jobs",0), ("buildings",0), ("items",0),
        ("terrain_cells",0), ("hidden_cells",0), ("unallocated_cells",0)]);
    let mut signals=vec![Signal::default(); RULES.len()];
    for (ordinal,(id,entity)) in snapshot.graph.entities.iter().enumerate() {
        if ordinal%64==0 { check()?; }
        if *id!=entity.id { return Err(failure(ErrorCode::InternalInvariantViolation,"situation entity map key differs")); }
        let category=match entity.kind { EntityKind::Unit=>Some("citizens"),EntityKind::Job=>Some("jobs"),
            EntityKind::Building=>Some("buildings"),EntityKind::Item=>Some("items"),EntityKind::TileFeature=>Some("terrain_cells"),_=>None };
        if let Some(category)=category { *counts.entry(category).or_default()+=1; }
        if entity.kind==EntityKind::TileFeature {
            match known(entity,"visibility",context.anchor,source) {
                Some(WorldValue::Text(text)) if text=="hidden"=>*counts.entry("hidden_cells").or_default()+=1,
                Some(WorldValue::Text(text)) if text=="unallocated"=>*counts.entry("unallocated_cells").or_default()+=1,
                _=>{}
            }
        }
        for (index,signal) in signals.iter_mut().enumerate() {
            if !applies(index,entity) { continue; }
            signal.examined+=1;
            match evaluate(index,entity,context.anchor,source) {
                None=>signal.unknown+=1,
                Some(false)=>{},
                Some(true)=>{
                    signal.matches+=1;
                    if signal.example.is_none() { signal.example=Some(Subject {id:id.to_string(),generation:entity.generation,revision:entity.revision}); }
                }
            }
        }
    }
    check()?;
    Ok(Report {anchor:context.anchor,source,paused:snapshot.paused,counts,signals})
}

impl Report {
    pub(super) fn summary(&self) -> Value {
        let signals:BTreeMap<_,_>=RULES.iter().zip(&self.signals).map(|(rule,signal)|
            (rule.code,json!({"observed":signal.matches,"unestablished":signal.unknown}))).collect();
        let findings=self.signals.iter().filter(|signal|signal.matches>0).count();
        json!({"policy":POLICY,"paused":self.paused,"counts":self.counts,"signals":signals,
            "attention_groups":findings,"attention_groups_omitted":findings.saturating_sub(ATTENTION_LIMIT),
            "scope":"strict_citizens_current_operations_requested_terrain_at_capture",
            "all_clear_proven":false,"food_drink_health_threat_coverage_complete":false})
    }
    pub(super) fn attention(&self, session: SessionId) -> Vec<Value> {
        RULES.iter().zip(&self.signals).enumerate().filter_map(|(rank,(rule,signal))| {
            let example=signal.example.as_ref()?;
            let mut identity=POLICY.as_bytes().to_vec();
            identity.extend_from_slice(self.source.as_bytes());
            identity.extend_from_slice(anchor_json(self.anchor).to_string().as_bytes());
            identity.extend_from_slice(rule.code.as_bytes());
            Some(json!({"attention_id":Digest32::of_bytes(&identity).to_string(),"rule":rule.code,
                "severity":rule.severity,"inspection_priority":rank,"finding":rule.finding,
                "observed_count":signal.matches,"unestablished_count":signal.unknown,
                "epistemic_state":"certified_derived","causal_diagnosis_proven":false,
                "limitation":rule.limitation,"source_digest":self.source.to_string(),
                "example":{"entity_id":example.id,"generation":example.generation,"revision":example.revision},
                "next_step":{"tool":"fortress.query","arguments":{"session_id":session.to_string(),
                    "query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(self.anchor),
                        "query":{"kind":"inspect","entity_id":example.id,"generation":example.generation,"fields":rule.fields}}}}}))
        }).take(ATTENTION_LIMIT).collect()
    }
    /// Endpoint count deltas only: equal counts do not prove unchanged members,
    /// cause, continuous conditions or success of any game effect.
    pub(super) fn comparison(&self, before: &Report) -> (Value, Vec<Value>) {
        let compatible=self.anchor.fortress_id==before.anchor.fortress_id
            && self.anchor.cursor.epoch==before.anchor.cursor.epoch
            && self.anchor.cursor.sequence>=before.anchor.cursor.sequence && self.anchor.tick>=before.anchor.tick
            && (self.anchor.cursor!=before.anchor.cursor || self.anchor==before.anchor);
        if !compatible {
            return (json!({"status":"reset","basis":anchor_json(before.anchor),"comparable":false,
                "reason":"observation_epoch_identity_or_order_changed"}),Vec::new());
        }
        let mut changes=Vec::new();
        if self.paused!=before.paused { changes.push(json!({"kind":"observed_pause_change","before":before.paused,"after":self.paused})); }
        // Safety-first rule order precedes ordinary roster counts. Preserve
        // changes in coverage/uncertainty even when positive counts agree.
        for ((rule,prior),next) in RULES.iter().zip(&before.signals).zip(&self.signals) {
            if (prior.matches,prior.unknown)!=(next.matches,next.unknown) {
                changes.push(json!({"kind":"situation_signal_count_change","rule":rule.code,
                    "before":{"observed":prior.matches,"unestablished":prior.unknown},
                    "after":{"observed":next.matches,"unestablished":next.unknown}}));
            }
        }
        for (name,next) in &self.counts {
            if let Some(prior)=before.counts.get(name) { if prior!=next { changes.push(json!({
                "kind":"situation_roster_count_change","domain":name,"before":prior,"after":next})); } }
        }
        let total=changes.len();changes.truncate(CHANGE_LIMIT);
        (json!({"status":if self.anchor==before.anchor {"heartbeat"} else {"compared"},"comparable":true,
            "basis":anchor_json(before.anchor),"changed_metrics":total,"returned_metrics":changes.len(),
            "omitted_metrics":total.saturating_sub(changes.len()),"coverage":"endpoint_counts_only",
            "unchanged_counts_prove_unchanged_world":false,"continuous_between_observations":false,
            "mutation_success_proven":false}),changes)
    }
}

pub(super) fn policy() -> Value {
    json!({"policy":POLICY,"ranking":"fixed inspection priority, then lowest canonical example entity ID",
        "max_attention_groups":ATTENTION_LIMIT,"max_change_metrics":CHANGE_LIMIT,
        "score_is_probability":false,"authority_granted":false,
        "rules":RULES.iter().enumerate().map(|(rank,rule)|json!({"rule":rule.code,"priority":rank,
            "severity":rule.severity,"finding":rule.finding,"fields":rule.fields,"limitation":rule.limitation})).collect::<Vec<_>>()})
}
