#[path = "support/production_portfolio_spatial.rs"]
mod fixture;
use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::workforce_analysis::portfolio::{self, ProductionTask};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, ErrorCode, OperationContext,
    RequestId, Result, RiskTier, SessionId, WorkBudget};
use std::collections::BTreeSet;

fn setup(workers: u32, units: u32, forbidden: bool) -> Result<(LiveSpatialCitizenState, OperationContext)> {
    let mut state = LiveSpatialCitizenState::default(); state.publish(fixture::observation(3,workers,units,forbidden)?)?;
    let snapshot = state.snapshot().expect("fixture snapshot");
    let context = OperationContext { session_id: SessionId::new(9701), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget { max_entities: 100_000, max_bytes: 1024*1024, max_output_tokens: 65536,
            max_wall_millis: 60_000, ..WorkBudget::default() }, cancellation_requested: false,
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }] };
    Ok((state,context))
}
fn task(key: &str, priority: u32, units: u64, workers: u32) -> ProductionTask {
    ProductionTask { key: key.into(), priority, workers, skill_key: "CARPENTRY".into(), min_effective_skill: 1,
        preserve_social: true, adults_only: true, materials: vec![MaterialDemand { key: "wood".into(), units,
            item_types: vec!["item_type_3".into()], subtype: None, material_type: None, material_index: None }] }
}

#[test]
fn joint_portfolio_reuses_candidates_not_a_stranded_full_demand_allocation() -> Result<()> {
    let (state,c) = setup(2,2,false)?;
    let requested = [task("a",3,2,2),task("b",2,1,1),task("c",2,1,1)];
    let report = portfolio::plan(&state,&c,[0,0,5],&requested,10_000_000)?;
    assert_eq!(report.selection.task_mask,0b110); assert_eq!(report.selection.priority,4);
    assert_eq!(report.inventory.allocation.allocated_by_demand,[2,0,0]);
    assert_eq!(report.selection.materials.allocated_by_demand,[0,1,1]);
    assert_eq!(report.selection.workers.allocated_by_demand,[0,1,1]);
    let ids: BTreeSet<_> = report.selection.workers.assignments.iter().map(|a|a.supply_id).collect();
    assert_eq!(ids.len(),2); assert!(report.selection.workers.assignments.iter().all(|a|a.units==1));
    assert_eq!(report.workforce.anchor,c.anchor); assert_eq!(report.inventory.anchor,c.anchor);
    assert_eq!(report.workforce.source_digest,state.source_digest()?);
    assert_eq!(report.inventory.source_digest,state.source_digest()?);
    assert_eq!(state.snapshot().expect("snapshot").anchor(),c.anchor);
    Ok(())
}

#[test]
fn unavailable_materials_or_unobserved_skills_never_support_a_complete_task() -> Result<()> {
    for forbidden in [false,true] {
        let (state,c) = setup(1,10,forbidden)?;
        let mut unknown = task("missing-skill",10,1,1); unknown.skill_key="NOT_OBSERVED".into();
        let report=portfolio::plan(&state,&c,[0,0,5],&[unknown,task("known",1,1,1)],10_000_000)?;
        assert_eq!(report.selection.task_mask,if forbidden {0}else{1});
        assert_eq!(report.workforce.skill_key_observed,[true,false]);
        assert!(!report.selection.rejected.is_empty());
        if forbidden {assert_eq!(report.inventory.item_counts.get("excluded_item_or_container_policy"),Some(&1));}
    }
    Ok(())
}

#[test]
fn equivalent_task_order_and_type_sets_produce_the_same_joint_plan() -> Result<()> {
    let (state,c)=setup(2,2,false)?;
    let requested=[task("a",1,1,1),task("b",1,1,1)];
    let first=portfolio::plan(&state,&c,[0,0,5],&requested,10_000_000)?;
    let mut other=requested.to_vec();other.reverse();other[0].materials[0].item_types.push("item_type_3".into());
    let second=portfolio::plan(&state,&c,[0,0,5],&other,10_000_000)?;
    assert_eq!(first,second); Ok(())
}

#[test]
fn authority_anchor_input_and_work_failures_do_not_produce_partial_plans() -> Result<()> {
    let (state,c)=setup(2,2,false)?;let requested=[task("a",1,1,1)];
    let mut denied=c.clone();denied.grants.clear();
    assert!(matches!(portfolio::plan(&state,&denied,[0,0,5],&requested,10_000_000),Err(e)if e.code==ErrorCode::CapabilityDenied));
    let mut stale=c.clone();stale.anchor.cursor.sequence+=1;
    assert!(matches!(portfolio::plan(&state,&stale,[0,0,5],&requested,10_000_000),Err(e)if e.code==ErrorCode::StaleAnchor));
    assert!(portfolio::plan(&state,&c,[0,0,5],&requested,1).is_err());
    assert!(portfolio::plan(&state,&c,[9,9,5],&requested,10_000_000).is_err());
    let mut bad=requested.to_vec();let duplicate=bad[0].materials[0].clone();bad[0].materials.push(duplicate);
    assert!(portfolio::plan(&state,&c,[0,0,5],&bad,10_000_000).is_err());
    assert!(portfolio::plan(&state,&c,[0,0,5],&[requested[0].clone(),requested[0].clone()],10_000_000).is_err());
    for case in 0..4 {
        let mut bad=requested.to_vec();
        match case {
            0=>bad[0].materials[0].item_types=vec!["item_type_3".into();9],
            1=>bad[0].materials[0].material_index=Some(1),
            2=>bad[0].priority=0,
            _=>bad[0].workers=129,
        }
        assert!(portfolio::plan(&state,&c,[0,0,5],&bad,10_000_000).is_err());
    }
    Ok(())
}
