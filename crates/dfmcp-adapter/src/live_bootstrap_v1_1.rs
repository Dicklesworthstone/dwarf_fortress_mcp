#![forbid(unsafe_code)]

//! Single-publication bootstrap for protocol-1.1 live observations.
//!
//! Fortress identity depends on the first complete citizen-plus-announcement
//! capsule, while [`LiveReadAdapterV1_1`](crate::LiveReadAdapterV1_1) requires
//! that identity in immutable configuration. Reading the live source again
//! could derive identity from one observation and publish another. This module
//! acquires one complete transactional capsule, derives identity from its base,
//! then replays the exact verified capsule through a temporary two-dimensional
//! page source. Citizen pages and announcement continuation pages are both
//! reproduced without another underlying bridge call.

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    AnnouncementContinuity, AnnouncementCoverage, BridgeManifest,
    LiveAnnouncementBatch, LiveObservationCapsuleV1_1,
    LiveObservationPublicationConfigV1_1, LiveObservationSourceV1_1,
    LiveReadAdapterConfigV1_1, LiveReadAdapterV1_1,
    MAX_ANNOUNCEMENTS_PER_BATCH, MAX_CAPSULE_CITIZENS,
    MAX_V1_1_CITIZENS_PER_PAGE, ObservationPageV1_1,
    derive_live_fortress_id, read_publishable_observation_v1_1,
};

pub const DEFAULT_MAX_LIVE_ANNOUNCEMENTS: u32 = 512;
pub const DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE: u32 = 128;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadBootstrapConfigV1_1 {
    pub citizen_page_size: u32,
    pub max_citizens: u32,
    pub include_names: bool,
    pub announcement_after_id: i32,
    pub announcement_page_size: u32,
    pub max_total_announcements: u32,
    pub initial_epoch: u64,
}

impl LiveReadBootstrapConfigV1_1 {
    pub fn validate(&self) -> Result<()> {
        LiveObservationPublicationConfigV1_1 {
            citizen_page_size: self.citizen_page_size,
            max_citizens: self.max_citizens,
            include_names: self.include_names,
            announcement_after_id: self.announcement_after_id,
            announcement_page_size: self.announcement_page_size,
            max_total_announcements: self.max_total_announcements,
        }
        .validate()?;
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

    fn publication_config(&self) -> LiveObservationPublicationConfigV1_1 {
        LiveObservationPublicationConfigV1_1 {
            citizen_page_size: self.citizen_page_size,
            max_citizens: self.max_citizens,
            include_names: self.include_names,
            announcement_after_id: self.announcement_after_id,
            announcement_page_size: self.announcement_page_size,
            max_total_announcements: self.max_total_announcements,
        }
    }
}

impl Default for LiveReadBootstrapConfigV1_1 {
    fn default() -> Self {
        Self {
            citizen_page_size: MAX_V1_1_CITIZENS_PER_PAGE,
            max_citizens: crate::DEFAULT_MAX_LIVE_CITIZENS,
            include_names: true,
            announcement_after_id: -1,
            announcement_page_size: DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE,
            max_total_announcements: DEFAULT_MAX_LIVE_ANNOUNCEMENTS,
            initial_epoch: 0,
        }
    }
}

pub struct PrimedLiveSourceV1_1<T> {
    source: T,
    primed: Option<LiveObservationCapsuleV1_1>,
    expected_announcement_after_id: i32,
    active_announcement_after_id: Option<i32>,
    next_citizen_offset: u32,
}

impl<T: LiveObservationSourceV1_1> PrimedLiveSourceV1_1<T> {
    pub fn new(source: T, primed: LiveObservationCapsuleV1_1) -> Result<Self> {
        primed.validate()?;
        if !primed
            .announcement_batch
            .coverage
            .complete_through_latest
        {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "primed protocol-1.1 capsule must contain a complete retained suffix",
            ));
        }
        let manifest = source.bridge_manifest_v1_1();
        manifest.validate()?;
        if manifest != primed.base.bridge {
            return Err(error(
                ErrorCode::StaleAnchor,
                "source manifest changed between the first protocol-1.1 capsule and adapter bootstrap",
            ));
        }
        let expected_announcement_after_id = primed
            .announcement_batch
            .coverage
            .requested_after_id;
        Ok(Self {
            source,
            primed: Some(primed),
            expected_announcement_after_id,
            active_announcement_after_id: None,
            next_citizen_offset: 0,
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
                "cannot extract a primed protocol-1.1 source before bootstrap consumes its capsule",
            ));
        }
        Ok(self.source)
    }

    fn announcement_page(
        capsule: &LiveObservationCapsuleV1_1,
        announcement_after_id: i32,
        max_announcements: u32,
    ) -> Result<LiveAnnouncementBatch> {
        let complete = &capsule.announcement_batch;
        let maximum = usize::try_from(max_announcements).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "primed announcement page size cannot be represented",
            )
        })?;
        let records = complete
            .announcements
            .iter()
            .filter(|record| record.report_id > announcement_after_id)
            .take(maximum)
            .cloned()
            .collect::<Vec<_>>();
        let next_after_id = records
            .last()
            .map_or(announcement_after_id, |record| record.report_id);
        let complete_through_latest = if complete.coverage.latest_available_id < 0 {
            true
        } else {
            next_after_id == complete.coverage.latest_available_id
        };
        let continuity = if announcement_after_id
            == complete.coverage.requested_after_id
        {
            complete.coverage.continuity
        } else {
            AnnouncementContinuity::CompleteSuffix
        };
        LiveAnnouncementBatch::new(
            complete.bridge_generation,
            complete.paused,
            complete.current_year,
            complete.current_year_tick,
            complete.site_id,
            AnnouncementCoverage {
                requested_after_id: announcement_after_id,
                oldest_available_id: complete.coverage.oldest_available_id,
                latest_available_id: complete.coverage.latest_available_id,
                returned: u32::try_from(records.len()).map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "primed announcement record count does not fit u32",
                    )
                })?,
                complete_through_latest,
                continuity,
                next_after_id,
            },
            records,
        )
    }
}

