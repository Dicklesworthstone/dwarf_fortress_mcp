//! Exact original prepared-plan inspection, independent of the MCP Task service.
//!
//! Semantic plan digests are not exact typed-object identities. Every detail
//! and archive page therefore binds both the semantic and archive digests.

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

const OPERATION: &str = "resources.read.plans";
const MAX_PAGE_RECORDS: usize = 25;
const MAX_PAGE_BYTES: usize = 4_096;
const MAX_RESOURCE_BYTES: u64 = 60_000;

#[derive(Debug, PartialEq, Eq)]
enum Address {
    Catalog {
        digest: Option<Digest32>,
        offset: usize,
    },
    Plan {
        seal: PlanArchiveSeal,
        page: Option<PlanPage>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum PlanPage {
    Archive(usize),
    Steps(usize),
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
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "plan resource digests require 64 lowercase hexadecimal digits",
        ));
    }
    Digest32::from_hex(raw).ok_or_else(|| invalid("invalid plan resource digest"))
}

fn offset(raw: &str) -> Result<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| invalid("invalid plan resource offset"))?;
    if value.to_string() != raw {
        return Err(invalid(
            "plan resource offsets require canonical nonnegative decimal integers",
        ));
    }
    Ok(value)
}

fn address(view: &str) -> Result<Address> {
    if view == "plans" {
        return Ok(Address::Catalog {
            digest: None,
            offset: 0,
        });
    }
    if view.len() > 180 {
        return Err(invalid("plan resource address exceeds its byte bound"));
    }
    if let Some(raw) = view.strip_prefix("plans-") {
        let (hash, start) = raw
            .split_once('~')
            .ok_or_else(|| invalid("missing plan catalog offset"))?;
        return Ok(Address::Catalog {
            digest: Some(digest(hash)?),
            offset: offset(start)?,
        });
    }
    let raw = view
        .strip_prefix("plan-")
        .ok_or_else(|| invalid("unknown plan resource"))?;
    let mut parts = raw.split('~');
    let semantic = digest(parts.next().ok_or_else(|| invalid("missing plan digest"))?)?;
    let object = digest(
        parts
            .next()
            .ok_or_else(|| invalid("missing exact archive digest"))?,
    )?;
    let page = match parts.next() {
        None => None,
        Some("archive") => Some(PlanPage::Archive(offset(
            parts
                .next()
                .ok_or_else(|| invalid("missing archive offset"))?,
        )?)),
        Some("steps") => Some(PlanPage::Steps(offset(
            parts.next().ok_or_else(|| invalid("missing step offset"))?,
        )?)),
        Some(_) => return Err(invalid("unknown plan resource section")),
    };
    if parts.next().is_some() {
        return Err(invalid("extra plan resource segments"));
    }
    Ok(Address::Plan {
        seal: PlanArchiveSeal {
            plan_digest: semantic,
            object_digest: object,
        },
        page,
    })
}

fn plan_uri(session: &str, seal: PlanArchiveSeal) -> String {
    format!(
        "df://session/{session}/plan-{}~{}",
        seal.plan_digest, seal.object_digest
    )
}

fn archive_uri(session: &str, seal: PlanArchiveSeal, start: usize) -> String {
    format!("{}~archive~{start}", plan_uri(session, seal))
}

fn steps_uri(session: &str, seal: PlanArchiveSeal, start: usize) -> String {
    format!("{}~steps~{start}", plan_uri(session, seal))
}

