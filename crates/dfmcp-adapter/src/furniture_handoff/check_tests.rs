//! Foreground cancellation must cross the complete allocation pipeline.
//! Beads: df-cx-authority-budget-threading-lmu; df-dfhack-bridge-plane-c-pic.3/.4/.5.
use super::*;
use crate::furniture_allocation::{Request, Slot};
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, MapCoord, RequestId, SessionId, WorkBudget,
};

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
            world_folder: "checked-allocation".into(),
            next_job_id: 200,
            jobs: Vec::new(),
        },
        next_building_id: 100,
        next_item_id: 66,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (42..66).map(|id| LiveItem {
            native_id: id,
            item_type: 101,
            type_key: "BED".into(),
            subtype: -1,
            material_type: if id == 42 { 7 } else { 3 },
            material_index: if id == 42 { 8 } else { -1 },
            stack_size: 1,
            raw_position: MapCoord::new(10 + (id - 42) as i32, 10, 2),
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
            max_entities: 128,
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
    let slots = (0..if shortage { 3 } else { 2 }).map(|index| Slot {
        name: format!("s{index}"),
        kind: Kind::Bed,
        target: [10 + index, 10, 2],
        after: if index == 0 { Vec::new() } else { vec!["s0".into()] },
        material: if index == 0 { None } else { Some((7, 8)) },
        subtype: Some(-1),
        max_distance: 100,
    }).collect();
    let request = FurnitureRequest::new("checked-allocation".into(), 2, Request {
        slots,
        excluded_items: Vec::new(),
    })?;
    Ok((state, context, request))
}
fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}
fn rejection(kind: u8) -> DfmcpError {
    DfmcpError::new(match kind {
        0 => ErrorCode::CancellationRequested,
        1 => ErrorCode::CapabilityDenied,
        _ => ErrorCode::BudgetExceeded,
    }, "foreground owner refused")
}

#[test]
fn guarded_and_legacy_results_keep_exact_choices_bytes_and_work_counts() -> Result<()> {
    for shortage in [false, true] {
        let (state, context, request) = fixture(shortage)?;
        let expected = Handoff::allocate(&state, &context, endpoint(), &request, furniture_supply::MAX_WORK)?;
        let mut calls = 0;
        let actual = Handoff::allocate_with_check(
            &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
            &mut || { calls += 1; Ok(()) },
        )?;
        assert!(calls > 6);
        assert_eq!(actual, expected);
        if shortage {
            assert!(actual.handoff.is_none());
            assert!(actual.report.allocation.assignments.is_empty());
            assert_eq!(actual.report.allocation.shortage.as_ref().unwrap().missing, 1);
        } else {
            let handoff = actual.handoff.as_ref().unwrap();
            assert_eq!(handoff.items().iter().map(|item| item.candidate.native_id).collect::<Vec<_>>(), [43, 42]);
            assert_eq!(Handoff::decode(handoff.canonical_bytes())?, *handoff);
        }
    }
    Ok(())
}

#[test]
fn every_handoff_checkpoint_can_abort_without_publishing_any_outcome() -> Result<()> {
    for shortage in [false, true] {
        let (state, context, request) = fixture(shortage)?;
        let snapshot = state.snapshot().cloned();
        let mut checkpoints = 0;
        let expected = Handoff::allocate_with_check(
            &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
            &mut || { checkpoints += 1; Ok(()) },
        )?;
        for kind in 0..3 {
            for stop in 1..=checkpoints {
                let mut visited = 0;
                let outcome = Handoff::allocate_with_check(
                    &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
                    &mut || {
                        visited += 1;
                        if visited == stop { Err(rejection(kind)) } else { Ok(()) }
                    },
                );
                assert_eq!(outcome.unwrap_err().code, rejection(kind).code, "checkpoint {stop}");
                assert_eq!(visited, stop, "work continued after owner refusal");
                assert_eq!(state.snapshot(), snapshot.as_ref());
            }
        }
        // A later explicit pure request can still inspect the unchanged source.
        // The failed operation created no durable batch and retained no permit.
        assert_eq!(Handoff::allocate(&state, &context, endpoint(), &request, furniture_supply::MAX_WORK)?, expected);
    }
    Ok(())
}

