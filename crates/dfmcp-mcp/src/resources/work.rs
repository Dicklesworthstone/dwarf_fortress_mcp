//! Bounded retained action and temporal-proof inspection without MCP Tasks.
//! Reads borrow the last proof state and inspect physical work separately;
//! they never sample a monitor or turn an archived frontier into fresh proof.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, Evidence, Result, RiskTier, StateAnchor,
    WorkBudget,
};
use dfmcp_intent::plan_archive::{self, ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{EffectWorkState, ObligationStatus, PlanStep, PreparedPlan};
use fastmcp_rust::prelude::{McpError, McpResult, ResourceContent};
use serde_json::{Value, json};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
};
use crate::server::{
    LabSession, anchor_json, authorize_entry, parse_session_id_arg, plan_resources, read_context,
    resolve_session, with_session_read,
};
use plan_resources::WorkEntry;

const OPERATION: &str = "resources.read.work";
const MAX_PAGE_RECORDS: usize = 8;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 60_000;

#[derive(Debug, PartialEq, Eq)]
enum Address {
    Catalog {
        digest: Option<Digest32>,
        offset: usize,
    },
    Action(Digest32),
    Obligation(Digest32),
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "work resource digests require 64 lowercase hexadecimal digits",
        ));
    }
    Digest32::from_hex(raw).ok_or_else(|| invalid("invalid work resource digest"))
}

fn address(view: &str) -> Result<Address> {
    if view == "work" {
        return Ok(Address::Catalog {
            digest: None,
            offset: 0,
        });
    }
    if view.len() > 96 {
        return Err(invalid("work resource address exceeds its byte bound"));
    }
    if let Some(raw) = view.strip_prefix("work-") {
        let (hash, offset) = raw
            .split_once('~')
            .ok_or_else(|| invalid("missing work catalog offset"))?;
        let start = offset
            .parse::<usize>()
            .map_err(|_| invalid("invalid work catalog offset"))?;
        if start.to_string() != offset {
            return Err(invalid(
                "work catalog offsets require canonical nonnegative decimals",
            ));
        }
        return Ok(Address::Catalog {
            digest: Some(digest(hash)?),
            offset: start,
        });
    }
    if let Some(raw) = view.strip_prefix("action-") {
        return digest(raw).map(Address::Action);
    }
    if let Some(raw) = view.strip_prefix("obligation-") {
        return digest(raw).map(Address::Obligation);
    }
    Err(invalid("unknown work resource"))
}

fn deadline(budget: WorkBudget, elapsed: &mut impl FnMut() -> Duration) -> Result<()> {
    if elapsed() >= Duration::from_millis(budget.max_wall_millis) {
        return Err(exhausted(
            "work resource exceeded its negotiated wall budget",
        ));
    }
    Ok(())
}

fn refusal(session: &str, error: DfmcpError, anchor: Option<StateAnchor>) -> McpError {
    let gap = matches!(error.code, ErrorCode::StaleAnchor | ErrorCode::CursorGap);
    let mut turn = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .continuity(
            if gap {
                ContinuityStatus::Gap
            } else {
                ContinuityStatus::Partial
            },
            None,
            gap.then(|| json!({"reason": error.message})),
            None,
        )
        .coverage(json!({"status":"unknown", "complete_domains":[], "absence_proof_scope":[]}))
        .uncertainty(vec![json!({"kind":"unknown", "reason":error.message})]);
    if let Ok(id) = parse_session_id_arg(session) {
        turn = turn.session_id(id.to_string());
    }
    if let Some(anchor) = anchor {
        turn = turn.anchor(anchor_json(&anchor));
    }
    let mut result =
        McpError::invalid_request(format!("{}: {}", error.code.as_str(), error.message));
    result.data = Some(json!({"schema":"dfmcp.resource-error/1", "ok":false,
        "error":{"code":error.code.as_str(), "message":error.message,
            "retryable":error.retryable, "effect_may_have_occurred":false},
        "agent_turn":turn.build()}));
    result
}

struct SourceWork {
    records: u64,
    bytes: u64,
    archive_bytes: u64,
}

