#![forbid(unsafe_code)]

//! Read-only [`GameAdapter`](crate::GameAdapter) backed by the authenticated
//! DFHack bridge protocol.
//!
//! The adapter has no mutation implementation and advertises only `Observe`,
//! `Query`-independent health diagnostics, and `Doctor`. A complete bridge page
//! set is assembled and projected into canonical world state before it becomes
//! observable. Unchanged canonical capsules produce heartbeats at the existing
//! anchor; semantic changes advance the session-owned observation sequence.

use std::collections::BTreeSet;

use dfmcp_core::{
    ActionId, Capability, CheckpointId, DfmcpError, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::WorldSnapshot;

use crate::live_projection::{LiveProjectionContext, project_live_observation};
use crate::{
    ActionReceipt, AdapterHealth, AdapterIdentity, BridgeManifest, CancelMode, CancelReceipt,
    CheckpointReceipt, CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus,
    LiveObservationCapsule, LiveObservationSource, MAX_CAPSULE_CITIZENS,
    MAX_CITIZENS_PER_PAGE, ObservationAssembler, ObservationFrame, ObservationPayload,
    ObservationRequest, PrepareReceipt, Projection, QueryRequest, QueryResponse, RestoreReceipt,
};

pub const DWARF_FORTRESS_TICKS_PER_YEAR: u64 = 403_200;
const FIRST_OBSERVATION_SEQUENCE: u64 = 0;
const MAX_LIVE_OBSERVATION_PAGES: u32 = 100_001;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadAdapterConfig {
    pub fortress_id: FortressId,
    pub observation_epoch: u64,
    pub page_size: u32,
    pub include_names: bool,
    pub expected_site_id: Option<i32>,
}

impl LiveReadAdapterConfig {
    pub fn validate(&self) -> Result<()> {
        if self.fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live read adapter fortress lineage must not be zero",
            ));
        }
        if self.page_size == 0 || self.page_size > MAX_CITIZENS_PER_PAGE {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "live bridge page_size must be in 1..={MAX_CITIZENS_PER_PAGE}"
                ),
            ));
        }
        if self.expected_site_id.is_some_and(|site| site < 0) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "expected site ID must be nonnegative when present",
            ));
        }
        Ok(())
    }
}

pub struct LiveReadAdapter<T> {
    source: T,
    config: LiveReadAdapterConfig,
    manifest: BridgeManifest,
    identity: AdapterIdentity,
    current_sequence: Option<u64>,
    last_capsule_digest: Option<Digest32>,
    snapshot: Option<WorldSnapshot>,
}

