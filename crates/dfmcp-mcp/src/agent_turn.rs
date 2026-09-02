//! Canonical agent-facing orientation envelope.
//!
//! The MCP tool-specific payload remains additive and transport-friendly, but
//! every result converges on one `agent_turn` spine so an agent never has to
//! reconstruct continuity, active work, uncertainty, or the safe next protocol
//! step from unrelated response shapes.
//!
//! This module is presentation-only. It accepts already-authorized semantic
//! facts and emits deterministic JSON. It cannot grant capabilities, satisfy a
//! precondition, dispatch an effect, or elevate a derived recommendation into
//! authority. See `docs/AGENT_OPERATING_MODEL.md` and
//! `architecture/agent_turn_contract.json`.

pub use dfmcp_core::{AgentPhase, ContinuityStatus, ObservationProfile, RecoveryClass};
use serde_json::{Value, json};

pub const AGENT_TURN_SCHEMA: &str = "dfmcp.agent_turn/1";

fn attach_live_admission_provenance(briefing: &mut Value) {
    let Some(provenance) = crate::admission::current_admission_provenance() else {
        return;
    };
    let Some(object) = briefing.as_object_mut() else {
        return;
    };
    object.insert(
        "admission".to_owned(),
        json!({
            "state": "admitted",
            "mode": "authenticated_live_read_only",
            "bridge_protocol": provenance.bridge_protocol(),
            "ticket_id": provenance.ticket_id(),
            "compatibility_entry_id": provenance.compatibility_entry_id(),
            "compatibility_registry_digest": provenance.compatibility_registry_digest(),
            "compatibility_decision_digest": provenance.compatibility_decision_digest(),
            "compatibility_floor_digest": provenance.compatibility_floor_digest(),
            "compatibility_floor_file_sha256": provenance.compatibility_floor_file_sha256(),
            "compatibility_floor_sequence": provenance.compatibility_floor_sequence(),
            "server_receipt_digest": provenance.server_receipt_digest(),
            "launch_digest": provenance.launch_digest(),
            "server_binary_sha256": provenance.server_binary_sha256(),
            "mutation_capabilities": [],
        }),
    );
}

/// Builder for the common agent-facing turn packet.
///
/// All collections are caller-supplied in canonical order. The builder does
/// not sort by opaque JSON values because doing so could silently redefine a
/// domain tie-break policy. Producers must order attention, affordances, and
/// recommendations before crossing this presentation seam.
#[derive(Clone, Debug)]
pub struct AgentTurnBuilder {
    operation: String,
    phase: AgentPhase,
    session_id: Option<String>,
    turn_id: Option<String>,
    request_id: Option<String>,
    anchor: Option<Value>,
    continuity_status: ContinuityStatus,
    continuity_basis: Option<Value>,
    continuity_gap: Option<Value>,
    reset_reason: Option<String>,
    profile: ObservationProfile,
    briefing: Value,
    changes: Vec<Value>,
    attention: Vec<Value>,
    active_work: Value,
    affordances: Vec<Value>,
    recommendations: Vec<Value>,
    uncertainty: Vec<Value>,
    coverage: Value,
    budget: Value,
    references: Vec<Value>,
}

impl AgentTurnBuilder {
    #[must_use]
    pub fn new(operation: impl Into<String>, phase: AgentPhase) -> Self {
        Self {
            operation: operation.into(),
            phase,
            session_id: None,
            turn_id: None,
            request_id: None,
            anchor: None,
            continuity_status: ContinuityStatus::Bootstrap,
            continuity_basis: None,
            continuity_gap: None,
            reset_reason: None,
            profile: ObservationProfile::Briefing,
            briefing: json!({}),
            changes: Vec::new(),
            attention: Vec::new(),
            active_work: empty_active_work(),
            affordances: Vec::new(),
            recommendations: Vec::new(),
            uncertainty: Vec::new(),
            coverage: empty_coverage(),
            budget: empty_budget(),
            references: Vec::new(),
        }
    }

    #[must_use]
    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Set the presentation-turn identity.
    ///
    /// This is distinct from the authority-bearing semantic `request_id`. A
    /// presentation facade may know its own turn sequence while the underlying
    /// semantic handler has not yet projected its request identity. It must
    /// leave `request_id` absent rather than aliasing the two.
    #[must_use]
    pub fn turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    #[must_use]
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    #[must_use]
    pub fn anchor(mut self, anchor: Value) -> Self {
        self.anchor = Some(anchor);
        self
    }