impl SourceWork {
    fn charge(&mut self, records: usize, bytes: usize, budget: WorkBudget) -> Result<()> {
        self.records = self.records.saturating_add(records as u64);
        self.bytes = self.bytes.saturating_add(bytes as u64);
        if self.records > u64::from(budget.max_entities)
            || self.bytes > budget.max_bytes.min(MAX_SOURCE_BYTES)
        {
            return Err(exhausted(
                "work inspection exceeds its complete source record or byte budget",
            ));
        }
        Ok(())
    }

    fn text(&mut self, text: &str, budget: WorkBudget) -> Result<()> {
        // Conservative escaped-string admission before JSON materialization.
        self.charge(0, text.len().saturating_mul(6).saturating_add(2), budget)
    }

    fn evidence(&mut self, evidence: &[Evidence], budget: WorkBudget) -> Result<()> {
        self.charge(evidence.len(), evidence.len().saturating_mul(512), budget)?;
        for item in evidence {
            self.text(&item.summary, budget)?;
        }
        Ok(())
    }

    fn status(&mut self, status: Option<&ObligationStatus>, budget: WorkBudget) -> Result<()> {
        match status {
            Some(ObligationStatus::Fulfilled { evidence, .. }) => self.evidence(evidence, budget),
            Some(ObligationStatus::Failed { reason, .. }) => self.text(reason, budget),
            _ => Ok(()),
        }
    }

    fn work(&mut self, state: &EffectWorkState, budget: WorkBudget) -> Result<()> {
        if let EffectWorkState::Unknown { reason, .. } = state {
            self.text(reason, budget)?;
        }
        Ok(())
    }

    fn json(&self) -> Value {
        json!({"records_and_steps":self.records, "admitted_bytes":self.bytes,
            "archive_bytes":self.archive_bytes,
            "byte_measure":"canonical world and typed archives plus conservative receipt metadata"})
    }
}

struct Original<'a> {
    plan: &'a PreparedPlan,
    seal: PlanArchiveSeal,
}

fn evidence_json(evidence: &[Evidence]) -> Value {
    json!(
        evidence
            .iter()
            .map(|item| json!({"id":item.id.to_string(),
        "kind":format!("{:?}", item.kind), "subject":item.subject.map(|id|id.to_string()),
        "anchor":anchor_json(&item.anchor), "digest":item.digest.to_hex(),
        "summary":item.summary}))
            .collect::<Vec<_>>()
    )
}

fn status_json(status: Option<&ObligationStatus>) -> Value {
    match status {
        None => json!({"state":"unavailable"}),
        Some(ObligationStatus::Pending) => json!({"state":"pending"}),
        Some(ObligationStatus::Active {
            ticks_elapsed,
            consecutive_stable_observations,
        }) => json!({"state":"active", "ticks_elapsed":ticks_elapsed,
                "consecutive_stable_observations":consecutive_stable_observations}),
        Some(ObligationStatus::Fulfilled {
            fulfilled_at_tick,
            evidence,
        }) => json!({"state":"fulfilled", "fulfilled_at_tick":fulfilled_at_tick.0,
                "evidence":evidence_json(evidence)}),
        Some(ObligationStatus::Failed {
            failed_at_tick,
            reason,
        }) => json!({"state":"failed", "failed_at_tick":failed_at_tick.0, "reason":reason}),
        Some(ObligationStatus::Draining { drain_started_tick }) => {
            json!({"state":"draining", "drain_started_tick":drain_started_tick.0})
        }
        Some(ObligationStatus::Cancelled { cancelled_at_tick }) => {
            json!({"state":"cancelled", "cancelled_at_tick":cancelled_at_tick.0})
        }
    }
}