impl<T: LiveObservationSource> LiveReadAdapter<T> {
    pub fn new(source: T, config: LiveReadAdapterConfig) -> Result<Self> {
        config.validate()?;
        let manifest = source.bridge_manifest();
        if !manifest.world_loaded || !manifest.fortress_mode {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "DFHack handshake does not report a loaded fortress-mode world",
            ));
        }
        let identity = AdapterIdentity {
            name: "dfhack-read-only-v1".to_owned(),
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
                Capability::Doctor,
            ]),
            schema_digest: Digest32::of_bytes(b"dfmcp-live-read-adapter-schema-v1"),
        };
        Ok(Self {
            source,
            config,
            manifest,
            identity,
            current_sequence: None,
            last_capsule_digest: None,
            snapshot: None,
        })
    }

    #[must_use]
    pub fn manifest(&self) -> &BridgeManifest {
        &self.manifest
    }

    #[must_use]
    pub fn snapshot(&self) -> Option<&WorldSnapshot> {
        self.snapshot.as_ref()
    }

    #[must_use]
    pub fn source(&self) -> &T {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut T {
        &mut self.source
    }

    fn validate_observation_request(
        &self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<u32> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if request.max_entities == 0
            || request.max_bytes == 0
            || request.max_output_tokens == 0
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live observation request bounds must all be positive",
            ));
        }
        if request.max_entities > context.budget.max_entities
            || request.max_bytes > context.budget.max_bytes
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "live observation request exceeds the authorized operation budget",
            ));
        }
        if request.continuation.is_some() {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                "live read V1 does not yet accept client continuations; bridge pagination is internal and atomic",
            ));
        }
        if !request.interest.entity_ids.is_empty()
            || !request.interest.entity_kinds.is_empty()
            || !request.interest.fields.is_empty()
            || !request.interest.map_areas.is_empty()
            || !request.interest.event_kinds.is_empty()
        {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                "live read V1 requires an empty interest set; selective canonical publication is not implemented",
            ));
        }
        if !matches!(
            request.projection,
            Projection::Summary | Projection::Entities | Projection::Full
        ) {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                "live read V1 supports summary, entities, and full projections only",
            ));
        }
        if let Some(since) = request.since {
            if since.epoch != self.config.observation_epoch {
                return Err(error(
                    ErrorCode::CursorGap,
                    format!(
                        "requested epoch {} does not match live adapter epoch {}",
                        since.epoch, self.config.observation_epoch
                    ),
                ));
            }
            if self
                .current_sequence
                .is_some_and(|current| since.sequence > current)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "requested observation sequence is ahead of the live adapter head",
                ));
            }
        }
        Ok(self.config.page_size.min(request.max_entities))
    }

    fn read_complete_capsule(
        &mut self,
        page_size: u32,
        max_entities: u32,
        max_bytes: u64,
    ) -> Result<LiveObservationCapsule> {
        let max_capsule_citizens = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "capsule citizen ceiling does not fit u32",
            )
        })?;
        let citizen_limit = max_entities.saturating_sub(1).min(max_capsule_citizens);
        if citizen_limit == 0 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "live observation budget must admit the fortress entity and at least one citizen slot",
            ));
        }

        let mut assembler = ObservationAssembler::new(self.manifest.clone());
        for _ in 0..MAX_LIVE_OBSERVATION_PAGES {
            let offset = assembler.next_offset();
            let page = self.source.read_observation_page(
                offset,
                page_size.min(citizen_limit),
                self.config.include_names,
            )?;
            if page.citizen_count_total > citizen_limit {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    format!(
                        "live fortress has {} citizens but this request admits at most {citizen_limit}",
                        page.citizen_count_total
                    ),
                ));
            }
            if page.citizens.is_empty() && !page.complete {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "DFHack bridge returned an empty nonterminal citizen page",
                ));
            }
            assembler.push_page(page)?;
            if assembler.is_complete() {
                let capsule = assembler.finalize()?;
                let capsule_bytes = u64::try_from(capsule.canonical_bytes.len()).map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "live capsule byte length does not fit u64",
                    )
                })?;
                if capsule_bytes > max_bytes {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        format!(
                            "canonical live capsule uses {capsule_bytes} bytes, exceeding the request ceiling {max_bytes}"
                        ),
                    ));
                }
                return Ok(capsule);
            }
        }
        Err(error(
            ErrorCode::BudgetExceeded,
            "live observation exceeded the maximum admitted page count",
        ))
    }

    fn next_sequence_for(&self, digest: Digest32) -> Result<u64> {
        match (self.current_sequence, self.last_capsule_digest) {
            (None, None) => Ok(FIRST_OBSERVATION_SEQUENCE),
            (Some(sequence), Some(previous)) if previous == digest => Ok(sequence),
            (Some(sequence), Some(_)) => sequence.checked_add(1).ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "live observation sequence space is exhausted",
                )
            }),
            _ => Err(error(
                ErrorCode::InternalInvariantViolation,
                "live adapter sequence and capsule digest lost lockstep",
            )),
        }
    }

    fn absolute_game_tick(capsule: &LiveObservationCapsule) -> Result<GameTick> {
        let within_year = u64::from(capsule.current_year_tick);
        if within_year >= DWARF_FORTRESS_TICKS_PER_YEAR {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "DFHack current_year_tick {within_year} exceeds the calendar bound {}",
                    DWARF_FORTRESS_TICKS_PER_YEAR - 1
                ),
            ));
        }
        let year_base = u64::from(capsule.current_year)
            .checked_mul(DWARF_FORTRESS_TICKS_PER_YEAR)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "Dwarf Fortress absolute game tick overflows u64",
                )
            })?;
        let absolute = year_base.checked_add(within_year).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "Dwarf Fortress absolute game tick overflows u64",
            )
        })?;
        Ok(GameTick::new(absolute))
    }

    fn publish_capsule(&mut self, capsule: LiveObservationCapsule) -> Result<(bool, StateAnchor)> {
        if capsule.bridge != self.manifest {
            return Err(error(
                ErrorCode::StaleAnchor,
                "bridge manifest changed after live adapter construction",
            ));
        }
        let sequence = self.next_sequence_for(capsule.content_digest)?;
        if self.last_capsule_digest == Some(capsule.content_digest) {
            let anchor = self
                .snapshot
                .as_ref()
                .map(WorldSnapshot::anchor)
                .ok_or_else(|| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "unchanged live capsule has no published snapshot",
                    )
                })?;
            return Ok((false, anchor));
        }

        let site_id = self.config.expected_site_id.or(Some(capsule.site_id));
        let snapshot = project_live_observation(
            &capsule,
            LiveProjectionContext {
                fortress_id: self.config.fortress_id,
                cursor: ObservationCursor {
                    epoch: self.config.observation_epoch,
                    sequence,
                },
                observed_at: Self::absolute_game_tick(&capsule)?,
                expected_site_id: site_id,
            },
        )?;
        let anchor = snapshot.anchor();
        self.config.expected_site_id = Some(capsule.site_id);
        self.current_sequence = Some(sequence);
        self.last_capsule_digest = Some(capsule.content_digest);
        self.snapshot = Some(snapshot);
        Ok((true, anchor))
    }
}

