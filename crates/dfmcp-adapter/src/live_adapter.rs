#![forbid(unsafe_code)]

//! Read-only `GameAdapter` over the authenticated DFHack observation source.
//!
//! The adapter has an explicit bootstrap step because no honest semantic
//! `StateAnchor` exists before the first complete capsule is read and projected.
//! Thereafter each operation is pinned to the current anchor. Unchanged state
//! yields a heartbeat, ordinary change advances the sequence, and a bridge
//! restart or game-clock regression advances the observation epoch. World/site
//! identity and negotiated version changes fail closed and require a new
//! session.

use std::collections::BTreeSet;

use dfmcp_core::{
    ActionId, Capability, CheckpointId, DfmcpError, Digest32, EntityId, ErrorCode, Evidence,
    EvidenceId, EvidenceKind, FortressId, ObservationCursor, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::execute_bounded_query;

use crate::{
    ActionReceipt, AdapterHealth, AdapterIdentity, CancelMode, CancelReceipt, CheckpointReceipt,
    CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus, InterestSet,
    LiveObservationCapsule, LiveObservationSource, LiveWorldProjection, MAX_CAPSULE_CITIZENS,
    MAX_CITIZENS_PER_PAGE, ObservationFrame, ObservationPayload, ObservationRequest,
    PrepareReceipt, Projection, QueryRequest, QueryResponse, QueryRow, RestoreReceipt,
    read_complete_observation_bounded, project_live_capsule,
};

const LIVE_ADAPTER_SCHEMA: &[u8] = b"dfmcp-live-read-adapter-v1";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadAdapterConfig {
    pub fortress_id: FortressId,
    pub page_size: u32,
    pub max_citizens: u32,
    pub include_names: bool,
    pub initial_epoch: u64,
}

impl LiveReadAdapterConfig {
    pub fn validate(&self) -> Result<()> {
        if self.fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live adapter fortress identity zero is reserved",
            ));
        }
        if self.page_size == 0 || self.page_size > MAX_CITIZENS_PER_PAGE {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "live adapter page size must be in 1..={MAX_CITIZENS_PER_PAGE}"
                ),
            ));
        }
        let hard_total = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "capsule citizen ceiling does not fit u32",
            )
        })?;
        if self.max_citizens > hard_total {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "live adapter citizen ceiling {} exceeds {hard_total}",
                    self.max_citizens
                ),
            ));
        }
        if u32::try_from(self.initial_epoch)
            .ok()
            .and_then(|epoch| epoch.checked_add(1))
            .is_none()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "initial observation epoch cannot be represented as an entity generation",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RefreshOutcome {
    prior_anchor: StateAnchor,
    current_anchor: StateAnchor,
    changed: bool,
    reset: bool,
}

pub struct LiveReadAdapter<T> {
    source: T,
    config: LiveReadAdapterConfig,
    identity: AdapterIdentity,
    current: Option<LiveWorldProjection>,
    last_capsule: Option<LiveObservationCapsule>,
    epoch: u64,
    sequence: u64,
}

