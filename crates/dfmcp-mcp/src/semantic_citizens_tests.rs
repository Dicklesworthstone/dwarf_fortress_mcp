use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use dfmcp_adapter::live_projection::{LiveWorldProjection, project_live_capsule};
use dfmcp_adapter::semantic_workforce::WorkforceEvidenceOwner;
use dfmcp_adapter::semantic_workforce::evidence_owner::CitizenWorkforceEvidenceOwner;
use dfmcp_adapter::{CitizenRecord, ObservationAssembler};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, RiskTier,
    SessionId, WorkBudget,
};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource};

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
enum Drift {
    #[default]
    None,
    Manifest,
    Tick,
    World,
    Total,
    Duplicate,
    Unpaused,
}
#[derive(Default)]
struct ReadState {
    total: u32,
    drift: Drift,
    opened: usize,
    closed: usize,
    policy_checks: usize,
    calls: Vec<(u32, u32, u64)>,
}
struct Source {
    state: Rc<RefCell<ReadState>>,
    manifest: BridgeManifest,
    calls: usize,
}
impl Source {
    fn new(state: Rc<RefCell<ReadState>>) -> Self {
        state.borrow_mut().opened += 1;
        Self {
            state,
            manifest: manifest(),
            calls: 0,
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.state.borrow_mut().closed += 1;
    }
}
impl CitizenEvidenceSource for Source {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.manifest.clone()
    }
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
        context: &OperationContext,
    ) -> Result<ObservationPage> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert!(!names);
        assert_eq!(maximum, 128, "the transport page limit is unchanged");
        assert!(context.budget.max_bytes <= MIB);
        assert!(context.budget.max_wall_millis <= 5000);
        let mut state = self.state.borrow_mut();
        state
            .calls
            .push((offset, maximum, context.budget.max_bytes));
        self.calls += 1;
        let mut page = page(state.total, offset, maximum);
        if self.calls == 2 {
            match state.drift {
                Drift::None => {}
                Drift::Manifest => self.manifest.bridge_generation += 1,
                Drift::Tick => page.current_year_tick += 1,
                Drift::World => page.world_folder = "another-region".into(),
                Drift::Total => page.citizen_count_total -= 1,
                Drift::Duplicate => page.citizens[0].unit_id = offset as i32 - 1,
                Drift::Unpaused => page.paused = false,
            }
        }
        Ok(page)
    }
}