    #[must_use]
    pub fn continuity(
        mut self,
        status: ContinuityStatus,
        basis: Option<Value>,
        gap: Option<Value>,
        reset_reason: Option<String>,
    ) -> Self {
        self.continuity_status = status;
        self.continuity_basis = basis;
        self.continuity_gap = gap;
        self.reset_reason = reset_reason;
        self
    }

    #[must_use]
    pub fn profile(mut self, profile: ObservationProfile) -> Self {
        self.profile = profile;
        self
    }

    #[must_use]
    pub fn briefing(mut self, briefing: Value) -> Self {
        self.briefing = briefing;
        self
    }

    #[must_use]
    pub fn changes(mut self, changes: Vec<Value>) -> Self {
        self.changes = changes;
        self
    }

    #[must_use]
    pub fn attention(mut self, attention: Vec<Value>) -> Self {
        self.attention = attention;
        self
    }

    #[must_use]
    pub fn active_work(mut self, active_work: Value) -> Self {
        self.active_work = active_work;
        self
    }

    #[must_use]
    pub fn affordances(mut self, affordances: Vec<Value>) -> Self {
        self.affordances = affordances;
        self
    }

    #[must_use]
    pub fn recommendations(mut self, recommendations: Vec<Value>) -> Self {
        self.recommendations = recommendations;
        self
    }

    #[must_use]
    pub fn uncertainty(mut self, uncertainty: Vec<Value>) -> Self {
        self.uncertainty = uncertainty;
        self
    }

    #[must_use]
    pub fn coverage(mut self, coverage: Value) -> Self {
        self.coverage = coverage;
        self
    }

    #[must_use]
    pub fn budget(mut self, budget: Value) -> Self {
        self.budget = budget;
        self
    }

    #[must_use]
    pub fn references(mut self, references: Vec<Value>) -> Self {
        self.references = references;
        self
    }

    #[must_use]
    pub fn build(mut self) -> Value {
        attach_live_admission_provenance(&mut self.briefing);
        json!({
            "schema": AGENT_TURN_SCHEMA,
            "operation": self.operation,
            "phase": self.phase.as_str(),
            "session_id": self.session_id,
            "turn_id": self.turn_id,
            "request_id": self.request_id,
            "anchor": self.anchor,
            "continuity": {
                "status": self.continuity_status.as_str(),
                "basis": self.continuity_basis,
                "gap": self.continuity_gap,
                "reset_reason": self.reset_reason,
            },
            "profile": self.profile.as_str(),
            "briefing": self.briefing,
            "changes": self.changes,
            "attention": self.attention,
            "active_work": self.active_work,
            "affordances": self.affordances,
            "recommendations": self.recommendations,
            "uncertainty": self.uncertainty,
            "coverage": self.coverage,
            "budget": self.budget,
            "references": self.references,
        })
    }

    /// Add the packet to a tool-specific JSON object and serialize it.
    ///
    /// A non-object payload is rejected into an explicit internal-error
    /// envelope rather than silently changing its meaning.
    #[must_use]
    pub fn attach(self, mut payload: Value) -> String {
        let turn = self.build();
        match payload.as_object_mut() {
            Some(object) => {
                object.insert("agent_turn".to_owned(), turn);
                payload.to_string()
            }
            None => json!({
                "ok": false,
                "error": {
                    "operation": "agent_turn.attach",
                    "code": "internal_invariant_violation",
                    "message": "tool payload must be a JSON object before the agent turn packet is attached",
                    "retryable": false,
                    "details": [],
                },
                "agent_turn": turn,
            })
            .to_string(),
        }
    }
}

#[must_use]
pub fn empty_active_work() -> Value {
    json!({
        "pending_plans": [],
        "actions": [],
        "obligations": [],
        "cancellation_drains": [],
        "indeterminate_effects": [],
        "publications": [],
        "confirmations": [],
    })
}

#[must_use]
pub fn empty_coverage() -> Value {
    json!({
        "status": "explicitly_empty",
        "complete_domains": [],
        "partial_domains": [],
        "omitted_domains": [],
        "continuation": null,
    })
}

#[must_use]
pub fn empty_budget() -> Value {
    json!({
        "requested": null,
        "admitted": null,
        "consumed": {},
        "remaining": null,
        "soft_stop_reason": null,
        "hard_stop_reason": null,
    })
}

