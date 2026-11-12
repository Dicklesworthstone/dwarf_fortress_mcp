#![forbid(unsafe_code)]

//! Read-only `GameAdapter` over the isolated protocol-1.1 observation source.
//!
//! The adapter publishes citizens and the configured retained-announcement
//! suffix as one canonical snapshot. Announcement transport pages are assembled
//! transactionally before projection; a partial suffix, moving multi-page read,
//! retained-window drift, or citizen-state drift leaves the prior anchor intact.
//! This source generation is not runtime-admitted merely because this adapter
//! exists.

use std::collections::BTreeSet;

use dfmcp_core::{
    ActionId, Capability, CheckpointId, DfmcpError, Digest32, EntityId, ErrorCode, Evidence,
    EvidenceId, EvidenceKind, FortressId, ObservationCursor, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::execute_bounded_query;

use crate::{
    ActionReceipt, AdapterHealth, AdapterIdentity, BridgeManifest, CancelMode, CancelReceipt,
    CheckpointReceipt, CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus,
    LiveObservationCapsuleV1_1, LiveObservationPublicationConfigV1_1, LiveObservationSourceV1_1,
    LiveWorldProjectionV1_1, MAX_CAPSULE_CITIZENS, ObservationFrame, ObservationPayload,
    ObservationRequest, PrepareReceipt, Projection, QueryRequest, QueryResponse, QueryRow,
    RestoreReceipt, project_live_capsule_v1_1, read_publishable_observation_v1_1,
};

const LIVE_ADAPTER_V1_1_SCHEMA: &[u8] = b"dfmcp-live-read-adapter-v1-1";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadAdapterConfigV1_1 {
    pub fortress_id: FortressId,
    pub citizen_page_size: u32,
    pub max_citizens: u32,
    pub include_names: bool,
    pub announcement_after_id: i32,
    pub announcement_page_size: u32,
    pub max_total_announcements: u32,
    pub initial_epoch: u64,
}

impl LiveReadAdapterConfigV1_1 {
    pub fn validate(&self) -> Result<()> {
        if self.fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "protocol-1.1 live adapter fortress identity zero is reserved",
            ));
        }
        self.publication_config(self.max_citizens)?.validate()?;
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

    fn publication_config(
        &self,
        max_citizens: u32,
    ) -> Result<LiveObservationPublicationConfigV1_1> {
        let hard_citizens = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "capsule citizen ceiling does not fit u32",
            )
        })?;
        if max_citizens > hard_citizens {
            return Err(error(
                ErrorCode::InvalidRequest,
                "protocol-1.1 adapter citizen ceiling exceeds the capsule ceiling",
            ));
        }
        Ok(LiveObservationPublicationConfigV1_1 {
            citizen_page_size: self.citizen_page_size,
            max_citizens,
            include_names: self.include_names,
            announcement_after_id: self.announcement_after_id,
            announcement_page_size: self.announcement_page_size,
            max_total_announcements: self.max_total_announcements,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RefreshOutcome {
    prior_anchor: StateAnchor,
    current_anchor: StateAnchor,
    changed: bool,
    reset: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionIdentity {
    site_id: i32,
    world_folder: String,
    world_name: String,
    bridge_version: String,
    dfhack_version: String,
    df_version: String,
    supported_methods: BTreeSet<String>,
}

impl SessionIdentity {
    fn from_capsule(capsule: &LiveObservationCapsuleV1_1) -> Self {
        Self {
            site_id: capsule.base.site_id,
            world_folder: capsule.base.world_folder.clone(),
            world_name: capsule.base.world_name.clone(),
            bridge_version: capsule.base.bridge.bridge_version.clone(),
            dfhack_version: capsule.base.bridge.dfhack_version.clone(),
            df_version: capsule.base.bridge.df_version.clone(),
            supported_methods: capsule.base.bridge.supported_methods.clone(),
        }
    }
}

pub struct LiveReadAdapterV1_1<T> {
    source: T,
    config: LiveReadAdapterConfigV1_1,
    identity: AdapterIdentity,
    current: Option<LiveWorldProjectionV1_1>,
    last_capsule: Option<LiveObservationCapsuleV1_1>,
    epoch: u64,
    sequence: u64,
}

impl<T: LiveObservationSourceV1_1> LiveReadAdapterV1_1<T> {
    pub fn new(source: T, config: LiveReadAdapterConfigV1_1) -> Result<Self> {
        config.validate()?;
        let manifest = source.bridge_manifest_v1_1();
        manifest.validate()?;
        if manifest.bridge_version != "0.2.0" {
            return Err(error(
                ErrorCode::VersionMismatch,
                "protocol-1.1 live adapter requires bridge version 0.2.0",
            ));
        }
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
    pub const fn config(&self) -> &LiveReadAdapterConfigV1_1 {
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
    pub fn current_projection(&self) -> Option<&LiveWorldProjectionV1_1> {
        self.current.as_ref()
    }

    #[must_use]
    pub fn last_capsule(&self) -> Option<&LiveObservationCapsuleV1_1> {
        self.last_capsule.as_ref()
    }

    pub fn bootstrap(&mut self) -> Result<&LiveWorldProjectionV1_1> {
        if self.current.is_some() {
            return self.current.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "protocol-1.1 adapter lost its bootstrap projection",
                )
            });
        }
        let capsule = self.read_capsule(self.config.max_citizens)?;
        let projection = project_live_capsule_v1_1(
            &capsule,
            self.config.fortress_id,
            ObservationCursor {
                epoch: self.epoch,
                sequence: self.sequence,
            },
        )?;
        self.identity = adapter_identity(&capsule.base.bridge);
        self.last_capsule = Some(capsule);
        self.current = Some(projection);
        self.current.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "protocol-1.1 adapter bootstrap projection was not retained",
            )
        })
    }

    fn read_capsule(&mut self, max_citizens: u32) -> Result<LiveObservationCapsuleV1_1> {
        let publication = self.config.publication_config(max_citizens)?;
        read_publishable_observation_v1_1(&mut self.source, &publication)
    }

    /// Begin a new bounded announcement suffix after an explicitly acknowledged
    /// complete published batch. The publisher must be side-effect free: it
    /// admits the complete response before any cursor, capsule or anchor moves.
    /// Reading the external source is not rolled back if publication is refused.
    pub fn observe_acknowledged_announcements_with_publisher<R>(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
        expected_anchor: StateAnchor,
        expected_batch_digest: Digest32,
        acknowledged_after_id: i32,
        publish: impl FnOnce(
            &LiveWorldProjectionV1_1,
            &LiveObservationCapsuleV1_1,
            &ObservationFrame,
        ) -> Result<R>,
    ) -> Result<R> {
        self.observe_acknowledged_announcements_started(
            request,
            context,
            expected_anchor,
            expected_batch_digest,
            acknowledged_after_id,
            std::time::Instant::now(),
            publish,
        )
    }

    /// Preserve the owning request's original deadline, including session
    /// ownership acquisition, argument validation and complete response rendering.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_acknowledged_announcements_started<R>(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
        expected_anchor: StateAnchor,
        expected_batch_digest: Digest32,
        acknowledged_after_id: i32,
        started: std::time::Instant,
        publish: impl FnOnce(
            &LiveWorldProjectionV1_1,
            &LiveObservationCapsuleV1_1,
            &ObservationFrame,
        ) -> Result<R>,
    ) -> Result<R> {
        let check_deadline = || {
            if started.elapsed()
                >= std::time::Duration::from_millis(context.budget.max_wall_millis)
            {
                Err(error(
                    ErrorCode::BudgetExceeded,
                    "announcement window publication exhausted its original wall deadline",
                ))
            } else {
                Ok(())
            }
        };
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.check_context_anchor(context)?;
        check_deadline()?;
        let current = self.current_projection_required()?.snapshot.anchor();
        if expected_anchor != current {
            return Err(error(
                ErrorCode::StaleAnchor,
                "announcement acknowledgment does not name the exact published anchor",
            )
            .retryable(true));
        }
        if request.since != Some(expected_anchor.cursor) {
            return Err(error(
                ErrorCode::CursorGap,
                "announcement acknowledgment requires its exact published observation cursor",
            ));
        }
        let prior = self.current_capsule_required()?;
        let coverage = &prior.announcement_batch.coverage;
        if prior.announcement_batch.content_digest != expected_batch_digest
            || !coverage.complete_through_latest
            || acknowledged_after_id != coverage.next_after_id
            || acknowledged_after_id != coverage.latest_available_id
            || coverage.requested_after_id != self.config.announcement_after_id
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "announcement acknowledgment must match the complete published batch and its exact high-water mark",
            )
            .retryable(true));
        }
        if acknowledged_after_id < 0
            || acknowledged_after_id <= self.config.announcement_after_id
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "announcement acknowledgment must advance beyond the current configured suffix",
            ));
        }
        let prior_generation = prior.base.bridge.bridge_generation;
        let prior_identity = SessionIdentity::from_capsule(prior);
        let max_citizens = self.validate_observation_request(request, context)?;
        let mut publication = self.config.publication_config(max_citizens)?;
        publication.announcement_after_id = acknowledged_after_id;
        check_deadline()?;
        let capsule = read_publishable_observation_v1_1(&mut self.source, &publication)?;
        check_deadline()?;
        ensure_same_session_identity(&prior_identity, &capsule)?;
        let next_tick = crate::DwarfFortressClock {
            year: capsule.base.current_year,
            year_tick: capsule.base.current_year_tick,
        }
        .absolute_tick()?;
        if capsule.base.bridge.bridge_generation != prior_generation || next_tick < current.tick {
            return Err(error(
                ErrorCode::CursorGap,
                "announcement acknowledgment cannot cross a bridge or game-clock reset; open a fresh session",
            ));
        }
        let candidate_sequence = self.sequence.checked_add(1).ok_or_else(|| {
            error(
                ErrorCode::CursorGap,
                "protocol-1.1 observation sequence space is exhausted",
            )
        })?;
        let projection = project_live_capsule_v1_1(
            &capsule,
            self.config.fortress_id,
            ObservationCursor {
                epoch: self.epoch,
                sequence: candidate_sequence,
            },
        )?;
        ensure_snapshot_budget(request, &projection)?;
        let mut candidate_context = context.clone();
        candidate_context.anchor = projection.snapshot.anchor();
        candidate_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let mut warnings = vec![
            "the explicitly acknowledged announcement prefix is outside the new current projection; no complete-history or durable-retention claim is made"
                .to_owned(),
        ];
        if capsule.announcement_batch.coverage.has_gap() {
            warnings.push(
                "new announcement suffix begins after a native retained-window gap; intervening history is unknown"
                    .to_owned(),
            );
        }
        let frame = ObservationFrame {
            payload: ObservationPayload::Snapshot(projection.snapshot.clone()),
            evidence: vec![observation_evidence(
                &capsule,
                projection.snapshot.anchor(),
                None,
                "complete protocol-1.1 observation after an exact acknowledged announcement window",
            )],
            warnings,
            truncated: false,
            continuation: None,
        };
        check_deadline()?;
        let published = publish(&projection, &capsule, &frame)?;
        check_deadline()?;
        candidate_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;

        self.config.announcement_after_id = acknowledged_after_id;
        self.sequence = candidate_sequence;
        self.identity = adapter_identity(&capsule.base.bridge);
        self.last_capsule = Some(capsule);
        self.current = Some(projection);
        Ok(published)
    }

    fn current_projection_required(&self) -> Result<&LiveWorldProjectionV1_1> {
        self.current.as_ref().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "protocol-1.1 live adapter has no canonical anchor; bootstrap first",
            )
        })
    }

    fn current_capsule_required(&self) -> Result<&LiveObservationCapsuleV1_1> {
        self.last_capsule.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "protocol-1.1 adapter has a projection without its source capsule",
            )
        })
    }

    fn check_context_anchor(&self, context: &OperationContext) -> Result<()> {
        let current = self.current_projection_required()?.snapshot.anchor();
        if context.anchor != current {
            return Err(error(
                ErrorCode::StaleAnchor,
                "operation context is not pinned to the current protocol-1.1 anchor",
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
                "protocol-1.1 observation request exceeds its operation budget",
            ));
        }
        if request.continuation.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "protocol-1.1 adapter publishes a complete configured suffix and accepts no cross-call continuation",
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
                "protocol-1.1 adapter does not implement interest-filtered canonical publication",
            ));
        }
        if !matches!(
            request.projection,
            Projection::Summary
                | Projection::Entities
                | Projection::Graph
                | Projection::Events
                | Projection::Full
        ) {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                "protocol-1.1 adapter does not observe the requested projection domain",
            ));
        }
        let max_citizens = request
            .max_entities
            .saturating_sub(1)
            .min(self.config.max_citizens);
        let current_citizens = self
            .last_capsule
            .as_ref()
            .map_or(0, |capsule| capsule.base.citizen_coverage.total);
        if current_citizens > max_citizens {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "current roster contains {current_citizens} citizens but this request admits {max_citizens}"
                ),
            ));
        }
        ensure_snapshot_budget(request, self.current_projection_required()?)?;
        Ok(max_citizens)
    }

    fn refresh(
        &mut self,
        max_citizens: u32,
        request: &ObservationRequest,
    ) -> Result<RefreshOutcome> {
        let (prior_anchor, prior_tick) = {
            let projection = self.current_projection_required()?;
            (projection.snapshot.anchor(), projection.snapshot.tick)
        };
        let (prior_digest, prior_bridge_generation, prior_identity) = {
            let capsule = self.current_capsule_required()?;
            (
                capsule.content_digest,
                capsule.base.bridge.bridge_generation,
                SessionIdentity::from_capsule(capsule),
            )
        };
        let capsule = self.read_capsule(max_citizens)?;

        ensure_same_session_identity(&prior_identity, &capsule)?;
        let next_tick = crate::DwarfFortressClock {
            year: capsule.base.current_year,
            year_tick: capsule.base.current_year_tick,
        }
        .absolute_tick()?;
        let bridge_reset = capsule.base.bridge.bridge_generation != prior_bridge_generation;
        let clock_regression = next_tick < prior_tick;
        let reset = bridge_reset || clock_regression;

        if !reset && capsule.content_digest == prior_digest {
            return Ok(RefreshOutcome {
                prior_anchor,
                current_anchor: prior_anchor,
                changed: false,
                reset: false,
            });
        }

        let (candidate_epoch, candidate_sequence) = if reset {
            (
                self.epoch.checked_add(1).ok_or_else(|| {
                    error(
                        ErrorCode::CursorGap,
                        "protocol-1.1 observation epoch space is exhausted",
                    )
                })?,
                0,
            )
        } else {
            (
                self.epoch,
                self.sequence.checked_add(1).ok_or_else(|| {
                    error(
                        ErrorCode::CursorGap,
                        "protocol-1.1 observation sequence space is exhausted",
                    )
                })?,
            )
        };
        let projection = project_live_capsule_v1_1(
            &capsule,
            self.config.fortress_id,
            ObservationCursor {
                epoch: candidate_epoch,
                sequence: candidate_sequence,
            },
        )?;
        ensure_snapshot_budget(request, &projection)?;
        let current_anchor = projection.snapshot.anchor();

        self.epoch = candidate_epoch;
        self.sequence = candidate_sequence;
        self.identity = adapter_identity(&capsule.base.bridge);
        self.last_capsule = Some(capsule);
        self.current = Some(projection);
        Ok(RefreshOutcome {
            prior_anchor,
            current_anchor,
            changed: true,
            reset,
        })
    }
}

