//! Discover retained laboratory recovery points without old transcript IDs.
//!
//! Inspection is metadata-only and never creates or restores a checkpoint.
//! Current whole-fortress Observe authority precedes every existence lookup;
//! checkpoint source anchors never substitute for current authorization.

use dfmcp_core::{
    AnchorEpochs, Capability, CheckpointId, DfmcpError, Digest32, ErrorCode, Result, RiskTier,
    StateAnchor, StateAnchorV2, WorkBudget,
};
use dfmcp_world::WorldSnapshot;
use fastmcp_rust::prelude::{McpError, McpResult, ResourceContent};
use serde_json::{Value, json};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
};
use crate::server::{
    anchor_json, authorize_entry, parse_session_id_arg, read_context, resolve_session,
    with_session_read,
};

const OPERATION: &str = "resources.read.checkpoints";
const DOMAIN: &str = "laboratory.retained_checkpoint_metadata";
const MAX_CATALOG_RECORDS: usize = 4_096;
const MAX_PAGE_RECORDS: usize = 25;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 60_000;

#[derive(Debug, PartialEq, Eq)]
enum Address {
    Catalog { digest: Option<Digest32>, offset: usize },
    Checkpoint(CheckpointId),
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn lower_hex(raw: &str, length: usize) -> bool {
    raw.len() == length
        && raw.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn address(view: &str) -> Result<Address> {
    if view == "checkpoints" {
        return Ok(Address::Catalog { digest: None, offset: 0 });
    }
    if view.len() > 128 {
        return Err(invalid("checkpoint resource address exceeds its byte bound"));
    }
    if let Some(raw) = view.strip_prefix("checkpoints-") {
        let (hash, raw_offset) = raw.split_once('~')
            .ok_or_else(|| invalid("checkpoint catalog requires a digest and offset"))?;
        if !lower_hex(hash, 64) {
            return Err(invalid("checkpoint catalog digest requires 64 lowercase hexadecimal digits"));
        }
        let digest = Digest32::from_hex(hash)
            .ok_or_else(|| invalid("invalid checkpoint catalog digest"))?;
        let offset = raw_offset.parse::<usize>()
            .map_err(|_| invalid("invalid checkpoint catalog offset"))?;
        if raw_offset != offset.to_string() {
            return Err(invalid("checkpoint catalog offset requires canonical nonnegative decimal digits"));
        }
        return Ok(Address::Catalog { digest: Some(digest), offset });
    }
    let raw = view.strip_prefix("checkpoint-")
        .ok_or_else(|| invalid("unknown checkpoint resource"))?;
    if !lower_hex(raw, 32) {
        return Err(invalid("checkpoint identity requires 32 lowercase hexadecimal digits"));
    }
    let id = u128::from_str_radix(raw, 16)
        .map_err(|_| invalid("invalid checkpoint identity"))?;
    if id == 0 {
        return Err(invalid("checkpoint identity zero is reserved"));
    }
    Ok(Address::Checkpoint(CheckpointId::new(id)))
}

fn checkpoint_uri(session: &str, id: CheckpointId) -> String {
    format!("df://session/{session}/checkpoint-{id}")
}

fn catalog_uri(session: &str, digest: Digest32, offset: usize) -> String {
    format!("df://session/{session}/checkpoints-{digest}~{offset}")
}

fn state_uri(session: &str, snapshot: &WorldSnapshot) -> String {
    format!("df://session/{session}/state-{}", snapshot.state_hash)
}

fn version_identity(anchor: StateAnchor) -> Result<Value> {
    let v2 = StateAnchorV2::from_v1(anchor, AnchorEpochs::laboratory(anchor.fortress_id))?;
    Ok(json!({
        "fortress_lineage": v2.fortress_lineage.to_hex(),
        "observation_epoch": v2.observation_epoch, "snapshot_sequence": v2.snapshot_sequence,
        "game_tick": v2.game_tick.map(|tick| tick.0), "bridge_generation": v2.bridge_generation,
        "bridge_protocol": v2.bridge_protocol, "adapter_epoch": v2.adapter_epoch,
        "schema_epoch": v2.schema_epoch, "policy_epoch": v2.policy_epoch,
        "semantic_world_root": v2.semantic_world_root.to_hex(), "digest": v2.digest().to_hex(),
    }))
}

fn refusal(session: &str, error: DfmcpError, authorized_anchor: Option<StateAnchor>) -> McpError {
    let gap = matches!(error.code, ErrorCode::StaleAnchor | ErrorCode::CursorGap);
    let mut turn = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .continuity(
            if gap { ContinuityStatus::Gap } else { ContinuityStatus::Partial },
            None, gap.then(|| json!({"reason": error.message})), None,
        )
        .coverage(json!({"status":"unknown", "complete_domains":[], "absence_proof_scope":[]}))
        .uncertainty(vec![json!({"kind":"unknown", "reason":error.message})]);
    if let Ok(id) = parse_session_id_arg(session) {
        turn = turn.session_id(id.to_string());
    }
    if let Some(anchor) = authorized_anchor {
        turn = turn.anchor(anchor_json(&anchor));
    }
    let mut result = McpError::invalid_request(format!("{}: {}", error.code.as_str(), error.message));
    result.data = Some(json!({
        "schema":"dfmcp.resource-error/1", "ok":false,
        "error":{"code":error.code.as_str(), "message":error.message,
            "retryable":error.retryable, "effect_may_have_occurred":false},
        "agent_turn":turn.build(),
    }));
    result
}

fn check_deadline(
    budget: WorkBudget,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<()> {
    if elapsed() >= std::time::Duration::from_millis(budget.max_wall_millis) {
        return Err(exhausted("checkpoint inspection exceeded its shared wall-time allowance"));
    }
    Ok(())
}

#[derive(Default)]
struct SourceWork {
    records: usize,
    metadata_bytes: usize,
    catalog_hash_bytes: usize,
}

impl SourceWork {
    fn check(&self, budget: WorkBudget) -> Result<()> {
        if self.records > budget.max_entities as usize || self.records > MAX_CATALOG_RECORDS {
            return Err(exhausted("checkpoint inspection exceeds its metadata record allowance"));
        }
        let bytes = self.metadata_bytes.saturating_add(self.catalog_hash_bytes);
        if bytes as u64 > budget.max_bytes.min(MAX_SOURCE_BYTES) {
            return Err(exhausted("checkpoint inspection exceeds its complete metadata byte allowance"));
        }
        Ok(())
    }

    fn row(
        &mut self,
        session: &str,
        id: CheckpointId,
        snapshot: &WorldSnapshot,
        label: Option<&str>,
        current: StateAnchor,
        budget: WorkBudget,
    ) -> Result<Value> {
        self.records = self.records.saturating_add(1);
        self.check(budget)?;
        if snapshot.fortress_id != current.fortress_id || id.get() == 0 {
            return Err(DfmcpError::new(ErrorCode::CorruptLedger,
                "retained checkpoint identity does not match its fortress"));
        }
        // Bound borrowed variable text before JSON cloning or escaping. The
        // remaining fields have fixed-size typed identities and anchors.
        let raw_bytes = label.map_or(0, str::len);
        if self.metadata_bytes.saturating_add(raw_bytes) as u64
            > budget.max_bytes.min(MAX_SOURCE_BYTES)
        {
            return Err(exhausted("checkpoint label exceeds the remaining metadata byte allowance"));
        }
        let row = json!({
            "checkpoint_id":id.to_string(), "label":label,
            "label_status":if label.is_some() { "retained" } else { "unavailable" },
            "content_digest":snapshot.state_hash.to_hex(),
            "source_anchor":anchor_json(&snapshot.anchor()),
            "source_anchor_v2":version_identity(snapshot.anchor())?,
            "uri":checkpoint_uri(session, id), "state":state_uri(session, snapshot),
        });
        self.metadata_bytes = self.metadata_bytes.saturating_add(row.to_string().len());
        self.check(budget)?;
        Ok(row)
    }

    fn json(&self) -> Value {
        json!({"scope":"checkpoint_metadata_only", "records":self.records,
            "metadata_bytes":self.metadata_bytes, "catalog_hash_bytes":self.catalog_hash_bytes,
            "snapshot_contents_scanned":false})
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
    if uri.len() > 192 || uri != format!("df://session/{session_id}/{view}") {
        return Err(refusal(session_id,
            invalid("checkpoint URI does not match its session parameters"), None));
    }
    let target = address(view).map_err(|error| refusal(session_id, error, None))?;
    let session = resolve_session(Some(session_id.to_owned()))
        .map_err(|error| refusal(session_id, error, None))?;
    with_session_read(
        &session,
        |error| Err(refusal(session_id, error, None)),
        |guard| {
            let context = read_context(guard);
            // Authority and durable ownership precede even existence and count.
            authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)
                .map_err(|error| refusal(session_id, error, None))?;
            let current = context.anchor;
            let result = (|| {
                check_deadline(guard.budget, &mut elapsed)?;
                let session = guard.session_id.to_string();
                let mut work = SourceWork::default();
                match target {
                    Address::Catalog { digest, offset } => {
                        let count = guard.adapter.checkpoint_count();
                        if count > guard.budget.max_entities as usize || count > MAX_CATALOG_RECORDS {
                            return Err(exhausted("checkpoint catalog exceeds its complete record allowance"));
                        }
                        let mut rows = Vec::with_capacity(count);
                        for (id, snapshot, label) in guard.adapter.checkpoint_entries() {
                            check_deadline(guard.budget, &mut elapsed)?;
                            rows.push(work.row(&session, id, snapshot, label, current, guard.budget)?);
                        }
                        if rows.len() != count {
                            return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation,
                                "checkpoint count changed under its immutable inspection boundary"));
                        }
                        rows.sort_by(|left, right| left["checkpoint_id"].as_str()
                            .cmp(&right["checkpoint_id"].as_str()));
                        catalog(&session, uri, digest, offset, rows, current, guard.budget,
                            &mut work, &mut elapsed)
                    }
                    Address::Checkpoint(id) => {
                        let snapshot = guard.adapter.checkpoint_snapshot(id)
                            .ok_or_else(|| DfmcpError::new(ErrorCode::StaleAnchor,
                                "the named checkpoint is not retained by this session's fortress; refresh its checkpoint catalog"))?;
                        let mut payload = work.row(&session, id, snapshot,
                            guard.adapter.checkpoint_label(id), current, guard.budget)?;
                        payload["schema"] = json!("dfmcp.lab-checkpoint/1");
                        payload["canonical_uri"] = json!(checkpoint_uri(&session, id));
                        payload["snapshot_contents"] = json!("inspect_the_exact_state_resource");
                        payload["restore"] = json!({
                            "tool":"fortress.restore",
                            "arguments":{"session_id":session,"checkpoint_id":id.to_string()},
                            "required_capability":"restore", "minimum_risk":"guarded",
                            "authorization":"not_evaluated", "requires_explicit_request":true,
                            "effect":"restore a retained laboratory world into a new observation epoch",
                        });
                        payload["coverage"] = json!({
                            "status":"selected_checkpoint_metadata", "complete_domains":[],
                            "absence_proof_scope":[], "snapshot_contents":"not_inspected",
                        });
                        render(payload, &session, uri, snapshot.anchor(), current,
                            guard.budget, &work, &mut elapsed)?
                            .ok_or_else(|| exhausted("checkpoint metadata exceeds the complete output allowance"))
                    }
                }
            })();
            result.map(|text| super::text_content(uri, text))
                .map_err(|error| refusal(session_id, error, Some(current)))
        },
    )
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
    work: &mut SourceWork,
    elapsed: &mut impl FnMut() -> std::time::Duration,
) -> Result<String> {
    check_deadline(budget, elapsed)?;
    let anchor_v2 = version_identity(current)?;
    let mut identity = json!({"domain":"dfmcp.checkpoint-resource-catalog/1",
        "session_id":session, "anchor_v2":anchor_v2, "rows":[]});
    // Admit the complete hash input before cloning all rows or serializing it.
    // The empty array's two delimiters remain; only rows and commas are added.
    work.catalog_hash_bytes = identity.to_string().len()
        .saturating_add(work.metadata_bytes)
        .saturating_add(rows.len().saturating_sub(1));
    work.check(budget)?;
    identity["rows"] = json!(rows);
    let bytes = identity.to_string();
    if bytes.len() != work.catalog_hash_bytes {
        return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation,
            "checkpoint catalog hash accounting differs from its admitted metadata"));
    }
    let digest = Digest32::of_bytes(bytes.as_bytes());
    check_deadline(budget, elapsed)?;
    if expected.is_some_and(|expected| expected != digest) {
        return Err(DfmcpError::new(ErrorCode::CursorGap,
            "checkpoint catalog or current anchor changed; restart at checkpoints"));
    }
    if start > rows.len() || (start == rows.len() && !rows.is_empty()) {
        return Err(invalid("checkpoint offset is outside its retained catalog"));
    }
    let total = rows.len();
    let mut returned = (total - start).min(MAX_PAGE_RECORDS);
    loop {
        let end = start + returned;
        let next = (end < total).then(|| catalog_uri(session, digest, end));
        let complete_domain = start == 0 && end == total;
        let payload = json!({
            "schema":"dfmcp.lab-checkpoint-catalog/1", "catalog_digest":digest.to_hex(),
            "canonical_uri":catalog_uri(session, digest, start),
            "offset":start, "returned":returned, "total":total, "rows":rows[start..end],
            "next":next, "complete":end == total, "complete_catalog":complete_domain,
            "complete_domain":complete_domain, "absence_proven":complete_domain && total == 0,
            "truncated":!complete_domain,
            "retention":{"pins_checkpoints":false, "scope":"current fortress's retained laboratory checkpoints"},
            "coverage":{
                "status":if complete_domain { "complete_metadata_catalog" } else { "partial_metadata_catalog" },
                "domain":DOMAIN,
                "complete_domains":if complete_domain { vec![DOMAIN] } else { vec![] },
                "partial_domains":if complete_domain { vec![] } else { vec![DOMAIN] },
                "absence_proof_scope":if complete_domain && total == 0 { vec![DOMAIN] } else { vec![] },
                "continuation":next, "snapshot_contents":"not_inspected",
                "game_checkpoint_completeness":"not_established",
            },
        });
        if let Some(text) = render(payload, session, uri, current, current,
            budget, work, elapsed)?
        {
            return Ok(text);
        }
        if returned <= 1 {
            return Err(exhausted("checkpoint catalog metadata or one indivisible row exceeds the output allowance"));
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
    check_deadline(budget, elapsed)?;
    let historical = anchor != current;
    let max_bytes = budget.max_bytes
        .min(u64::from(budget.max_output_tokens)
            .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64))
        .min(MAX_RESOURCE_BYTES);
    payload["ok"] = json!(true);
    payload["session_id"] = json!(session);
    payload["resource"] = json!(uri);
    payload["anchor"] = anchor_json(&anchor);
    payload["anchor_v2"] = version_identity(anchor)?;
    payload["current_anchor"] = anchor_json(&current);
    payload["historical"] = json!(historical);
    payload["scope"] = json!("laboratory_reference_checkpoint");
    payload["game_mutation_dispatched"] = json!(false);
    payload["restore_authority_granted"] = json!(false);
    payload["durability"] = json!({
        "durable":null, "status":"not_verified_by_resource",
        "reason":"retained adapter metadata does not certify checkpoint publication or current storage durability",
    });
    payload["source_work"] = work.json();
    let mut active = empty_active_work();
    active["coverage"] = json!({"status":"omitted", "reason":"use the authorized handoff resource"});
    let mut references = vec![
        json!({"kind":"checkpoint_catalog", "uri":format!("df://session/{session}/checkpoints")}),
        json!({"kind":"active_work", "uri":format!("df://session/{session}/handoff")}),
    ];
    if let Some(state) = payload["state"].as_str() {
        references.push(json!({"kind":"immutable_checkpoint_world", "uri":state}));
    }
    payload["agent_turn"] = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .session_id(session).anchor(anchor_json(&anchor))
        .continuity(if historical { ContinuityStatus::Stale } else { ContinuityStatus::Partial },
            Some(anchor_json(&anchor)), None, None)
        .profile(ObservationProfile::Forensic)
        .briefing(json!({"scope":"laboratory_reference_checkpoint", "historical":historical,
            "mutation_authority":false, "restore_authority_granted":false}))
        .active_work(active).coverage(payload["coverage"].clone())
        .uncertainty(vec![
            json!({"kind":"unknown", "domain":"live_game_evidence",
                "reason":"laboratory recovery metadata does not establish a native game checkpoint"}),
            json!({"kind":"unknown", "domain":"checkpoint_durability",
                "reason":"this read does not verify persistent storage or authorize restore"}),
        ])
        .budget(json!({"admitted":true, "max_output_bytes":max_bytes,
            "max_output_tokens":budget.max_output_tokens, "source_work":work.json()}))
        .references(references).build();
    let serialized = payload.to_string();
    let fits = super::task_resource_bytes(uri, &serialized) as u64 <= max_bytes;
    check_deadline(budget, elapsed)?;
    Ok(fits.then_some(serialized))
}

#[cfg(test)]
mod tests;
