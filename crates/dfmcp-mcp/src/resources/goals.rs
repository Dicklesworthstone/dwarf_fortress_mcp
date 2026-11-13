//! Bounded inspection of one retained original goal, without evaluating it.
//!
//! The discovery URI resolves a retained plan digest. Every returned detail
//! and source-page URI additionally seals the complete original source and
//! exact prepared-plan object, so collection or replacement cannot change it.

use std::io::Write;
use std::time::Duration;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, Result, RiskTier, StateAnchor, WorkBudget,
};
use dfmcp_intent::plan_archive::{self, ArchiveLimits, PlanArchiveSeal};
use fastmcp_rust::prelude::{McpError, McpResult, ResourceContent};
use serde_json::{Value, json};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
};
use crate::server::{
    anchor_json, authorize_entry, parse_session_id_arg, plan_resources, read_context,
    resolve_session, with_session_read,
};

const OPERATION: &str = "resources.read.goal";
const SOURCE_FORMAT: &str = "dfmcp.original-goal-source/1";
const MAX_RESOURCE_BYTES: u64 = 60_000;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 4_096;

#[derive(Debug, PartialEq, Eq)]
struct Address {
    plan: Digest32,
    source: Option<Digest32>,
    offset: Option<usize>,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "goal resource digests require 64 lowercase hexadecimal digits",
        ));
    }
    Digest32::from_hex(raw).ok_or_else(|| invalid("invalid goal resource digest"))
}

fn address(view: &str) -> Result<Address> {
    if view.len() > 180 {
        return Err(invalid("goal resource address exceeds its byte bound"));
    }
    let raw = view
        .strip_prefix("goal-")
        .ok_or_else(|| invalid("unknown goal resource"))?;
    let mut parts = raw.split('~');
    let plan = digest(parts.next().ok_or_else(|| invalid("missing plan digest"))?)?;
    let source = parts.next().map(digest).transpose()?;
    let offset = match parts.next() {
        None => None,
        Some("source") => {
            let raw = parts
                .next()
                .ok_or_else(|| invalid("missing source offset"))?;
            let offset = raw
                .parse::<usize>()
                .map_err(|_| invalid("invalid goal source offset"))?;
            if offset.to_string() != raw {
                return Err(invalid(
                    "goal source offsets require canonical nonnegative decimal integers",
                ));
            }
            Some(offset)
        }
        Some(_) => return Err(invalid("unknown goal resource section")),
    };
    if parts.next().is_some() {
        return Err(invalid("extra goal resource segments"));
    }
    Ok(Address {
        plan,
        source,
        offset,
    })
}

fn goal_uri(session: &str, plan: Digest32, source: Digest32) -> String {
    format!("df://session/{session}/goal-{plan}~{source}")
}

fn source_uri(session: &str, plan: Digest32, source: Digest32, offset: usize) -> String {
    format!("{}~source~{offset}", goal_uri(session, plan, source))
}

fn refusal(session: &str, error: DfmcpError, authorized_anchor: Option<StateAnchor>) -> McpError {
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
        .coverage(json!({"status": "unknown", "complete_domains": [], "absence_proof_scope": []}))
        .uncertainty(vec![json!({"kind": "unknown", "reason": error.message})]);
    if let Ok(id) = parse_session_id_arg(session) {
        turn = turn.session_id(id.to_string());
    }
    if let Some(anchor) = authorized_anchor {
        turn = turn.anchor(anchor_json(&anchor));
    }
    let mut result =
        McpError::invalid_request(format!("{}: {}", error.code.as_str(), error.message));
    result.data = Some(json!({
        "schema": "dfmcp.resource-error/1", "ok": false,
        "error": {"code": error.code.as_str(), "message": error.message,
            "retryable": error.retryable, "effect_may_have_occurred": false},
        "agent_turn": turn.build(),
    }));
    result
}

