#![forbid(unsafe_code)]

//! Single-read bootstrap for the canonical live adapter.
//!
//! A live fortress ID depends on the first complete observation, while
//! [`LiveReadAdapter`](crate::LiveReadAdapter) needs that ID in its immutable
//! configuration. Reading twice would be expensive and could bind the identity
//! to one game state while publishing another. This module reads once, derives
//! the identity, then replays that exact verified capsule through a temporary
//! page source so the ordinary adapter bootstrap path remains the sole state
//! transition implementation.

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BridgeManifest, LiveObservationCapsule, LiveObservationSource, LiveReadAdapter,
    LiveReadAdapterConfig, MAX_CAPSULE_CITIZENS, MAX_CITIZENS_PER_PAGE, ObservationPage,
    derive_live_fortress_id, read_complete_observation_bounded,
};

pub const DEFAULT_MAX_LIVE_CITIZENS: u32 = 100_000;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadBootstrapConfig {
    pub page_size: u32,
    pub max_citizens: u32,
    pub include_names: bool,
    pub initial_epoch: u64,
}

impl LiveReadBootstrapConfig {
    pub fn validate(&self) -> Result<()> {
        if self.page_size == 0 || self.page_size > MAX_CITIZENS_PER_PAGE {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "live bootstrap page size must be in 1..={MAX_CITIZENS_PER_PAGE}"
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
                    "live bootstrap citizen ceiling {} exceeds {hard_total}",
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

impl Default for LiveReadBootstrapConfig {
    fn default() -> Self {
        Self {
            page_size: MAX_CITIZENS_PER_PAGE,
            max_citizens: DEFAULT_MAX_LIVE_CITIZENS,
            include_names: true,
            initial_epoch: 0,
        }
    }
}

pub struct PrimedLiveSource<T> {
    source: T,
    primed: Option<LiveObservationCapsule>,
    next_offset: u32,
}

impl<T: LiveObservationSource> PrimedLiveSource<T> {
    pub fn new(source: T, primed: LiveObservationCapsule) -> Result<Self> {
        primed.validate()?;
        let manifest = source.bridge_manifest();
        manifest.validate()?;
        if manifest != primed.bridge {
            return Err(error(
                ErrorCode::StaleAnchor,
                "source manifest changed between the first capsule and adapter bootstrap",
            ));
        }
        Ok(Self {
            source,
            primed: Some(primed),
            next_offset: 0,
        })
    }

    #[must_use]
    pub const fn source(&self) -> &T {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut T {
        &mut self.source
    }

    #[must_use]
    pub const fn has_primed_capsule(&self) -> bool {
        self.primed.is_some()
    }

    pub fn into_inner(self) -> Result<T> {
        if self.primed.is_some() {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "cannot extract a primed source before bootstrap consumes its capsule",
            ));
        }
        Ok(self.source)
    }
}

impl<T: LiveObservationSource> LiveObservationSource for PrimedLiveSource<T> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.primed
            .as_ref()
            .map_or_else(|| self.source.bridge_manifest(), |capsule| capsule.bridge.clone())
    }

    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
    ) -> Result<ObservationPage> {
        let Some(capsule) = self.primed.as_ref() else {
            return self
                .source
                .read_observation_page(offset, maximum, include_names);
        };
        if maximum == 0 || maximum > MAX_CITIZENS_PER_PAGE {
            return Err(error(
                ErrorCode::InvalidRequest,
                "primed replay page size is outside the bridge V1 bound",
            ));
        }
        if include_names != capsule.names_included {
            return Err(error(
                ErrorCode::InvalidRequest,
                "primed replay projection does not match the verified source capsule",
            ));
        }
        if offset != self.next_offset {
            return Err(error(
                ErrorCode::CursorGap,
                format!(
                    "primed replay received offset {offset}, expected {}",
                    self.next_offset
                ),
            ));
        }

        let total = capsule.citizen_coverage.total;
        let start = usize::try_from(offset.min(total)).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "primed replay offset cannot be represented",
            )
        })?;
        let end_u32 = offset.saturating_add(maximum).min(total);
        let end = usize::try_from(end_u32).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "primed replay end offset cannot be represented",
            )
        })?;
        let citizens = capsule.citizens.get(start..end).ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "primed capsule coverage does not match its citizen vector",
            )
        })?;
        let complete = end_u32 == total;
        let page = ObservationPage {
            bridge_generation: capsule.bridge.bridge_generation,
            world_loaded: capsule.bridge.world_loaded,
            fortress_mode: capsule.bridge.fortress_mode,
            paused: capsule.paused,
            current_year: capsule.current_year,
            current_year_tick: capsule.current_year_tick,
            world_name: capsule.world_name.clone(),
            world_folder: capsule.world_folder.clone(),
            site_id: capsule.site_id,
            citizen_count_total: total,
            citizen_offset: offset.min(total),
            complete,
            citizens: citizens.to_vec(),
        };
        self.next_offset = end_u32;
        if complete {
            self.primed = None;
        }
        Ok(page)
    }
}

