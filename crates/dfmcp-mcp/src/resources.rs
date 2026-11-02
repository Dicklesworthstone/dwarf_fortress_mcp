//! Read-only `df://` resource families (MCP_SURFACE.md §Resources),
//! laboratory slice.
//!
//! Session-scoped URIs carry their own authority: the `{session_id}` slot
//! names the session whose negotiated grants gate the read. Fortress-scoped
//! families (`df://fortress/{id}/anchor`, map chunks, events, …) stay
//! unimplemented on purpose until `resources/read` gains a session-binding
//! design: a fortress URI without a session slot could only be authorized
//! from the transport context, and transport identity grants nothing
//! (CAPABILITIES.md: no ambient authority).
//!
//! Facade note: the pinned `fastmcp_rust` facade does not re-export
//! `UriParams`, so template handlers spell the parameter type as the
//! transparent underlying `HashMap<String, String>` (type aliases are
//! transparent). Recorded as DRAFT-D in `docs/DOGFOODING_FASTMCP.md`.

use std::collections::HashMap;
use std::sync::Arc;

use dfmcp_adapter::GameAdapter;
use fastmcp_rust::prelude::{
    McpContext, McpError, McpResult, Resource, ResourceContent, ResourceHandler,
};
use fastmcp_rust::{
    CacheTtl, CompleteResult, FinalReadResourceResult, FinalResourceReadCacheHintProvenance,
    ResourceTemplate,
};
use serde_json::{Value, json};

use crate::server::{
    active_session_count, anchor_json, authorize_entry, lookup_session, next_context,
    parse_session_id_arg, snapshot_json,
};
use dfmcp_core::{Capability, Digest32, EntityId, RiskTier, SessionId, StateAnchor};

/// One template serves every session view. The pinned fastmcp router admits
/// at most one template per compatible leading literal (`df://session/`), so
/// separate per-view templates would be silently dropped; dispatching on the
/// `view` capture keeps the documented URIs exactly.
const SESSION_VIEW_TEMPLATE: &str = "df://session/{session_id}/{view}";
/// Views served under `df://session/{session_id}/{view}`.
pub const SESSION_VIEWS: [&str; 6] = [
    "summary",
    "capabilities",
    "handoff",
    "replay",
    "anchor",
    "tasks",
];
const DOCTOR_BUNDLE_TEMPLATE: &str = "df://doctor/{session_id}";

fn template_definition(uri_template: &str, name: &str, description: &str) -> Resource {
    Resource {
        uri: uri_template.to_owned(),
        name: name.to_owned(),
        description: Some(description.to_owned()),
        mime_type: Some("application/json".to_owned()),
        icon: None,
        version: None,
        tags: Vec::new(),
    }
}

fn template_of(uri_template: &str, name: &str, description: &str) -> ResourceTemplate {
    ResourceTemplate {
        uri_template: uri_template.to_owned(),
        name: name.to_owned(),
        description: Some(description.to_owned()),
        mime_type: Some("application/json".to_owned()),
        icon: None,
        version: None,
        tags: Vec::new(),
    }
}

fn session_id_param(raw: &str) -> std::result::Result<dfmcp_core::SessionId, McpError> {
    if raw.is_empty() {
        return Err(McpError::invalid_params(
            "invalid_params: missing session_id template parameter (expected a 32-character hexadecimal identifier)",
        ));
    }
    parse_session_id_arg(raw)
        .map_err(|error| McpError::invalid_params(format!("invalid_params: {}", error.message)))
}

fn lookup(
    raw: &str,
    operation: &str,
) -> std::result::Result<Arc<std::sync::Mutex<crate::server::LabSession>>, McpError> {
    let session_id = session_id_param(raw)?;
    lookup_session(session_id).map_err(|error| denial(operation, error))
}

fn denial(operation: &str, error: dfmcp_core::DfmcpError) -> McpError {
    McpError::invalid_request(format!(
        "{}: {} ({})",
        operation,
        error.code.as_str(),
        error.message
    ))
}

fn poisoned(operation: &str) -> McpError {
    McpError::internal_error(format!(
        "internal_invariant_violation: {operation} session mutex poisoned"
    ))
}

fn text_content(uri: &str, payload: String) -> Vec<ResourceContent> {
    vec![ResourceContent {
        uri: uri.to_owned(),
        mime_type: Some("application/json".to_owned()),
        text: Some(payload),
        blob: None,
    }]
}

fn template_read_refusal() -> McpError {
    McpError::invalid_request(
        "capability_denied: df:// resources require a concrete session-scoped URI; \
         template URIs carry no authority",
    )
}

// Session resources reflect current authority and state, even when their URI
// names an immutable anchor. A client must ask the server again on every read.
fn uncached_resource_contents(
    contents: Vec<ResourceContent>,
) -> McpResult<CompleteResult<FinalReadResourceResult>> {
    let mut result = fastmcp_server::promote_legacy_resource_contents(contents)?;
    result.payload.ttl_ms = CacheTtl::milliseconds(0);
    Ok(result)
}

fn entity_anchor_digest(anchor: StateAnchor) -> dfmcp_core::Result<Digest32> {
    Ok(dfmcp_core::StateAnchorV2::from_v1(
        anchor,
        dfmcp_core::AnchorEpochs::laboratory(anchor.fortress_id),
    )?
    .digest())
}