fn check_deadline(budget: WorkBudget, elapsed: &mut impl FnMut() -> Duration) -> Result<()> {
    if elapsed() >= Duration::from_millis(budget.max_wall_millis) {
        return Err(exhausted(
            "goal inspection exceeded its negotiated wall budget",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct SourceWork {
    records_and_steps: usize,
    input_bytes: usize,
    plan_archive_bytes: usize,
    source_object_bytes: usize,
}

impl SourceWork {
    fn bytes(&self) -> usize {
        self.input_bytes
            .saturating_add(self.plan_archive_bytes)
            .saturating_add(self.source_object_bytes)
    }

    fn remaining(&self, budget: WorkBudget) -> usize {
        budget
            .max_bytes
            .min(MAX_SOURCE_BYTES)
            .saturating_sub(self.bytes() as u64) as usize
    }

    fn json(&self) -> Value {
        json!({"records_and_steps": self.records_and_steps,
            "original_input_bytes": self.input_bytes,
            "plan_archive_bytes": self.plan_archive_bytes,
            "source_object_bytes": self.source_object_bytes})
    }
}

/// Stop the serializer before it allocates an oversized escaped JSON source.
/// Raw requests remain strings: inspection never reparses arbitrary nesting.
struct BoundedWriter<'a, F> {
    bytes: Vec<u8>,
    maximum: usize,
    budget: WorkBudget,
    elapsed: &'a mut F,
}

impl<F: FnMut() -> Duration> Write for BoundedWriter<'_, F> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.maximum
            || check_deadline(self.budget, self.elapsed).is_err()
        {
            return Err(std::io::Error::other(
                "goal source serialization exceeds its bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct OriginalSource {
    bytes: Vec<u8>,
    digest: Digest32,
    plan_seal: Option<PlanArchiveSeal>,
}

fn original_source(
    entry: &plan_resources::GoalEntry<'_>,
    current: StateAnchor,
    budget: WorkBudget,
    work: &mut SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<OriginalSource> {
    // These O(1) lengths precede string cloning, JSON projection, hashing and
    // recursive typed-plan serialization. No unrelated original is rendered.
    work.input_bytes = 512usize
        .saturating_add(entry.summary.len())
        .saturating_add(entry.request_json.map_or(0, str::len))
        .saturating_add(entry.verification_error.map_or(0, str::len))
        .saturating_add(entry.withdrawal.map_or(0, |withdrawal| {
            256usize.saturating_add(withdrawal.never_dispatched_steps.len().saturating_mul(11))
        }));
    if work.bytes() as u64 > budget.max_bytes.min(MAX_SOURCE_BYTES) {
        return Err(exhausted(
            "the complete original goal source exceeds its byte budget",
        ));
    }
    for anchor in [
        entry.sealed_anchor,
        entry.first_satisfied_anchor,
        entry.restore_abandoned_anchor,
        entry.withdrawal.map(|withdrawal| withdrawal.anchor),
    ]
    .into_iter()
    .flatten()
    {
        if anchor.fortress_id != current.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "retained original goal history belongs to another fortress",
            ));
        }
    }
    if entry.withdrawal.is_some_and(|withdrawal| {
        withdrawal.plan_digest != entry.digest || withdrawal.fortress_id != current.fortress_id
    }) {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "retained withdrawal belongs to another original goal",
        ));
    }
    if entry
        .sealed_anchor
        .is_some_and(|anchor| anchor.state_hash != entry.sealed_state_hash)
    {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "retained original goal seal does not match its original state hash",
        ));
    }
    let plan_seal = if let Some(plan) = entry.plan {
        if plan.digest != entry.digest || Some(plan.anchor) != entry.sealed_anchor {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "retained original goal and prepared plan have different identities",
            ));
        }
        work.records_and_steps = work.records_and_steps.saturating_add(plan.steps.len());
        if work.records_and_steps > budget.max_entities as usize {
            return Err(exhausted(
                "goal inspection exceeds its source record and step budget",
            ));
        }
        let maximum = work.remaining(budget).min(plan_archive::MAX_ARCHIVE_BYTES);
        if maximum < 45 {
            return Err(exhausted(
                "the original goal leaves no bounded plan archive allowance",
            ));
        }
        let (bytes, seal) = plan_archive::encode_with_check(
            plan,
            ArchiveLimits {
                max_bytes: maximum,
                ..ArchiveLimits::default()
            },
            || check_deadline(budget, elapsed),
        )?;
        work.plan_archive_bytes = bytes.len();
        Some(seal)
    } else {
        None
    };
    check_deadline(budget, elapsed)?;
    let source = json!({
        "schema": SOURCE_FORMAT,
        "plan_digest": entry.digest.to_hex(),
        "plan_object_digest": plan_seal.map(|seal| seal.object_digest.to_hex()),
        "owner_session_id": entry.owner_session_id.to_string(),
        "sealed_state_hash": entry.sealed_state_hash.to_hex(),
        "sealed_anchor": entry.sealed_anchor.as_ref().map(anchor_json),
        "summary": entry.summary,
        "original_source": {"kind": entry.source_kind,
            "request_json": entry.request_json, "paused_target": entry.paused_target},
        "terminal_condition": entry.plan
            .map(|plan| crate::lab_world::predicate_json(&plan.terminal_condition)),
        "terminal_condition_semantics": "sealed_requirement_not_current_evidence",
        "exact_plan_available": entry.plan.is_some(),
    });
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        maximum: work.remaining(budget),
        budget,
        elapsed,
    };
    serde_json::to_writer(&mut writer, &source).map_err(|_| {
        exhausted("the complete original goal source exceeds its byte or wall budget")
    })?;
    work.source_object_bytes = writer.bytes.len();
    let bytes = writer.bytes;
    let digest = Digest32::of_bytes(&bytes);
    check_deadline(budget, elapsed)?;
    Ok(OriginalSource {
        bytes,
        digest,
        plan_seal,
    })
}

