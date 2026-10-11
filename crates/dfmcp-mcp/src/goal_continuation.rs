//! Replan the complete retained production request under a new reviewed seal.
//!
//! This codec carries intent and lineage, never grants, native receipts or
//! dispatch authority. The owner must prove all earlier lineage work quiet
//! before using it. Recovery can reproduce a seal without consulting a mutable
//! parent book because every continuation retains the complete original request.

use crate::lab_world::{MAX_ACTIONS_JSON_BYTES, ProductionPlanner, ProductionRequest, parse_steps};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, IntentId, OperationContext, Result, RiskTier,
};
use dfmcp_intent::{Constraint, Intent, PreparedPlan, StaticPlanner};
use dfmcp_world::{PredicateEvidence, PredicateTruth, WorldSnapshot};
use serde::Deserialize;
use serde_json::{Value, json};

const LEGACY_SOURCE_SCHEMA: &str = "dfmcp.production-continuation/1";
const SOURCE_SCHEMA: &str = "dfmcp.production-continuation/2";

/// Archive parsing selects the historical algorithm; new planning selects one
/// fixed generation without editing the original retained quota/setup request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceVersion {
    Legacy,
    ConsumptionAwareV1,
}

impl SourceVersion {
    fn schema(self) -> &'static str {
        match self {
            Self::Legacy => LEGACY_SOURCE_SCHEMA,
            Self::ConsumptionAwareV1 => SOURCE_SCHEMA,
        }
    }

    fn domain(self) -> &'static [u8] {
        match self {
            Self::Legacy => b"dfmcp-production-continuation-source/1\0",
            Self::ConsumptionAwareV1 => b"dfmcp-production-continuation-source/2\0",
        }
    }
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(raw: &str) -> Result<()> {
    if raw.len() > MAX_ACTIONS_JSON_BYTES || raw.contains('\0') {
        return Err(invalid(
            "goal continuation exceeds its bounded UTF-8 request contract",
        ));
    }
    Ok(())
}

fn digest(raw: &str) -> Result<Digest32> {
    Digest32::from_hex(raw)
        .filter(|digest| *digest != Digest32::ZERO)
        .ok_or_else(|| invalid("goal continuation requires a nonzero 64-character plan digest"))
}

pub(crate) fn is_request(raw: &str) -> bool {
    raw.len() <= MAX_ACTIONS_JSON_BYTES
        && serde_json::from_str::<Value>(raw)
            .is_ok_and(|value| value["template"] == "continue_goal")
}

/// The public form names a retained goal only. It cannot supply replacement
/// quotas, parent/root assertions, authority, completion or physical-work flags.
pub(crate) fn parse_request(raw: &str) -> Result<Digest32> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        template: String,
        plan_digest: String,
    }
    bounded(raw)?;
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid continue_goal request: {error}")))?;
    if request.template != "continue_goal" {
        return Err(invalid("goal continuation requires template=continue_goal"));
    }
    digest(&request.plan_digest)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProductionContinuation {
    pub(crate) parent: Digest32,
    pub(crate) root: Digest32,
    request: ProductionRequest,
    version: SourceVersion,
}

impl ProductionContinuation {
    pub(crate) fn new(
        parent: Digest32,
        root: Digest32,
        request: ProductionRequest,
    ) -> Result<Self> {
        Self::from_parts(parent, root, request, SourceVersion::ConsumptionAwareV1)
    }

    fn from_parts(
        parent: Digest32,
        root: Digest32,
        request: ProductionRequest,
        version: SourceVersion,
    ) -> Result<Self> {
        if parent == Digest32::ZERO || root == Digest32::ZERO {
            return Err(invalid(
                "goal continuation lineage cannot contain a zero digest",
            ));
        }
        let source = Self {
            parent,
            root,
            request,
            version,
        };
        bounded(&source.canonical_json())?;
        Ok(source)
    }

    pub(crate) fn parse(raw: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Source {
            schema: String,
            parent_plan_digest: String,
            root_plan_digest: String,
            production: Value,
        }
        bounded(raw)?;
        let source: Source = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid retained goal continuation: {error}")))?;
        let version = match source.schema.as_str() {
            LEGACY_SOURCE_SCHEMA => SourceVersion::Legacy,
            SOURCE_SCHEMA => SourceVersion::ConsumptionAwareV1,
            _ => return Err(invalid("unknown retained goal continuation schema")),
        };
        let request = ProductionRequest::parse(&source.production.to_string())?;
        if version == SourceVersion::Legacy && request.planner().is_some() {
            return Err(invalid(
                "legacy continuation source cannot contain a later production compiler generation",
            ));
        }
        Self::from_parts(
            digest(&source.parent_plan_digest)?,
            digest(&source.root_plan_digest)?,
            request,
            version,
        )
    }

