use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_adapter::live_operations::{
    LiveOperationsObservation, OperationsProfile, item_entity_id,
};
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::spatial_inventory::plan;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, MapCoord,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
fn error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "invalid spatial test fixture",
    )
}
fn fixture() -> Result<LiveSpatialObservation> {
    let s = include_str!("fixtures/spatial_v1_6.hex").trim();
    let b = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| error()))
        .collect::<Result<Vec<_>>>()?;
    LiveSpatialObservation::decode_payload(&b, 7, "df".to_owned(), "dfhack".to_owned())
}
fn state(op: LiveOperationsObservation, map: LiveMapObservation) -> Result<LiveSpatialState> {
    let mut b = b"DFMS1600".to_vec();
    for p in [
        op.encode_profile(OperationsProfile::PagedV1_4)?,
        map.encode_payload()?,
    ] {
        b.extend_from_slice(&(p.len() as u32).to_be_bytes());
        b.extend_from_slice(&p);
    }
    let mut state = LiveSpatialState::default();
    state.publish(LiveSpatialObservation::decode_payload(
        &b,
        7,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?)?;
    Ok(state)
}
fn context(state: &LiveSpatialState) -> Result<OperationContext> {
    let anchor = state.snapshot().ok_or_else(error)?.anchor();
    Ok(OperationContext {
        session_id: SessionId::new(77),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(anchor.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    })
}
fn demand(key: &str, kind: &str, units: u64) -> MaterialDemand {
    MaterialDemand {
        key: key.to_owned(),
        units,
        item_types: vec![kind.to_owned()],
        subtype: None,
        material_type: None,
        material_index: None,
    }
}
#[test]
fn shared_reachable_stack_is_not_counted_twice() -> Result<()> {
    let f = fixture()?;
    let s = state(f.operations().clone(), f.terrain().clone())?;
    let p = plan(
        &s,
        &context(&s)?,
        [0, 0, 5],
        &[demand("a", "item_type_3", 1), demand("b", "item_type_3", 1)],
        100_000,
    )?;
    assert_eq!(p.allocation.allocated_units, 1);
    assert_eq!(p.allocation.cut_capacity, 1);
    assert_eq!(p.allocation.shortage.as_ref().map(|s| s.deficit), Some(1));
    assert_eq!(p.locations[&item_entity_id(32).get()].candidate_steps, 3);
    assert_eq!(p.item_counts.values().sum::<u64>(), 3);
    assert_eq!(p.source_digest, s.source_digest()?);
    Ok(())
}
#[test]
fn contained_supply_uses_ground_root_and_inherits_exclusions() -> Result<()> {
    let f = fixture()?;
    let mut op = f.operations().clone();
    op.jobs.jobs.clear();
    op.attachments.clear();
    op.items[0].flags = 0;
    op.items[1].flags = 64;
    op.items[1].holder_building_native_id = None;
    op.items[1].raw_position = MapCoord::new(1, 2, 5);
    let s = state(op.clone(), f.terrain().clone())?;
    let p = plan(
        &s,
        &context(&s)?,
        [0, 0, 5],
        &[demand("drink", "item_type_1", 5)],
        100_000,
    )?;
    assert_eq!(p.allocation.allocated_units, 5);
    let location = &p.locations[&item_entity_id(30).get()];
    assert_eq!(location.position, [1, 2, 5]);
    assert_eq!(location.outermost_item_id, item_entity_id(31));
    op.items[1].flags |= 1;
    let s = state(op, f.terrain().clone())?;
    let p = plan(
        &s,
        &context(&s)?,
        [0, 0, 5],
        &[demand("drink", "item_type_1", 5)],
        100_000,
    )?;
    assert_eq!(p.allocation.allocated_units, 0);
    assert_eq!(p.item_counts["excluded_item_or_container_policy"], 2);
    Ok(())
}
#[test]
fn hidden_outside_and_unestablished_locations_are_distinct_model_exclusions() -> Result<()> {
    let f = fixture()?;
    for (pos, flags, reason) in [
        (
            MapCoord::new(2, 2, 5),
            64,
            "no_candidate_route_in_observed_model",
        ),
        (MapCoord::new(16, 16, 5), 64, "outside_observed_region"),
        (MapCoord::new(1, 2, 5), 0, "unestablished_ground_location"),
    ] {
        let mut op = f.operations().clone();
        op.items[2].raw_position = pos;
        op.items[2].flags = flags;
        let s = state(op, f.terrain().clone())?;
        let p = plan(
            &s,
            &context(&s)?,
            [0, 0, 5],
            &[demand("wood", "item_type_3", 1)],
            100_000,
        )?;
        assert_eq!(p.allocation.allocated_units, 0);
        assert_eq!(p.item_counts[reason], 1);
        assert!(
            plan(
                &s,
                &context(&s)?,
                [2, 2, 5],
                &[demand("wood", "item_type_3", 1)],
                100_000
            )
            .is_err()
        );
    }
    Ok(())
}
#[test]
fn authority_anchor_work_and_demand_failures_return_no_partial_allocation() -> Result<()> {
    let f = fixture()?;
    let s = state(f.operations().clone(), f.terrain().clone())?;
    let mut c = context(&s)?;
    let d = [demand("wood", "item_type_3", 1)];
    assert!(matches!(plan(&s,&c,[0,0,5],&d,1),Err(e)if e.code==ErrorCode::BudgetExceeded));
    c.anchor.cursor.sequence += 1;
    assert!(matches!(plan(&s,&c,[0,0,5],&d,100_000),Err(e)if e.code==ErrorCode::StaleAnchor));
    c = context(&s)?;
    c.grants.clear();
    assert!(matches!(plan(&s,&c,[0,0,5],&d,100_000),Err(e)if e.code==ErrorCode::CapabilityDenied));
    c = context(&s)?;
    let dup = [
        demand("same", "item_type_3", 1),
        demand("same", "item_type_3", 1),
    ];
    assert!(plan(&s, &c, [0, 0, 5], &dup, 100_000).is_err());
    Ok(())
}
