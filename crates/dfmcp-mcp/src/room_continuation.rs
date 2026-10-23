//! A reviewed residual pursuit of one complete retained room request.
//!
//! The envelope binds the original request and flat lineage to a fixed residual
//! compiler. It carries neither dispatch authority nor replacement evidence for
//! the historical plan's exact item and building identities.

use super::room_provisioning::ProvisionedRoomRequest;
use crate::lab_world::MAX_ACTIONS_JSON_BYTES;
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, IntentId, Result};
use dfmcp_intent::{Intent, StaticPlanner};
use dfmcp_world::WorldSnapshot;
use serde::Deserialize;
use serde_json::{Value, json};

const SOURCE_SCHEMA: &str = "dfmcp.room-continuation/1";
const PLANNER: &str = "provisioned_room_residual_v1";
const SOURCE_DOMAIN: &[u8] = b"dfmcp-room-continuation-source/1\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomContinuation {
    pub(crate) parent: Digest32,
    pub(crate) root: Digest32,
    request: ProvisionedRoomRequest,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(raw: &str) -> Result<()> {
    if raw.len() > MAX_ACTIONS_JSON_BYTES || raw.contains('\0') {
        return Err(invalid(
            "room continuation exceeds its bounded source contract",
        ));
    }
    Ok(())
}

fn digest(raw: &str) -> Result<Digest32> {
    Digest32::from_hex(raw)
        .filter(|value| *value != Digest32::ZERO)
        .ok_or_else(|| invalid("room continuation requires a nonzero 64-character plan digest"))
}

impl RoomContinuation {
    pub(crate) fn new(
        parent: Digest32,
        root: Digest32,
        request: ProvisionedRoomRequest,
    ) -> Result<Self> {
        if parent == Digest32::ZERO || root == Digest32::ZERO {
            return Err(invalid(
                "room continuation lineage cannot contain a zero digest",
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

    /// Only retained private source custody uses this parser. Public intake
    /// accepts continue_goal plus a retained digest, never this envelope.
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Source {
            schema: String,
            planner: String,
            parent_plan_digest: String,
            root_plan_digest: String,
            room: Value,
        }
        bounded(raw)?;
        let source: Source = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid retained room continuation: {error}")))?;
        if source.schema != SOURCE_SCHEMA || source.planner != PLANNER {
            return Err(invalid(
                "unknown room continuation schema or residual compiler",
            ));
        }
        let request = ProvisionedRoomRequest::parse(&source.room.to_string())?;
        Self::new(
            digest(&source.parent_plan_digest)?,
            digest(&source.root_plan_digest)?,
            request,
        )
    }

    pub(crate) fn next(&self, parent: Digest32) -> Result<Self> {
        Self::new(parent, self.root, self.request.clone())
    }

    pub(crate) fn canonical_json(&self) -> String {
        // Embed the existing canonical request unchanged, including its v1
        // marker and supplied setup options that happen not to be needed now.
        format!(
            "{{\"schema\":\"{SOURCE_SCHEMA}\",\"planner\":\"{PLANNER}\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"room\":{}}}",
            self.parent.to_hex(),
            self.root.to_hex(),
            self.request.canonical_json(),
        )
    }

    pub(crate) fn source_digest(&self) -> Digest32 {
        let mut bytes = SOURCE_DOMAIN.to_vec();
        bytes.extend_from_slice(self.canonical_json().as_bytes());
        Digest32::of_bytes(&bytes)
    }

    pub(crate) fn lineage_json(&self) -> Value {
        json!({
            "source_schema":SOURCE_SCHEMA,
            "planner":PLANNER,
            "parent_plan_digest":self.parent.to_hex(),
            "root_plan_digest":self.root.to_hex(),
            "continuation_source_digest":self.source_digest().to_hex(),
            "semantic_request_preserved":true,
            "historical_exact_goal_unchanged":true,
            "completion_proves":"this_reviewed_pursuit_only",
            "requires_explicit_commit":true,
            "replacement_work_dispatched":false,
        })
    }

    pub(crate) fn same_original_request(&self, other: &Self) -> bool {
        self == other
    }

    pub(crate) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Value)> {
        let summary = format!(
            "{summary} [room continuation {}]",
            self.source_digest().to_hex()
        );
        if summary.len() > StaticPlanner::default().policy.max_string_bytes
            || summary.contains('\0')
        {
            return Err(invalid(
                "room continuation summary plus its source seal exceeds the planner bound; supply a shorter summary",
            ));
        }
        let (intent, mut analysis) = self.request.compile_residual(id, snapshot, &summary)?;
        analysis["continuation"] = self.lineage_json();
        Ok((intent, analysis))
    }
}

#[cfg(test)]
#[path = "room_continuation_tests.rs"]
mod tests;