fn read_only<T>(operation: &str) -> Result<T> {
    Err(error(
        ErrorCode::CapabilityDenied,
        format!(
            "live DFHack protocol V1 is read-only; {operation} has no registered mutation route"
        ),
    ))
}

impl<T: LiveObservationSource> GameAdapter for LiveReadAdapter<T> {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn current_anchor(&self) -> Option<StateAnchor> {
        self.snapshot.as_ref().map(WorldSnapshot::anchor)
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth> {
        context.authorize(Capability::Doctor, RiskTier::ReadOnly, &[], None)?;
        Ok(AdapterHealth {
            status: HealthStatus::ReadOnly,
            identity: self.identity.clone(),
            fortress_loaded: self.manifest.world_loaded && self.manifest.fortress_mode,
            paused: self.snapshot.as_ref().map(|snapshot| snapshot.paused),
            current_anchor: self.current_anchor(),
            warnings: vec![
                "authenticated DFHack bridge protocol V1 is read-only".to_owned(),
                "live compatibility remains degraded until the named-version acceptance matrix passes"
                    .to_owned(),
            ],
        })
    }

    fn observe(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<ObservationFrame> {
        let page_size = self.validate_observation_request(request, context)?;
        let capsule = self.read_complete_capsule(
            page_size,
            request.max_entities,
            request.max_bytes,
        )?;
        let (changed, anchor) = self.publish_capsule(capsule)?;
        if !changed && request.since == Some(anchor.cursor) {
            return Ok(ObservationFrame {
                payload: ObservationPayload::Heartbeat(anchor),
                evidence: Vec::new(),
                warnings: vec![
                    "live capsule digest is unchanged at the requested anchor".to_owned(),
                ],
                truncated: false,
                continuation: None,
            });
        }
        let snapshot = self.snapshot.clone().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live capsule publication completed without a snapshot",
            )
        })?;
        let snapshot_bytes = u64::try_from(snapshot.canonical_bytes().len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "projected snapshot byte length does not fit u64",
            )
        })?;
        if snapshot_bytes > request.max_bytes {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "projected world snapshot uses {snapshot_bytes} bytes, exceeding the request ceiling {}",
                    request.max_bytes
                ),
            ));
        }
        Ok(ObservationFrame {
            payload: ObservationPayload::Snapshot(snapshot),
            evidence: Vec::new(),
            warnings: vec![
                "live read V1 publishes a complete fortress-and-citizen snapshot; semantic deltas are not yet admitted"
                    .to_owned(),
            ],
            truncated: false,
            continuation: None,
        })
    }

    fn query(
        &mut self,
        _request: &QueryRequest,
        context: &OperationContext,
    ) -> Result<QueryResponse> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Err(error(
            ErrorCode::CompatibilityUnknown,
            "live read adapter query projection is not yet admitted; query the published canonical world through the world engine",
        ))
    }

    fn prepare(
        &mut self,
        _plan: &PreparedPlan,
        _context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        read_only("prepare")
    }

    fn commit(
        &mut self,
        _plan: &PreparedPlan,
        _prepared: &PrepareReceipt,
        _context: &OperationContext,
    ) -> Result<CommitReceipt> {
        read_only("commit")
    }

    fn poll_action(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<ActionReceipt> {
        read_only("action polling")
    }

    fn request_cancel(
        &mut self,
        _action_id: ActionId,
        _mode: CancelMode,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        read_only("cancellation")
    }

    fn finalize_cancel(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        read_only("cancellation finalization")
    }

    fn checkpoint(
        &mut self,
        _label: &str,
        _context: &OperationContext,
    ) -> Result<CheckpointReceipt> {
        read_only("checkpoint")
    }

    fn restore(
        &mut self,
        _checkpoint_id: CheckpointId,
        _context: &OperationContext,
    ) -> Result<RestoreReceipt> {
        read_only("restore")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget,
    };

    use super::*;
    use crate::{CitizenRecord, InterestSet, ObservationPage};

    #[derive(Clone)]
    struct ScriptedSource {
        manifest: BridgeManifest,
        pages: Vec<ObservationPage>,
        cursor: usize,
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
            let page = self.pages.get(self.cursor).cloned().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted live source exhausted its pages",
                )
            })?;
            self.cursor = self.cursor.saturating_add(1);
            Ok(page)
        }
    }

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: 42,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        }
    }

    fn citizen(id: i32) -> CitizenRecord {
        CitizenRecord {
            unit_id: id,
            name: format!("Urist {id}"),
            race: "dwarf".to_owned(),
            profession: 4,
            x: id,
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

    fn page(ids: &[i32]) -> ObservationPage {
        ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: u32::try_from(ids.len()).unwrap_or(u32::MAX),
            citizen_offset: 0,
            complete: true,
            citizens: ids.iter().copied().map(citizen).collect(),
        }
    }

    fn config() -> LiveReadAdapterConfig {
        LiveReadAdapterConfig {
            fortress_id: FortressId::new(77),
            observation_epoch: 3,
            page_size: 256,
            include_names: true,
            expected_site_id: Some(7),
        }
    }

    fn context(capability: Capability) -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: FortressId::new(77),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick::new(0),
                state_hash: Digest32::ZERO,
            },
            budget: WorkBudget {
                max_entities: 100,
                max_bytes: 1_000_000,
                max_output_tokens: 10_000,
                ..WorkBudget::CONSERVATIVE_DEFAULT
            },
            grants: vec![CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(FortressId::new(77)),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        }
    }

    fn request(since: Option<ObservationCursor>) -> ObservationRequest {
        ObservationRequest {
            since,
            projection: Projection::Full,
            interest: InterestSet::default(),
            max_entities: 100,
            max_bytes: 1_000_000,
            max_output_tokens: 10_000,
            continuation: None,
        }
    }

    #[test]
    fn first_live_read_publishes_a_canonical_snapshot() -> Result<()> {
        let source = ScriptedSource {
            manifest: manifest(),
            pages: vec![page(&[1, 2])],
            cursor: 0,
        };
        let mut adapter = LiveReadAdapter::new(source, config())?;
        let frame = adapter.observe(&request(None), &context(Capability::Observe))?;
        match frame.payload {
            ObservationPayload::Snapshot(snapshot) => {
                assert_eq!(snapshot.cursor.epoch, 3);
                assert_eq!(snapshot.cursor.sequence, 0);
                assert_eq!(snapshot.graph.entities.len(), 3);
                assert!(snapshot.hash_is_valid());
            }
            _ => {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "first live read did not publish a snapshot",
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn identical_second_read_becomes_a_heartbeat() -> Result<()> {
        let source = ScriptedSource {
            manifest: manifest(),
            pages: vec![page(&[1, 2]), page(&[1, 2])],
            cursor: 0,
        };
        let mut adapter = LiveReadAdapter::new(source, config())?;
        let first = adapter.observe(&request(None), &context(Capability::Observe))?;
        let anchor = match first.payload {
            ObservationPayload::Snapshot(snapshot) => snapshot.anchor(),
            _ => {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "first read did not return a snapshot",
                ));
            }
        };
        let second = adapter.observe(
            &request(Some(anchor.cursor)),
            &context(Capability::Observe),
        )?;
        assert!(matches!(second.payload, ObservationPayload::Heartbeat(value) if value == anchor));
        Ok(())
    }

    #[test]
    fn mutation_methods_are_structurally_unavailable() -> Result<()> {
        let source = ScriptedSource {
            manifest: manifest(),
            pages: Vec::new(),
            cursor: 0,
        };
        let mut adapter = LiveReadAdapter::new(source, config())?;
        let result: Result<CheckpointReceipt> =
            adapter.checkpoint("not-allowed", &context(Capability::Doctor));
        let failure = result.err().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "read-only adapter unexpectedly accepted checkpoint",
            )
        })?;
        assert_eq!(failure.code, ErrorCode::CapabilityDenied);
        Ok(())
    }

    #[test]
    fn observation_requires_explicit_observe_authority() -> Result<()> {
        let source = ScriptedSource {
            manifest: manifest(),
            pages: vec![page(&[1])],
            cursor: 0,
        };
        let mut adapter = LiveReadAdapter::new(source, config())?;
        assert!(
            adapter
                .observe(&request(None), &context(Capability::Doctor))
                .is_err()
        );
        Ok(())
    }
}
