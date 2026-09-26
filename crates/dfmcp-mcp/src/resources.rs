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
use fastmcp_rust::ResourceTemplate;
use fastmcp_rust::prelude::{
    McpContext, McpError, McpResult, Resource, ResourceContent, ResourceHandler,
};
use serde_json::json;

use crate::server::{
    active_session_count, anchor_json, authorize_entry, lookup_session, next_context,
    parse_session_id_arg, snapshot_json,
};
use dfmcp_core::{Capability, RiskTier};

const SESSION_SUMMARY_TEMPLATE: &str = "df://session/{session_id}/summary";
const SESSION_CAPABILITIES_TEMPLATE: &str = "df://session/{session_id}/capabilities";
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

fn lookup(raw: &str, operation: &str) -> std::result::Result<
    Arc<std::sync::Mutex<crate::server::LabSession>>,
    McpError,
> {
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

// ============================================================================
// Pure read implementations (also exercised directly by tests)
// ============================================================================

/// `df://session/{session_id}/summary` — bounded snapshot projection.
/// Requires the session's negotiated `observe` capability.
pub(crate) fn session_summary(
    session_id_hex: &str,
    uri: &str,
) -> McpResult<Vec<ResourceContent>> {
    let operation = "df://session/summary";
    let session = lookup(session_id_hex, operation)?;
    let mut guard = session.lock().map_err(|_| poisoned(operation))?;
    let (_, ctx) = next_context(&mut guard).map_err(|error| denial(operation, error))?;
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
    let mut guard = session.lock().map_err(|_| poisoned(operation))?;
    let (_, ctx) = next_context(&mut guard).map_err(|error| denial(operation, error))?;
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
            "current_anchor": health.current_anchor.as_ref().map(|anchor| anchor_json(anchor)),
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

/// `df://session/{session_id}/summary` — requires `observe`.
pub struct SessionSummaryResource;

impl ResourceHandler for SessionSummaryResource {
    fn definition(&self) -> Resource {
        template_definition(
            SESSION_SUMMARY_TEMPLATE,
            "session-summary",
            "Bounded snapshot projection at the session's laboratory anchor",
        )
    }

    fn template(&self) -> Option<ResourceTemplate> {
        Some(template_of(
            SESSION_SUMMARY_TEMPLATE,
            "session-summary",
            "Bounded snapshot projection at the session's laboratory anchor",
        ))
    }

    fn read(&self, _ctx: &McpContext) -> McpResult<Vec<ResourceContent>> {
        Err(template_read_refusal())
    }

    fn read_with_uri(
        &self,
        _ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<Vec<ResourceContent>> {
        read_param_or_refuse(params, |raw| session_summary(raw, uri))
    }
}

/// `df://session/{session_id}/capabilities` — self-description, no capability
/// beyond a valid session.
pub struct SessionCapabilitiesResource;

impl ResourceHandler for SessionCapabilitiesResource {
    fn definition(&self) -> Resource {
        template_definition(
            SESSION_CAPABILITIES_TEMPLATE,
            "session-capabilities",
            "The session's negotiated capability grants and version negotiation record",
        )
    }

    fn template(&self) -> Option<ResourceTemplate> {
        Some(template_of(
            SESSION_CAPABILITIES_TEMPLATE,
            "session-capabilities",
            "The session's negotiated capability grants and version negotiation record",
        ))
    }

    fn read(&self, _ctx: &McpContext) -> McpResult<Vec<ResourceContent>> {
        Err(template_read_refusal())
    }

    fn read_with_uri(
        &self,
        _ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<Vec<ResourceContent>> {
        read_param_or_refuse(params, |raw| session_capabilities(raw, uri))
    }
}

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

    fn read_with_uri(
        &self,
        _ctx: &McpContext,
        uri: &str,
        params: &HashMap<String, String>,
    ) -> McpResult<Vec<ResourceContent>> {
        read_param_or_refuse(params, |raw| doctor_bundle(raw, uri))
    }
}

/// Registration helper used by `server.rs::run_stdio` so the resource order
/// stays next to the tool registrations.
pub fn register_all(
    builder: fastmcp_rust::modern::ServerBuilder,
) -> fastmcp_rust::modern::ServerBuilder {
    builder
        .resource(SessionSummaryResource)
        .resource(SessionCapabilitiesResource)
        .resource(DoctorBundleResource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{
        fortress_cancel, fortress_checkpoint, fortress_commit, fortress_doctor, fortress_explain,
        fortress_open_session, fortress_observe, fortress_plan, fortress_query, fortress_restore,
        fortress_wait,
    };
    use serde_json::Value;

    fn open_default() -> Value {
        serde_json::from_str(&fortress_open_session(
            Some(true), None, None, None, None, None, None, None, None,
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
        payload["session_id"].as_str().expect("session_id").to_owned()
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
            negotiation["schema_catalog_digest"],
            second["negotiation"]["schema_catalog_digest"],
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
    fn resource_templates_advertise_session_scoped_families() {
        let summary = SessionSummaryResource.template().expect("summary template");
        assert_eq!(summary.uri_template, "df://session/{session_id}/summary");
        let capabilities = SessionCapabilitiesResource
            .template()
            .expect("capabilities template");
        assert_eq!(
            capabilities.uri_template,
            "df://session/{session_id}/capabilities"
        );
        let doctor = DoctorBundleResource.template().expect("doctor template");
        assert_eq!(doctor.uri_template, "df://doctor/{session_id}");
    }
}