/// Discovery uses the same complete anchor digest as the resource consumer.
/// The URI is a reference, not an authority grant or a historical fallback.
pub(crate) fn link_entity_resources(
    payload: &mut Value,
    session_id: SessionId,
    anchor: StateAnchor,
) -> dfmcp_core::Result<()> {
    if payload.get("mode").and_then(Value::as_str) != Some("entities") {
        return Ok(());
    }
    let digest = entity_anchor_digest(anchor)?.to_hex();
    if let Some(rows) = payload.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows {
            if let (Some(id), Some(generation)) = (
                row.get("entity_id").and_then(Value::as_str),
                row.get("generation").and_then(Value::as_u64),
            ) {
                row["resource_uri"] = json!(format!(
                    "df://session/{session_id}/entity-{id}~{generation}~{digest}"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn session_entity(
    session_id_hex: &str,
    uri: &str,
    view: &str,
) -> McpResult<Vec<ResourceContent>> {
    const OPERATION: &str = "df://session/entity";
    let started = std::time::Instant::now();
    // At most 20 ID digits, 10 generation digits and one 64-digit digest.
    if view.len() > 104 {
        return Err(McpError::invalid_params(
            "entity reference exceeds its byte bound",
        ));
    }
    let mut fields = view.strip_prefix("entity-").unwrap_or("").split('~');
    let (Some(raw_id), Some(raw_generation), Some(raw_anchor), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(McpError::invalid_params(
            "expected entity-{id}~{generation}~{anchor_digest}",
        ));
    };
    let id = raw_id.parse::<u64>().map(EntityId::new).map_err(|_| {
        McpError::invalid_params("entity ID must be a canonical nonzero decimal integer")
    })?;
    let generation = raw_generation.parse::<u32>().map_err(|_| {
        McpError::invalid_params("entity generation must be a canonical decimal integer")
    })?;
    let requested_anchor = Digest32::from_hex(raw_anchor)
        .filter(|digest| digest.to_hex() == raw_anchor)
        .ok_or_else(|| {
            McpError::invalid_params("entity anchor must be 64 lowercase hexadecimal digits")
        })?;
    if id == EntityId::NIL
        || id.get().to_string() != raw_id
        || generation.to_string() != raw_generation
    {
        return Err(McpError::invalid_params("noncanonical entity identity"));
    }
    if session_id_hex.len() != 32 || uri != format!("df://session/{session_id_hex}/{view}") {
        return Err(McpError::invalid_params("noncanonical session entity URI"));
    }
    let session = lookup(session_id_hex, OPERATION)?;
    crate::server::with_session_read(
        &session,
        |error| Err(denial(OPERATION, error)),
        |guard| {
            let ctx = crate::server::read_context(guard);
            // Authorize the requested ID before inspecting its existence or generation.
            ctx.authorize(Capability::Query, RiskTier::ReadOnly, &[id], None)
                .map_err(|error| denial(OPERATION, error))?;
            let snapshot = guard.adapter.snapshot();
            let anchor = snapshot.anchor();
            let current = entity_anchor_digest(anchor).map_err(|error| denial(OPERATION, error))?;
            if current != requested_anchor {
                return Err(McpError::invalid_request(
                    "stale_anchor: entity reference does not name the current complete anchor; query fresh references",
                ));
            }
            let entity = snapshot.graph.entities.get(&id).ok_or_else(|| {
                McpError::invalid_request(
                    "entity_unavailable: exact entity is unavailable at this anchor",
                )
            })?;
            if entity.generation != generation {
                return Err(McpError::invalid_request(
                    "stale_anchor: entity generation changed; query fresh references",
                ));
            }
            let payload = json!({
                "ok": true, "schema": "dfmcp.lab-entity/1", "resource": uri,
                "session_id": session_id_hex, "anchor": anchor_json(&anchor),
                "anchor_digest": current.to_hex(),
                "entity": crate::observation_projection::entity_json(entity),
                "coverage": {"status": "complete_for_named_projection",
                    "domain": "one_current_laboratory_entity", "absence_proof_scope": [],
                    "live_dwarf_fortress": "unavailable"},
            });
            let serialized = payload.to_string();
            let max_bytes = guard.budget.max_bytes.min(
                u64::from(guard.budget.max_output_tokens)
                    .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
            );
            if task_resource_bytes(uri, &serialized) as u64 > max_bytes
                || started.elapsed()
                    >= std::time::Duration::from_millis(guard.budget.max_wall_millis)
            {
                return Err(McpError::invalid_request(
                    "budget_exceeded: entity resource exceeds negotiated output bounds; no fields were silently omitted",
                ));
            }
            Ok(text_content(uri, serialized))
        },
    )
}

// ============================================================================
// Pure read implementations (also exercised directly by tests)
// ============================================================================

/// `df://session/{session_id}/summary` — bounded snapshot projection.
/// Requires the session's negotiated `observe` capability.
pub(crate) fn session_summary(session_id_hex: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/summary";
    let session = lookup(session_id_hex, operation)?;
    crate::server::with_session(
        &session,
        || Err(poisoned(operation)),
        |guard| {
            let (_, ctx) = next_context(guard).map_err(|error| denial(operation, error))?;
            if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
                return Err(denial(operation, error));
            }
            let request = dfmcp_adapter::ObservationRequest {
                since: None,
                projection: dfmcp_adapter::Projection::Summary,
                interest: dfmcp_adapter::InterestSet::default(),
                max_entities: guard.budget.max_entities,
                max_bytes: guard.budget.max_bytes,
                max_output_tokens: guard.budget.max_output_tokens,
                continuation: None,
            };
            match guard.adapter.observe(&request, &ctx) {
                Ok(frame) => match frame.payload {
                    dfmcp_adapter::ObservationPayload::Snapshot(snapshot) => {
                        let mut payload = snapshot_json(&snapshot);
                        payload["projection"] = json!("summary");
                        payload["session_id"] = json!(format!("{}", guard.session_id));
                        payload["resource"] = json!(uri);
                        Ok(text_content(uri, payload.to_string()))
                    }
                    _ => Err(McpError::internal_error(
                        "internal_invariant_violation: summary observation returned a non-snapshot payload",
                    )),
                },
                Err(error) => Err(denial(operation, error)),
            }
        },
    )
}

/// `df://session/{session_id}/capabilities` — the session's own negotiation
/// record and grants. Self-description confers no authority, so this view
/// needs no capability beyond a valid session.
pub(crate) fn session_capabilities(
    session_id_hex: &str,
    uri: &str,
) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/capabilities";
    let session = lookup(session_id_hex, operation)?;
    let guard = session.lock().map_err(|_| poisoned(operation))?;
    let payload = json!({
        "ok": true,
        "session_id": format!("{}", guard.session_id),
        "fortress_id": format!("{}", guard.fortress_id),
        "granted_capabilities": guard
            .grants
            .iter()
            .map(|grant| json!({
                "capability": grant.capability.as_str(),
                "max_risk": grant.max_risk.as_str(),
            }))
            .collect::<Vec<_>>(),
        "budget": {
            "max_wall_millis": guard.budget.max_wall_millis,
            "max_game_ticks": guard.budget.max_game_ticks,
            "max_entities": guard.budget.max_entities,
            "max_bytes": guard.budget.max_bytes,
            "max_output_tokens": guard.budget.max_output_tokens,
            "max_actions": guard.budget.max_actions,
        },
        "negotiation": guard.negotiation.to_json(),
        "note": "self-description of the session's own authority; confers nothing",
        "resource": uri,
    });
    Ok(text_content(uri, payload.to_string()))
}

/// `df://doctor/{session_id}` — doctor report bundle for the session.
/// Requires the session's negotiated `doctor` capability.
pub(crate) fn doctor_bundle(session_id_hex: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://doctor";
    let session = lookup(session_id_hex, operation)?;
    crate::server::with_session(
        &session,
        || Err(poisoned(operation)),
        |guard| {
            let (_, ctx) = next_context(guard).map_err(|error| denial(operation, error))?;
            if let Err(error) = authorize_entry(&ctx, Capability::Doctor, RiskTier::ReadOnly) {
                return Err(denial(operation, error));
            }
            let health_res = guard.adapter.health(&ctx);
            let health_opt = health_res.as_ref().ok();
            let report = crate::doctor::DoctorInspector.generate_report(
                active_session_count(),
                health_opt,
                None,
                0,
                0,
            );
            let payload = match health_res {
                Ok(health) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "status": if report.is_healthy { "healthy" } else { "degraded" },
                    "active_sessions_count": report.active_sessions_count,
                    "adapter": health.identity.name,
                    "compatibility": format!("{:?}", health.identity.compatibility),
                    "fortress_loaded": health.fortress_loaded,
                    "findings": report.findings,
                    "warnings": health.warnings,
                    "current_anchor": health.current_anchor.as_ref().map(anchor_json),
                    "resource": uri,
                }),
                Err(error) => json!({
                    "ok": false,
                    "error": {
                        "operation": operation,
                        "code": error.code.as_str(),
                        "message": error.message,
                        "retryable": error.retryable,
                    },
                    "resource": uri,
                }),
            };
            Ok(text_content(uri, payload.to_string()))
        },
    )
}

/// `df://session/{session_id}/handoff` — resumable handoff packet: anchor,
/// grants, pending plan, open actions with obligations, and an ordered resume
/// protocol. Requires the session's negotiated `observe` capability. Reading
/// it never polls, dispatches, or changes state.
pub(crate) fn session_handoff(session_id_hex: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/handoff";
    let session = lookup(session_id_hex, operation)?;
    crate::server::with_session(
        &session,
        || Err(poisoned(operation)),
        |guard| {
            let (_, ctx) = next_context(guard).map_err(|error| denial(operation, error))?;
            if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
                return Err(denial(operation, error));
            }
            let mut payload = crate::server::handoff_json(guard);
            payload["resource"] = json!(uri);
            Ok(text_content(uri, payload.to_string()))
        },
    )
}