fn work_json(state: &EffectWorkState, anchor: Option<StateAnchor>) -> Value {
    let mut value = match state {
        EffectWorkState::NeverDispatched => json!({"state":"never_dispatched", "quiescent":true}),
        EffectWorkState::Active { entity_id } => json!({"state":"active", "quiescent":false,
            "entity_id":entity_id.to_string()}),
        EffectWorkState::Quiescent { entity_id } => json!({"state":"quiescent", "quiescent":true,
            "entity_id":entity_id.map(|id|id.to_string())}),
        EffectWorkState::Unknown { entity_id, reason } => {
            json!({"state":"unknown", "quiescent":null,
            "entity_id":entity_id.map(|id|id.to_string()), "reason":reason})
        }
    };
    value["observed_anchor"] = anchor
        .map(|anchor| anchor_json(&anchor))
        .unwrap_or(Value::Null);
    value
}

fn obligation_json(step: Option<&PlanStep>, status: Value, next_tick: Option<u64>) -> Value {
    let Some(spec) = step.and_then(|step| step.obligation.as_ref()) else {
        return Value::Null;
    };
    json!({"terminal":crate::lab_world::predicate_json(&spec.terminal),
        "failure":spec.failure.as_ref().map(crate::lab_world::predicate_json),
        "deadline_tick":spec.deadline_tick.0, "poll_interval_ticks":spec.poll_interval_ticks,
        "required_stable_observations":spec.stable_for_observations,
        "next_observation_tick":next_tick, "monitor":status,
        "postconditions_also_required":true, "sampled_by_this_read":false})
}

/// Complete admitted original custody, including originals outside the output
/// page, so a small continuation cannot bypass source-work bounds.
fn originals<'a>(
    session: &'a LabSession,
    source: &mut SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<BTreeMap<Digest32, Original<'a>>> {
    let mut originals = BTreeMap::new();
    for entry in plan_resources::entries(session).filter(|entry| entry.retention != "pending_plan")
    {
        deadline(session.budget, elapsed)?;
        let Some(plan) = entry.plan else { continue };
        if plan.digest != entry.digest || plan.anchor.fortress_id != session.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "work original has contradictory plan identity",
            ));
        }
        source.charge(plan.steps.len(), 0, session.budget)?;
        let remaining = session
            .budget
            .max_bytes
            .min(MAX_SOURCE_BYTES)
            .saturating_sub(source.bytes)
            .min(plan_archive::MAX_ARCHIVE_BYTES as u64) as usize;
        if remaining < 45 {
            return Err(exhausted(
                "work inspection exceeds its original archive byte budget",
            ));
        }
        let (bytes, seal) = plan_archive::encode_with_check(
            plan,
            ArchiveLimits {
                max_bytes: remaining,
                ..ArchiveLimits::default()
            },
            || deadline(session.budget, elapsed),
        )?;
        source.charge(0, bytes.len(), session.budget)?;
        source.archive_bytes = source.archive_bytes.saturating_add(bytes.len() as u64);
        if originals
            .insert(plan.digest, Original { plan, seal })
            .is_some()
        {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "work original digest has ambiguous retained custody",
            ));
        }
    }
    Ok(originals)
}