impl<T: LiveObservationSourceV1_1> LiveObservationSourceV1_1
    for PrimedLiveSourceV1_1<T>
{
    fn bridge_manifest_v1_1(&self) -> BridgeManifest {
        self.primed.as_ref().map_or_else(
            || self.source.bridge_manifest_v1_1(),
            |capsule| capsule.base.bridge.clone(),
        )
    }

    fn read_observation_page_v1_1(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        announcement_after_id: i32,
        max_announcements: u32,
    ) -> Result<ObservationPageV1_1> {
        let Some(capsule) = self.primed.as_ref() else {
            return self.source.read_observation_page_v1_1(
                offset,
                maximum,
                include_names,
                announcement_after_id,
                max_announcements,
            );
        };
        if maximum == 0 || maximum > MAX_V1_1_CITIZENS_PER_PAGE {
            return Err(error(
                ErrorCode::InvalidRequest,
                "primed replay citizen page size is outside the protocol-1.1 bound",
            ));
        }
        let maximum_announcements = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH)
            .map_err(|_| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "announcement page ceiling does not fit u32",
                )
            })?;
        if max_announcements == 0 || max_announcements > maximum_announcements {
            return Err(error(
                ErrorCode::InvalidRequest,
                "primed replay announcement page size is outside the protocol-1.1 bound",
            ));
        }
        if include_names != capsule.base.names_included {
            return Err(error(
                ErrorCode::InvalidRequest,
                "primed replay projection does not match the verified protocol-1.1 capsule",
            ));
        }

        match self.active_announcement_after_id {
            Some(active) => {
                if announcement_after_id != active {
                    return Err(error(
                        ErrorCode::CursorGap,
                        "primed replay changed announcement cursor before completing citizen pagination",
                    ));
                }
            }
            None => {
                if offset != 0 {
                    return Err(error(
                        ErrorCode::CursorGap,
                        "primed replay must begin each announcement page at citizen offset zero",
                    ));
                }
                if announcement_after_id != self.expected_announcement_after_id {
                    return Err(error(
                        ErrorCode::CursorGap,
                        format!(
                            "primed replay received announcement cursor {announcement_after_id}, expected {}",
                            self.expected_announcement_after_id
                        ),
                    ));
                }
                self.active_announcement_after_id = Some(announcement_after_id);
            }
        }
        if offset != self.next_citizen_offset {
            return Err(error(
                ErrorCode::CursorGap,
                format!(
                    "primed replay received citizen offset {offset}, expected {}",
                    self.next_citizen_offset
                ),
            ));
        }

        let announcement_batch = Self::announcement_page(
            capsule,
            announcement_after_id,
            max_announcements,
        )?;
        let total = capsule.base.citizen_coverage.total;
        let bounded_offset = offset.min(total);
        let start = usize::try_from(bounded_offset).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "primed replay citizen offset cannot be represented",
            )
        })?;
        let end_u32 = bounded_offset.saturating_add(maximum).min(total);
        let end = usize::try_from(end_u32).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "primed replay citizen end offset cannot be represented",
            )
        })?;
        let citizens = capsule.base.citizens.get(start..end).ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "primed protocol-1.1 capsule coverage does not match its citizen vector",
            )
        })?;
        let citizens_complete = end_u32 == total;
        let page = ObservationPageV1_1 {
            bridge_generation: capsule.base.bridge.bridge_generation,
            world_loaded: capsule.base.bridge.world_loaded,
            fortress_mode: capsule.base.bridge.fortress_mode,
            paused: capsule.base.paused,
            current_year: capsule.base.current_year,
            current_year_tick: capsule.base.current_year_tick,
            world_name: capsule.base.world_name.clone(),
            world_folder: capsule.base.world_folder.clone(),
            site_id: capsule.base.site_id,
            citizen_count_total: total,
            citizen_offset: bounded_offset,
            complete: citizens_complete,
            citizens: citizens.to_vec(),
            announcement_batch: announcement_batch.clone(),
        };

        self.next_citizen_offset = end_u32;
        if citizens_complete {
            self.active_announcement_after_id = None;
            self.next_citizen_offset = 0;
            if announcement_batch.coverage.complete_through_latest {
                self.primed = None;
            } else {
                self.expected_announcement_after_id =
                    announcement_batch.coverage.next_after_id;
            }
        }
        Ok(page)
    }
}