/// `df://session/{session_id}/anchor` — the session fortress's current
/// canonical anchor only (the session-bound form of `df://fortress/{id}/anchor`;
/// a fortress URI without a session slot would have no authority source).
/// Requires `observe`; never changes state.
pub(crate) fn session_anchor(session_id_hex: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/anchor";
    let session = lookup(session_id_hex, operation)?;
    crate::server::with_session(
        &session,
        || Err(poisoned(operation)),
        |guard| {
            let (_, ctx) = next_context(guard).map_err(|error| denial(operation, error))?;
            if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
                return Err(denial(operation, error));
            }
            let snapshot = guard.adapter.snapshot();
            let v2 = dfmcp_core::StateAnchorV2::from_v1(
                snapshot.anchor(),
                dfmcp_core::AnchorEpochs::laboratory(snapshot.fortress_id),
            )
            .map_err(|error| denial(operation, error))?;
            let payload = json!({
                "ok": true,
                "resource": uri,
                "fortress_id": snapshot.fortress_id.get().to_string(),
                "anchor": anchor_json(&snapshot.anchor()),
                "anchor_v2": {
                    "fortress_lineage": v2.fortress_lineage.to_hex(),
                    "observation_epoch": v2.observation_epoch,
                    "snapshot_sequence": v2.snapshot_sequence,
                    "game_tick": v2.game_tick.map(|tick| tick.0),
                    "bridge_generation": v2.bridge_generation,
                    "bridge_protocol": v2.bridge_protocol,
                    "adapter_epoch": v2.adapter_epoch,
                    "schema_epoch": v2.schema_epoch,
                    "policy_epoch": v2.policy_epoch,
                    "semantic_world_root": v2.semantic_world_root.to_hex(),
                    "digest": v2.digest().to_hex(),
                    "note": "complete version tuple; a changed observation_epoch invalidates every older handle",
                },
                "paused": snapshot.paused,
            });
            Ok(text_content(uri, payload.to_string()))
        },
    )
}

/// `df://session/{session_id}/replay` — the session's deterministic replay
/// bundle. Requires the negotiated `observe` capability; never changes state.
pub(crate) fn session_replay(session_id_hex: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/replay";
    let session = lookup(session_id_hex, operation)?;
    let (_, ctx) = {
        let mut guard = session.lock().map_err(|_| poisoned(operation))?;
        next_context(&mut guard).map_err(|error| denial(operation, error))?
    };
    if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
        return Err(denial(operation, error));
    }
    let mut payload = crate::server::replay_bundle_for(session_id_hex)
        .map_err(|error| denial(operation, error))?;
    payload["resource"] = json!(uri);
    Ok(text_content(uri, payload.to_string()))
}

