//! Exact-version laboratory resources for bead `dfmcp-reality-wiring-proof-4bf.11`.
//!
//! Reads reuse the canonical presence-aware observation projection. A session
//! slot supplies an authority lookup, never authority by itself. Every read,
//! including an old version, rechecks grants against the current world tick.

use dfmcp_core::{
    AnchorEpochs, Capability, DfmcpError, ErrorCode, Result, RiskTier, StateAnchor, StateAnchorV2,
    WorkBudget,
};
use dfmcp_world::WorldSnapshot;
use fastmcp_rust::prelude::{McpError, McpResult, ResourceContent};
use serde_json::{Value, json};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
};
use crate::server::{
    anchor_json, authorize_entry, next_context, parse_session_id_arg, resolve_session,
    retained_resource_snapshot, with_session,
};

const OPERATION: &str = "resources.read.state";
const SECTIONS: [&str; 4] = ["entities", "relations", "chunks", "events"];
const MAX_PAGE_RECORDS: usize = 25;
// Below the pinned facade's text block bound, including escaped JSON overhead.
const MAX_RESOURCE_BYTES: u64 = 60_000;

fn response_bytes(uri: &str, serialized: &str) -> usize {
    // The shared resource wrapper includes 128 bytes of envelope allowance.
    // The pinned modern protocol accepts exact integer ID lexemes up to 4096
    // bytes (string IDs are smaller). McpContext exposes only an internal u64
    // sequence, so reserve the actual wire maximum rather than that sequence.
    super::task_resource_bytes(uri, serialized).saturating_add(4096)
}

#[derive(Debug, PartialEq, Eq)]
struct Address<'a> {
    hash: Option<&'a str>,
    page: Option<(&'a str, usize)>,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn address(view: &str) -> Result<Address<'_>> {
    if view == "state" {
        return Ok(Address {
            hash: None,
            page: None,
        });
    }
    if view.len() > 128 {
        return Err(invalid("state resource address exceeds its byte bound"));
    }
    let suffix = view
        .strip_prefix("state-")
        .ok_or_else(|| invalid("unknown state resource"))?;
    let mut parts = suffix.split('~');
    let hash = parts.next().ok_or_else(|| invalid("missing state hash"))?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "state hash must be exactly 64 lowercase hexadecimal characters",
        ));
    }
    let page = match parts.next() {
        None => None,
        Some(section) => {
            if !SECTIONS.contains(&section) {
                return Err(invalid(
                    "state section must be entities, relations, chunks, or events",
                ));
            }
            let raw = parts
                .next()
                .ok_or_else(|| invalid("missing state page offset"))?;
            let offset = raw
                .parse::<usize>()
                .map_err(|_| invalid("invalid state page offset"))?;
            if raw != offset.to_string() || parts.next().is_some() {
                return Err(invalid(
                    "state page offset must be a canonical nonnegative decimal integer",
                ));
            }
            Some((section, offset))
        }
    };
    Ok(Address {
        hash: Some(hash),
        page,
    })
}

