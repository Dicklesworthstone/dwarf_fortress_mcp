//! A reviewed new pursuit of an immutable original resource stock request.
//!
//! Flat lineage and the exact fixed compiler are sealed together. This source
//! neither changes earlier task identities nor supplies authority or quiescence.

use super::resource_acquisition::{MAX_REQUEST_BYTES, PLANNER, ResourceAcquisitionRequest};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, IntentId, Result};
use dfmcp_intent::Intent;
use dfmcp_world::{PredicateTruth, WorldSnapshot};
use serde::Deserialize;
use serde_json::{Value, json};

const SOURCE_SCHEMA: &str = "dfmcp.resource-acquisition-continuation/1";
const SOURCE_DOMAIN: &[u8] = b"dfmcp-resource-acquisition-continuation-source/1\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResourceAcquisitionContinuation {
    pub(super) parent: Digest32,
    pub(super) root: Digest32,
    request: ResourceAcquisitionRequest,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(raw: &str) -> Result<()> {
    if raw.len() > MAX_REQUEST_BYTES || raw.contains('\0') {
        return Err(invalid(
            "resource acquisition continuation exceeds its 16-KiB non-NUL source bound",
        ));
    }
    Ok(())
}

fn digest(raw: &str) -> Result<Digest32> {
    Digest32::from_hex(raw)
        .filter(|value| *value != Digest32::ZERO)
        .ok_or_else(|| {
            invalid("resource acquisition continuation requires a nonzero 64-character plan digest")
        })
}

impl ResourceAcquisitionContinuation {
    pub(super) fn new(
        parent: Digest32,
        root: Digest32,
        request: ResourceAcquisitionRequest,
    ) -> Result<Self> {
        if parent == Digest32::ZERO || root == Digest32::ZERO {
            return Err(invalid(
                "resource acquisition continuation lineage cannot contain a zero digest",
            ));
        }
        let source = Self {
            parent,
            root,
            request,
        };
        bounded(&source.canonical_json())?;
        Ok(source)
    }

    /// Retained private custody only: public intake supplies a retained parent
    /// digest through continue_goal and cannot assert lineage or original quotas.
    pub(super) fn parse(raw: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Source {
            schema: String,
            planner: String,
            parent_plan_digest: String,
            root_plan_digest: String,
            resource_acquisition: ResourceAcquisitionRequest,
        }
        bounded(raw)?;
        let source: Source = serde_json::from_str(raw).map_err(|error| {
            invalid(format!(
                "invalid retained resource acquisition continuation: {error}"
            ))
        })?;
        if source.schema != SOURCE_SCHEMA || source.planner != PLANNER {
            return Err(invalid(
                "unknown resource acquisition continuation schema or compiler generation",
            ));
        }
        Self::new(
            digest(&source.parent_plan_digest)?,
            digest(&source.root_plan_digest)?,
            source.resource_acquisition,
        )
    }

    pub(super) fn next(&self, parent: Digest32) -> Result<Self> {
        Self::new(parent, self.root, self.request.clone())
    }

    pub(super) fn canonical_json(&self) -> String {
        format!(
            "{{\"schema\":\"{SOURCE_SCHEMA}\",\"planner\":\"{PLANNER}\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"resource_acquisition\":{}}}",
            self.parent.to_hex(),
            self.root.to_hex(),
            self.request.canonical_json(),
        )
    }

    pub(super) fn source_digest(&self) -> Digest32 {
        let mut bytes = SOURCE_DOMAIN.to_vec();
        bytes.extend_from_slice(self.canonical_json().as_bytes());
        Digest32::of_bytes(&bytes)
    }

    pub(super) fn lineage_json(&self) -> Value {
        json!({
            "source_schema": SOURCE_SCHEMA,
            "planner": PLANNER,
            "parent_plan_digest": self.parent.to_hex(),
            "root_plan_digest": self.root.to_hex(),
            "continuation_source_digest": self.source_digest().to_hex(),
            "semantic_request_preserved": true,
            "historical_exact_goal_unchanged": true,
            "completion_proves": "this_reviewed_pursuit_only",
            "requires_explicit_commit": true,
            "replacement_work_dispatched": false,
        })
    }

    pub(super) fn same_original_request(&self, other: &Self) -> bool {
        self == other
    }

    pub(super) fn stock_goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        self.request.stock_goal_truth(snapshot)
    }

    pub(super) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Value)> {
        let summary = if summary.is_empty() {
            "continue original resource stock minima"
        } else {
            summary
        };
        let (intent, mut analysis) = self.request.compile_sealed(
            id,
            snapshot,
            summary,
            self.source_digest(),
            "resource acquisition continuation",
        )?;
        analysis["continuation"] = self.lineage_json();
        Ok((intent, analysis))
    }
}

#[cfg(test)]
#[path = "resource_acquisition_continuation_tests.rs"]
mod tests;