fn ensure_snapshot_budget(
    request: &ObservationRequest,
    projection: &LiveWorldProjectionV1_1,
) -> Result<()> {
    let entity_count = u32::try_from(projection.snapshot.graph.entities.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 snapshot entity count cannot be represented",
        )
    })?;
    let snapshot_bytes =
        u64::try_from(projection.snapshot.canonical_bytes().len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "protocol-1.1 snapshot byte count cannot be represented",
            )
        })?;
    if entity_count > request.max_entities || snapshot_bytes > request.max_bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "canonical protocol-1.1 snapshot exceeds the requested entity or byte bound",
        ));
    }
    Ok(())
}

fn adapter_identity(manifest: &BridgeManifest) -> AdapterIdentity {
    AdapterIdentity {
        name: "dfmcp-dfhack-live-read-v1-1".to_owned(),
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        bridge_protocol_version: "dfmcp-bridge/1.1".to_owned(),
        dwarf_fortress_version: manifest.df_version.clone(),
        dfhack_version: manifest.dfhack_version.clone(),
        compatibility: CompatibilityLevel::DegradedReadOnly,
        capabilities: BTreeSet::from([Capability::Observe, Capability::Query, Capability::Doctor]),
        schema_digest: Digest32::of_bytes(LIVE_ADAPTER_V1_1_SCHEMA),
    }
}