impl<T: LiveObservationSource> LiveReadAdapter<T> {
    pub fn new(source: T, config: LiveReadAdapterConfig) -> Result<Self> {
        config.validate()?;
        let manifest = source.bridge_manifest();
        manifest.validate()?;
        let identity = adapter_identity(&manifest);
        let epoch = config.initial_epoch;
        Ok(Self {
            source,
            config,
            identity,
            current: None,
            last_capsule: None,
            epoch,
            sequence: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &LiveReadAdapterConfig {
        &self.config
    }

    #[must_use]
    pub const fn source(&self) -> &T {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut T {
        &mut self.source
    }

    #[must_use]
    pub fn current_projection(&self) -> Option<&LiveWorldProjection> {
        self.current.as_ref()
    }

    #[must_use]
    pub fn last_capsule(&self) -> Option<&LiveObservationCapsule> {
        self.last_capsule.as_ref()
    }

    pub fn bootstrap(&mut self) -> Result<&LiveWorldProjection> {
        if self.current.is_some() {
            return self.current.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "live adapter lost its bootstrap projection",
                )
            });
        }
        let capsule = self.read_capsule(self.config.max_citizens)?;
        let projection = project_live_capsule(
            &capsule,
            self.config.fortress_id,
            ObservationCursor {
                epoch: self.epoch,
                sequence: self.sequence,
            },
        )?;
        self.identity = adapter_identity(&capsule.bridge);
        self.last_capsule = Some(capsule);
        self.current = Some(projection);
        self.current.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live adapter bootstrap projection was not retained",
            )
        })
    }

    fn read_capsule(&mut self, max_citizens: u32) -> Result<LiveObservationCapsule> {
        let page_size = self
            .config
            .page_size
            .min(max_citizens.max(1));
        read_complete_observation_bounded(
            &mut self.source,
            page_size,
            self.config.include_names,
            max_citizens,
        )
    }

    fn current_projection_required(&self) -> Result<&LiveWorldProjection> {
        self.current.as_ref().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "live adapter has no canonical anchor; call bootstrap before opening a session",
            )
        })
    }

    fn current_capsule_required(&self) -> Result<&LiveObservationCapsule> {
        self.last_capsule.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live adapter has a projection without its source capsule",
            )
        })
    }

    fn check_context_anchor(&self, context: &OperationContext) -> Result<()> {
        let current = self.current_projection_required()?.snapshot.anchor();
        if context.anchor != current {
            return Err(error(
                ErrorCode::StaleAnchor,
                "operation context is not pinned to the current live adapter anchor",
            )
            .retryable(true));
        }
        Ok(())
    }

    fn validate_observation_request(
        &self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<u32> {
        if request.max_entities == 0
            || request.max_entities > context.budget.max_entities
            || request.max_bytes == 0
            || request.max_bytes > context.budget.max_bytes
            || request.max_output_tokens == 0
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "live observation request exceeds its operation budget",
            ));
        }
        if request.continuation.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live bridge V1 publishes only complete capsules and has no cross-call continuation",
            ));
        }
        if !request.interest.entity_ids.is_empty()
            || !request.interest.entity_kinds.is_empty()
            || !request.interest.fields.is_empty()
            || !request.interest.map_areas.is_empty()
            || !request.interest.event_kinds.is_empty()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live bridge V1 does not yet implement interest-filtered canonical publication",
            ));
        }
        if !matches!(
            request.projection,
            Projection::Summary | Projection::Entities | Projection::Graph | Projection::Full
        ) {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                "live bridge V1 does not observe the requested projection domain",
            ));
        }
        let max_citizens = request
            .max_entities
            .saturating_sub(1)
            .min(self.config.max_citizens);
        let current_citizens = self
            .last_capsule
            .as_ref()
            .map_or(0, |capsule| capsule.citizen_coverage.total);
        if current_citizens > max_citizens {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "current live roster contains {current_citizens} citizens but this request admits {max_citizens}"
                ),
            ));
        }
        Ok(max_citizens)
    }

    fn refresh(&mut self, max_citizens: u32) -> Result<RefreshOutcome> {
        let prior_projection = self.current_projection_required()?.clone();
        let prior_capsule = self.current_capsule_required()?.clone();
        let prior_anchor = prior_projection.snapshot.anchor();
        let capsule = self.read_capsule(max_citizens)?;

        ensure_same_session_identity(&prior_capsule, &capsule)?;
        let prior_tick = prior_projection.snapshot.tick;
        let next_tick = crate::DwarfFortressClock {
            year: capsule.current_year,
            year_tick: capsule.current_year_tick,
        }
        .absolute_tick()?;
        let bridge_reset =
            capsule.bridge.bridge_generation != prior_capsule.bridge.bridge_generation;
        let clock_regression = next_tick < prior_tick;
        let reset = bridge_reset || clock_regression;

        if !reset && capsule.content_digest == prior_capsule.content_digest {
            return Ok(RefreshOutcome {
                prior_anchor,
                current_anchor: prior_anchor,
                changed: false,
                reset: false,
            });
        }

        if reset {
            self.epoch = self.epoch.checked_add(1).ok_or_else(|| {
                error(
                    ErrorCode::CursorGap,
                    "live observation epoch space is exhausted",
                )
            })?;
            self.sequence = 0;
        } else {
            self.sequence = self.sequence.checked_add(1).ok_or_else(|| {
                error(
                    ErrorCode::CursorGap,
                    "live observation sequence space is exhausted",
                )
            })?;
        }
        let projection = project_live_capsule(
            &capsule,
            self.config.fortress_id,
            ObservationCursor {
                epoch: self.epoch,
                sequence: self.sequence,
            },
        )?;
        let current_anchor = projection.snapshot.anchor();
        self.identity = adapter_identity(&capsule.bridge);
        self.last_capsule = Some(capsule);
        self.current = Some(projection);
        Ok(RefreshOutcome {
            prior_anchor,
            current_anchor,
            changed: true,
            reset,
        })
    }

    fn ensure_snapshot_budget(
        &self,
        request: &ObservationRequest,
        projection: &LiveWorldProjection,
    ) -> Result<()> {
        let entity_count = u32::try_from(projection.snapshot.graph.entities.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "live snapshot entity count cannot be represented",
            )
        })?;
        let snapshot_bytes =
            u64::try_from(projection.snapshot.canonical_bytes().len()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "live snapshot byte count cannot be represented",
                )
            })?;
        if entity_count > request.max_entities || snapshot_bytes > request.max_bytes {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical live snapshot exceeds the requested entity or byte bound",
            ));
        }
        Ok(())
    }
}