#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn recommendation(
    recommendation_id: impl Into<String>,
    tool: impl Into<String>,
    reason: impl Into<String>,
    expected_utility: impl Into<String>,
    expected_information_value: impl Into<String>,
    risk: impl Into<String>,
    reversibility: impl Into<String>,
    requires_confirmation: bool,
    arguments: Value,
) -> Value {
    json!({
        "recommendation_id": recommendation_id.into(),
        "tool": tool.into(),
        "reason": reason.into(),
        "expected_utility": expected_utility.into(),
        "expected_information_value": expected_information_value.into(),
        "risk": risk.into(),
        "reversibility": reversibility.into(),
        "estimated_cost": {
            "output_tokens": null,
            "bridge_bytes": null,
            "wall_millis": null,
            "game_ticks": null,
        },
        "prerequisites": [],
        "invalidating_conditions": [],
        "confidence": {
            "epistemic_state": "certified_derived",
            "value": null,
            "evidence": [],
        },
        "requires_confirmation": requires_confirmation,
        "arguments": arguments,
    })
}

#[must_use]
pub fn uncertainty(
    uncertainty_id: impl Into<String>,
    epistemic_state: impl Into<String>,
    statement: impl Into<String>,
    consequence: impl Into<String>,
    resolution_tool: Option<&str>,
    resolution_arguments: Value,
) -> Value {
    json!({
        "uncertainty_id": uncertainty_id.into(),
        "epistemic_state": epistemic_state.into(),
        "statement": statement.into(),
        "consequence": consequence.into(),
        "resolution": resolution_tool.map(|tool| json!({
            "tool": tool,
            "arguments": resolution_arguments,
        })),
        "evidence": [],
    })
}

#[must_use]
pub fn recovery_guidance(
    class: RecoveryClass,
    tool: Option<&str>,
    reason: impl Into<String>,
    arguments: Value,
) -> Value {
    json!({
        "class": class.as_str(),
        "minimum_safe_next_step": tool.map(|name| json!({
            "tool": name,
            "arguments": arguments,
        })),
        "reason": reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_turn_has_every_orientation_section() {
        let turn = AgentTurnBuilder::new("fortress.observe", AgentPhase::Orient).build();
        for field in [
            "schema",
            "operation",
            "phase",
            "session_id",
            "turn_id",
            "request_id",
            "anchor",
            "continuity",
            "profile",
            "briefing",
            "changes",
            "attention",
            "active_work",
            "affordances",
            "recommendations",
            "uncertainty",
            "coverage",
            "budget",
            "references",
        ] {
            assert!(turn.get(field).is_some(), "missing required field {field}");
        }
    }

    #[test]
    fn presentation_turn_and_semantic_request_are_not_aliased() {
        let turn = AgentTurnBuilder::new("fortress.observe", AgentPhase::Orient)
            .turn_id("presentation-turn-7")
            .build();
        assert_eq!(turn["turn_id"], "presentation-turn-7");
        assert!(turn["request_id"].is_null());
    }

    #[test]
    fn turn_serialization_is_byte_stable_for_identical_input() {
        let build = || {
            AgentTurnBuilder::new("fortress.open_session", AgentPhase::Bootstrap)
                .session_id("00000000000000000000000000000001")
                .turn_id("presentation-turn-1")
                .profile(ObservationProfile::Briefing)
                .recommendations(vec![recommendation(
                    "inspect-summary",
                    "fortress.query",
                    "establish a bounded semantic baseline",
                    "medium",
                    "high",
                    "read_only",
                    "not_applicable",
                    false,
                    json!({"mode": "summary"}),
                )])
                .build()
                .to_string()
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn attach_preserves_tool_specific_fields() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let encoded = AgentTurnBuilder::new("fortress.query", AgentPhase::Inspect)
            .attach(json!({"ok": true, "matched": 7}));
        let value: Value = serde_json::from_str(&encoded)?;
        assert_eq!(value["matched"], 7);
        assert_eq!(value["agent_turn"]["operation"], "fortress.query");
        Ok(())
    }

    #[test]
    fn non_object_payload_fails_closed_with_the_original_turn() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let encoded = AgentTurnBuilder::new("fortress.observe", AgentPhase::Orient)
            .attach(json!(["not", "an", "object"]));
        let value: Value = serde_json::from_str(&encoded)?;
        assert_eq!(value["ok"], false);
        assert_eq!(
            value["error"]["code"],
            "internal_invariant_violation"
        );
        assert_eq!(value["agent_turn"]["operation"], "fortress.observe");
        Ok(())
    }

    #[test]
    fn recovery_guidance_never_hides_the_retry_class() {
        let guidance = recovery_guidance(
            RecoveryClass::ReconciliationRequired,
            Some("fortress.wait"),
            "dispatch outcome is not yet known",
            json!({"scope": "last_action"}),
        );
        assert_eq!(guidance["class"], "reconciliation_required");
        assert_eq!(
            guidance["minimum_safe_next_step"]["tool"],
            "fortress.wait"
        );
    }
}