fn refusal(
    session_id: &str,
    error: DfmcpError,
    authorized_current_anchor: Option<StateAnchor>,
) -> McpError {
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
    if let Ok(id) = parse_session_id_arg(session_id) {
        turn = turn.session_id(id.to_string());
    }
    if let Some(anchor) = authorized_current_anchor {
        turn = turn
            .anchor(anchor_json(&anchor))
            .recommendations(vec![json!({
                "tool": "fortress.observe", "reason": "obtain a fresh authorized observation basis"
            })]);
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

pub(super) fn read(session_id: &str, uri: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    if uri.len() > 192 || uri != format!("df://session/{session_id}/{view}") {
        return Err(refusal(
            session_id,
            invalid("state resource URI does not match its session parameters"),
            None,
        ));
    }
    let target = address(view).map_err(|e| refusal(session_id, e, None))?;
    // resolve_session checks the durable-owner fence as well as session identity.
    let session =
        resolve_session(Some(session_id.to_owned())).map_err(|e| refusal(session_id, e, None))?;
    with_session(
        &session,
        || {
            Err(refusal(
                session_id,
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "state resource session is unavailable",
                ),
                None,
            ))
        },
        |guard| {
            let (_, ctx) = next_context(guard).map_err(|e| refusal(session_id, e, None))?;
            authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
                .map_err(|e| refusal(session_id, e, None))?;
            let current = ctx.anchor;
            let snapshot = retained_resource_snapshot(guard, target.hash)
                .map_err(|e| refusal(session_id, e, Some(current)))?;
            if snapshot.fortress_id != current.fortress_id {
                return Err(refusal(
                    session_id,
                    invalid("retained version belongs to a different fortress"),
                    Some(current),
                ));
            }
            let canonical_session = guard.session_id.to_string();
            let payload = match target.page {
                None => manifest(&canonical_session, snapshot),
                Some((section, offset)) => page(snapshot, section, offset, guard.budget),
            }
            .and_then(|payload| {
                render(
                    payload,
                    &canonical_session,
                    uri,
                    snapshot,
                    current,
                    guard.budget,
                )
            });
            payload
                .map(|text| super::text_content(uri, text))
                .map_err(|e| refusal(session_id, e, Some(current)))
        },
    )
}

fn state_uri(session_id: &str, hash: &str) -> String {
    format!("df://session/{session_id}/state-{hash}")
}

fn page_uri(session_id: &str, hash: &str, section: &str, offset: usize) -> String {
    format!("df://session/{session_id}/state-{hash}~{section}~{offset}")
}

fn manifest(session_id: &str, snapshot: &WorldSnapshot) -> Result<Value> {
    let hash = snapshot.state_hash.to_hex();
    let counts = [
        snapshot.graph.entities.len(),
        snapshot.graph.edges.len(),
        snapshot.graph.chunks.len(),
        snapshot.graph.events.len(),
    ];
    Ok(json!({
        "schema": "dfmcp.lab-state-manifest/1", "state_hash": hash,
        "sections": SECTIONS.iter().zip(counts).map(|(section, count)| json!({
            "section": section, "record_count": count,
            "uri": page_uri(session_id, &hash, section, 0)
        })).collect::<Vec<_>>(),
        "observation_coverage": {"status": "manifest_only", "complete_domains": [],
            "absence_proof_scope": [], "reason": "record counts do not prove field knowledge or live game completeness"},
        "retention": {"pins_version": false, "unavailable": "stale_anchor",
            "policy": "session recent versions plus versions retained by pending plans and checkpoints"},
    }))
}

fn page(
    snapshot: &WorldSnapshot,
    section: &str,
    offset: usize,
    budget: WorkBudget,
) -> Result<Value> {
    // The existing projection verifies the entire canonical source. Charge its
    // work, not just returned rows, before constructing a research-full profile.
    let records = snapshot
        .graph
        .entities
        .len()
        .saturating_add(snapshot.graph.edges.len())
        .saturating_add(snapshot.graph.chunks.len())
        .saturating_add(snapshot.graph.events.len());
    if records as u64 > u64::from(budget.max_entities) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "canonical state projection exceeds the negotiated source record budget",
        ));
    }
    let source_bytes = snapshot.canonical_bytes().len();
    if source_bytes as u64 > budget.max_bytes {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "canonical state projection exceeds the negotiated source byte budget",
        ));
    }
    let mut payload = crate::lab_world::query(
        snapshot,
        &json!({
            "mode": "observation", "completeness_profile": "research-full",
            "section": section, "limit": MAX_PAGE_RECORDS, "offset": offset,
        })
        .to_string(),
    )?;
    payload["schema"] = json!("dfmcp.lab-state-page/1");
    payload["source_work"] = json!({"records": records, "canonical_bytes": source_bytes});
    if let Some(object) = payload.as_object_mut() {
        object.remove("query");
        object.remove("continuation");
    }
    Ok(payload)
}

fn version_identity(snapshot: &WorldSnapshot) -> Result<Value> {
    let v2 = StateAnchorV2::from_v1(
        snapshot.anchor(),
        AnchorEpochs::laboratory(snapshot.fortress_id),
    )?;
    Ok(json!({
        "fortress_lineage": v2.fortress_lineage.to_hex(),
        "observation_epoch": v2.observation_epoch, "snapshot_sequence": v2.snapshot_sequence,
        "game_tick": v2.game_tick.map(|tick| tick.0), "bridge_generation": v2.bridge_generation,
        "bridge_protocol": v2.bridge_protocol, "adapter_epoch": v2.adapter_epoch,
        "schema_epoch": v2.schema_epoch, "policy_epoch": v2.policy_epoch,
        "semantic_world_root": v2.semantic_world_root.to_hex(), "digest": v2.digest().to_hex(),
    }))
}