fn adapter_identity(manifest: &crate::BridgeManifest) -> AdapterIdentity {
    AdapterIdentity {
        name: "dfmcp-dfhack-live-read".to_owned(),
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        bridge_protocol_version: format!(
            "dfmcp-bridge/{}.{}",
            crate::BRIDGE_PROTOCOL_MAJOR,
            crate::BRIDGE_PROTOCOL_MINOR
        ),
        dwarf_fortress_version: manifest.df_version.clone(),
        dfhack_version: manifest.dfhack_version.clone(),
        compatibility: CompatibilityLevel::DegradedReadOnly,
        capabilities: BTreeSet::from([
            Capability::Observe,
            Capability::Query,
            Capability::Doctor,
        ]),
        schema_digest: Digest32::of_bytes(LIVE_ADAPTER_SCHEMA),
    }
}

fn ensure_same_session_identity(
    prior: &LiveObservationCapsule,
    next: &LiveObservationCapsule,
) -> Result<()> {
    if prior.site_id != next.site_id
        || prior.world_folder != next.world_folder
        || prior.world_name != next.world_name
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "live source switched world or fortress identity; open a new session",
        ));
    }
    if prior.bridge.bridge_version != next.bridge.bridge_version
        || prior.bridge.dfhack_version != next.bridge.dfhack_version
        || prior.bridge.df_version != next.bridge.df_version
        || prior.bridge.supported_methods != next.bridge.supported_methods
    {
        return Err(error(
            ErrorCode::VersionMismatch,
            "live bridge or game version manifest changed; open a new session",
        ));
    }
    Ok(())
}

fn observation_evidence(
    capsule: &LiveObservationCapsule,
    anchor: StateAnchor,
    subject: Option<EntityId>,
    summary: &str,
) -> Evidence {
    let mut identity_bytes = Vec::new();
    identity_bytes.extend_from_slice(b"dfmcp-live-observation-evidence-v1");
    identity_bytes.extend_from_slice(capsule.content_digest.as_bytes());
    identity_bytes.extend_from_slice(anchor.state_hash.as_bytes());
    if let Some(entity_id) = subject {
        identity_bytes.extend_from_slice(&entity_id.get().to_be_bytes());
    }
    let identity_digest = Digest32::of_bytes(&identity_bytes);
    Evidence {
        id: EvidenceId::new(identity_digest.first_u128() | 1),
        kind: EvidenceKind::Observation,
        subject,
        anchor,
        digest: capsule.content_digest,
        summary: summary.to_owned(),
    }
}

fn read_only_rejection<T>(operation: &str) -> Result<T> {
    Err(error(
        ErrorCode::AdapterRejected,
        format!(
            "live DFHack adapter protocol V1 is read-only; {operation} is not implemented"
        ),
    ))
}

impl<T: LiveObservationSource> GameAdapter for LiveReadAdapter<T> {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn current_anchor(&self) -> Option<StateAnchor> {
        self.current
            .as_ref()
            .map(|projection| projection.snapshot.anchor())
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth> {
        context.authorize(Capability::Doctor, RiskTier::ReadOnly, &[], None)?;
        let projection = self.current_projection_required()?;
        self.check_context_anchor(context)?;
        Ok(AdapterHealth {
            status: HealthStatus::ReadOnly,
            identity: self.identity(),
            fortress_loaded: true,
            paused: Some(projection.snapshot.paused),
            current_anchor: Some(projection.snapshot.anchor()),
            warnings: vec![
                "authenticated live bridge protocol V1 is read-only".to_owned(),
                "items, jobs, map, economy, welfare, military, and history are omitted"
                    .to_owned(),
            ],
        })
    }