fn progress(
    session: &LabSession,
    entry: WorkEntry<'_>,
    originals: &BTreeMap<Digest32, Original<'_>>,
    source: &mut SourceWork,
) -> Result<Value> {
    let current = session.adapter.snapshot().anchor();
    let (plan_digest, step_id, mut detail, step) = match entry {
        WorkEntry::Live {
            plan_digest,
            plan_id,
            receipt,
        } => {
            let original = plan_digest.and_then(|digest| originals.get(&digest));
            let step = original
                .filter(|original| Some(original.plan.id) == plan_id)
                .and_then(|original| {
                    original
                        .plan
                        .steps
                        .iter()
                        .find(|step| step.id == receipt.step_id)
                });
            if original.is_some() && step.is_none() {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "retained action commit or step identity contradicts its exact original",
                ));
            }
            if step.is_some_and(|step| session.adapter.action_step(receipt.action_id) != Some(step))
            {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "action step contradicts its exact original",
                ));
            }
            source.text(&receipt.message, session.budget)?;
            source.evidence(&receipt.evidence, session.budget)?;
            let status = session.adapter.action_obligation_status(receipt.action_id);
            source.status(status, session.budget)?;
            let physical = if step.is_some() {
                session
                    .adapter
                    .action_work_state(receipt.action_id)
                    .unwrap_or_else(|error| EffectWorkState::Unknown {
                        entity_id: None,
                        reason: format!("{}: {}", error.code.as_str(), error.message),
                    })
            } else {
                // Unavailable original custody cannot admit the action's
                // complete typed source for a fresh physical-work inspection.
                EffectWorkState::Unknown {
                    entity_id: None,
                    reason: "exact original step is unavailable for physical-work inspection"
                        .to_owned(),
                }
            };
            source.work(&physical, session.budget)?;
            let (next, scheduling_error) = match session
                .adapter
                .action_next_observation_tick(receipt.action_id)
            {
                Ok(next) => (next.map(|tick| tick.0), None),
                Err(error) => (
                    None,
                    Some(format!("{}: {}", error.code.as_str(), error.message)),
                ),
            };
            if let Some(error) = &scheduling_error {
                source.text(error, session.budget)?;
            }
            let detail = json!({"source":"live_receipt", "action_id":receipt.action_id.to_string(),
                "proof_state":format!("{:?}", receipt.state), "proof_terminal":receipt.state.is_terminal(),
                "proof_anchor":anchor_json(&receipt.observed_anchor),
                "receipt_digest":receipt.adapter_receipt_digest.to_hex(), "message":receipt.message,
                "evidence":evidence_json(&receipt.evidence),
                "physical_work":work_json(&physical, Some(current)),
                "obligation":obligation_json(step, status_json(status), next),
                "scheduling_error":scheduling_error});
            (plan_digest, Some(receipt.step_id), detail, step)
        }
        WorkEntry::Recovered {
            plan_digest,
            step: step_id,
            kind,
            state,
            recorded_state,
            proof_anchor,
            work_anchor,
            work_state,
            work_step,
            monitor,
            observation_error,
            failure_reason,
            physical_drain,
        } => {
            let original = originals.get(&plan_digest);
            let step =
                step_id.and_then(|id| original?.plan.steps.iter().find(|step| step.id == id));
            if original.is_some() && step.is_none() {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "recovered step identity contradicts its exact original",
                ));
            }
            if step.is_some_and(|step| work_step.is_some_and(|stored| stored != step)) {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "recovered work step contradicts its exact original",
                ));
            }
            for text in [
                Some(state),
                Some(recorded_state),
                observation_error,
                failure_reason,
            ]
            .into_iter()
            .flatten()
            {
                source.text(text, session.budget)?;
            }
            source.work(work_state, session.budget)?;
            if let (Some(monitor), Some(original), Some(step)) = (monitor, original, step) {
                if !plan_resources::recovered_monitor_matches(monitor, original.plan, step) {
                    return Err(DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "recovered proof monitor contradicts its exact original obligation",
                    ));
                }
            }
            let status = monitor.and_then(|monitor| monitor.status());
            source.status(status, session.budget)?;
            if let Some(drain) = physical_drain {
                if drain.plan_digest != plan_digest || Some(drain.step_id) != step_id {
                    return Err(DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "recovered physical drain contradicts its original step identity",
                    ));
                }
                source.evidence(&drain.evidence, session.budget)?;
                source.work(&drain.before, session.budget)?;
                source.work(&drain.after, session.budget)?;
            }
            let detail = json!({"source":"recovered_original_step", "action_id":null,
                "action_kind":kind, "proof_state":state,
                "proof_terminal":matches!(state, "verified"|"failed"|"cancelled"|"compensated"|"not_dispatched"|"abandoned"),
                "recorded_state":recorded_state, "proof_anchor":proof_anchor.map(|anchor|anchor_json(&anchor)),
                "observation_error":observation_error, "failure_reason":failure_reason,
                "physical_work":work_json(work_state, work_anchor),
                "obligation":obligation_json(step, status_json(status),
                    monitor.and_then(|monitor|monitor.next_observation_tick()).map(|tick|tick.0)),
                "recovery_anchor":monitor.map(|monitor|anchor_json(&monitor.recovery_anchor())),
                "last_monitor_observation_anchor":monitor.and_then(|monitor|monitor.last_observation_anchor())
                    .map(|anchor|anchor_json(&anchor)),
                "stability_policy":"unfinished stability resets at recovery; archived frontier is not a positive sample",
                "physical_drain":physical_drain.map(|drain|json!({
                    "plan_digest":drain.plan_digest.to_hex(), "step_id":drain.step_id.get(),
                    "custody_digest":drain.custody_digest.to_hex(),
                    "recorded_anchor":anchor_json(&drain.recorded_anchor),
                    "recovery_anchor":anchor_json(&drain.recovery_anchor),
                    "before_anchor":anchor_json(&drain.before_anchor),
                    "observed_anchor":anchor_json(&drain.observed_anchor),
                    "before":work_json(&drain.before, Some(drain.before_anchor)),
                    "after":work_json(&drain.after, Some(drain.observed_anchor)),
                    "stopped_work":drain.stopped_work, "evidence":evidence_json(&drain.evidence)})),
                "blind_retry_allowed":false});
            (Some(plan_digest), step_id, detail, step)
        }
    };
    let original = plan_digest.and_then(|digest| originals.get(&digest));
    detail["plan_digest"] = json!(plan_digest.map(|digest| digest.to_hex()));
    detail["step_id"] = json!(step_id.map(|step| step.get()));
    detail["original_custody"] = json!(if step.is_some() {
        "exact_original_step"
    } else {
        "original_unavailable"
    });
    detail["plan_object_digest"] =
        json!(original.map(|original| original.seal.object_digest.to_hex()));
    detail["plan_anchor"] = original
        .map(|original| anchor_json(&original.plan.anchor))
        .unwrap_or(Value::Null);
    detail["original_plan"] = json!(original.map(|original| format!(
        "df://session/{}/plan-{}~{}",
        session.session_id, original.seal.plan_digest, original.seal.object_digest
    )));
    detail["goal"] = json!(
        plan_digest.map(|digest| format!("df://session/{}/goal-{digest}", session.session_id))
    );
    if let Some(step) = step {
        detail["action_kind"] = json!(crate::lab_world::action_kind(&step.action));
    }
    detail["current_goal_truth"] = json!("not_evaluated");
    let owner = plan_digest.and_then(|digest| plan_resources::goal(session, digest));
    detail["owner_session_id"] = json!(
        owner
            .as_ref()
            .map(|owner| owner.owner_session_id.to_string())
    );
    detail["owned_by_current_session"] =
        json!(owner.as_ref().map(|owner| owner.owned_by_current_session));
    detail["physical_work"]["current"] =
        json!(detail["physical_work"]["observed_anchor"] == anchor_json(&current));
    detail["historical_proof_updated"] = json!(false);
    detail["dispatch_authority"] = json!(false);
    Ok(detail)
}