fn render(
    mut payload: Value,
    session_id: &str,
    uri: &str,
    snapshot: &WorldSnapshot,
    current: StateAnchor,
    budget: WorkBudget,
) -> Result<String> {
    let anchor = snapshot.anchor();
    let hash = snapshot.state_hash.to_hex();
    let historical = anchor != current;
    let max_bytes = budget
        .max_bytes
        .min(
            u64::from(budget.max_output_tokens)
                .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
        )
        .min(MAX_RESOURCE_BYTES);
    payload["ok"] = json!(true);
    payload["session_id"] = json!(session_id);
    payload["resource"] = json!(uri);
    payload["canonical_uri"] = json!(state_uri(session_id, &hash));
    payload["anchor"] = anchor_json(&anchor);
    payload["anchor_v2"] = version_identity(snapshot)?;
    payload["current_anchor"] = anchor_json(&current);
    payload["historical"] = json!(historical);
    payload["scope"] = json!("laboratory_reference_world");
    payload["game_mutation_dispatched"] = json!(false);
    loop {
        if payload["rows"].is_array() {
            update_page(&mut payload, session_id, &hash)?;
        }
        let mut active_work = empty_active_work();
        active_work["coverage"] =
            json!({"status": "omitted", "reason": "use the authorized handoff resource"});
        payload["agent_turn"] = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
            .session_id(session_id).anchor(anchor_json(&anchor))
            .continuity(if historical { ContinuityStatus::Stale } else { ContinuityStatus::Partial },
                Some(anchor_json(&anchor)), None, None)
            .profile(ObservationProfile::Tactical)
            .briefing(json!({"scope": "laboratory_reference_world", "paused": snapshot.paused,
                "historical": historical, "mutation_authority": false}))
            .active_work(active_work)
            .coverage(payload["observation_coverage"].clone())
            .uncertainty(vec![json!({"kind": "unknown", "domain": "live_game_evidence",
                "reason": "records describe the process-local reference world"})])
            .budget(json!({"admitted": true, "max_output_bytes": max_bytes,
                "max_output_tokens": budget.max_output_tokens, "source_work": payload["source_work"]}))
            .references(vec![json!({"kind": "immutable_world", "uri": state_uri(session_id, &hash)}),
                json!({"kind": "active_work", "uri": format!("df://session/{session_id}/handoff")})])
            .build();
        let serialized = payload.to_string();
        if response_bytes(uri, &serialized) as u64 <= max_bytes {
            return Ok(serialized);
        }
        let Some(rows) = payload["rows"].as_array_mut() else {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "state manifest metadata exceeds the negotiated output budget",
            ));
        };
        if rows.len() <= 1 {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "state page metadata or one indivisible record exceeds the negotiated output budget",
            ));
        }
        rows.truncate(rows.len() / 2);
    }
}

fn update_page(payload: &mut Value, session_id: &str, hash: &str) -> Result<()> {
    let section = payload["section"]
        .as_str()
        .ok_or_else(|| invalid("state section missing"))?
        .to_owned();
    let offset = payload["offset"]
        .as_u64()
        .ok_or_else(|| invalid("state offset missing"))?;
    let total = payload["total"]
        .as_u64()
        .ok_or_else(|| invalid("state total missing"))?;
    let returned = payload["rows"].as_array().map_or(0, Vec::len);
    let end = offset
        .checked_add(returned as u64)
        .ok_or_else(|| invalid("state page offset overflow"))?;
    let next = if end < total {
        let offset = usize::try_from(end).map_err(|_| invalid("state page offset overflow"))?;
        Some(page_uri(session_id, hash, &section, offset))
    } else {
        None
    };
    let complete_domain = offset == 0 && end == total;
    let canonical_offset = usize::try_from(offset)
        .map_err(|_| invalid("state page offset exceeds the addressable range"))?;
    payload["canonical_uri"] = json!(page_uri(session_id, hash, &section, canonical_offset));
    payload["returned"] = json!(returned);
    payload["next_offset"] = json!((end < total).then_some(end));
    payload["next"] = json!(next);
    payload["complete"] = json!(end == total);
    payload["complete_domain"] = json!(complete_domain);
    payload["absence_proven"] = json!(complete_domain && total == 0);
    payload["truncated"] = json!(!complete_domain);
    let coverage = &mut payload["observation_coverage"];
    coverage["continuation"] = json!(next);
    coverage["page_complete"] = json!(complete_domain);
    if !complete_domain {
        // A row boundary clipped by the byte budget must also retract every
        // full-domain/absence claim made by the original larger projection.
        let complete = coverage["complete_domains"]
            .as_array()
            .map_or_else(Vec::new, Clone::clone);
        if !complete.is_empty() {
            coverage["partial_domains"] = json!(complete);
        }
        coverage["status"] = json!("partial");
        coverage["complete_domains"] = json!([]);
        coverage["absence_proof_scope"] = json!([]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
