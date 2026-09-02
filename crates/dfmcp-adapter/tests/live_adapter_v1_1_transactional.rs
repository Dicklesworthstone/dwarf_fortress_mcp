#![forbid(unsafe_code)]

use std::collections::{BTreeSet, VecDeque};

use dfmcp_adapter::{
    AnnouncementBatchRecord, AnnouncementContinuity, AnnouncementCoverage,
    BridgeManifest, CitizenRecord, GameAdapter, InterestSet,
    LiveAnnouncementBatch, LiveObservationSourceV1_1, LiveReadAdapterConfigV1_1,
    LiveReadAdapterV1_1, ObservationPageV1_1, ObservationRequest,
    Projection,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode,
    FortressId, OperationContext, RequestId, Result, RiskTier, SessionId,
    StateAnchor, WorkBudget,
};

#[derive(Clone)]
struct ScriptedSource {
    manifest: BridgeManifest,
    pages: VecDeque<ObservationPageV1_1>,
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
        self.pages.pop_front().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::AdapterFailure,
                "transactional regression source exhausted its pages",
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

fn citizen() -> CitizenRecord {
    CitizenRecord {
        unit_id: 1,
        name: "Urist".to_owned(),
        race: "dwarf".to_owned(),
        profession: 4,
        x: 1,
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

fn announcement(report_id: i32) -> AnnouncementBatchRecord {
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

fn page(year_tick: u32, report_ids: &[i32]) -> Result<ObservationPageV1_1> {
    let records = report_ids
        .iter()
        .copied()
        .map(announcement)
        .collect::<Vec<_>>();
    let oldest = report_ids.first().copied().unwrap_or(-1);
    let latest = report_ids.last().copied().unwrap_or(-1);
    let returned = u32::try_from(records.len()).map_err(|_| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "transactional regression record count does not fit u32",
        )
    })?;
    let batch = LiveAnnouncementBatch::new(
        42,
        true,
        105,
        year_tick,
        7,
        AnnouncementCoverage {
            requested_after_id: -1,
            oldest_available_id: oldest,
            latest_available_id: latest,
            returned,
            complete_through_latest: true,
            continuity: AnnouncementContinuity::CompleteSuffix,
            next_after_id: latest,
        },
        records,
    )?;
    Ok(ObservationPageV1_1 {
        bridge_generation: 42,
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: 105,
        current_year_tick: year_tick,
        world_name: "The Balanced Realm".to_owned(),
        world_folder: "region1".to_owned(),
        site_id: 7,
        citizen_count_total: 1,
        citizen_offset: 0,
        complete: true,
        citizens: vec![citizen()],
        announcement_batch: batch,
    })
}

fn context(anchor: StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 2,
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
fn larger_candidate_over_budget_does_not_advance_anchor() -> Result<()> {
    let source = ScriptedSource {
        manifest: manifest(),
        pages: VecDeque::from([
            page(12_345, &[])?,
            page(12_346, &[10])?,
        ]),
    };
    let mut adapter = LiveReadAdapterV1_1::new(
        source,
        LiveReadAdapterConfigV1_1 {
            fortress_id: FortressId::new(9),
            citizen_page_size: 64,
            max_citizens: 64,
            include_names: true,
            announcement_after_id: -1,
            announcement_page_size: 64,
            max_total_announcements: 64,
            initial_epoch: 3,
        },
    )?;
    let prior = adapter.bootstrap()?.snapshot.anchor();
    let request = ObservationRequest {
        since: Some(prior.cursor),
        projection: Projection::Full,
        interest: InterestSet::default(),
        max_entities: 2,
        max_bytes: 4 * 1024 * 1024,
        max_output_tokens: 16_384,
        continuation: None,
    };
    let failure = adapter.observe(&request, &context(prior)).err().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "over-budget candidate unexpectedly published",
        )
    })?;
    assert_eq!(failure.code, ErrorCode::BudgetExceeded);
    assert_eq!(adapter.current_anchor(), Some(prior));
    let current = adapter.current_projection().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "adapter lost its prior projection after candidate rejection",
        )
    })?;
    assert_eq!(current.snapshot.graph.entities.len(), 2);
    Ok(())
}