fn ensure_same_session_identity(
    prior: &SessionIdentity,
    next: &LiveObservationCapsuleV1_1,
) -> Result<()> {
    if prior.site_id != next.base.site_id
        || prior.world_folder != next.base.world_folder
        || prior.world_name != next.base.world_name
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "protocol-1.1 source switched world or fortress identity; open a new session",
        ));
    }
    if prior.bridge_version != next.base.bridge.bridge_version
        || prior.dfhack_version != next.base.bridge.dfhack_version
        || prior.df_version != next.base.bridge.df_version
        || prior.supported_methods != next.base.bridge.supported_methods
    {
        return Err(error(
            ErrorCode::VersionMismatch,
            "protocol-1.1 bridge or game version manifest changed; open a new session",
        ));
    }
    Ok(())
}

fn observation_evidence(
    capsule: &LiveObservationCapsuleV1_1,
    anchor: StateAnchor,
    subject: Option<EntityId>,
    summary: &str,
) -> Evidence {
    let mut identity_bytes = Vec::new();
    identity_bytes.extend_from_slice(b"dfmcp-live-observation-evidence-v1-1");
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
        format!("DFHack adapter protocol 1.1 is read-only; {operation} is not implemented"),
    ))
}

impl<T: LiveObservationSourceV1_1> GameAdapter for LiveReadAdapterV1_1<T> {
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
                "authenticated DFHack protocol 1.1 is read-only and not admitted by source presence"
                    .to_owned(),
                "announcement coverage is a retained suffix, never complete fortress history"
                    .to_owned(),
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
        let outcome = self.refresh(max_citizens, request)?;
        let projection = self.current_projection_required()?;
        let capsule = self.current_capsule_required()?;
        let current_anchor = projection.snapshot.anchor();
        if outcome.current_anchor != current_anchor {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "protocol-1.1 refresh outcome does not match the published anchor",
            ));
        }

        let (payload, mut warnings) = match request.since {
            None => (
                ObservationPayload::Snapshot(projection.snapshot.clone()),
                Vec::new(),
            ),
            Some(cursor) if cursor == current_anchor.cursor => {
                (ObservationPayload::Heartbeat(current_anchor), Vec::new())
            }
            Some(cursor) if cursor == outcome.prior_anchor.cursor => {
                let warning = if outcome.reset {
                    "observation epoch changed; returned a full protocol-1.1 snapshot"
                } else {
                    "semantic state advanced from the exact basis; returned a full protocol-1.1 snapshot"
                };
                (
                    ObservationPayload::Snapshot(projection.snapshot.clone()),
                    vec![warning.to_owned()],
                )
            }
            Some(_) => {
                return Err(error(
                    ErrorCode::CursorGap,
                    "requested cursor is not the exact current or prior protocol-1.1 basis",
                )
                .retryable(true));
            }
        };
        if outcome.changed && request.since.is_none() {
            warnings.push(if outcome.reset {
                "protocol-1.1 source continuity reset into a new observation epoch".to_owned()
            } else {
                "protocol-1.1 source advanced to a new canonical snapshot".to_owned()
            });
        }
        if capsule.announcement_batch.coverage.has_gap() {
            warnings.push(
                "announcement coverage begins after a retained-window gap; earlier history is unknown"
                    .to_owned(),
            );
        }
        Ok(ObservationFrame {
            payload,
            evidence: vec![observation_evidence(
                capsule,
                current_anchor,
                None,
                "complete transactional protocol-1.1 citizen and retained-announcement observation",
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
                "query is not pinned to the current protocol-1.1 snapshot",
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
                "query would scan more protocol-1.1 entities than its budget permits",
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
                    "query row projected from the complete protocol-1.1 capsule",
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
                "deterministic canonical protocol-1.1 world query; no relevance scoring".to_owned(),
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
        CapabilityGrant, CapabilityScope, CoverageStatus, RequestId, SessionId, WorkBudget,
    };
    use dfmcp_world::{EntityKind, QueryOrder, WorldQuery};

    use super::*;
    use crate::{
        AnnouncementBatchRecord, AnnouncementContinuity, AnnouncementCoverage, CitizenRecord,
        InterestSet, LiveAnnouncementBatch, MAX_ANNOUNCEMENTS_PER_BATCH,
        MAX_V1_1_CITIZENS_PER_PAGE, ObservationPageV1_1,
    };

    #[derive(Clone)]
    struct ScriptedSource {
        manifest: BridgeManifest,
        pages: VecDeque<ObservationPageV1_1>,
        calls: usize,
        requested_after_ids: Vec<i32>,
    }

    impl LiveObservationSourceV1_1 for ScriptedSource {
        fn bridge_manifest_v1_1(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page_v1_1(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
            announcement_after_id: i32,
            _max_announcements: u32,
        ) -> Result<ObservationPageV1_1> {
            self.calls = self.calls.saturating_add(1);
            self.requested_after_ids.push(announcement_after_id);
            self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted protocol-1.1 source exhausted its pages",
                )
            })
        }
    }

    fn manifest(generation: u64) -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.2.0".to_owned(),
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

    fn report(report_id: i32) -> AnnouncementBatchRecord {
        AnnouncementBatchRecord {
            report_id,
            report_type: 7,
            text: format!("report-{report_id}"),
            year: 105,
            year_tick: 12_000 + report_id,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn batch(
        generation: u64,
        requested_after_id: i32,
        oldest_available_id: i32,
        latest_available_id: i32,
        ids: &[i32],
        complete: bool,
        continuity: AnnouncementContinuity,
        paused: bool,
        year_tick: u32,
    ) -> Result<LiveAnnouncementBatch> {
        let records = ids.iter().copied().map(report).collect::<Vec<_>>();
        LiveAnnouncementBatch::new(
            generation,
            paused,
            105,
            year_tick,
            7,
            AnnouncementCoverage {
                requested_after_id,
                oldest_available_id,
                latest_available_id,
                returned: u32::try_from(records.len()).map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "test announcement length does not fit u32",
                    )
                })?,
                complete_through_latest: complete,
                continuity,
                next_after_id: records
                    .last()
                    .map_or(requested_after_id, |record| record.report_id),
            },
            records,
        )
    }

    fn page(
        generation: u64,
        year_tick: u32,
        citizen_ids: &[i32],
        announcements: LiveAnnouncementBatch,
    ) -> ObservationPageV1_1 {
        ObservationPageV1_1 {
            bridge_generation: generation,
            world_loaded: true,
            fortress_mode: true,
            paused: announcements.paused,
            current_year: 105,
            current_year_tick: year_tick,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: u32::try_from(citizen_ids.len()).map_or(u32::MAX, |value| value),
            citizen_offset: 0,
            complete: true,
            citizens: citizen_ids.iter().copied().map(citizen).collect(),
            announcement_batch: announcements,
        }
    }

    fn complete_page(
        generation: u64,
        year_tick: u32,
        citizen_ids: &[i32],
        report_ids: &[i32],
    ) -> Result<ObservationPageV1_1> {
        Ok(page(
            generation,
            year_tick,
            citizen_ids,
            batch(
                generation,
                -1,
                report_ids.first().copied().unwrap_or(-1),
                report_ids.last().copied().unwrap_or(-1),
                report_ids,
                true,
                AnnouncementContinuity::CompleteSuffix,
                true,
                year_tick,
            )?,
        ))
    }

    fn source(generation: u64, pages: Vec<ObservationPageV1_1>) -> ScriptedSource {
        ScriptedSource {
            manifest: manifest(generation),
            pages: pages.into(),
            calls: 0,
            requested_after_ids: Vec::new(),
        }
    }

    fn config() -> LiveReadAdapterConfigV1_1 {
        LiveReadAdapterConfigV1_1 {
            fortress_id: FortressId::new(9),
            citizen_page_size: 64,
            max_citizens: 64,
            include_names: true,
            announcement_after_id: -1,
            announcement_page_size: 64,
            max_total_announcements: 64,
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
    fn bootstrap_publishes_citizens_and_announcements_under_one_anchor() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0, 1], &[10, 11])?]),
            config(),
        )?;
        let projection = adapter.bootstrap()?;
        assert_eq!(projection.snapshot.cursor.epoch, 3);
        assert_eq!(projection.snapshot.cursor.sequence, 0);
        assert_eq!(projection.snapshot.graph.entities.len(), 5);
        assert_eq!(
            projection
                .snapshot
                .graph
                .entities
                .values()
                .filter(|entity| entity.kind == EntityKind::Announcement)
                .count(),
            2
        );
        assert!(projection.snapshot.hash_is_valid());
        Ok(())
    }

    #[test]
    fn unchanged_combined_capsule_becomes_a_heartbeat() -> Result<()> {
        let first = complete_page(42, 12_345, &[0], &[10])?;
        let mut adapter =
            LiveReadAdapterV1_1::new(source(42, vec![first.clone(), first]), config())?;
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
    fn new_announcement_advances_sequence() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(
                42,
                vec![
                    complete_page(42, 12_345, &[0], &[10])?,
                    complete_page(42, 12_346, &[0], &[10, 11])?,
                ],
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
                "changed protocol-1.1 read did not return a snapshot",
            ));
        };
        assert_eq!(snapshot.cursor.epoch, prior.cursor.epoch);
        assert_eq!(snapshot.cursor.sequence, prior.cursor.sequence + 1);
        assert_eq!(snapshot.graph.entities.len(), 4);
        Ok(())
    }

    #[test]
    fn partial_suffix_is_completed_before_bootstrap_publication() -> Result<()> {
        let first = page(
            42,
            12_345,
            &[0],
            batch(
                42,
                -1,
                10,
                12,
                &[10, 11],
                false,
                AnnouncementContinuity::CompleteSuffix,
                true,
                12_345,
            )?,
        );
        let second = page(
            42,
            12_345,
            &[0],
            batch(
                42,
                11,
                10,
                12,
                &[12],
                true,
                AnnouncementContinuity::CompleteSuffix,
                true,
                12_345,
            )?,
        );
        let mut adapter_config = config();
        adapter_config.announcement_page_size = 2;
        adapter_config.max_total_announcements = 4;
        let mut adapter =
            LiveReadAdapterV1_1::new(source(42, vec![first, second]), adapter_config)?;
        let projection = adapter.bootstrap()?;
        assert_eq!(
            projection
                .snapshot
                .graph
                .entities
                .values()
                .filter(|entity| entity.kind == EntityKind::Announcement)
                .count(),
            3
        );
        assert!(
            adapter
                .last_capsule()
                .is_some_and(|capsule| capsule.announcement_batch.coverage.complete_through_latest)
        );
        Ok(())
    }

    #[test]
    fn over_ceiling_suffix_leaves_adapter_unbootstrapped() -> Result<()> {
        let partial = page(
            42,
            12_345,
            &[0],
            batch(
                42,
                -1,
                10,
                12,
                &[10, 11],
                false,
                AnnouncementContinuity::CompleteSuffix,
                true,
                12_345,
            )?,
        );
        let mut adapter_config = config();
        adapter_config.announcement_page_size = 2;
        adapter_config.max_total_announcements = 2;
        let mut adapter = LiveReadAdapterV1_1::new(source(42, vec![partial]), adapter_config)?;
        assert!(adapter.bootstrap().is_err());
        assert!(adapter.current_anchor().is_none());
        assert!(adapter.current_projection().is_none());
        Ok(())
    }

    #[test]
    fn retained_window_gap_is_visible_but_history_is_not_upgraded() -> Result<()> {
        let retained = page(
            42,
            12_345,
            &[0],
            batch(
                42,
                1,
                10,
                11,
                &[10, 11],
                true,
                AnnouncementContinuity::GapBeforeRetainedWindow,
                true,
                12_345,
            )?,
        );
        let mut adapter_config = config();
        adapter_config.announcement_after_id = 1;
        let mut adapter =
            LiveReadAdapterV1_1::new(source(42, vec![retained.clone(), retained]), adapter_config)?;
        let anchor = adapter.bootstrap()?.snapshot.anchor();
        assert_eq!(
            adapter.current_projection().and_then(|projection| {
                projection
                    .receipt
                    .coverage()
                    .domains
                    .get("fortress.announcements.history")
                    .map(|domain| domain.status)
            }),
            Some(CoverageStatus::Partial)
        );
        let frame = adapter.observe(
            &observation_request(Some(anchor.cursor)),
            &context(anchor, Capability::Observe),
        )?;
        assert!(frame.warnings.iter().any(|warning| warning.contains("gap")));
        Ok(())
    }

    #[test]
    fn bridge_restart_advances_epoch() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(
                42,
                vec![
                    complete_page(42, 12_345, &[0], &[10])?,
                    complete_page(43, 12_346, &[0], &[10])?,
                ],
            ),
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
                "bridge restart did not return a full snapshot",
            ));
        };
        assert_eq!(snapshot.cursor.epoch, prior.cursor.epoch + 1);
        assert_eq!(snapshot.cursor.sequence, 0);
        Ok(())
    }

    #[test]
    fn candidate_over_budget_does_not_advance_anchor() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(
                42,
                vec![
                    complete_page(42, 12_345, &[0], &[10])?,
                    complete_page(42, 12_346, &[0], &[10, 11])?,
                ],
            ),
            config(),
        )?;
        let prior = adapter.bootstrap()?.snapshot.anchor();
        let mut request = observation_request(Some(prior.cursor));
        request.max_entities = 2;
        let mut operation = context(prior, Capability::Observe);
        operation.budget.max_entities = 2;
        assert!(adapter.observe(&request, &operation).is_err());
        assert_eq!(adapter.current_anchor(), Some(prior));
        Ok(())
    }

    #[test]
    fn pinned_query_returns_announcement_entities_with_evidence() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0], &[10, 11])?]),
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
        assert_eq!(response.rows.len(), 4);
        assert!(response.rows.iter().all(|row| row.evidence.len() == 1));
        assert!(response.rows.iter().any(|row| {
            row.fields
                .iter()
                .any(|(key, value)| key == "kind" && value == "announcement")
        }));
        Ok(())
    }

    #[test]
    fn mutation_surface_remains_absent() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0], &[10])?]),
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

    #[test]
    fn hard_configuration_bounds_are_rejected() {
        let mut invalid = config();
        invalid.citizen_page_size = MAX_V1_1_CITIZENS_PER_PAGE.saturating_add(1);
        assert!(invalid.validate().is_err());

        let mut invalid = config();
        invalid.max_total_announcements = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH)
            .map_or(u32::MAX, |value| value)
            .saturating_add(1);
        assert!(invalid.validate().is_err());
    }

    fn acknowledgment_basis(
        adapter: &LiveReadAdapterV1_1<ScriptedSource>,
    ) -> Result<(StateAnchor, Digest32, i32)> {
        let anchor = adapter.current_projection_required()?.snapshot.anchor();
        let batch = &adapter.current_capsule_required()?.announcement_batch;
        Ok((anchor, batch.content_digest, batch.coverage.next_after_id))
    }

    fn suffix_page(
        generation: u64,
        year_tick: u32,
        after: i32,
        oldest: i32,
        latest: i32,
        ids: &[i32],
        continuity: AnnouncementContinuity,
    ) -> Result<ObservationPageV1_1> {
        Ok(page(
            generation,
            year_tick,
            &[0],
            batch(
                generation, after, oldest, latest, ids, true, continuity, true, year_tick,
            )?,
        ))
    }

    #[test]
    fn acknowledged_window_recovers_a_full_suffix_without_silently_skipping_reports() -> Result<()> {
        let initial = complete_page(42, 12_345, &[0], &[10, 11])?;
        let overflowing = page(
            42,
            12_346,
            &[0],
            batch(
                42, -1, 10, 13, &[10, 11], false,
                AnnouncementContinuity::CompleteSuffix, true, 12_346,
            )?,
        );
        let next = suffix_page(
            42, 12_346, 11, 10, 13, &[12, 13], AnnouncementContinuity::CompleteSuffix,
        )?;
        let mut limits = config();
        limits.announcement_page_size = 2;
        limits.max_total_announcements = 2;
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![initial, overflowing, next.clone(), next]),
            limits,
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let request = observation_request(Some(prior.cursor));
        let operation = context(prior, Capability::Observe);
        assert!(adapter.observe(&request, &operation).is_err());
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        let published_anchor = adapter.observe_acknowledged_announcements_with_publisher(
            &request, &operation, prior, digest, high_water,
            |projection, capsule, frame| {
                assert_eq!(capsule.announcement_batch.coverage.requested_after_id, 11);
                assert_eq!(capsule.announcement_batch.announcements.iter()
                    .map(|report| report.report_id).collect::<Vec<_>>(), vec![12, 13]);
                assert!(matches!(&frame.payload, ObservationPayload::Snapshot(_)));
                assert!(frame.warnings.iter().any(|warning| warning.contains("acknowledged")));
                Ok(projection.snapshot.anchor())
            },
        )?;
        assert_eq!(adapter.config().announcement_after_id, 11);
        assert_eq!(published_anchor.cursor.sequence, prior.cursor.sequence + 1);
        assert_eq!(adapter.current_anchor(), Some(published_anchor));
        let heartbeat = adapter.observe(
            &observation_request(Some(published_anchor.cursor)),
            &context(published_anchor, Capability::Observe),
        )?;
        assert_eq!(heartbeat.payload, ObservationPayload::Heartbeat(published_anchor));
        assert_eq!(adapter.source().requested_after_ids, vec![-1, -1, 11, 11]);
        Ok(())
    }

    #[test]
    fn acknowledgment_rejects_foreign_stale_or_unpublished_values_before_reading() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0], &[10, 11])?]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let mut foreign = prior;
        foreign.fortress_id = FortressId::new(10);
        let mut stale = prior;
        stale.cursor.sequence = stale.cursor.sequence.saturating_add(1);
        for (anchor, batch, cursor) in [
            (foreign, digest, high_water),
            (stale, digest, high_water),
            (prior, Digest32::of_bytes(b"unpublished"), high_water),
            (prior, digest, high_water - 1),
            (prior, digest, high_water + 1),
        ] {
            assert!(adapter.observe_acknowledged_announcements_with_publisher(
                &observation_request(Some(prior.cursor)),
                &context(prior, Capability::Observe),
                anchor, batch, cursor, |_, _, _| Ok(()),
            ).is_err());
        }
        assert_eq!(adapter.source().calls, 1);
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        assert_eq!(adapter.config().announcement_after_id, -1);
        Ok(())
    }

    #[test]
    fn rejected_complete_packet_preserves_the_original_acknowledgment_for_retry() -> Result<()> {
        let next = suffix_page(
            42, 12_346, 11, 10, 12, &[12], AnnouncementContinuity::CompleteSuffix,
        )?;
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![
                complete_page(42, 12_345, &[0], &[10, 11])?,
                next.clone(), next,
            ]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let request = observation_request(Some(prior.cursor));
        let operation = context(prior, Capability::Observe);
        let failure = adapter.observe_acknowledged_announcements_with_publisher(
            &request, &operation, prior, digest, high_water,
            |_, _, _| Err::<(), _>(error(ErrorCode::BudgetExceeded, "complete packet does not fit")),
        );
        assert!(failure.is_err());
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        assert_eq!(adapter.config().announcement_after_id, -1);
        adapter.observe_acknowledged_announcements_with_publisher(
            &request, &operation, prior, digest, high_water, |_, _, _| Ok(()),
        )?;
        assert_eq!(adapter.config().announcement_after_id, 11);
        let calls = adapter.source().calls;
        assert!(adapter.observe_acknowledged_announcements_with_publisher(
            &request, &operation, prior, digest, high_water, |_, _, _| Ok(()),
        ).is_err());
        assert_eq!(adapter.source().calls, calls);
        Ok(())
    }

    #[test]
    fn acknowledgment_checks_expiry_at_the_candidate_tick_before_publication() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![
                complete_page(42, 12_345, &[0], &[10, 11])?,
                suffix_page(42, 12_346, 11, 10, 12, &[12],
                    AnnouncementContinuity::CompleteSuffix)?,
            ]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let mut operation = context(prior, Capability::Observe);
        operation.grants[0].expires_at_tick = Some(prior.tick);
        let mut published = false;
        assert!(adapter.observe_acknowledged_announcements_with_publisher(
            &observation_request(Some(prior.cursor)), &operation, prior, digest, high_water,
            |_, _, _| { published = true; Ok(()) },
        ).is_err());
        assert!(!published);
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        assert_eq!(adapter.config().announcement_after_id, -1);
        Ok(())
    }

    #[test]
    fn acknowledgment_preserves_source_on_candidate_budget_failure() -> Result<()> {
        let candidate = page(
            42, 12_346, &[0, 1, 2],
            batch(42, 11, 10, 12, &[12], true,
                AnnouncementContinuity::CompleteSuffix, true, 12_346)?,
        );
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0], &[10, 11])?, candidate]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let mut request = observation_request(Some(prior.cursor));
        request.max_entities = 4;
        let mut operation = context(prior, Capability::Observe);
        operation.budget.max_entities = 4;
        assert!(adapter.observe_acknowledged_announcements_with_publisher(
            &request, &operation, prior, digest, high_water, |_, _, _| Ok(()),
        ).is_err());
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        assert_eq!(adapter.config().announcement_after_id, -1);
        Ok(())
    }

    #[test]
    fn acknowledgment_never_carries_a_report_cursor_across_source_resets() -> Result<()> {
        for (generation, tick) in [(43, 12_346), (42, 12_344)] {
            let mut adapter = LiveReadAdapterV1_1::new(
                source(42, vec![
                    complete_page(42, 12_345, &[0], &[10, 11])?,
                    suffix_page(generation, tick, 11, 10, 12, &[12],
                        AnnouncementContinuity::CompleteSuffix)?,
                ]),
                config(),
            )?;
            adapter.bootstrap()?;
            let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
            adapter.source_mut().manifest = manifest(generation);
            assert!(adapter.observe_acknowledged_announcements_with_publisher(
                &observation_request(Some(prior.cursor)),
                &context(prior, Capability::Observe),
                prior, digest, high_water, |_, _, _| Ok(()),
            ).is_err());
            assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
            assert_eq!(adapter.config().announcement_after_id, -1);
        }
        Ok(())
    }

    #[test]
    fn acknowledged_prefix_and_later_native_retention_gap_remain_distinct() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![
                complete_page(42, 12_345, &[0], &[10, 11])?,
                suffix_page(42, 12_346, 11, 20, 21, &[20, 21],
                    AnnouncementContinuity::GapBeforeRetainedWindow)?,
            ]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        adapter.observe_acknowledged_announcements_with_publisher(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
            prior, digest, high_water,
            |projection, capsule, frame| {
                assert!(capsule.announcement_batch.coverage.has_gap());
                assert!(frame.warnings.iter().any(|warning| warning.contains("intervening")));
                assert_eq!(projection.receipt.coverage().domains
                    .get("fortress.announcements.history").map(|domain| domain.status),
                    Some(CoverageStatus::Partial));
                Ok(())
            },
        )?;
        Ok(())
    }

    #[test]
    fn empty_acknowledged_suffix_is_valid_but_cannot_be_acknowledged_twice() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![
                complete_page(42, 12_345, &[0], &[10, 11])?,
                suffix_page(42, 12_346, 11, 10, 11, &[],
                    AnnouncementContinuity::CompleteSuffix)?,
            ]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        adapter.observe_acknowledged_announcements_with_publisher(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
            prior, digest, high_water, |_, _, _| Ok(()),
        )?;
        let (current, batch, after) = acknowledgment_basis(&adapter)?;
        assert_eq!(after, 11);
        assert!(adapter.current_capsule_required()?.announcement_batch.announcements.is_empty());
        let calls = adapter.source().calls;
        assert!(adapter.observe_acknowledged_announcements_with_publisher(
            &observation_request(Some(current.cursor)),
            &context(current, Capability::Observe),
            current, batch, after, |_, _, _| Ok(()),
        ).is_err());
        assert_eq!(adapter.source().calls, calls);
        Ok(())
    }

    #[test]
    fn first_report_id_zero_can_be_explicitly_acknowledged() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![
                complete_page(42, 12_345, &[0], &[0])?,
                suffix_page(42, 12_346, 0, 0, 1, &[1],
                    AnnouncementContinuity::CompleteSuffix)?,
            ]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        assert_eq!(high_water, 0);
        adapter.observe_acknowledged_announcements_with_publisher(
            &observation_request(Some(prior.cursor)),
            &context(prior, Capability::Observe),
            prior, digest, high_water, |_, _, _| Ok(()),
        )?;
        assert_eq!(adapter.config().announcement_after_id, 0);
        assert_eq!(adapter.source().requested_after_ids, vec![-1, 0]);
        assert_eq!(acknowledgment_basis(&adapter)?.2, 1);
        Ok(())
    }

    #[test]
    fn original_request_deadline_refuses_before_reading_or_advancing() -> Result<()> {
        let mut adapter = LiveReadAdapterV1_1::new(
            source(42, vec![complete_page(42, 12_345, &[0], &[10, 11])?]),
            config(),
        )?;
        adapter.bootstrap()?;
        let (prior, digest, high_water) = acknowledgment_basis(&adapter)?;
        let mut operation = context(prior, Capability::Observe);
        operation.budget.max_wall_millis = 1;
        let started = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "fixture deadline overflow"))?;
        let mut published = false;
        let outcome = adapter.observe_acknowledged_announcements_started(
            &observation_request(Some(prior.cursor)),
            &operation,
            prior, digest, high_water, started,
            |_, _, _| {
                published = true;
                Ok(())
            },
        );
        assert!(matches!(outcome, Err(failure) if failure.code == ErrorCode::BudgetExceeded));
        assert!(!published);
        assert_eq!(adapter.source().calls, 1);
        assert_eq!(adapter.config().announcement_after_id, -1);
        assert_eq!(acknowledgment_basis(&adapter)?, (prior, digest, high_water));
        Ok(())
    }

}