pub fn bootstrap_live_read_adapter_v1_1<T: LiveObservationSourceV1_1>(
    mut source: T,
    config: LiveReadBootstrapConfigV1_1,
) -> Result<LiveReadAdapterV1_1<PrimedLiveSourceV1_1<T>>> {
    config.validate()?;
    let capsule = read_publishable_observation_v1_1(
        &mut source,
        &config.publication_config(),
    )?;
    let source_digest = capsule.content_digest;
    let fortress_id = derive_live_fortress_id(&capsule.base)?;
    let primed = PrimedLiveSourceV1_1::new(source, capsule)?;
    let mut adapter = LiveReadAdapterV1_1::new(
        primed,
        LiveReadAdapterConfigV1_1 {
            fortress_id,
            citizen_page_size: config.citizen_page_size,
            max_citizens: config.max_citizens,
            include_names: config.include_names,
            announcement_after_id: config.announcement_after_id,
            announcement_page_size: config.announcement_page_size,
            max_total_announcements: config.max_total_announcements,
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
            "protocol-1.1 adapter bootstrap did not preserve the verified first capsule",
        ));
    }
    Ok(adapter)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, OperationContext,
        RequestId, RiskTier, SessionId, WorkBudget,
    };

    use super::*;
    use crate::{
        AnnouncementBatchRecord, CitizenRecord, GameAdapter, InterestSet,
        ObservationPayload, ObservationRequest, Projection,
    };

    #[derive(Clone)]
    struct ScriptedSource {
        manifest: BridgeManifest,
        pages: VecDeque<ObservationPageV1_1>,
        calls: usize,
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
            _announcement_after_id: i32,
            _max_announcements: u32,
        ) -> Result<ObservationPageV1_1> {
            self.calls = self.calls.saturating_add(1);
            self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted protocol-1.1 bootstrap source exhausted its pages",
                )
            })
        }
    }

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.2.0".to_owned(),
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

    fn announcement_batch(
        requested_after_id: i32,
        ids: &[i32],
        complete: bool,
    ) -> Result<LiveAnnouncementBatch> {
        let records = ids.iter().copied().map(report).collect::<Vec<_>>();
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id,
                oldest_available_id: 10,
                latest_available_id: 12,
                returned: u32::try_from(records.len()).map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "bootstrap test record count does not fit u32",
                    )
                })?,
                complete_through_latest: complete,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: records
                    .last()
                    .map_or(requested_after_id, |record| record.report_id),
            },
            records,
        )
    }

    fn page(
        ids: &[i32],
        announcement_batch: LiveAnnouncementBatch,
    ) -> ObservationPageV1_1 {
        ObservationPageV1_1 {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: u32::try_from(ids.len())
                .map_or(u32::MAX, |value| value),
            citizen_offset: 0,
            complete: true,
            citizens: ids.iter().copied().map(citizen).collect(),
            announcement_batch,
        }
    }

    fn complete_page(ids: &[i32]) -> Result<ObservationPageV1_1> {
        Ok(page(
            ids,
            announcement_batch(-1, &[10, 11, 12], true)?,
        ))
    }

    fn config() -> LiveReadBootstrapConfigV1_1 {
        LiveReadBootstrapConfigV1_1 {
            citizen_page_size: 64,
            max_citizens: 64,
            include_names: true,
            announcement_after_id: -1,
            announcement_page_size: 4,
            max_total_announcements: 4,
            initial_epoch: 7,
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
    fn bootstrap_does_not_repeat_the_underlying_publication() -> Result<()> {
        let first = complete_page(&[0, 1])?;
        let source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([first.clone(), first]),
            calls: 0,
        };
        let mut adapter = bootstrap_live_read_adapter_v1_1(source, config())?;
        assert_eq!(adapter.source().source().calls, 1);
        assert!(!adapter.source().has_primed_capsule());
        let anchor = adapter.current_anchor().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "protocol-1.1 bootstrap did not publish an anchor",
            )
        })?;
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
        assert!(matches!(frame.payload, ObservationPayload::Heartbeat(_)));
        assert_eq!(adapter.source().source().calls, 2);
        Ok(())
    }

    #[test]
    fn primed_source_replays_citizen_and_announcement_pages() -> Result<()> {
        let source_for_capsule = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([complete_page(&[0, 1, 2])?]),
            calls: 0,
        };
        let mut source_for_capsule = source_for_capsule;
        let capsule = read_publishable_observation_v1_1(
            &mut source_for_capsule,
            &config().publication_config(),
        )?;
        let source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::new(),
            calls: 0,
        };
        let mut primed = PrimedLiveSourceV1_1::new(source, capsule)?;

        let first = primed.read_observation_page_v1_1(0, 2, true, -1, 2)?;
        let second = primed.read_observation_page_v1_1(2, 2, true, -1, 2)?;
        assert_eq!(first.citizens.len(), 2);
        assert_eq!(second.citizens.len(), 1);
        assert!(!first.announcement_batch.coverage.complete_through_latest);
        assert!(!second.announcement_batch.coverage.complete_through_latest);
        assert_eq!(first.announcement_batch.announcements.len(), 2);

        let third = primed.read_observation_page_v1_1(0, 2, true, 11, 2)?;
        let fourth = primed.read_observation_page_v1_1(2, 2, true, 11, 2)?;
        assert_eq!(third.announcement_batch.announcements.len(), 1);
        assert!(third.announcement_batch.coverage.complete_through_latest);
        assert!(fourth.announcement_batch.coverage.complete_through_latest);
        assert!(!primed.has_primed_capsule());
        assert_eq!(primed.source().calls, 0);
        Ok(())
    }

    #[test]
    fn primed_source_rejects_cursor_or_projection_drift() -> Result<()> {
        let mut source_for_capsule = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([complete_page(&[0])?]),
            calls: 0,
        };
        let capsule = read_publishable_observation_v1_1(
            &mut source_for_capsule,
            &config().publication_config(),
        )?;
        let source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::new(),
            calls: 0,
        };
        let mut primed = PrimedLiveSourceV1_1::new(source, capsule)?;
        assert!(
            primed
                .read_observation_page_v1_1(0, 64, true, 10, 4)
                .is_err()
        );
        assert!(
            primed
                .read_observation_page_v1_1(0, 64, false, -1, 4)
                .is_err()
        );
        assert!(primed.has_primed_capsule());
        Ok(())
    }

    #[test]
    fn primed_source_rejects_manifest_drift() -> Result<()> {
        let mut source_for_capsule = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([complete_page(&[0])?]),
            calls: 0,
        };
        let capsule = read_publishable_observation_v1_1(
            &mut source_for_capsule,
            &config().publication_config(),
        )?;
        let mut changed = manifest();
        changed.bridge_generation = 43;
        let source = ScriptedSource {
            manifest: changed,
            pages: VecDeque::new(),
            calls: 0,
        };
        assert!(PrimedLiveSourceV1_1::new(source, capsule).is_err());
        Ok(())
    }
}
