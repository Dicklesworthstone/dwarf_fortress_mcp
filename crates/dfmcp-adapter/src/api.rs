#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use dfmcp_core::{
    ActionId, Capability, CheckpointId, CommitState, Digest32, EntityId, Evidence, GameTick,
    MapCuboid, ObservationCursor, OperationContext, PlanId, Result, StateAnchor, StepId,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::{EntityKind, StateDelta, WorldEventKind, WorldQuery, WorldSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompatibilityLevel {
    Exact,
    Compatible,
    DegradedReadOnly,
    Unknown,
    Incompatible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterIdentity {
    pub name: String,
    pub adapter_version: String,
    pub bridge_protocol_version: String,
    pub dwarf_fortress_version: String,
    pub dfhack_version: String,
    pub compatibility: CompatibilityLevel,
    pub capabilities: BTreeSet<Capability>,
    pub schema_digest: Digest32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    ReadOnly,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterHealth {
    pub status: HealthStatus,
    pub identity: AdapterIdentity,
    pub fortress_loaded: bool,
    pub paused: Option<bool>,
    pub current_anchor: Option<StateAnchor>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Projection {
    Summary,
    Entities,
    Graph,
    Map,
    Events,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct InterestSet {
    pub entity_ids: BTreeSet<EntityId>,
    pub entity_kinds: BTreeSet<EntityKind>,
    pub fields: BTreeSet<String>,
    pub map_areas: Vec<MapCuboid>,
    pub event_kinds: BTreeSet<WorldEventKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationRequest {
    pub since: Option<ObservationCursor>,
    pub projection: Projection,
    pub interest: InterestSet,
    pub max_entities: u32,
    pub max_bytes: u64,
    pub max_output_tokens: u32,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationPayload {
    Snapshot(WorldSnapshot),
    Delta(StateDelta),
    Heartbeat(StateAnchor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationFrame {
    pub payload: ObservationPayload,
    pub evidence: Vec<Evidence>,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRequest {
    pub anchor: StateAnchor,
    pub query: WorldQuery,
    pub max_output_tokens: u32,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRow {
    pub entity_id: EntityId,
    pub revision: u64,
    pub fields: Vec<(String, String)>,
    pub score_micros: Option<i64>,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResponse {
    pub anchor: StateAnchor,
    pub rows: Vec<QueryRow>,
    pub matched: u64,
    pub truncated: bool,
    pub continuation: Option<String>,
    pub score_ledger: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareReceipt {
    pub plan_id: PlanId,
    pub plan_digest: Digest32,
    pub revalidated_anchor: StateAnchor,
    pub adapter_token: Vec<u8>,
    pub adapter_token_digest: Digest32,
    pub expires_at_tick: GameTick,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionReceipt {
    pub action_id: ActionId,
    pub step_id: StepId,
    pub state: CommitState,
    pub observed_anchor: StateAnchor,
    pub adapter_receipt_digest: Digest32,
    pub evidence: Vec<Evidence>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitReceipt {
    pub plan_id: PlanId,
    pub plan_digest: Digest32,
    pub actions: Vec<ActionReceipt>,
    pub checkpoint: Option<CheckpointReceipt>,
    pub observed_anchor: StateAnchor,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CancelMode {
    StopFutureSteps,
    CompensateReversible,
    EmergencyPauseAndDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelReceipt {
    pub action_id: ActionId,
    pub state: CommitState,
    pub observed_anchor: StateAnchor,
    pub compensation_action: Option<ActionId>,
    pub evidence: Vec<Evidence>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointReceipt {
    pub checkpoint_id: CheckpointId,
    pub label: String,
    pub anchor: StateAnchor,
    pub content_digest: Digest32,
    pub durable: bool,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreReceipt {
    pub checkpoint_id: CheckpointId,
    pub prior_anchor: StateAnchor,
    pub restored_anchor: StateAnchor,
    pub content_digest: Digest32,
    pub evidence: Vec<Evidence>,
}

pub trait GameAdapter {
    fn identity(&self) -> AdapterIdentity;

    fn current_anchor(&self) -> Option<StateAnchor> {
        None
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth>;

    fn observe(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<ObservationFrame>;

    fn query(
        &mut self,
        request: &QueryRequest,
        context: &OperationContext,
    ) -> Result<QueryResponse>;

    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        context: &OperationContext,
    ) -> Result<PrepareReceipt>;

    fn commit(
        &mut self,
        plan: &PreparedPlan,
        prepared: &PrepareReceipt,
        context: &OperationContext,
    ) -> Result<CommitReceipt>;

    fn poll_action(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<ActionReceipt>;

    fn request_cancel(
        &mut self,
        action_id: ActionId,
        mode: CancelMode,
        context: &OperationContext,
    ) -> Result<CancelReceipt>;

    fn finalize_cancel(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<CancelReceipt>;

    fn checkpoint(&mut self, label: &str, context: &OperationContext) -> Result<CheckpointReceipt>;

    fn restore(
        &mut self,
        checkpoint_id: CheckpointId,
        context: &OperationContext,
    ) -> Result<RestoreReceipt>;
}