/// Modern Tasks have no tasks/list method. Discovery stays in the existing
/// capability-checked session resource waist.
fn task_resource_bytes(uri: &str, serialized: &str) -> usize {
    // Resource text is JSON inside a JSON-RPC result. Count its escaping and
    // the resource wrapper, with room for the bounded outer request envelope.
    json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serialized}]})
        .to_string()
        .len()
        .saturating_add(128)
}

fn session_tasks(
    session_id_hex: &str,
    uri: &str,
    offset: usize,
) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/tasks";
    // This enters with_session so a shared world's current clock and grants
    // govern the read. Reading a task must not consume a semantic request ID.
    let budget = crate::server::task_session::read_budget(session_id_hex)
        .map_err(|error| denial(operation, error))?;
    let mut payload = crate::task_service::session_tasks(session_id_hex, offset);
    payload["ok"] = json!(payload.get("unavailable").is_none());
    payload["schema"] = json!("dfmcp.lab-tasks/1");
    payload["session_id"] = json!(session_id_hex);
    payload["resource"] = json!(uri);
    payload["max_active"] = json!(1);
    payload["max_retained"] = json!(crate::task_store::MAX_RETAINED_TASKS);
    payload["retention"] = json!("process_lifetime");
    payload["scope"] = json!("laboratory_process_only");
    let max_bytes = budget.max_bytes.min(
        u64::from(budget.max_output_tokens)
            .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
    );
    loop {
        let serialized = payload.to_string();
        if task_resource_bytes(uri, &serialized) as u64 <= max_bytes {
            return Ok(text_content(uri, serialized));
        }
        let Some(tasks) = payload.get_mut("tasks").and_then(Value::as_array_mut) else {
            return Err(McpError::invalid_params(
                "task diagnostics exceed the negotiated output budget",
            ));
        };
        if tasks.len() <= 1 {
            return Err(McpError::invalid_params(format!(
                "task discovery requires {} bytes, exceeding the negotiated {max_bytes}-byte output bound; no active handle or drain evidence was silently omitted",
                task_resource_bytes(uri, &serialized)
            )));
        }
        // Active work sorts first. Trimming only the last history handle
        // preserves the sole active monitor, with an explicit continuation.
        tasks.pop();
        let end = offset + tasks.len();
        payload["complete"] = json!(false);
        payload["next"] = json!(format!("df://session/{session_id_hex}/tasks-{end}"));
        payload["output_budget"] = json!({"max_bytes": max_bytes,
            "max_output_tokens": budget.max_output_tokens, "clipped": true,
            "reason": "terminal history continues in the next resource page"});
    }
}

fn session_task_detail(
    session_id_hex: &str,
    uri: &str,
    task_id: &str,
) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/task-detail";
    let budget = crate::server::task_session::read_budget(session_id_hex)
        .map_err(|error| denial(operation, error))?;
    let mut payload = crate::task_service::task_detail(session_id_hex, task_id)?;
    payload["ok"] = json!(true);
    payload["schema"] = json!("dfmcp.lab-task-detail/1");
    payload["session_id"] = json!(session_id_hex);
    payload["resource"] = json!(uri);
    let mut serialized = payload.to_string();
    let max_bytes = budget.max_bytes.min(
        u64::from(budget.max_output_tokens)
            .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
    );
    if task_resource_bytes(uri, &serialized) as u64 > max_bytes {
        let (source, evidence) = task_evidence_document(&payload);
        let progress = &payload["progress"];
        payload = json!({"ok": true, "schema": "dfmcp.lab-task-detail/1",
            "session_id": session_id_hex, "resource": uri, "task_id": task_id,
            "plan_digest": payload["plan_digest"], "scope": "laboratory_process_only",
            "task": {"taskId": task_id, "status": payload["task"]["status"]},
            "progress": {"stage": progress["stage"],
                "request": {"drain_progress": progress["request"]["drain_progress"]},
                "finalization": {"drain_progress": progress["finalization"]["drain_progress"],
                    "finalize_certificate": progress["finalization"]["finalize_certificate"]}},
            "service_error": payload["service_error"],
            "evidence": {"coverage": "summary_with_complete_evidence_retained", "source": source,
                "digest": dfmcp_core::Digest32::of_bytes(evidence.as_bytes()).to_hex(), "bytes": evidence.len(),
                "next": format!("df://session/{session_id_hex}/task-{task_id}~evidence-0")},
            "output_budget": {"max_bytes": max_bytes, "max_output_tokens": budget.max_output_tokens,
                "clipped": true, "omitted": "full proof continues in digest-checked evidence pages"}});
        serialized = payload.to_string();
        if task_resource_bytes(uri, &serialized) as u64 > max_bytes {
            return Err(McpError::invalid_params(
                "task detail summary exceeds the negotiated output budget; no proof was silently omitted",
            ));
        }
    }
    Ok(text_content(uri, serialized))
}

fn task_evidence_document(detail: &Value) -> (&'static str, String) {
    if !detail["final_evidence"].is_null() {
        ("final_evidence", detail["final_evidence"].to_string())
    } else {
        ("progress", detail["progress"].to_string())
    }
}

fn session_task_evidence(
    session_id_hex: &str,
    uri: &str,
    task_id: &str,
    offset: usize,
) -> McpResult<Vec<ResourceContent>> {
    let budget = crate::server::task_session::read_budget(session_id_hex)
        .map_err(|error| denial("df://session/task-evidence", error))?;
    let detail = crate::task_service::task_detail(session_id_hex, task_id)?;
    let (source, evidence) = task_evidence_document(&detail);
    if offset > evidence.len() || !evidence.is_char_boundary(offset) {
        return Err(McpError::invalid_params(
            "task evidence offset must be a valid UTF-8 boundary inside the retained document",
        ));
    }
    let digest = dfmcp_core::Digest32::of_bytes(evidence.as_bytes()).to_hex();
    let max_bytes = budget.max_bytes.min(
        u64::from(budget.max_output_tokens)
            .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
    );
    let mut end = offset.saturating_add(16 * 1024).min(evidence.len());
    loop {
        while !evidence.is_char_boundary(end) {
            end -= 1;
        }
        let payload = json!({"ok": true, "schema": "dfmcp.lab-task-evidence-page/1",
            "session_id": session_id_hex, "resource": uri, "task_id": task_id, "source": source,
            "encoding": "json_utf8", "digest": digest, "total_bytes": evidence.len(), "offset": offset,
            "part": &evidence[offset..end], "complete": end == evidence.len(),
            "next": (end < evidence.len()).then(|| format!("df://session/{session_id_hex}/task-{task_id}~evidence-{end}")),
            "reassembly": "concatenate part strings in offset order, require one unchanged digest, then parse the complete JSON document",
            "scope": "laboratory_process_only"});
        let serialized = payload.to_string();
        if task_resource_bytes(uri, &serialized) as u64 <= max_bytes {
            return Ok(text_content(uri, serialized));
        }
        if end <= offset + 4 {
            return Err(McpError::invalid_params(
                "task evidence page metadata exceeds the negotiated output budget",
            ));
        }
        end = offset + (end - offset) / 2;
    }
}

