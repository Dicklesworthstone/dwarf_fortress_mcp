//! Closed descriptors for one process-local Task monitor.
//! Scalar descriptor fields and their serialization order remain unchanged.

use fastmcp_rust::{McpError, McpResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{OBSERVATION_WORK_SCHEMA, SET_OBSERVATION_WORK_SCHEMA, WORK_SCHEMA};

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum TaskWork {
    Original(OriginalTaskWork),
    Set(OriginalSetTaskWork),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalTaskWork {
    schema: String,
    session_id: String,
    plan_digest: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalSetTaskWork {
    schema: String,
    session_id: String,
    plan_digests: Vec<String>,
    plan_policy: String,
}

fn exact_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl TaskWork {
    pub(crate) fn parse(value: &Value) -> McpResult<Self> {
        let mut work: Self = serde_json::from_value(value.clone())
            .map_err(|_| McpError::invalid_params("invalid laboratory task work descriptor"))?;
        if work.session_id().len() != 32
            || !work
                .session_id()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(McpError::invalid_params(
                "laboratory task work must bind one exact session",
            ));
        }
        match &mut work {
            Self::Original(original) => {
                if !matches!(
                    original.schema.as_str(),
                    WORK_SCHEMA | OBSERVATION_WORK_SCHEMA
                ) || original.plan_digest.len() != 64
                    || !original
                        .plan_digest
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
                    || (original.schema == OBSERVATION_WORK_SCHEMA
                        && !exact_digest(&original.plan_digest))
                {
                    return Err(McpError::invalid_params(
                        "laboratory task work must bind one exact session and sealed plan digest",
                    ));
                }
                original.session_id.make_ascii_lowercase();
                original.plan_digest.make_ascii_lowercase();
            }
            Self::Set(group) => {
                let distinct: std::collections::BTreeSet<_> = group.plan_digests.iter().collect();
                if group.schema != SET_OBSERVATION_WORK_SCHEMA
                    || !(1..=8).contains(&group.plan_digests.len())
                    || distinct.len() != group.plan_digests.len()
                    || !group.plan_digests.iter().all(|digest| exact_digest(digest))
                    || !matches!(group.plan_policy.as_str(), "all" | "any")
                {
                    return Err(McpError::invalid_params(
                        "original-set observation work requires 1..8 distinct exact lowercase digests and all or any policy",
                    ));
                }
                group.session_id.make_ascii_lowercase();
            }
        }
        Ok(work)
    }

    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::Original(original) => &original.session_id,
            Self::Set(group) => &group.session_id,
        }
    }

    pub(crate) fn plan_digest(&self) -> Option<&str> {
        match self {
            Self::Original(original) => Some(&original.plan_digest),
            Self::Set(_) => None,
        }
    }

    pub(crate) fn original_digest(&self) -> McpResult<&str> {
        self.plan_digest().ok_or_else(|| {
            McpError::internal_error("a group observer cannot dispatch or drain one original")
        })
    }

    pub(crate) fn plan_set(&self) -> Option<(&[String], &str)> {
        match self {
            Self::Set(group) => Some((&group.plan_digests, &group.plan_policy)),
            Self::Original(_) => None,
        }
    }

    pub(crate) fn is_set_observer(&self) -> bool {
        matches!(self, Self::Set(_))
    }

    pub(crate) fn is_observer(&self) -> bool {
        match self {
            Self::Original(original) => original.schema == OBSERVATION_WORK_SCHEMA,
            Self::Set(_) => true,
        }
    }

    pub(crate) fn monitor_kind(&self) -> &'static str {
        if self.is_set_observer() {
            "observe_original_set"
        } else if self.is_observer() {
            "observe_original"
        } else {
            "commit_original"
        }
    }

    pub(crate) fn cancellation_scope(&self) -> &'static str {
        if self.is_observer() {
            "monitor_only"
        } else {
            "original_plan"
        }
    }

    /// Group rows carry the exact requested set, never a fabricated scalar
    /// digest or one arbitrarily chosen member. Existing scalar rows are kept.
    pub(crate) fn attach_selection(&self, payload: &mut Value) {
        if let Some((digests, policy)) = self.plan_set()
            && let Some(fields) = payload.as_object_mut()
        {
            fields.remove("plan_digest");
            fields.insert("plan_digests".to_owned(), json!(digests));
            fields.insert("plan_policy".to_owned(), json!(policy));
        }
    }

    /// Retain identity at the creation boundary, separately from changing
    /// outcome evidence. Later custody loss cannot erase a selected member or
    /// turn the previous observation into a claim about the current frontier.
    pub(crate) fn admitted_set_binding(&self, payload: &Value) -> McpResult<Value> {
        let (digests, policy) = self.plan_set().ok_or_else(|| {
            McpError::internal_error("only an original-set Task has a set binding")
        })?;
        let selected = &payload["selected_plan_set_wait"];
        let plans = selected["plans"].as_array().ok_or_else(|| {
            McpError::internal_error("original-set admission has no complete identity census")
        })?;
        if plans.len() != digests.len()
            || selected["plan_digests"].as_array().map(Vec::len) != Some(digests.len())
            || selected["requested_plan_digests"].as_array().map(Vec::len) != Some(digests.len())
        {
            return Err(McpError::internal_error(
                "original-set admission omitted a selected identity",
            ));
        }
        Ok(json!({
            "schema":"dfmcp.selected-plan-set-wait/1",
            "plan_policy":policy,"selected":digests.len(),
            "plan_digests":selected["plan_digests"],
            "requested_plan_digests":selected["requested_plan_digests"],
            "identity_scope":"admitted_original_identity_without_current_outcome_evidence",
            "admitted_anchor":payload["observed_anchor"],
            "plans":plans.iter().map(|plan| json!({
                "requested_plan_digest":plan["requested_plan_digest"],
                "plan_digest":plan["plan_digest"],"plan_id":plan["plan_id"],
                "plan_object_digest":plan["plan_object_digest"],
                "selection_origin":plan["selection_origin"],
                "selection_scope":plan["selection_scope"],
                "original_owner_session_id":plan["original_owner_session_id"],
            })).collect::<Vec<_>>(),
        }))
    }

    pub(crate) fn matches_admitted_set(&self, binding: &Value, payload: &Value) -> bool {
        let Ok(current) = self.admitted_set_binding(payload) else {
            return false;
        };
        if [
            "plan_policy",
            "selected",
            "plan_digests",
            "requested_plan_digests",
        ]
        .iter()
        .any(|field| binding.get(*field) != current.get(*field))
        {
            return false;
        }
        let (Some(before), Some(after)) =
            (binding["plans"].as_array(), current["plans"].as_array())
        else {
            return false;
        };
        before.len() == after.len()
            && before.iter().zip(after).all(|(before, after)| {
                [
                    "requested_plan_digest",
                    "plan_digest",
                    "plan_id",
                    "plan_object_digest",
                    "original_owner_session_id",
                ]
                .iter()
                .all(|field| before.get(*field) == after.get(*field))
            })
    }

    pub(crate) fn validate_observation(&self) -> McpResult<()> {
        if let Some((digests, policy)) = self.plan_set() {
            crate::server::task_session::validate_observation_set(
                self.session_id(),
                digests,
                policy,
            )
        } else {
            crate::server::task_session::validate_observation(
                self.session_id(),
                self.original_digest()?,
            )
        }
        .map_err(|error| McpError::invalid_params(error.to_string()))
    }

    pub(crate) fn observation_view(
        &self,
    ) -> dfmcp_core::Result<crate::server::task_session::PlanTaskView> {
        match self {
            Self::Set(group) => crate::server::task_session::observation_set_view(
                &group.session_id,
                &group.plan_digests,
                &group.plan_policy,
            ),
            Self::Original(original) => crate::server::task_session::observation_view(
                &original.session_id,
                &original.plan_digest,
            ),
        }
    }
}