pub(crate) fn read(session: &str, uri: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    let started = Instant::now();
    read_with_elapsed(session, uri, view, || started.elapsed())
}

fn read_with_elapsed(
    session: &str,
    uri: &str,
    view: &str,
    mut elapsed: impl FnMut() -> Duration,
) -> McpResult<Vec<ResourceContent>> {
    if uri.len() > 256 || uri != format!("df://session/{session}/{view}") {
        return Err(refusal(
            session,
            invalid("work resource URI does not match its session parameters"),
            None,
        ));
    }
    let target = address(view).map_err(|error| refusal(session, error, None))?;
    let handle =
        resolve_session(Some(session.to_owned())).map_err(|error| refusal(session, error, None))?;
    with_session_read(
        &handle,
        |error| Err(refusal(session, error, None)),
        |guard| {
            let context = read_context(guard);
            authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)
                .and_then(|()| authorize_entry(&context, Capability::Query, RiskTier::ReadOnly))
                .map_err(|error| refusal(session, error, None))?;
            let result = (|| {
                context.budget.validate()?;
                deadline(guard.budget, &mut elapsed)?;
                let world = plan_resources::work_world_source(guard, &context)?;
                let mut source = SourceWork {
                    records: world["records"]
                        .as_u64()
                        .ok_or_else(|| invalid("missing world source record measure"))?,
                    bytes: world["canonical_bytes"]
                        .as_u64()
                        .ok_or_else(|| invalid("missing world source byte measure"))?,
                    archive_bytes: 0,
                };
                source.charge(
                    plan_resources::goal_count(guard)
                        .saturating_add(plan_resources::work_count(guard)),
                    plan_resources::work_count(guard).saturating_mul(1_024),
                    guard.budget,
                )?;
                let originals = originals(guard, &mut source, &mut elapsed)?;
                let mut entries = Vec::new();
                for entry in plan_resources::work_entries(guard) {
                    deadline(guard.budget, &mut elapsed)?;
                    let mut detail = progress(guard, entry, &originals, &mut source)?;
                    let pin = Digest32::of_bytes(json!({"domain":"dfmcp.work-resource-progress/1",
                    "session_id":guard.session_id.to_string(), "current_anchor":anchor_json(&context.anchor),
                    "progress":detail}).to_string().as_bytes());
                    detail["progress_digest"] = json!(pin.to_hex());
                    detail["canonical_uri"] =
                        json!(format!("df://session/{}/action-{pin}", guard.session_id));
                    detail["obligation_uri"] = if detail["obligation"].is_null() {
                        Value::Null
                    } else {
                        json!(format!(
                            "df://session/{}/obligation-{pin}",
                            guard.session_id
                        ))
                    };
                    entries.push(detail);
                }
                entries.sort_by(|a, b| {
                    (
                        a["plan_digest"].as_str(),
                        a["step_id"].as_u64(),
                        a["progress_digest"].as_str(),
                    )
                        .cmp(&(
                            b["plan_digest"].as_str(),
                            b["step_id"].as_u64(),
                            b["progress_digest"].as_str(),
                        ))
                });
                let canonical_session = guard.session_id.to_string();
                match target {
                    Address::Catalog { digest, offset } => catalog(
                        &canonical_session,
                        uri,
                        digest,
                        offset,
                        &entries,
                        context.anchor,
                        guard.budget,
                        &source,
                        &mut elapsed,
                    ),
                    Address::Action(pin) | Address::Obligation(pin) => {
                        let mut detail = entries.into_iter().find(|entry|entry["progress_digest"] == pin.to_hex())
                        .ok_or_else(||DfmcpError::new(ErrorCode::CursorGap,
                            "work progress or its current anchor changed; refresh the work catalog"))?;
                        if matches!(target, Address::Obligation(_)) {
                            if detail["obligation"].is_null() {
                                return Err(invalid(
                                    "the retained step has no temporal obligation",
                                ));
                            }
                            detail["schema"] = json!("dfmcp.lab-obligation-progress/1");
                            detail["canonical_uri"] = detail["obligation_uri"].clone();
                        } else {
                            detail["schema"] = json!("dfmcp.lab-action-progress/1");
                        }
                        detail["coverage"] = coverage(None);
                        render(
                            detail,
                            &canonical_session,
                            uri,
                            context.anchor,
                            guard.budget,
                            &source,
                            &mut elapsed,
                        )?
                        .ok_or_else(|| {
                            exhausted(
                                "one complete work detail exceeds the negotiated output budget",
                            )
                        })
                    }
                }
            })();
            result
                .map(|text| super::text_content(uri, text))
                .map_err(|error| refusal(session, error, Some(context.anchor)))
        },
    )
}