    /// New planning always uses the current fixed generation. The original
    /// request and root remain unchanged, including for a legacy parent.
    pub(crate) fn next(&self, parent: Digest32) -> Result<Self> {
        Self::new(parent, self.root, self.request.clone())
    }

    pub(crate) fn canonical_json(&self) -> String {
        // ProductionRequest generates canonical, valid JSON. Embed its exact
        // bytes directly so no fallback can replace an original request.
        format!(
            "{{\"schema\":\"{}\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"production\":{}}}",
            self.version.schema(),
            self.parent.to_hex(),
            self.root.to_hex(),
            self.request.canonical_json(),
        )
    }

    pub(crate) fn source_digest(&self) -> Digest32 {
        let mut bytes = self.version.domain().to_vec();
        bytes.extend_from_slice(self.canonical_json().as_bytes());
        Digest32::of_bytes(&bytes)
    }

    pub(crate) fn lineage_json(&self) -> Value {
        let mut lineage = json!({
            "parent_plan_digest": self.parent.to_hex(),
            "root_plan_digest": self.root.to_hex(),
            "continuation_source_digest": self.source_digest().to_hex(),
            "requires_explicit_commit": true,
            "replacement_work_dispatched": false,
        });
        if self.version == SourceVersion::ConsumptionAwareV1 {
            lineage["source_schema"] = json!(SOURCE_SCHEMA);
            lineage["planner"] = json!("consumption_aware_v1");
        }
        lineage
    }

    /// Current authorization validates retained lineage independently of the
    /// already sealed compiler generation. Replaying a known legacy candidate
    /// must not silently upgrade its source or invalidate its exact retry.
    pub(crate) fn same_original_request(&self, other: &Self) -> bool {
        self.parent == other.parent && self.root == other.root && self.request == other.request
    }

    pub(crate) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Value)> {
        // PreparedPlan hashes its summary. Bind the complete source, including
        // unused setup permissions, as well as parent/root identity into that
        // existing sealed field without changing the workspace plan format.
        let summary = format!(
            "{summary} [production continuation {}]",
            self.source_digest().to_hex()
        );
        if summary.len() > StaticPlanner::default().policy.max_string_bytes
            || summary.contains('\0')
        {
            return Err(invalid(
                "continuation summary plus its source seal exceeds the planner bound; supply a shorter summary",
            ));
        }
        let compiled = match self.version {
            SourceVersion::Legacy => self.request.compile(snapshot)?,
            SourceVersion::ConsumptionAwareV1 => self
                .request
                .clone()
                .with_planner(ProductionPlanner::ConsumptionAwareV1)
                .compile(snapshot)?,
        };
        let requested_actions = parse_steps(&compiled.actions)?;
        let max_risk = requested_actions
            .iter()
            .map(|requested| requested.action.risk())
            .max()
            .map_or(RiskTier::ReadOnly, |risk| risk);
        let mut intent = Intent {
            id,
            anchor: snapshot.anchor(),
            summary,
            terminal_condition: compiled.terminal.clone(),
            constraints: vec![Constraint::MaxRisk(max_risk)],
            requested_actions,
        };
        compiled.apply_capacity_horizon(&mut intent)?;
        let mut analysis = compiled.analysis;
        analysis["continuation"] = self.lineage_json();
        Ok((intent, analysis))
    }
}

/// Authorize and evaluate the retained original terminal at the exact current
/// snapshot. `work_quiescent` is supplied only by the owner's original-effect
/// inspection, never from MCP JSON or a terminal receipt alone.
pub(crate) fn validate_goal_evidence(
    original: &PreparedPlan,
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    work_quiescent: bool,
) -> Result<()> {
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor()
        || original.anchor.fortress_id != snapshot.fortress_id
        || original.anchor.cursor.epoch != snapshot.anchor().cursor.epoch
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "continuation evidence belongs to a different current anchor or fortress",
        ));
    }
    match PredicateEvidence::laboratory(snapshot)?.evaluate(&original.terminal_condition)? {
        PredicateTruth::True => {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "the original goal currently holds; no continuation work is required",
            ));
        }
        PredicateTruth::Unknown => {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "the original goal is unknown; reconcile current evidence before proposing more work",
            ));
        }
        PredicateTruth::False => {}
    }
    if !work_quiescent {
        return Err(DfmcpError::new(
            ErrorCode::PreconditionsFailed,
            "the original goal still has unresolved physical or deferred work; wait for it or explicitly drain it before continuing",
        ));
    }
    Ok(())
}

pub(crate) fn request_json(session_id: &str, parent: Digest32) -> Value {
    json!({
        "tool": "fortress.plan",
        "arguments": {
            "session_id": session_id,
            "blueprint": json!({"template": "continue_goal", "plan_digest": parent.to_hex()}).to_string(),
        },
        "note": "compile the complete retained original production goal at current evidence, review the new seal, then explicitly commit it",
    })
}

#[cfg(test)]
#[path = "goal_continuation_tests.rs"]
mod tests;