fn manifest() -> BridgeManifest {
    BridgeManifest {
        bridge_version: "0.1.0".into(),
        dfhack_version: "0.51.11-r1".into(),
        df_version: "0.51.11".into(),
        world_loaded: true,
        fortress_mode: true,
        bridge_generation: 42,
        supported_methods: BTreeSet::from(["Handshake".into(), "ReadObservation".into()]),
    }
}
fn page(total: u32, offset: u32, maximum: u32) -> ObservationPage {
    let end = total.min(offset + maximum);
    ObservationPage {
        bridge_generation: 42,
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: 105,
        current_year_tick: 12_345,
        world_name: "The Balanced Realm".into(),
        world_folder: "region1".into(),
        site_id: 7,
        citizen_count_total: total,
        citizen_offset: offset,
        complete: end == total,
        citizens: (offset..end)
            .map(|id| CitizenRecord {
                unit_id: id as i32,
                name: String::new(),
                race: "dwarf".into(),
                profession: 4,
                x: id as i32,
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
            })
            .collect(),
    }
}
fn reference(total: u32) -> Result<LiveObservationCapsule> {
    let mut assembler = ObservationAssembler::with_names(manifest(), false);
    assembler.push_page(page(total, 0, total.max(1)))?;
    assembler.finalize()
}
fn context(capsule: &LiveObservationCapsule) -> Result<OperationContext> {
    let fortress = dfmcp_adapter::workforce_control::fortress_id("region1", 7);
    let projection = project_live_capsule(
        capsule,
        fortress,
        ObservationCursor {
            epoch: 4,
            sequence: 20,
        },
    )?;
    Ok(OperationContext {
        session_id: SessionId::new(91),
        request_id: RequestId::new(92),
        anchor: projection.snapshot.anchor(),
        budget: WorkBudget {
            max_bytes: 16 * MIB,
            max_entities: 4096,
            max_actions: 1,
            max_wall_millis: 5000,
            max_output_tokens: 32768,
            max_game_ticks: 0,
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    })
}
fn acquire(state: &Rc<RefCell<ReadState>>) -> Result<LiveObservationCapsule> {
    let context = context(&reference(state.borrow().total)?)?;
    let mut source = Source::new(Rc::clone(state));
    // The concrete shells spend 2 MiB on connection, then pass this reservation
    // closure to the same driver. Every page still receives exactly 1 MiB.
    let mut remaining = context.budget.max_bytes - 2 * MIB;
    read_complete(&mut source, |bytes| {
        remaining = remaining
            .checked_sub(bytes)
            .ok_or_else(|| DfmcpError::new(ErrorCode::BudgetExceeded, "test page allowance"))?;
        let mut page_context = context.clone();
        page_context.budget.max_bytes = bytes;
        Ok(page_context)
    })
}

type Factory = Box<dyn FnMut(&OperationContext) -> Result<Source>>;
type Authorizer = Box<
    dyn FnMut(
        &LiveObservationCapsule,
        &LiveWorldProjection,
        &OperationContext,
    ) -> Result<EvidencePolicy>,
>;
type EvidenceOwner = CitizenWorkforceEvidenceOwner<Factory, Authorizer>;

fn owner(
    capsule: LiveObservationCapsule,
    state: &Rc<RefCell<ReadState>>,
    context: &OperationContext,
) -> Result<EvidenceOwner> {
    let factory_state = Rc::clone(state);
    let factory: Factory = Box::new(move |context| {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(context.budget.max_bytes, MIB);
        Ok(Source::new(Rc::clone(&factory_state)))
    });
    let policy_state = Rc::clone(state);
    let authorizer: Authorizer = Box::new(move |capsule, projection, context| {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(context.anchor, projection.snapshot.anchor());
        assert_eq!(capsule.bridge, manifest());
        let mut state = policy_state.borrow_mut();
        assert_eq!(
            state.opened, state.closed,
            "sources close before policy issuance"
        );
        state.policy_checks += 1;
        let mut policy = EvidencePolicy::at(projection.snapshot.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Unit, EvidenceCoverage::Observed);
        policy.sources.insert(EvidenceSource::Observed {
            field: "dfmcp_bridge.ReadObservation.citizen.unit_id".into(),
            source_digest: capsule.content_digest,
        });
        Ok(policy)
    });
    CitizenWorkforceEvidenceOwner::new::<Source>(capsule, LIMITS, factory, authorizer, context)
}

#[test]
fn complete_roster_limits_match_the_machine_contract() -> Result<()> {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../architecture/live_production_executor_v1.json"
    ))
    .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "invalid production contract"))?;
    let host = &contract["development_mcp_host"];
    let limits = &host["canonical_citizens"];
    assert_eq!(
        limits["page_max"].as_u64(),
        Some(u64::from(LIMITS.page_size))
    );
    assert_eq!(
        limits["complete_roster_max"].as_u64(),
        Some(u64::from(LIMITS.max_citizens))
    );
    assert_eq!(
        limits["evidence_refresh_budget_bytes"].as_u64(),
        Some(16 * MIB)
    );
    assert_eq!(limits["capture_policy"], "complete_coherent_or_refuse");
    for path in [
        "crates/dfmcp-mcp/src/semantic_citizens.rs",
        "crates/dfmcp-mcp/src/semantic_citizens_tests.rs",
    ] {
        assert!(
            host["source_paths"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|entry| entry == path))
        );
    }
    Ok(())
}

#[test]
fn complete_rosters_cross_the_page_boundary_without_changing_canonical_identity() -> Result<()> {
    for total in [0, 1, 128, 129, 200] {
        let state = Rc::new(RefCell::new(ReadState {
            total,
            ..ReadState::default()
        }));
        let capsule = acquire(&state)?;
        assert_eq!(
            capsule,
            reference(total)?,
            "pagination is not canonical state"
        );
        assert_eq!(capsule.citizens.len(), total as usize);
        assert!(capsule.citizen_coverage.proves_complete_roster());
        let calls = &state.borrow().calls;
        let expected = if total > 128 { vec![0, 128] } else { vec![0] };
        assert_eq!(
            calls.iter().map(|call| call.0).collect::<Vec<_>>(),
            expected
        );
        assert!(calls.iter().all(|call| call.1 == 128 && call.2 == MIB));
    }
    Ok(())
}

#[test]
fn two_hundred_one_citizens_refuse_on_the_first_page_without_partial_publication() -> Result<()> {
    let state = Rc::new(RefCell::new(ReadState {
        total: 201,
        ..ReadState::default()
    }));
    let cause = acquire(&state).err().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "oversized complete roster was accepted",
        )
    })?;
    assert_eq!(cause.code, ErrorCode::BudgetExceeded);
    assert!(cause.message.contains("caller ceiling of 200"));
    assert_eq!(state.borrow().calls.len(), 1);
    assert_eq!(state.borrow().closed, 1);
    Ok(())
}