    fn observe(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<ObservationFrame> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.check_context_anchor(context)?;
        let max_citizens = self.validate_observation_request(request, context)?;
        let outcome = self.refresh(max_citizens)?;
        let projection = self.current_projection_required()?;
        self.ensure_snapshot_budget(request, projection)?;
        let capsule = self.current_capsule_required()?;
        let current_anchor = projection.snapshot.anchor();

        let (payload, mut warnings) = match request.since {
            None => (ObservationPayload::Snapshot(projection.snapshot.clone()), Vec::new()),
            Some(cursor) if cursor == current_anchor.cursor => {
                (ObservationPayload::Heartbeat(current_anchor), Vec::new())
            }
            Some(cursor)
                if cursor == outcome.prior_anchor.cursor
                    || (cursor.epoch == current_anchor.cursor.epoch
                        && cursor.sequence < current_anchor.cursor.sequence) =>
            {
                let warning = if outcome.reset {
                    "observation epoch changed; returned a full canonical snapshot"
                } else {
                    "delta history is not admitted for live bridge V1; returned a full snapshot"
                };
                (
                    ObservationPayload::Snapshot(projection.snapshot.clone()),
                    vec![warning.to_owned()],
                )
            }
            Some(_) => {
                return Err(error(
                    ErrorCode::CursorGap,
                    "requested live observation cursor is not resumable",
                )
                .retryable(true));
            }
        };
        if outcome.changed && request.since.is_none() {
            warnings.push(if outcome.reset {
                "live source continuity reset into a new observation epoch".to_owned()
            } else {
                "live source advanced to a new canonical snapshot".to_owned()
            });
        }
        Ok(ObservationFrame {
            payload,
            evidence: vec![observation_evidence(
                capsule,
                current_anchor,
                None,
                "complete authenticated DFHack read-only observation capsule",
            )],
            warnings,
            truncated: false,
            continuation: None,
        })
    }

