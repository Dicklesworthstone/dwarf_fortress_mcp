//! Allocation must remain owned after the native read has released its capture.
//! Beads: df-cx-authority-budget-threading-lmu; df-dfhack-bridge-plane-c-pic.3/.4/.5.
use super::{FurnitureRequest, Handoff, runtime};
use asupersync::Cx;
use asupersync::cx::cap::CapMask;
use dfmcp_adapter::furniture_supply;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, OperationsProfile,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ErrorCode, MapCoord, OperationContext,
    RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use std::net::SocketAddr;

fn fixture(shortage: bool) -> Result<(LiveOperationsState, OperationContext, FurnitureRequest)> {
    let observed = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 987,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
            year: 2,
            year_tick: 100,
            paused: true,
            site_id: 2,
            world_folder: "owner-allocation".into(),
            next_job_id: 200,
            jobs: Vec::new(),
        },
        next_building_id: 100,
        next_item_id: 554,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (42..554).map(|id| LiveItem {
            native_id: id,
            item_type: 101,
            type_key: "BED".into(),
            subtype: -1,
            material_type: 419,
            material_index: -1,
            stack_size: 1,
            raw_position: MapCoord::new(10, 10, 2),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        }).collect(),
    };
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(observed)?;
    let context = OperationContext {
        session_id: SessionId::new(41),
        request_id: RequestId::new(1),
        anchor: state.snapshot().unwrap().anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 64 * 1024 * 1024,
            max_entities: 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    let raw = if shortage {
        br#"{"schema":"dfmcp.furniture-request/1","world_folder":"owner-allocation","site":2,"slots":[{"name":"bed","kind":"bed","target":[15,15,2],"material":[7,8]}]}"#.as_slice()
    } else {
        br#"{"schema":"dfmcp.furniture-request/1","world_folder":"owner-allocation","site":2,"slots":[{"name":"bed","kind":"bed","target":[15,15,2]}]}"#.as_slice()
    };
    Ok((state, context, FurnitureRequest::decode(raw)?))
}

fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}

#[test]
fn joined_runtime_owner_survives_complete_allocation_and_shortage()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = crate::run_with_runtime_cx(|_| async move {
        runtime::owned("fortress.open_session", |control| {
            for shortage in [false, true] {
                let (state, context, request) = fixture(shortage).unwrap();
                control.check().unwrap();
                let expected = Handoff::allocate(
                    &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                ).unwrap();
                let mut checks = 0;
                let actual = Handoff::allocate_with_check(
                    &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                    &mut || {
                        checks += 1;
                        control.check()
                    },
                ).unwrap();
                assert!(checks > 6, "owner was checked only outside the solver");
                assert_eq!(actual, expected);
                assert_eq!(actual.handoff.is_none(), shortage);
                control.check().unwrap();
            }
            "owned-allocation".into()
        }).await
    })?;
    assert_eq!(output, "owned-allocation");
    Ok(())
}

#[test]
fn inherited_runtime_restriction_interrupts_allocation_before_result_publication()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = crate::run_with_runtime_cx(|_| async move {
        runtime::owned("fortress.open_session", |control| {
            for shortage in [false, true] {
                let (state, context, request) = fixture(shortage).unwrap();
                let before = state.snapshot().cloned();
                let mut total = 0;
                let expected = Handoff::allocate_with_check(
                    &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                    &mut || {
                        total += 1;
                        control.check()
                    },
                ).unwrap();
                assert!(total > 6);
                // Include source entry, interior scan/matching, and the final
                // handoff/shortage check. No environment mutation or test-only
                // production cancellation switch is needed to revoke authority.
                for stop in [1, 2, total / 2, total - 1, total] {
                    let mut restriction = None;
                    let mut visited = 0;
                    let outcome = Handoff::allocate_with_check(
                        &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                        &mut || {
                            visited += 1;
                            if visited == stop {
                                restriction = Some(Cx::push_restriction(CapMask::none()));
                            }
                            control.check()
                        },
                    );
                    drop(restriction);
                    assert_eq!(outcome.unwrap_err().code, ErrorCode::CapabilityDenied);
                    assert_eq!(visited, stop, "solver continued after losing its owner authority");
                    assert_eq!(state.snapshot(), before.as_ref());
                    control.check().unwrap();
                }
                assert_eq!(Handoff::allocate_with_check(
                    &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                    &mut || control.check(),
                ).unwrap(), expected);
            }
            "owner-restriction-observed".into()
        }).await
    })?;
    assert_eq!(output, "owner-restriction-observed");
    Ok(())