fn read_param_or_refuse(
    params: &HashMap<String, String>,
    read: impl FnOnce(&str) -> McpResult<Vec<ResourceContent>>,
) -> McpResult<Vec<ResourceContent>> {
    let raw = params.get("session_id").map_or("", String::as_str);
    if raw.is_empty() {
        return Err(template_read_refusal());
    }
    read(raw)
}

// ============================================================================
// Resource handlers
// ============================================================================

/// `df://doctor/{session_id}` — requires `doctor`.
pub struct DoctorBundleResource;

impl ResourceHandler for DoctorBundleResource {
    fn definition(&self) -> Resource {
        template_definition(
            DOCTOR_BUNDLE_TEMPLATE,
            "doctor-bundle",
            "Doctor report for the session: adapter health, telemetry inspector, active sessions, anchor",
        )
    }

    fn template(&self) -> Option<ResourceTemplate> {
        Some(template_of(
            DOCTOR_BUNDLE_TEMPLATE,
            "doctor-bundle",
            "Doctor report for the session: adapter health, telemetry inspector, active sessions, anchor",
        ))
    }

    fn read(&self, _ctx: &McpContext) -> McpResult<Vec<ResourceContent>> {
        Err(template_read_refusal())
    }

    fn final_resource_read_cache_hint_provenance(&self) -> FinalResourceReadCacheHintProvenance {
        FinalResourceReadCacheHintProvenance::Explicit
    }