pub(crate) fn read(session: &str, uri: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    let started = std::time::Instant::now();
    read_with_elapsed(session, uri, view, || started.elapsed())
}

fn read_with_elapsed(
    session_id: &str,
    uri: &str,
    view: &str,
    mut elapsed: impl FnMut() -> Duration,
) -> McpResult<Vec<ResourceContent>> {
    if uri.len() > 256 || uri != format!("df://session/{session_id}/{view}") {
        return Err(refusal(
            session_id,
            invalid("goal URI does not match its session parameters"),
            None,
        ));
    }
    let canonical_session =
        parse_session_id_arg(session_id).map_err(|error| refusal(session_id, error, None))?;
    if canonical_session.to_string() != session_id {
        return Err(refusal(
            session_id,
            invalid("goal resources require the canonical lowercase session identifier"),
            None,
        ));
    }
    let target = address(view).map_err(|error| refusal(session_id, error, None))?;
    let session = resolve_session(Some(session_id.to_owned()))
        .map_err(|error| refusal(session_id, error, None))?;
    with_session_read(
        &session,
        |error| Err(refusal(session_id, error, None)),
        |guard| {
            let ctx = read_context(guard);
            // A complete original can name any fortress entity or region.
            // Both grants precede lookup, source hashing and history disclosure.
            authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
                .and_then(|()| authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly))
                .map_err(|error| refusal(session_id, error, None))?;
            let result = (|| {
                check_deadline(guard.budget, &mut elapsed)?;
                let count = plan_resources::goal_count(guard);
                if count > guard.budget.max_entities as usize {
                    return Err(exhausted("goal lookup exceeds its retained record budget"));
                }
                let entry = plan_resources::goal(guard, target.plan).ok_or_else(|| {
                    DfmcpError::new(ErrorCode::StaleAnchor,
                        "the exact original goal is not retained; no substitute can be inferred from action receipts")
                })?;
                let mut work = SourceWork {
                    records_and_steps: count,
                    ..SourceWork::default()
                };
                let original =
                    original_source(&entry, ctx.anchor, guard.budget, &mut work, &mut elapsed)?;
                if target
                    .source
                    .is_some_and(|expected| expected != original.digest)
                {
                    return Err(DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        "the retained original source does not match this exact goal reference; rediscover the original goal",
                    ));
                }
                let session = guard.session_id.to_string();
                match target.offset {
                    Some(offset) => source_page(
                        &session,
                        uri,
                        &entry,
                        &original,
                        offset,
                        ctx.anchor,
                        guard.budget,
                        &work,
                        &mut elapsed,
                    ),
                    None => manifest(
                        &session,
                        uri,
                        &entry,
                        &original,
                        ctx.anchor,
                        guard.budget,
                        &work,
                        &mut elapsed,
                    ),
                }
            })();
            result
                .map(|text| super::text_content(uri, text))
                .map_err(|error| refusal(session_id, error, Some(ctx.anchor)))
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn manifest(
    session: &str,
    uri: &str,
    entry: &plan_resources::GoalEntry<'_>,
    original: &OriginalSource,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<String> {
    let historical = entry.first_satisfied_anchor.filter(|_| {
        entry.historical_proof_verified
            && entry.plan.is_some()
            && entry.verification_error.is_none()
    });
    let source = source_uri(session, entry.digest, original.digest, 0);
    let mut payload = json!({
        "schema": "dfmcp.lab-goal-manifest/1",
        "canonical_uri": goal_uri(session, entry.digest, original.digest),
        "plan_digest": entry.digest.to_hex(), "source_digest": original.digest.to_hex(),
        "source_kind": entry.source_kind,
        "owner_session_id": entry.owner_session_id.to_string(),
        "owned_by_current_session": entry.owned_by_current_session,
        "sealed_anchor": entry.sealed_anchor.as_ref().map(anchor_json),
        "sealed_state_hash": entry.sealed_state_hash.to_hex(),
        "first_satisfied_anchor": historical.as_ref().map(anchor_json),
        "recorded_first_satisfied_anchor": entry.first_satisfied_anchor.as_ref().map(anchor_json),
        "historical_achievement": if historical.is_some() { "verified" }
            else if entry.first_satisfied_anchor.is_some() { "unverifiable" } else { "not_recorded" },
        "historical_achievement_scope": "original_sealed_terminal_condition",
        "restore_abandoned_anchor": entry.restore_abandoned_anchor.as_ref().map(anchor_json),
        "abandonment_publication_pending": entry.abandonment_publication_pending,
        "withdrawal": entry.withdrawal.map(|withdrawal| json!({
            "anchor": anchor_json(&withdrawal.anchor),
            "requester_session_id": withdrawal.requester_session_id.to_string(),
            "proof_digest": withdrawal.proof_digest.to_hex(),
            "never_dispatched_steps": withdrawal.never_dispatched_steps.iter().copied().collect::<Vec<_>>(),
            "goal_achievement_recorded": false,
        })),
        "archived": entry.withdrawal.is_some(),
        "original_source_verification": if entry.plan.is_some() { "verified" } else { "unavailable" },
        "verification_error": entry.verification_error,
        "source": {"format": SOURCE_FORMAT, "sha256": original.digest.to_hex(),
            "bytes": original.bytes.len(), "uri": source,
            "complete_fields": ["summary", "original_source", "terminal_condition", "sealed_anchor"],
            "request_encoding": "request_json preserves the exact original UTF-8 request"},
        "plan": original.plan_seal.map(|seal| json!({
            "object_digest": seal.object_digest.to_hex(),
            "uri": format!("df://session/{session}/plan-{}~{}", seal.plan_digest, seal.object_digest),
            "archive": format!("df://session/{session}/plan-{}~{}~archive~0", seal.plan_digest, seal.object_digest),
        })),
        "retention": {"pins_goals": false, "scope": "one_retained_original_goal"},
        "coverage": {"status": "retained_original_history_only", "complete_domains": [],
            "absence_proof_scope": [], "original_source": "complete_via_exact_source_pages",
            "action_receipts": "omitted", "physical_work": "not_evaluated"},
    });
    // The source artifact is always lossless. Keep the direct manifest bounded
    // even for an unusually long retained diagnostic; disclose its omission.
    if entry
        .verification_error
        .is_some_and(|message| message.len() > 1_024)
    {
        payload["verification_error"] = Value::Null;
        payload["verification_error_omitted"] = json!("diagnostic_exceeds_manifest_bound");
    }
    render(payload, session, uri, current, budget, work, elapsed)?
        .ok_or_else(|| exhausted("original goal manifest exceeds the negotiated output budget"))
}

#[allow(clippy::too_many_arguments)]
fn source_page(
    session: &str,
    uri: &str,
    entry: &plan_resources::GoalEntry<'_>,
    original: &OriginalSource,
    start: usize,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> Duration,
) -> Result<String> {
    if start >= original.bytes.len() {
        return Err(invalid(
            "source offset is outside the exact original goal object",
        ));
    }
    let mut returned = (original.bytes.len() - start).min(MAX_PAGE_BYTES);
    loop {
        let end = start + returned;
        let next = (end < original.bytes.len())
            .then(|| source_uri(session, entry.digest, original.digest, end));
        let data_hex: String = original.bytes[start..end]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let payload = json!({
            "schema": "dfmcp.lab-goal-source-page/1", "format": SOURCE_FORMAT, "encoding": "hex",
            "plan_digest": entry.digest.to_hex(), "source_digest": original.digest.to_hex(),
            "canonical_uri": source_uri(session, entry.digest, original.digest, start),
            "manifest": goal_uri(session, entry.digest, original.digest),
            "offset": start, "returned_bytes": returned, "total_bytes": original.bytes.len(),
            "data_hex": data_hex, "next": next, "complete": end == original.bytes.len(),
            "complete_object": start == 0 && end == original.bytes.len(),
            "coverage": {"status": "original_source_bytes_only", "complete_domains": [],
                "absence_proof_scope": [], "continuation": next},
        });
        if let Some(text) = render(payload, session, uri, current, budget, work, elapsed)? {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted(
                "original source page metadata or one byte exceeds the output budget",
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
    work: &SourceWork,
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
    payload["anchor"] = anchor_json(&current);
    payload["scope"] = json!("laboratory_retained_original_goal");
    payload["current_goal_truth"] = json!("not_evaluated");
    payload["current_semantic_project_truth"] = json!("not_evaluated");
    payload["dispatch_authority"] = json!(false);
    payload["game_mutation_dispatched"] = json!(false);
    payload["historical_proof_updated"] = json!(false);
    payload["source_work"] = work.json();
    let mut active = empty_active_work();
    active["coverage"] = json!({"status": "omitted", "reason": "inspect current work through fortress.observe or fortress.wait"});
    payload["agent_turn"] = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .session_id(session)
        .anchor(anchor_json(&current))
        .continuity(ContinuityStatus::Partial, None, None, None)
        .profile(ObservationProfile::Forensic)
        .briefing(json!({"scope": "retained_original_goal", "current_goal_truth": "not_evaluated",
            "mutation_authority": false}))
        .active_work(active)
        .coverage(payload["coverage"].clone())
        .uncertainty(vec![json!({"kind": "unknown", "domain": "current_goal_and_physical_work",
            "reason": "original intent and historical achievement do not prove current predicates, project completion, quiescence or live game compatibility"})])
        .budget(json!({"admitted": true, "max_output_bytes": max_bytes,
            "max_output_tokens": budget.max_output_tokens, "source_work": work.json()}))
        .build();
    let serialized = payload.to_string();
    let fits = super::task_resource_bytes(uri, &serialized) as u64 <= max_bytes;
    check_deadline(budget, elapsed)?;
    Ok(fits.then_some(serialized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_addresses_are_closed_and_canonical() {
        let plan = "a".repeat(64);
        let source = "b".repeat(64);
        for valid in [
            format!("goal-{plan}"),
            format!("goal-{plan}~{source}"),
            format!("goal-{plan}~{source}~source~0"),
        ] {
            assert!(address(&valid).is_ok(), "{valid}");
        }
        for invalid in [
            String::new(),
            "goal-".to_owned(),
            format!("goal-{}", "A".repeat(64)),
            format!("goal-{plan}~"),
            format!("goal-{plan}~{source}~source"),
            format!("goal-{plan}~{source}~source~01"),
            format!("goal-{plan}~{source}~source~-1"),
            format!("goal-{plan}~{source}~source~+1"),
            format!("goal-{plan}~{source}~source~0~extra"),
            format!("goal-{plan}~{source}~steps~0"),
            format!("goal-{plan}~{source}~source~184467440737095516160"),
        ] {
            assert!(address(&invalid).is_err(), "{invalid}");
        }
    }
}