fn coverage(next: Option<&str>) -> Value {
    json!({"status":"retained_session_work_only", "complete_domains":[], "absence_proof_scope":[],
        "continuation":next, "excludes":["uncommitted pending plans", "untracked fortress work", "current goal truth"]})
}

#[allow(clippy::too_many_arguments)]
fn catalog(
    session: &str,
    uri: &str,
    expected: Option<Digest32>,
    start: usize,
    entries: &[Value],
    current: StateAnchor,
    budget: WorkBudget,
    source: &SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<String> {
    let hash = Digest32::of_bytes(
        json!({"domain":"dfmcp.work-resource-catalog/1",
        "session_id":session, "anchor":anchor_json(&current),
        "progress":entries.iter().map(|row|&row["progress_digest"]).collect::<Vec<_>>()})
        .to_string()
        .as_bytes(),
    );
    if expected.is_some_and(|expected| expected != hash) {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "work catalog changed; restart at the work resource",
        ));
    }
    let total = entries.len();
    if start > total || (start == total && total != 0) {
        return Err(invalid(
            "work catalog offset is outside the retained catalog",
        ));
    }
    let mut returned = (total - start).min(MAX_PAGE_RECORDS);
    loop {
        let end = start + returned;
        let next = (end < total).then(|| format!("df://session/{session}/work-{hash}~{end}"));
        let rows: Vec<_> = entries[start..end]
            .iter()
            .map(|row| {
                json!({
            "plan_digest":row["plan_digest"], "plan_object_digest":row["plan_object_digest"],
            "step_id":row["step_id"], "source":row["source"], "action_id":row["action_id"],
            "owner_session_id":row["owner_session_id"], "owned_by_current_session":row["owned_by_current_session"],
            "action_kind":row["action_kind"], "proof_state":row["proof_state"],
            "proof_terminal":row["proof_terminal"], "proof_anchor":row["proof_anchor"],
            "physical_work":row["physical_work"], "original_custody":row["original_custody"],
            "progress_digest":row["progress_digest"], "action":row["canonical_uri"],
            "obligation":row["obligation_uri"]})
            })
            .collect();
        let payload = json!({"schema":"dfmcp.lab-work-catalog/1", "catalog_digest":hash.to_hex(),
            "canonical_uri":format!("df://session/{session}/work-{hash}~{start}"),
            "offset":start, "returned":returned, "total":total, "rows":rows,
            "complete":end==total, "complete_catalog":start==0 && end==total, "next":next,
            "retention":{"pins_work":false, "scope":"retained live receipts and recovered original steps"},
            "coverage":coverage(next.as_deref())});
        if let Some(text) = render(payload, session, uri, current, budget, source, elapsed)? {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted(
                "work catalog metadata or one complete row exceeds the negotiated output budget",
            ));
        }
        returned /= 2;
    }
}