#[test]
fn every_supply_checkpoint_can_abort_matching_or_shortage_construction() -> Result<()> {
    for shortage in [false, true] {
        let (state, context, request) = fixture(shortage)?;
        let mut checkpoints = 0;
        let expected = furniture_supply::plan_with_check(
            &state, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
            &mut || { checkpoints += 1; Ok(()) },
        )?;
        assert!(checkpoints >= 6);
        for kind in 0..3 {
            for stop in 1..=checkpoints {
                let mut visited = 0;
                let outcome = furniture_supply::plan_with_check(
                    &state, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
                    &mut || {
                        visited += 1;
                        if visited == stop { Err(rejection(kind)) } else { Ok(()) }
                    },
                );
                assert_eq!(outcome.unwrap_err().code, rejection(kind).code, "checkpoint {stop}");
                assert_eq!(visited, stop);
            }
        }
        assert_eq!(furniture_supply::plan(
            &state, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
        )?, expected);
    }
    Ok(())
}

#[test]
fn allowing_owner_check_cannot_grant_missing_expired_or_cancelled_query() -> Result<()> {
    let (state, base, request) = fixture(false)?;
    for variant in 0..3 {
        let mut context = base.clone();
        match variant {
            0 => context.grants.clear(),
            1 => context.grants[0].expires_at_tick = Some(GameTick(base.anchor.tick.get() - 1)),
            _ => context.cancellation_requested = true,
        }
        let mut calls = 0;
        assert!(Handoff::allocate_with_check(
            &state, &context, endpoint(), &request, furniture_supply::MAX_WORK,
            &mut || { calls += 1; Ok(()) },
        ).is_err());
        assert!(furniture_supply::plan_with_check(
            &state, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
            &mut || { calls += 1; Ok(()) },
        ).is_err());
        assert_eq!(calls, 0, "callback was treated as a substitute for Query authority");
    }
    Ok(())
}

#[test]
fn guarded_allocation_does_not_renew_the_shared_work_allowance() -> Result<()> {
    let (state, context, request) = fixture(false)?;
    let expected = Handoff::allocate(&state, &context, endpoint(), &request, furniture_supply::MAX_WORK)?;
    let exact = expected.report.work_units;
    assert_eq!(Handoff::allocate_with_check(
        &state, &context, endpoint(), &request, exact, &mut || Ok(()),
    )?, expected);
    assert_eq!(Handoff::allocate_with_check(
        &state, &context, endpoint(), &request, exact - 1, &mut || Ok(()),
    ).unwrap_err().code, ErrorCode::BudgetExceeded);

    let expected = furniture_supply::plan(
        &state, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
    )?;
    assert_eq!(furniture_supply::plan_with_check(
        &state, &context, request.folder(), request.site(), request.request(), expected.work_units, &mut || Ok(()),
    )?, expected);
    assert_eq!(furniture_supply::plan_with_check(
        &state, &context, request.folder(), request.site(), request.request(), expected.work_units - 1, &mut || Ok(()),
    ).unwrap_err().code, ErrorCode::BudgetExceeded);
    Ok(())
}

#[test]
fn owner_refusal_precedes_access_to_an_unpublished_inventory() -> Result<()> {
    let (_, context, request) = fixture(false)?;
    let missing = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    assert_eq!(Handoff::allocate_with_check(
        &missing, &context, endpoint(), &request, furniture_supply::MAX_WORK,
        &mut || Err(rejection(0)),
    ).unwrap_err().code, ErrorCode::CancellationRequested);
    assert_eq!(furniture_supply::plan_with_check(
        &missing, &context, request.folder(), request.site(), request.request(), furniture_supply::MAX_WORK,
        &mut || Err(rejection(1)),
    ).unwrap_err().code, ErrorCode::CapabilityDenied);
    Ok(())
}