    fn read_final_with_uri(
        &self,
        ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<CompleteResult<FinalReadResourceResult>> {
        uncached_resource_contents(self.read_with_uri(ctx, uri, params)?)
    }

    fn read_with_uri(
        &self,
        _ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<Vec<ResourceContent>> {
        read_param_or_refuse(params, |raw| doctor_bundle(raw, uri))
    }
}

/// `df://session/{session_id}/{view}` for `summary` (requires `observe`),
/// `capabilities` (self-description, a valid session suffices) and `handoff`
/// (requires `observe`).
pub struct SessionViewResource;

const SESSION_VIEW_DESCRIPTION: &str = "Session views: summary (bounded snapshot projection), \
     capabilities (negotiated grants and version record), handoff (resumable packet with anchor, \
     grants, pending plan, open actions, obligations and an ordered resume protocol), replay \
     (dfmcp.replay.bundle/1 of every recorded call for deterministic re-execution), anchor \
     (the session fortress's current canonical anchor), tasks and tasks-{offset} (bounded modern \
     MCP task handles, active first), task-{taskId} (original plan progress, cancellation drains \
     and final evidence; laboratory process only), entity-{id}~{generation}~{anchor_digest} \
     (one Query-authorized entity at the exact current complete anchor, discovered in entity query rows)";

impl ResourceHandler for SessionViewResource {
    fn definition(&self) -> Resource {
        template_definition(
            SESSION_VIEW_TEMPLATE,
            "session-view",
            SESSION_VIEW_DESCRIPTION,
        )
    }

    fn template(&self) -> Option<ResourceTemplate> {
        Some(template_of(
            SESSION_VIEW_TEMPLATE,
            "session-view",
            SESSION_VIEW_DESCRIPTION,
        ))
    }

    fn read(&self, _ctx: &McpContext) -> McpResult<Vec<ResourceContent>> {
        Err(template_read_refusal())
    }

    fn final_resource_read_cache_hint_provenance(&self) -> FinalResourceReadCacheHintProvenance {
        FinalResourceReadCacheHintProvenance::Explicit
    }

    fn read_final_with_uri(
        &self,
        ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<CompleteResult<FinalReadResourceResult>> {
        uncached_resource_contents(self.read_with_uri(ctx, uri, params)?)
    }

    fn read_with_uri(
        &self,
        _ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<Vec<ResourceContent>> {
        let view = params.get("view").map_or("", String::as_str);
        read_param_or_refuse(params, |raw| match view {
            "summary" => session_summary(raw, uri),
            "capabilities" => session_capabilities(raw, uri),
            "handoff" => session_handoff(raw, uri),
            "replay" => session_replay(raw, uri),
            "anchor" => session_anchor(raw, uri),
            entity if entity.starts_with("entity-") => session_entity(raw, uri, entity),
            "tasks" => session_tasks(raw, uri, 0),
            page if page.starts_with("tasks-") => {
                let offset = page[6..].parse::<usize>().map_err(|_| {
                    McpError::invalid_params("task page offset must be a nonnegative integer")
                })?;
                session_tasks(raw, uri, offset)
            }
            detail if detail.starts_with("task-") => {
                if let Some((task_id, offset)) = detail[5..].split_once("~evidence-") {
                    let offset = offset.parse::<usize>().map_err(|_| {
                        McpError::invalid_params(
                            "task evidence offset must be a nonnegative integer",
                        )
                    })?;
                    session_task_evidence(raw, uri, task_id, offset)
                } else {
                    session_task_detail(raw, uri, &detail[5..])
                }
            }
            _ => Err(McpError::invalid_params(format!(
                "invalid_params: unknown session view {view:?}; expected one of {SESSION_VIEWS:?}"
            ))),
        })
    }
}

/// Registration helper used by `server.rs::run_stdio` so the resource order
/// stays next to the tool registrations.
pub fn register_all(
    builder: fastmcp_rust::modern::ServerBuilder,
) -> fastmcp_rust::modern::ServerBuilder {
    builder
        .resource(SessionViewResource)
        .resource(DoctorBundleResource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{
        fortress_cancel, fortress_checkpoint, fortress_commit, fortress_doctor, fortress_explain,
        fortress_observe, fortress_open_session, fortress_plan, fortress_query, fortress_restore,
        fortress_wait,
    };
    use serde_json::Value;

    fn open_default() -> Value {
        serde_json::from_str(&fortress_open_session(
            Some(true),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ))
        .expect("open_session returns JSON")
    }

    fn open_with_caps(caps: &[(&str, &str)]) -> Value {
        let requested: Vec<(String, String)> = caps
            .iter()
            .map(|(capability, risk)| ((*capability).to_owned(), (*risk).to_owned()))
            .collect();
        serde_json::from_str(&fortress_open_session(
            Some(true),
            None,
            Some(requested),
            None,
            None,
            None,
            None,
            None,
            None,
        ))
        .expect("open_session returns JSON")
    }

    fn session_id_of(payload: &Value) -> String {
        payload["session_id"]
            .as_str()
            .expect("session_id")
            .to_owned()
    }

    fn error_code(response: &Value) -> &str {
        response["error"]["code"].as_str().unwrap_or_default()
    }

    const DIGEST_PLACEHOLDER: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const CHECKPOINT_PLACEHOLDER: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn entry_gate_denies_capabilities_the_session_was_not_granted() {
        let open = open_with_caps(&[("observe", "read_only")]);
        assert_eq!(open["ok"], true);
        let session_id = session_id_of(&open);

        let observe = fortress_observe(Some(session_id.clone()));
        assert_eq!(serde_json::from_str::<Value>(&observe).unwrap()["ok"], true);

        for (name, response) in [
            ("query", fortress_query(Some(session_id.clone()), None)),
            (
                "plan",
                fortress_plan(Some(session_id.clone()), None, Some(false)),
            ),
            (
                "commit",
                fortress_commit(Some(session_id.clone()), DIGEST_PLACEHOLDER.to_owned()),
            ),
            (
                "checkpoint",
                fortress_checkpoint(Some(session_id.clone()), None),
            ),
            (
                "restore",
                fortress_restore(Some(session_id.clone()), CHECKPOINT_PLACEHOLDER.to_owned()),
            ),
            ("explain", fortress_explain(Some(session_id.clone()), None)),
            ("doctor", fortress_doctor(Some(session_id.clone()))),
            ("cancel", fortress_cancel(Some(session_id.clone()), None)),
        ] {
            let parsed: Value = serde_json::from_str(&response).unwrap();
            assert_eq!(parsed["ok"], false, "{name} must be denied: {response}");
            assert_eq!(
                error_code(&parsed),
                "capability_denied",
                "{name} denial carries the stable code: {response}"
            );
        }

        // wait is an observe-class read: it passes the gate and hits the
        // no-committed-action conflict instead.
        let wait = fortress_wait(Some(session_id));
        let parsed: Value = serde_json::from_str(&wait).unwrap();
        assert_eq!(parsed["ok"], false);
        assert_eq!(error_code(&parsed), "conflict");
    }

    #[test]
    fn default_session_executes_the_full_pause_flow() {
        let open = open_default();
        assert_eq!(open["ok"], true);
        let session_id = session_id_of(&open);

        let plan = fortress_plan(Some(session_id.clone()), None, Some(false));
        let plan: Value = serde_json::from_str(&plan).unwrap();
        assert_eq!(plan["ok"], true, "plan: {plan}");
        let digest = plan["plan_digest"].as_str().unwrap().to_owned();

        let commit = fortress_commit(Some(session_id.clone()), digest);
        let commit: Value = serde_json::from_str(&commit).unwrap();
        assert_eq!(commit["ok"], true, "commit: {commit}");
        assert_eq!(commit["paused"], false);

        let wait = fortress_wait(Some(session_id.clone()));
        let wait: Value = serde_json::from_str(&wait).unwrap();
        assert_eq!(wait["ok"], true, "wait: {wait}");

        let checkpoint = fortress_checkpoint(Some(session_id.clone()), None);
        let checkpoint: Value = serde_json::from_str(&checkpoint).unwrap();
        assert_eq!(checkpoint["ok"], true, "checkpoint: {checkpoint}");
        let checkpoint_id = checkpoint["checkpoint_id"].as_str().unwrap().to_owned();

        let explain = fortress_explain(Some(session_id.clone()), None);
        let explain: Value = serde_json::from_str(&explain).unwrap();
        assert_eq!(explain["ok"], true, "explain: {explain}");

        let doctor = fortress_doctor(Some(session_id.clone()));
        let doctor: Value = serde_json::from_str(&doctor).unwrap();
        assert_eq!(doctor["ok"], true, "doctor: {doctor}");

        // Restore returns to the checkpointed paused state in a new epoch.
        let restore = fortress_restore(Some(session_id.clone()), checkpoint_id);
        let restore: Value = serde_json::from_str(&restore).unwrap();
        assert_eq!(restore["ok"], true, "restore: {restore}");
    }

    #[test]
    fn risk_ceiling_is_enforced_not_just_capability_kind() {
        // restore is a guarded operation: a read_only restore grant must not pass.
        let open = open_with_caps(&[("restore", "read_only")]);
        assert_eq!(open["ok"], true);
        let session_id = session_id_of(&open);
        let restore = fortress_restore(Some(session_id), CHECKPOINT_PLACEHOLDER.to_owned());
        let parsed: Value = serde_json::from_str(&restore).unwrap();
        assert_eq!(parsed["ok"], false);
        assert_eq!(error_code(&parsed), "capability_denied");
    }

    #[test]
    fn negotiation_record_carries_the_seven_surface_items_and_is_stable() {
        let first = open_default();
        let second = open_default();
        let negotiation = &first["negotiation"];
        for key in [
            "mcp_protocol_version",
            "dfmcp_protocol_version",
            "schema_catalog_digest",
            "bridge_protocol_version",
            "canonical_schema_version",
            "manifests",
            "compatibility_level",
        ] {
            assert!(
                negotiation.get(key).is_some(),
                "negotiation missing {key}: {negotiation}"
            );
        }
        assert_eq!(negotiation["mcp_protocol_version"], "2026-07-28");
        assert_eq!(negotiation["dfmcp_protocol_version"], "dfmcp/0");
        assert_eq!(
            negotiation["schema_catalog_digest"], second["negotiation"]["schema_catalog_digest"],
            "schema catalog digest must be deterministic across sessions"
        );
        assert_eq!(negotiation["canonical_schema_version"], "0.1.0");
    }

    #[test]
    fn session_resources_are_session_scoped_and_capability_gated() {
        let observe_only = open_with_caps(&[("observe", "read_only")]);
        let restricted_id = session_id_of(&observe_only);

        // Granted: summary works; capabilities is a self-description; doctor denied.
        let summary = session_summary(
            &restricted_id,
            &format!("df://session/{restricted_id}/summary"),
        )
        .expect("summary resource requires only observe");
        let parsed: Value =
            serde_json::from_str(summary[0].text.as_deref().unwrap_or_default()).unwrap();
        assert_eq!(parsed["projection"], "summary");
        assert_eq!(parsed["session_id"], restricted_id.as_str());

        let capabilities = session_capabilities(
            &restricted_id,
            &format!("df://session/{restricted_id}/capabilities"),
        )
        .expect("capabilities self-description needs no capability");
        let parsed: Value =
            serde_json::from_str(capabilities[0].text.as_deref().unwrap_or_default()).unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["negotiation"]["mcp_protocol_version"], "2026-07-28");
        let granted = parsed["granted_capabilities"].as_array().unwrap();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0]["capability"], "observe");

        let doctor = doctor_bundle(&restricted_id, &format!("df://doctor/{restricted_id}"));
        let error = doctor.expect_err("doctor resource must deny without the doctor capability");
        assert!(
            error.to_string().contains("capability_denied"),
            "denial names the stable code: {error}"
        );

        // Unknown but well-formed session id fails with a stable error.
        let unknown = "99999999999999999999999999999999";
        let missing = session_summary(unknown, &format!("df://session/{unknown}/summary"));
        let error = missing.expect_err("unknown session must fail");
        assert!(
            error.to_string().contains("no open session"),
            "unknown session names the problem: {error}"
        );

        // Missing template parameter refuses: template URIs carry no authority.
        let refusal = session_summary("", "df://session/{session_id}/summary");
        assert!(refusal.is_err(), "template-shaped read must refuse");
    }

    #[test]
    fn anchor_resource_is_gated_deterministic_and_rejects_malformed_ids() {
        let observer = session_id_of(&open_with_caps(&[("observe", "read_only")]));
        let blind = session_id_of(&open_with_caps(&[("query", "read_only")]));
        let uri = format!("df://session/{observer}/anchor");
        let first = session_anchor(&observer, &uri).expect("observe grants the anchor");
        let second = session_anchor(&observer, &uri).expect("repeatable");
        assert_eq!(
            first[0].text, second[0].text,
            "identical reads, identical bytes"
        );
        let parsed: Value =
            serde_json::from_str(first[0].text.as_deref().unwrap_or_default()).unwrap();
        assert!(parsed["anchor"]["state_hash"].is_string());
        let denied = session_anchor(&blind, &format!("df://session/{blind}/anchor"))
            .expect_err("URI knowledge without observe grants nothing");
        assert!(denied.to_string().contains("capability_denied"), "{denied}");
        for malformed in ["zz", "../../etc/passwd", "0", &"f".repeat(64)] {
            assert!(
                session_anchor(malformed, "df://session/x/anchor").is_err(),
                "malformed id {malformed:?} must be refused"
            );
        }
    }

    #[test]
    fn session_and_doctor_resources_recheck_revoked_grants() {
        let id = session_id_of(&open_with_caps(&[
            ("observe", "read_only"),
            ("doctor", "read_only"),
        ]));
        let summary_uri = format!("df://session/{id}/summary");
        let doctor_uri = format!("df://doctor/{id}");
        assert!(session_summary(&id, &summary_uri).is_ok());
        assert!(doctor_bundle(&id, &doctor_uri).is_ok());
        let session = lookup(&id, "test").expect("session");
        session.lock().expect("lock").grants.clear();
        assert!(
            session_summary(&id, &summary_uri)
                .expect_err("revoked observe grant")
                .to_string()
                .contains("capability_denied")
        );
        assert!(
            doctor_bundle(&id, &doctor_uri)
                .expect_err("revoked doctor grant")
                .to_string()
                .contains("capability_denied")
        );
    }

    #[test]
    fn resource_templates_advertise_session_scoped_families() {
        let views = SessionViewResource
            .template()
            .expect("session view template");
        assert_eq!(views.uri_template, "df://session/{session_id}/{view}");
        assert_eq!(
            SESSION_VIEWS,
            [
                "summary",
                "capabilities",
                "handoff",
                "replay",
                "anchor",
                "tasks"
            ]
        );
        let doctor = DoctorBundleResource.template().expect("doctor template");
        assert_eq!(doctor.uri_template, "df://doctor/{session_id}");
    }

    fn entity_session() -> (String, Value) {
        entity_session_at("1")
    }

    fn entity_session_at(fortress: &str) -> (String, Value) {
        let opened: Value = serde_json::from_str(&crate::server::open_session_in_scenario(
            Some(true),
            Some(fortress.to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("starter_fortress".to_owned()),
            None,
            None,
        ))
        .expect("open starter session");
        let session_id = session_id_of(&opened);
        let page: Value = serde_json::from_str(&fortress_query(
            Some(session_id.clone()),
            Some(r#"{"mode":"entities","limit":2}"#.to_owned()),
        ))
        .expect("entity query JSON");
        assert_eq!(page["ok"], true, "{page}");
        (session_id, page)
    }

    fn entity_read(session: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
        let view = uri.rsplit('/').next().expect("view");
        session_entity(session, uri, view)
    }

    #[test]
    fn entity_resource_discovers_and_reads_the_exact_canonical_record() {
        let (session_id, page) = entity_session();
        let row = &page["rows"][0];
        let uri = row["resource_uri"]
            .as_str()
            .expect("query must discover resource");
        let session = lookup(&session_id, "test").expect("session");
        let request_before = next_context(&mut session.lock().expect("lock"))
            .expect("context")
            .0;
        let first = entity_read(&session_id, uri).expect("authorized entity");
        let second = entity_read(&session_id, uri).expect("repeatable read");
        let request_after = next_context(&mut session.lock().expect("lock"))
            .expect("context")
            .0;
        assert_eq!(
            request_after,
            request_before + 1,
            "resource reads must not consume semantic call IDs"
        );
        assert_eq!(first[0].text, second[0].text);
        let payload: Value =
            serde_json::from_str(first[0].text.as_deref().expect("resource text")).expect("JSON");
        let mut canonical = row.clone();
        canonical
            .as_object_mut()
            .expect("row")
            .remove("resource_uri");
        assert_eq!(payload["entity"], canonical);
        assert_eq!(payload["anchor"], page["anchor"]);
        assert_eq!(payload["coverage"]["live_dwarf_fortress"], "unavailable");
        assert_eq!(payload["coverage"]["absence_proof_scope"], json!([]));
    }

    #[test]
    fn entity_resource_checks_scope_expiry_and_output_before_publishing() {
        let (session_id, page) = entity_session();
        let uri = page["rows"][0]["resource_uri"].as_str().expect("resource");
        let id = EntityId::new(
            page["rows"][0]["entity_id"]
                .as_str()
                .expect("id")
                .parse()
                .expect("numeric id"),
        );
        let session = lookup(&session_id, "test").expect("session");
        let original = session.lock().expect("lock").grants.clone();
        {
            let mut guard = session.lock().expect("lock");
            guard.grants.retain(|g| g.capability == Capability::Query);
            for grant in &mut guard.grants {
                grant.scope.entity_ids = [id].into();
            }
        }
        assert!(
            entity_read(&session_id, uri).is_ok(),
            "exact entity grant must work"
        );
        let other = page["rows"][1]["resource_uri"]
            .as_str()
            .expect("second resource");
        assert!(
            entity_read(&session_id, other)
                .expect_err("other entity denied")
                .to_string()
                .contains("capability_denied")
        );
        {
            let mut guard = session.lock().expect("lock");
            for grant in &mut guard.grants {
                grant.expires_at_tick = Some(dfmcp_core::GameTick(0));
            }
        }
        assert!(
            entity_read(&session_id, uri)
                .expect_err("expired grant denied")
                .to_string()
                .contains("capability_denied")
        );
        {
            let mut guard = session.lock().expect("lock");
            guard.grants = original;
            guard.budget.max_bytes = 64;
        }
        assert!(
            entity_read(&session_id, uri)
                .expect_err("small output budget denied")
                .to_string()
                .contains("budget_exceeded")
        );
        session.lock().expect("lock").budget.max_bytes = 4 * 1024 * 1024;
        assert!(
            entity_read(&session_id, uri).is_ok(),
            "same reference works when output fits"
        );
    }

    #[test]
    fn entity_resource_refuses_wrong_generation_fortress_epoch_and_malformed_handles() {
        let (session_id, page) = entity_session();
        let uri = page["rows"][0]["resource_uri"].as_str().expect("resource");
        let fields: Vec<&str> = uri.rsplit('/').next().expect("view").split('~').collect();
        let wrong_generation = format!(
            "df://session/{session_id}/{}~4294967295~{}",
            fields[0], fields[2]
        );
        assert!(
            entity_read(&session_id, &wrong_generation)
                .expect_err("generation denied")
                .to_string()
                .contains("stale_anchor")
        );
        let (same_fortress, _) = entity_session();
        assert!(
            entity_read(&same_fortress, &uri.replace(&session_id, &same_fortress)).is_ok(),
            "a separately Query-authorized session can read its own identical fortress state"
        );
        let (other_session, other_page) = entity_session_at("2");
        assert_ne!(
            page["anchor"]["fortress_id"], other_page["anchor"]["fortress_id"],
            "cross-fortress fixture must actually use different fortress identities"
        );
        let cross = uri.replace(&session_id, &other_session);
        assert!(
            entity_read(&other_session, &cross)
                .expect_err("fortress-bound anchor denied")
                .to_string()
                .contains("stale_anchor")
        );
        for view in [
            "entity-0~1~bad",
            "entity-01~1~bad",
            "entity-1~1",
            "entity-1~1~bad~extra",
            &format!("entity-{}", "x".repeat(105)),
        ] {
            let bad = format!("df://session/{session_id}/{view}");
            assert!(entity_read(&session_id, &bad).is_err(), "{view}");
        }
        let checkpoint: Value =
            serde_json::from_str(&fortress_checkpoint(Some(session_id.clone()), None))
                .expect("checkpoint");
        let restored: Value = serde_json::from_str(&fortress_restore(
            Some(session_id.clone()),
            checkpoint["checkpoint_id"]
                .as_str()
                .expect("checkpoint ID")
                .to_owned(),
        ))
        .expect("restore");
        assert_eq!(restored["ok"], true, "{restored}");
        assert!(
            entity_read(&session_id, uri)
                .expect_err("old epoch denied even for same graph")
                .to_string()
                .contains("stale_anchor")
        );
        let fresh: Value = serde_json::from_str(&fortress_query(
            Some(session_id.clone()),
            Some("entities".to_owned()),
        ))
        .expect("fresh query");
        assert!(
            entity_read(
                &session_id,
                fresh["rows"][0]["resource_uri"]
                    .as_str()
                    .expect("fresh URI")
            )
            .is_ok()
        );
    }
}
