#[path = "support/production_portfolio_spatial.rs"]
mod fixture;
use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::workforce_analysis::portfolio::{
    self, ProductionTask, selection::RESERVE_OWNER,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ErrorCode, OperationContext, RequestId, Result,
    RiskTier, SessionId, WorkBudget,
};

fn setup(units: u32, forbidden: bool) -> Result<(LiveSpatialCitizenState, OperationContext)> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(fixture::observation(3, 2, units, forbidden)?)?;
    let c = OperationContext {
        session_id: SessionId::new(9801),
        request_id: RequestId::new(1),
        anchor: state.snapshot().expect("snapshot").anchor(),
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60000,
            ..WorkBudget::default()
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
    Ok((state, c))
}
fn material(key: &str, units: u64) -> MaterialDemand {
    MaterialDemand {
        key: key.into(),
        units,
        item_types: vec!["item_type_3".into()],
        subtype: None,
        material_type: None,
        material_index: None,
    }
}
fn task(key: &str, units: u64) -> ProductionTask {
    ProductionTask {
        key: key.into(),
        priority: 1,
        workers: 1,
        skill_key: "CARPENTRY".into(),
        min_effective_skill: 1,
        preserve_social: true,
        adults_only: true,
        materials: vec![material("wood", units)],
    }
}

#[test]
fn coherent_stock_is_split_between_complete_tasks_and_unconsumed_reserves() -> Result<()> {
    let (state, c) = setup(5, false)?;
    let tasks = [task("a", 3), task("b", 3)];
    let out = portfolio::plan_with_reserves(
        &state,
        &c,
        [0, 0, 5],
        &tasks,
        &[material("buffer", 2)],
        10_000_000,
    )?;
    assert_eq!(out.selection.task_mask, 1);
    assert_eq!(out.selection.materials.allocated_by_demand, [3, 0, 2]);
    assert_eq!(out.material_owners, [0, 1, RESERVE_OWNER]);
    assert_eq!(out.inventory.demands[2].key, "reserve.buffer");
    assert_eq!(out.inventory.source_digest, out.workforce.source_digest);
    assert_eq!(out.workforce.anchor, c.anchor);
    assert_eq!(state.snapshot().expect("snapshot").anchor(), c.anchor);
    assert_eq!(
        out.selection
            .materials
            .assignments
            .iter()
            .map(|a| a.units)
            .sum::<u64>(),
        5
    );
    Ok(())
}

#[test]
fn reserve_shortfalls_preserve_conservative_item_exclusions() -> Result<()> {
    for forbidden in [false, true] {
        let (state, c) = setup(1, forbidden)?;
        let out = portfolio::plan_with_reserves(
            &state,
            &c,
            [0, 0, 5],
            &[task("a", 1)],
            &[material("buffer", 2)],
            10_000_000,
        )?;
        assert_eq!(out.selection.task_mask, 0);
        assert!(out.selection.workers.assignments.is_empty());
        let shortage = out
            .selection
            .materials
            .shortage
            .as_ref()
            .expect("reserve shortage");
        assert_eq!(shortage.required_units, 2);
        assert_eq!(shortage.eligible_units, u64::from(!forbidden));
        assert_eq!(out.selection.materials.allocated_by_demand[0], 0);
    }
    Ok(())
}

#[test]
fn reserve_normalization_is_order_independent_and_no_reserve_api_is_preserved() -> Result<()> {
    let (state, c) = setup(8, false)?;
    let tasks = [task("a", 1)];
    assert_eq!(
        portfolio::plan(&state, &c, [0, 0, 5], &tasks, 10_000_000)?,
        portfolio::plan_with_reserves(&state, &c, [0, 0, 5], &tasks, &[], 10_000_000)?
    );
    let reserves = [material("winter", 2), material("emergency", 3)];
    let first =
        portfolio::plan_with_reserves(&state, &c, [0, 0, 5], &tasks, &reserves, 10_000_000)?;
    let mut other = reserves.to_vec();
    other.reverse();
    other[0].item_types.push("item_type_3".into());
    assert_eq!(
        first,
        portfolio::plan_with_reserves(&state, &c, [0, 0, 5], &tasks, &other, 10_000_000)?
    );
    assert_eq!(first.selection.materials.allocated_units, 6);
    Ok(())
}

#[test]
fn reserves_validate_raw_bounds_duplicates_and_aggregate_demand_count() -> Result<()> {
    let (state, c) = setup(8, false)?;
    let tasks = [task("a", 1)];
    let base = material("buffer", 1);
    for reserves in [
        vec![base.clone(); 9],
        vec![base.clone(), base.clone()],
        vec![MaterialDemand {
            units: 0,
            ..base.clone()
        }],
        vec![MaterialDemand {
            item_types: vec!["item_type_3".into(); 9],
            ..base.clone()
        }],
        vec![MaterialDemand {
            material_index: Some(1),
            material_type: None,
            ..base.clone()
        }],
    ] {
        assert!(
            portfolio::plan_with_reserves(&state, &c, [0, 0, 5], &tasks, &reserves, 10_000_000)
                .is_err()
        );
    }
    let full: Vec<_> = (0..8)
        .map(|i| {
            let mut t = task(&format!("t{i}"), 1);
            t.materials = (0..4).map(|j| material(&format!("m{j}"), 1)).collect();
            t
        })
        .collect();
    assert!(
        matches!(portfolio::plan_with_reserves(&state,&c,[0,0,5],&full,&[base],10_000_000),
        Err(e)if e.code==ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn reserved_models_do_not_bypass_authority_identity_or_work_limits() -> Result<()> {
    let (state, c) = setup(8, false)?;
    let tasks = [task("a", 1)];
    let reserves = [material("buffer", 2)];
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        matches!(portfolio::plan_with_reserves(&state,&denied,[0,0,5],&tasks,&reserves,10_000_000),
        Err(e)if e.code==ErrorCode::CapabilityDenied)
    );
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(
        matches!(portfolio::plan_with_reserves(&state,&stale,[0,0,5],&tasks,&reserves,10_000_000),
        Err(e)if e.code==ErrorCode::StaleAnchor)
    );
    assert!(portfolio::plan_with_reserves(&state, &c, [0, 0, 5], &tasks, &reserves, 1).is_err());
    Ok(())
}