#[allow(clippy::too_many_arguments)]
fn render(
    mut payload: Value,
    session: &str,
    uri: &str,
    current: StateAnchor,
    budget: WorkBudget,
    source: &SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<Option<String>> {
    let max_bytes = budget
        .max_bytes
        .min(
            u64::from(budget.max_output_tokens)
                .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
        )
        .min(MAX_RESOURCE_BYTES);
    payload["ok"] = json!(true);
    payload["session_id"] = json!(session);
    payload["resource"] = json!(uri);
    payload["current_anchor"] = anchor_json(&current);
    payload["scope"] = json!("laboratory_retained_work");
    payload["game_mutation_dispatched"] = json!(false);
    payload["source_work"] = source.json();
    let mut active = empty_active_work();
    active["coverage"] = json!({"status":"partial", "reason":"this page inspects retained proof and physical work separately"});
    payload["agent_turn"] = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .session_id(session).anchor(anchor_json(&current))
        .continuity(ContinuityStatus::Partial, Some(anchor_json(&current)), None, None)
        .profile(ObservationProfile::Forensic)
        .briefing(json!({"scope":"laboratory_retained_work", "mutation_authority":false,
            "proof_sampled":false, "current_goal_truth":"not_evaluated"}))
        .active_work(active).coverage(payload["coverage"].clone())
        .uncertainty(vec![json!({"kind":"unknown", "domain":"current_goal_truth",
            "reason":"a retained terminal proof does not establish current goal truth or physical quiescence"})])
        .budget(json!({"admitted":true, "max_output_bytes":max_bytes,
            "max_output_tokens":budget.max_output_tokens, "source_work":source.json()}))
        .references(vec![json!({"kind":"work_catalog", "uri":format!("df://session/{session}/work")}),
            json!({"kind":"plan_catalog", "uri":format!("df://session/{session}/plans")})]).build();
    let serialized = payload.to_string();
    let fits = super::task_resource_bytes(uri, &serialized) as u64 <= max_bytes;
    deadline(budget, elapsed)?;
    Ok(fits.then_some(serialized))
}

#[cfg(test)]
mod tests;