#[test]
fn multipage_drift_never_yields_a_complete_capsule() -> Result<()> {
    for drift in [
        Drift::Manifest,
        Drift::Tick,
        Drift::World,
        Drift::Total,
        Drift::Duplicate,
        Drift::Unpaused,
    ] {
        let state = Rc::new(RefCell::new(ReadState {
            total: 200,
            drift,
            ..ReadState::default()
        }));
        assert!(acquire(&state).is_err(), "second-page {drift:?}");
        assert_eq!(state.borrow().calls.len(), 2);
        assert_eq!(state.borrow().closed, 1);
        assert_eq!(state.borrow().policy_checks, 0);
    }
    Ok(())
}

#[test]
fn per_page_budget_failure_stops_before_the_next_native_read() -> Result<()> {
    let state = Rc::new(RefCell::new(ReadState {
        total: 200,
        ..ReadState::default()
    }));
    let context = context(&reference(200)?)?;
    let mut source = Source::new(Rc::clone(&state));
    let mut remaining = MIB;
    let result = read_complete(&mut source, |bytes| {
        remaining = remaining.checked_sub(bytes).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "test exhausted page allowance")
        })?;
        let mut page_context = context.clone();
        page_context.budget.max_bytes = bytes;
        Ok(page_context)
    });
    let cause = result.err().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "unreserved second page was accepted",
        )
    })?;
    assert_eq!(cause.code, ErrorCode::BudgetExceeded);
    assert_eq!(state.borrow().calls.len(), 1);
    Ok(())
}

#[test]
fn two_hundred_citizen_revalidation_fits_the_unchanged_sixteen_mib_grant() -> Result<()> {
    for total in [129, 200] {
        let state = Rc::new(RefCell::new(ReadState {
            total,
            ..ReadState::default()
        }));
        let capsule = acquire(&state)?;
        let context = context(&capsule)?;
        let mut evidence = owner(capsule, &state, &context)?;
        assert!(evidence.routing_evidence().is_ok());
        evidence.refresh::<Source>(&context)?;
        assert!(evidence.routing_evidence().is_ok());
        let state = state.borrow();
        assert_eq!(state.opened, 3);
        assert_eq!(state.closed, 3);
        assert_eq!(state.policy_checks, 4);
        assert_eq!(
            state.calls.iter().map(|call| call.0).collect::<Vec<_>>(),
            [0, 128, 0, 128, 0, 128]
        );
        assert!(state.calls[2..].iter().all(|call| call.2 == 663_552));
    }
    Ok(())
}

#[test]
fn revalidation_rejects_changed_pages_and_rosters_and_withdraws_old_routing() -> Result<()> {
    for drift in [
        Drift::Manifest,
        Drift::Tick,
        Drift::World,
        Drift::Total,
        Drift::Duplicate,
        Drift::Unpaused,
    ] {
        let state = Rc::new(RefCell::new(ReadState {
            total: 200,
            ..ReadState::default()
        }));
        let capsule = acquire(&state)?;
        let context = context(&capsule)?;
        let mut evidence = owner(capsule, &state, &context)?;
        assert!(evidence.routing_evidence().is_ok());
        state.borrow_mut().drift = drift;
        assert!(
            evidence.refresh::<Source>(&context).is_err(),
            "revalidation {drift:?}"
        );
        assert!(evidence.routing_evidence().is_err());
        assert_eq!(state.borrow().policy_checks, 2);
        assert_eq!(state.borrow().calls.len(), 6);
        assert_eq!(state.borrow().opened, state.borrow().closed);
    }
    let state = Rc::new(RefCell::new(ReadState {
        total: 200,
        ..ReadState::default()
    }));
    let capsule = acquire(&state)?;
    let context = context(&capsule)?;
    let mut evidence = owner(capsule, &state, &context)?;
    state.borrow_mut().total = 201;
    assert!(evidence.refresh::<Source>(&context).is_err());
    assert!(evidence.routing_evidence().is_err());
    assert_eq!(
        state.borrow().calls.len(),
        5,
        "an expanded roster stops on its first page"
    );
    assert_eq!(state.borrow().policy_checks, 2);
    Ok(())
}

#[test]
fn oversized_or_underfunded_revalidation_refuses_before_opening_a_source() -> Result<()> {
    let state = Rc::new(RefCell::new(ReadState {
        total: 201,
        ..ReadState::default()
    }));
    let capsule = reference(201)?;
    let c = context(&capsule)?;
    assert!(owner(capsule, &state, &c).is_err());
    assert_eq!(state.borrow().opened, 0);

    let capsule = reference(200)?;
    let mut c = context(&capsule)?;
    c.budget.max_bytes = 12 * MIB;
    state.borrow_mut().total = 200;
    assert!(owner(capsule, &state, &c).is_err());
    assert_eq!(state.borrow().opened, 0);
    Ok(())
}