    fn query(
        &mut self,
        request: &QueryRequest,
        context: &OperationContext,
    ) -> Result<QueryResponse> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.check_context_anchor(context)?;
        let projection = self.current_projection_required()?;
        if request.anchor != projection.snapshot.anchor() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "query is not pinned to the current live snapshot",
            )
            .retryable(true));
        }
        if request.max_output_tokens == 0
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "query output-token budget is invalid",
            ));
        }
        if projection.snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "query would scan more entities than its operation budget permits",
            ));
        }
        let mut query = request.query.clone();
        if let (Some(outer), Some(inner)) = (&request.continuation, &query.continuation)
            && outer != inner
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "query continuation fields disagree",
            ));
        }
        query.continuation = request
            .continuation
            .clone()
            .or_else(|| query.continuation.clone());
        let byte_limit_u64 = context
            .budget
            .max_bytes
            .min(u64::from(request.max_output_tokens).saturating_mul(4));
        let byte_limit = usize::try_from(byte_limit_u64).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "query byte budget cannot be represented on this platform",
            )
        })?;
        let result = execute_bounded_query(
            &projection.snapshot,
            &query,
            context.budget.max_entities,
            Some(byte_limit),
        )?;
        let capsule = self.current_capsule_required()?;
        let anchor = projection.snapshot.anchor();
        let rows = result
            .entities
            .into_iter()
            .map(|entity| QueryRow {
                entity_id: entity.id,
                revision: entity.revision,
                fields: vec![
                    ("label".to_owned(), entity.label),
                    ("kind".to_owned(), entity.kind.as_str().to_owned()),
                ],
                score_micros: None,
                evidence: vec![observation_evidence(
                    capsule,
                    anchor,
                    Some(entity.id),
                    "query row projected from the complete live capsule",
                )],
            })
            .collect();
        Ok(QueryResponse {
            anchor,
            rows,
            matched: result.matched,
            truncated: result.truncated,
            continuation: result.continuation,
            score_ledger: vec![
                "deterministic canonical world query; no relevance scoring".to_owned(),
            ],
        })
    }

    fn prepare(
        &mut self,
        _plan: &PreparedPlan,
        _context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        read_only_rejection("prepare")
    }

    fn commit(
        &mut self,
        _plan: &PreparedPlan,
        _prepared: &PrepareReceipt,
        _context: &OperationContext,
    ) -> Result<CommitReceipt> {
        read_only_rejection("commit")
    }

    fn poll_action(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<ActionReceipt> {
        read_only_rejection("action polling")
    }

    fn request_cancel(
        &mut self,
        _action_id: ActionId,
        _mode: CancelMode,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        read_only_rejection("cancellation")
    }

    fn finalize_cancel(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        read_only_rejection("cancellation finalization")
    }

    fn checkpoint(
        &mut self,
        _label: &str,
        _context: &OperationContext,
    ) -> Result<CheckpointReceipt> {
        read_only_rejection("checkpoint")
    }

    fn restore(
        &mut self,
        _checkpoint_id: CheckpointId,
        _context: &OperationContext,
    ) -> Result<RestoreReceipt> {
        read_only_rejection("restore")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget,
    };
    use dfmcp_world::{QueryOrder, WorldQuery};

    use super::*;
    use crate::{BridgeManifest, CitizenRecord, ObservationPage};

    #[derive(Clone)]
    struct ScriptedSource {
        manifest: BridgeManifest,
        pages: VecDeque<ObservationPage>,
        calls: usize,
    }

    impl LiveObservationSource for ScriptedSource {
        fn bridge_manifest(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
        ) -> Result<ObservationPage> {
            self.calls = self.calls.saturating_add(1);
            self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted live source exhausted its pages",
                )
            })
        }
    }

    fn manifest(generation: u64) -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: generation,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        }
    }

    fn citizen(unit_id: i32) -> CitizenRecord {
        CitizenRecord {
            unit_id,
            name: format!("Urist {unit_id}"),
            race: "dwarf".to_owned(),
            profession: 4,
            x: unit_id,
            y: 2,
            z: 3,
            alive: true,
            sane: true,
            active: true,
            visible: true,
            citizen: true,
            resident: false,
            baby: false,
            child: false,
            adult: true,
        }
    }

    fn page(generation: u64, year_tick: u32, ids: &[i32]) -> ObservationPage {
        ObservationPage {
            bridge_generation: generation,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: year_tick,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: u32::try_from(ids.len()).unwrap_or(u32::MAX),
            citizen_offset: 0,
            complete: true,
            citizens: ids.iter().copied().map(citizen).collect(),
        }
    }

    fn source(generation: u64, pages: Vec<ObservationPage>) -> ScriptedSource {
        ScriptedSource {
            manifest: manifest(generation),
            pages: pages.into(),
            calls: 0,
        }
    }

    fn config() -> LiveReadAdapterConfig {
        LiveReadAdapterConfig {
            fortress_id: FortressId::new(9),
            page_size: 64,
            max_citizens: 64,
            include_names: true,
            initial_epoch: 3,
        }
    }

    fn context(anchor: StateAnchor, capability: Capability) -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor,
            budget: WorkBudget {
                max_entities: 128,
                max_bytes: 4 * 1024 * 1024,
                max_output_tokens: 16_384,
                ..WorkBudget::CONSERVATIVE_DEFAULT
            },
            grants: vec![CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        }
    }

    fn observation_request(since: Option<ObservationCursor>) -> ObservationRequest {
        ObservationRequest {
            since,
            projection: Projection::Full,
            interest: InterestSet::default(),
            max_entities: 128,
            max_bytes: 4 * 1024 * 1024,
            max_output_tokens: 16_384,
            continuation: None,
        }
    }

    #[test]
    fn bootstrap_establishes_the_first_honest_anchor() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0, 1])]),
            config(),
        )?;
        assert!(adapter.current_anchor().is_none());
        let projection = adapter.bootstrap()?;
        assert_eq!(projection.snapshot.cursor.epoch, 3);
        assert_eq!(projection.snapshot.cursor.sequence, 0);
        assert!(projection.snapshot.hash_is_valid());
        assert_eq!(projection.snapshot.graph.entities.len(), 3);
        assert_eq!(adapter.identity().capabilities.len(), 3);
        Ok(())
    }

    #[test]
    fn unchanged_read_becomes_a_heartbeat() -> Result<()> {
        let first = page(42, 12_345, &[0, 1]);
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![first.clone(), first]),
            config(),
        )?;
        let anchor = adapter.bootstrap()?.snapshot.anchor();
        let frame = adapter.observe(
            &observation_request(Some(anchor.cursor)),
            &context(anchor, Capability::Observe),
        )?;
        assert_eq!(frame.payload, ObservationPayload::Heartbeat(anchor));
        assert_eq!(adapter.current_anchor(), Some(anchor));
        Ok(())
    }

    #[test]
    fn ordinary_change_advances_sequence_and_returns_snapshot() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(
                42,
                vec![page(42, 12_345, &[0]), page(42, 12_346, &[0, 1])],
            ),
            config(),
        )?;
        let prior = adapter.bootstrap()?.snapshot.anchor();
        let frame = adapter.observe(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
        )?;
        let ObservationPayload::Snapshot(snapshot) = frame.payload else {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "changed live read did not return a snapshot",
            ));
        };
        assert_eq!(snapshot.cursor.epoch, prior.cursor.epoch);
        assert_eq!(snapshot.cursor.sequence, prior.cursor.sequence + 1);
        assert_eq!(snapshot.graph.entities.len(), 3);
        Ok(())
    }

    #[test]
    fn bridge_restart_advances_epoch() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0]), page(43, 12_346, &[0])]),
            config(),
        )?;
        let prior = adapter.bootstrap()?.snapshot.anchor();
        adapter.source_mut().manifest = manifest(43);
        let frame = adapter.observe(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
        )?;
        let ObservationPayload::Snapshot(snapshot) = frame.payload else {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "restart did not return a full snapshot",
            ));
        };
        assert_eq!(snapshot.cursor.epoch, prior.cursor.epoch + 1);
        assert_eq!(snapshot.cursor.sequence, 0);
        assert!(frame.warnings.iter().any(|warning| warning.contains("epoch")));
        Ok(())
    }

    #[test]
    fn clock_regression_advances_epoch() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0]), page(42, 12_000, &[0])]),
            config(),
        )?;
        let prior = adapter.bootstrap()?.snapshot.anchor();
        let frame = adapter.observe(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
        )?;
        let ObservationPayload::Snapshot(snapshot) = frame.payload else {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "clock regression did not return a full snapshot",
            ));
        };
        assert_eq!(snapshot.cursor.epoch, prior.cursor.epoch + 1);
        Ok(())
    }

    #[test]
    fn world_identity_switch_fails_closed() -> Result<()> {
        let mut switched = page(42, 12_346, &[0]);
        switched.world_folder = "region2".to_owned();
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0]), switched]),
            config(),
        )?;
        let prior = adapter.bootstrap()?.snapshot.anchor();
        assert!(
            adapter
                .observe(
                    &observation_request(Some(prior.cursor)),
                    &context(prior, Capability::Observe),
                )
                .is_err()
        );
        assert_eq!(adapter.current_anchor(), Some(prior));
        Ok(())
    }

    #[test]
    fn pinned_query_returns_provenanced_rows() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0, 1])]),
            config(),
        )?;
        let anchor = adapter.bootstrap()?.snapshot.anchor();
        let response = adapter.query(
            &QueryRequest {
                anchor,
                query: WorldQuery {
                    kinds: Vec::new(),
                    predicate: None,
                    order: QueryOrder::EntityIdAscending,
                    limit: 16,
                    continuation: None,
                },
                max_output_tokens: 16_384,
                continuation: None,
            },
            &context(anchor, Capability::Query),
        )?;
        assert_eq!(response.rows.len(), 3);
        assert!(response.rows.iter().all(|row| row.evidence.len() == 1));
        Ok(())
    }

    #[test]
    fn mutation_surface_remains_absent() -> Result<()> {
        let mut adapter = LiveReadAdapter::new(
            source(42, vec![page(42, 12_345, &[0])]),
            config(),
        )?;
        let anchor = adapter.bootstrap()?.snapshot.anchor();
        assert!(
            adapter
                .checkpoint("forbidden", &context(anchor, Capability::Observe))
                .is_err()
        );
        Ok(())
    }
}