fn catalog_uri(session: &str, hash: Digest32, start: usize) -> String {
    format!("df://session/{session}/plans-{hash}~{start}")
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

fn check_deadline(
    budget: WorkBudget,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<()> {
    if elapsed() >= std::time::Duration::from_millis(budget.max_wall_millis) {
        return Err(exhausted(
            "plan resource exceeded its negotiated wall budget",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct SourceWork {
    records: usize,
    archive_bytes: usize,
}

impl SourceWork {
    fn encode(
        &mut self,
        plan: &dfmcp_intent::PreparedPlan,
        budget: WorkBudget,
        elapsed: &mut impl FnMut() -> std::time::Duration,
    ) -> Result<(Vec<u8>, PlanArchiveSeal)> {
        self.records = self.records.saturating_add(plan.steps.len());
        if self.records > budget.max_entities as usize {
            return Err(exhausted(
                "plan inspection exceeds its source record and step budget",
            ));
        }
        let remaining = budget
            .max_bytes
            .saturating_sub(self.archive_bytes as u64)
            .min(plan_archive::MAX_ARCHIVE_BYTES as u64) as usize;
        if remaining < 45 {
            return Err(exhausted(
                "plan inspection exceeds its complete archive byte budget",
            ));
        }
        let limits = ArchiveLimits {
            max_bytes: remaining,
            ..ArchiveLimits::default()
        };
        let (bytes, seal) =
            plan_archive::encode_with_check(plan, limits, || check_deadline(budget, elapsed))?;
        self.archive_bytes = self.archive_bytes.saturating_add(bytes.len());
        Ok((bytes, seal))
    }

    fn json(&self) -> Value {
        json!({"records_and_steps": self.records, "archive_bytes": self.archive_bytes})
    }
}

pub(crate) fn read(session: &str, uri: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    let started = std::time::Instant::now();
    read_with_elapsed(session, uri, view, || started.elapsed())
}

fn read_with_elapsed(
    session_id: &str,
    uri: &str,
    view: &str,
    mut elapsed: impl FnMut() -> std::time::Duration,
) -> McpResult<Vec<ResourceContent>> {
    if uri.len() > 256 || uri != format!("df://session/{session_id}/{view}") {
        return Err(refusal(
            session_id,
            invalid("plan resource URI does not match its session parameters"),
            None,
        ));
    }
    let target = address(view).map_err(|e| refusal(session_id, e, None))?;
    let session =
        resolve_session(Some(session_id.to_owned())).map_err(|e| refusal(session_id, e, None))?;
    with_session_read(
        &session,
        |e| Err(refusal(session_id, e, None)),
        |guard| {
            let ctx = read_context(guard);
            // Full original plans can name any fortress domain. Both whole-fortress
            // grants precede even digest existence or unavailable-custody disclosure.
            authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
                .and_then(|()| authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly))
                .map_err(|e| refusal(session_id, e, None))?;
            let current = ctx.anchor;
            let result = (|| {
                check_deadline(guard.budget, &mut elapsed)?;
                let count = plan_resources::count(guard);
                if count > guard.budget.max_entities as usize {
                    return Err(exhausted(
                        "plan inspection exceeds its retained record budget",
                    ));
                }
                let mut work = SourceWork {
                    records: count,
                    ..SourceWork::default()
                };
                let canonical_session = guard.session_id.to_string();
                match target {
                    Address::Catalog { digest, offset } => {
                        let mut rows = Vec::with_capacity(count);
                        for entry in plan_resources::entries(guard) {
                            check_deadline(guard.budget, &mut elapsed)?;
                            let mut row = json!({"plan_digest": entry.digest.to_hex(),
                                "retention": entry.retention, "source_kind": entry.source_kind,
                                "status": "exact_plan_unavailable", "uri": null,
                                "archive": null, "step_details": null});
                            if let Some(plan) = entry.plan {
                                validate_identity(plan, entry.digest, current)?;
                                let (bytes, seal) =
                                    work.encode(plan, guard.budget, &mut elapsed)?;
                                row["status"] = json!("available");
                                row["object_digest"] = json!(seal.object_digest.to_hex());
                                row["summary"] = json!(plan.summary);
                                row["steps"] = json!(plan.steps.len());
                                row["archive_bytes"] = json!(bytes.len());
                                row["uri"] = json!(plan_uri(&canonical_session, seal));
                                row["archive"] = json!(archive_uri(&canonical_session, seal, 0));
                                row["step_details"] = json!(steps_uri(&canonical_session, seal, 0));
                            }
                            rows.push(row);
                        }
                        rows.sort_by(|a, b| {
                            (
                                a["plan_digest"].as_str(),
                                a["object_digest"].as_str(),
                                a["retention"].as_str(),
                            )
                                .cmp(&(
                                    b["plan_digest"].as_str(),
                                    b["object_digest"].as_str(),
                                    b["retention"].as_str(),
                                ))
                        });
                        catalog(
                            &canonical_session,
                            uri,
                            digest,
                            offset,
                            rows,
                            current,
                            guard.budget,
                            &work,
                            &mut elapsed,
                        )
                    }
                    Address::Plan { seal, page } => {
                        for entry in plan_resources::entries(guard)
                            .filter(|entry| entry.digest == seal.plan_digest)
                        {
                            let Some(plan) = entry.plan else {
                                continue;
                            };
                            validate_identity(plan, entry.digest, current)?;
                            let (bytes, actual) = work.encode(plan, guard.budget, &mut elapsed)?;
                            if actual != seal {
                                continue;
                            }
                            return match page {
                                None => {
                                    let payload = json!({
                                        "schema": "dfmcp.lab-plan-manifest/1",
                                        "inspection_projection": true,
                                        "plan_digest": seal.plan_digest.to_hex(),
                                        "object_digest": seal.object_digest.to_hex(),
                                        "plan_id": plan.id.to_string(),
                                        "intent_id": plan.intent_id.to_string(),
                                        "summary": plan.summary,
                                        "terminal_condition": crate::lab_world::predicate_json(&plan.terminal_condition),
                                        "steps": plan.steps.len(),
                                        "step_details": steps_uri(&canonical_session, seal, 0),
                                        "max_risk": plan.max_risk.as_str(),
                                        "required_capabilities": plan.required_capabilities.iter()
                                            .map(|cap| cap.as_str()).collect::<Vec<_>>(),
                                        "requires_checkpoint": plan.requires_checkpoint,
                                        "expires_at_tick": plan.expires_at_tick.0,
                                        "retention": entry.retention,
                                        "source_kind": entry.source_kind,
                                        "archive": {"format": "DFMPAR01", "bytes": bytes.len(),
                                            "sha256": Digest32::of_bytes(&bytes).to_hex(),
                                            "uri": archive_uri(&canonical_session, seal, 0)},
                                        "canonical_uri": plan_uri(&canonical_session, seal),
                                        "goal_truth": "not_evaluated",
                                        "dispatch_authority": false,
                                        "coverage": {"status": "manifest_only",
                                            "complete_domains": [], "absence_proof_scope": []},
                                    });
                                    render(
                                        payload, &canonical_session, uri, plan.anchor,
                                        current, guard.budget, &work, &mut elapsed,
                                    )?
                                    .ok_or_else(|| exhausted(
                                        "plan manifest exceeds the negotiated output budget; use its catalog archive link",
                                    ))
                                }
                                Some(PlanPage::Archive(start)) => archive(
                                    &canonical_session,
                                    uri,
                                    seal,
                                    &bytes,
                                    start,
                                    plan.anchor,
                                    current,
                                    guard.budget,
                                    &work,
                                    &mut elapsed,
                                ),
                                Some(PlanPage::Steps(start)) => steps(
                                    &canonical_session,
                                    uri,
                                    seal,
                                    plan,
                                    start,
                                    current,
                                    guard.budget,
                                    &work,
                                    &mut elapsed,
                                ),
                            };
                        }
                        Err(DfmcpError::new(
                            ErrorCode::StaleAnchor,
                            "the exact original plan archive is unavailable in this session; refresh its plan catalog",
                        ))
                    }
                }
            })();
            result
                .map(|text| super::text_content(uri, text))
                .map_err(|e| refusal(session_id, e, Some(current)))
        },
    )
}

fn validate_identity(
    plan: &dfmcp_intent::PreparedPlan,
    retained_digest: Digest32,
    current: StateAnchor,
) -> Result<()> {
    if plan.anchor.fortress_id != current.fortress_id || plan.digest != retained_digest {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "retained plan identity does not match its fortress and original digest",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn catalog(
    session: &str,
    uri: &str,
    expected: Option<Digest32>,
    start: usize,
    rows: Vec<Value>,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<String> {
    // The continuation binds the entire catalog, owner session and current
    // anchor. Replacement, commit or collection cannot silently shift a page.
    let hash = Digest32::of_bytes(
        json!({"domain": "dfmcp.plan-resource-catalog/1",
            "session_id": session, "anchor": anchor_json(&current), "rows": rows})
        .to_string()
        .as_bytes(),
    );
    if expected.is_some_and(|expected| expected != hash) {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "plan catalog changed; restart at the plans resource",
        ));
    }
    let total = rows.len();
    if start > total || (start == total && total != 0) {
        return Err(invalid(
            "plan catalog offset is outside the retained catalog",
        ));
    }
    let mut returned = (total - start).min(MAX_PAGE_RECORDS);
    loop {
        let end = start + returned;
        let next = (end < total).then(|| catalog_uri(session, hash, end));
        let payload = json!({
            "schema": "dfmcp.lab-plan-catalog/1", "catalog_digest": hash.to_hex(),
            "canonical_uri": catalog_uri(session, hash, start),
            "offset": start, "returned": returned, "total": total,
            "rows": rows[start..end], "complete": end == total,
            "complete_catalog": start == 0 && end == total, "next": next,
            "retention": {"pins_plans": false,
                "scope": "pending plan and retained original objectives"},
            "coverage": {"status": "retained_catalog_only", "complete_domains": [],
                "absence_proof_scope": [], "continuation": next},
        });
        if let Some(text) = render(
            payload, session, uri, current, current, budget, work, elapsed,
        )? {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted(
                "plan catalog metadata or one record exceeds the negotiated output budget",
            ));
        }
        returned /= 2;
    }
}

#[allow(clippy::too_many_arguments)]
fn archive(
    session: &str,
    uri: &str,
    seal: PlanArchiveSeal,
    bytes: &[u8],
    start: usize,
    anchor: StateAnchor,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<String> {
    if start >= bytes.len() {
        return Err(invalid(
            "archive offset is outside the exact original object",
        ));
    }
    let archive_sha256 = Digest32::of_bytes(bytes).to_hex();
    let mut returned = (bytes.len() - start).min(MAX_PAGE_BYTES);
    loop {
        let end = start + returned;
        let next = (end < bytes.len()).then(|| archive_uri(session, seal, end));
        let data_hex: String = bytes[start..end]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let payload = json!({
            "schema": "dfmcp.lab-plan-archive-page/1",
            "plan_digest": seal.plan_digest.to_hex(),
            "object_digest": seal.object_digest.to_hex(),
            "archive_sha256": archive_sha256, "format": "DFMPAR01", "encoding": "hex",
            "offset": start, "returned_bytes": returned, "total_bytes": bytes.len(),
            "data_hex": data_hex, "next": next, "complete": end == bytes.len(),
            "complete_object": start == 0 && end == bytes.len(),
            "canonical_uri": archive_uri(session, seal, start), "dispatch_authority": false,
            "coverage": {"status": "archive_bytes_only", "complete_domains": [],
                "absence_proof_scope": [], "continuation": next},
        });
        if let Some(text) = render(
            payload, session, uri, anchor, current, budget, work, elapsed,
        )? {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted(
                "plan archive metadata or one byte exceeds the negotiated output budget",
            ));
        }
        returned /= 2;
    }
}

#[allow(clippy::too_many_arguments)]
fn steps(
    session: &str,
    uri: &str,
    seal: PlanArchiveSeal,
    plan: &dfmcp_intent::PreparedPlan,
    start: usize,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<String> {
    let total = plan.steps.len();
    if start > total || (start == total && total != 0) {
        return Err(invalid("step offset is outside the exact original plan"));
    }
    // The caller already admitted and validated the entire exact archive.
    // Reuse the existing readable projection without treating its deliberately
    // omitted action/contract fields as an exact typed plan representation.
    check_deadline(budget, elapsed)?;
    let Value::Array(rows) = crate::lab_world::plan_steps_json(plan) else {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "the laboratory step projection did not return an array",
        ));
    };
    check_deadline(budget, elapsed)?;
    if rows.len() != total {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "the laboratory step projection changed the original step count",
        ));
    }
    let mut returned = (total - start).min(MAX_PAGE_RECORDS);
    loop {
        let end = start + returned;
        let next = (end < total).then(|| steps_uri(session, seal, end));
        let payload = json!({
            "schema": "dfmcp.lab-plan-steps-page/1", "inspection_projection": true,
            "plan_digest": seal.plan_digest.to_hex(), "object_digest": seal.object_digest.to_hex(),
            "canonical_uri": steps_uri(session, seal, start),
            "offset": start, "returned": returned, "total": total, "rows": rows[start..end],
            "complete": end == total, "complete_projection": start == 0 && end == total,
            "next": next, "archive": archive_uri(session, seal, 0),
            "goal_truth": "not_evaluated", "dispatch_authority": false,
            "projection": {
                "renderer": "laboratory_plan_steps", "exact_typed_contract": false,
                "omitted_contract_fields": ["full_typed_action", "full_typed_predicates", "preconditions",
                    "compensation_action", "obligation_failure_predicate", "idempotency_key"],
                "predicates": "sealed_requirements_not_current_evidence",
                "created_entity_ids": "predicted_from_sealed_steps_not_dispatch_evidence"
            },
            "coverage": {"status": "step_projection_only", "complete_domains": [],
                "absence_proof_scope": [], "continuation": next},
        });
        if let Some(text) = render(
            payload,
            session,
            uri,
            plan.anchor,
            current,
            budget,
            work,
            elapsed,
        )? {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted(
                "step page metadata or one indivisible projection row exceeds the negotiated output budget",
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
    anchor: StateAnchor,
    current: StateAnchor,
    budget: WorkBudget,
    work: &SourceWork,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<Option<String>> {
    let historical = anchor != current;
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
    payload["anchor"] = anchor_json(&anchor);
    payload["current_anchor"] = anchor_json(&current);
    payload["historical"] = json!(historical);
    payload["scope"] = json!("laboratory_original_plan");
    payload["game_mutation_dispatched"] = json!(false);
    payload["source_work"] = work.json();
    let mut active = empty_active_work();
    active["coverage"] =
        json!({"status": "omitted", "reason": "use the authorized handoff resource"});
    payload["agent_turn"] = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .session_id(session)
        .anchor(anchor_json(&anchor))
        .continuity(
            if historical {
                ContinuityStatus::Stale
            } else {
                ContinuityStatus::Partial
            },
            Some(anchor_json(&anchor)),
            None,
            None,
        )
        .profile(ObservationProfile::Forensic)
        .briefing(json!({"scope": "laboratory_original_plan",
            "historical": historical, "mutation_authority": false}))
        .active_work(active)
        .coverage(payload["coverage"].clone())
        .uncertainty(vec![json!({"kind": "unknown", "domain": "current_goal_truth",
            "reason": "retained plan bytes do not prove current predicates or live game compatibility"})])
        .budget(json!({"admitted": true, "max_output_bytes": max_bytes,
            "max_output_tokens": budget.max_output_tokens, "source_work": work.json()}))
        .references(vec![
            json!({"kind": "plan_catalog", "uri": format!("df://session/{session}/plans")}),
            json!({"kind": "active_work", "uri": format!("df://session/{session}/handoff")}),
        ])
        .build();
    let serialized = payload.to_string();
    let fits = super::task_resource_bytes(uri, &serialized) as u64 <= max_bytes;
    check_deadline(budget, elapsed)?;
    Ok(fits.then_some(serialized))
}

#[cfg(test)]
mod tests;