pub fn bootstrap_live_read_adapter<T: LiveObservationSource>(
    mut source: T,
    config: LiveReadBootstrapConfig,
) -> Result<LiveReadAdapter<PrimedLiveSource<T>>> {
    config.validate()?;
    let capsule = read_complete_observation_bounded(
        &mut source,
        config.page_size,
        config.include_names,
        config.max_citizens,
    )?;
    let source_digest = capsule.content_digest;
    let fortress_id = derive_live_fortress_id(&capsule)?;
    let primed = PrimedLiveSource::new(source, capsule)?;
    let mut adapter = LiveReadAdapter::new(
        primed,
        LiveReadAdapterConfig {
            fortress_id,
            page_size: config.page_size,
            max_citizens: config.max_citizens,
            include_names: config.include_names,
            initial_epoch: config.initial_epoch,
        },
    )?;
    let projection_fortress_id = adapter.bootstrap()?.snapshot.fortress_id;
    let digest_preserved = adapter
        .last_capsule()
        .is_some_and(|capsule| capsule.content_digest == source_digest);
    if !digest_preserved
        || projection_fortress_id != fortress_id
        || adapter.source().has_primed_capsule()
    {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "live adapter bootstrap did not preserve the verified first capsule",
        ));
    }
    Ok(adapter)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, OperationContext, RequestId, RiskTier,
        SessionId, WorkBudget,
    };

    use super::*;
    use crate::{
        CitizenRecord, GameAdapter, InterestSet, ObservationPayload, ObservationRequest,
        Projection,
    };

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
                    "scripted source exhausted its pages",
                )
            })
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

    fn page(year_tick: u32, ids: &[i32]) -> ObservationPage {
        ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: year_tick,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: u32::try_from(ids.len()).map_or(u32::MAX, |value| value),
            citizen_offset: 0,
            complete: true,
            citizens: ids.iter().copied().map(citizen).collect(),
        }
    }

    fn context(anchor: dfmcp_core::StateAnchor) -> OperationContext {
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
                capability: Capability::Observe,
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

    #[test]
    fn bootstrap_reads_the_underlying_source_once() -> Result<()> {
        let first = page(12_345, &[0, 1]);
        let source_digest = {
            let mut assembler = crate::ObservationAssembler::new(manifest());
            assembler.push_page(first.clone())?;
            assembler.finalize()?.content_digest
        };
        let source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([first, page(12_346, &[0, 1, 2])]),
            calls: 0,
        };
        let mut adapter = bootstrap_live_read_adapter(
            source,
            LiveReadBootstrapConfig {
                page_size: MAX_CITIZENS_PER_PAGE,
                max_citizens: 64,
                include_names: true,
                initial_epoch: 7,
            },
        )?;
        assert_eq!(adapter.source().source().calls, 1);
        assert_eq!(
            adapter
                .last_capsule()
                .map(|capsule| capsule.content_digest),
            Some(source_digest)
        );
        let anchor = adapter
            .current_anchor()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "missing anchor"))?;
        let frame = adapter.observe(
            &ObservationRequest {
                since: Some(anchor.cursor),
                projection: Projection::Full,
                interest: InterestSet::default(),
                max_entities: 128,
                max_bytes: 4 * 1024 * 1024,
                max_output_tokens: 16_384,
                continuation: None,
            },
            &context(anchor),
        )?;
        assert!(matches!(frame.payload, ObservationPayload::Snapshot(_)));
        assert_eq!(adapter.source().source().calls, 2);
        Ok(())
    }

    #[test]
    fn primed_source_replays_arbitrary_valid_page_sizes() -> Result<()> {
        let first = page(12_345, &[0, 1, 2]);
        let mut assembler = crate::ObservationAssembler::new(manifest());
        assembler.push_page(first)?;
        let capsule = assembler.finalize()?;
        let source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::new(),
            calls: 0,
        };
        let mut primed = PrimedLiveSource::new(source, capsule)?;
        let first_page = primed.read_observation_page(0, 2, true)?;
        let second_page = primed.read_observation_page(2, 2, true)?;
        assert_eq!(first_page.citizens.len(), 2);
        assert!(!first_page.complete);
        assert_eq!(second_page.citizens.len(), 1);
        assert!(second_page.complete);
        assert!(!primed.has_primed_capsule());
        assert_eq!(primed.source().calls, 0);
        Ok(())
    }
}
