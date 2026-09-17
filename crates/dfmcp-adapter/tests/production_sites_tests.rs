#[path="support/production_site_spatial.rs"]
mod fixture;
use std::collections::{BTreeMap, BTreeSet};
use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::workforce_analysis::portfolio::{self, ProductionTask, selection::RESERVE_OWNER};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, ErrorCode, OperationContext,
    RequestId, Result, RiskTier, SessionId, WorkBudget};

const WEST:[u32;3]=[0,0,5];
const EAST:[u32;3]=[4,0,5];
fn setup(west:u32,east:u32,separated:bool)->Result<(LiveSpatialCitizenState,OperationContext)> {
    let mut state=LiveSpatialCitizenState::default(); state.publish(fixture::observation(3,west,east,separated)?)?;
    let anchor=state.snapshot().expect("fixture snapshot").anchor();
    let c=OperationContext {session_id:SessionId::new(98601),request_id:RequestId::new(1),anchor,
        budget:WorkBudget {max_entities:100_000,max_bytes:1024*1024,max_output_tokens:65536,
            max_wall_millis:60000,..WorkBudget::default()},cancellation_requested:false,
        grants:vec![CapabilityGrant {capability:Capability::Query,scope:CapabilityScope::default(),
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}]};
    Ok((state,c))
}
fn material(key:&str,units:u64)->MaterialDemand {
    MaterialDemand {key:key.into(),units,item_types:vec!["item_type_3".into()],
        subtype:None,material_type:None,material_index:None}
}
fn task(key:&str,priority:u32,units:u64)->ProductionTask {
    ProductionTask {key:key.into(),priority,workers:1,skill_key:"CARPENTRY".into(),
        min_effective_skill:1,preserve_social:true,adults_only:true,materials:vec![material("wood",units)]}
}
fn sites()->BTreeMap<String,[u32;3]> {BTreeMap::from([("b".into(),EAST)])}

#[test]
fn disconnected_sites_can_support_separate_complete_tasks()->Result<()> {
    let(state,c)=setup(1,1,true)?;let tasks=[task("a",2,1),task("b",3,1)];
    let common=portfolio::plan(&state,&c,WEST,&tasks,10_000_000)?;
    assert_eq!(common.selection.task_mask,2);
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&sites(),10_000_000)?;
    assert_eq!(joint.selection.task_mask,3);assert_eq!(joint.selection.workers.allocated_units,2);
    assert_eq!(joint.selection.materials.allocated_units,2);assert_eq!(joint.sites.supplies.len(),2);
    assert_eq!(joint.sites.supplies[0].eligible,1);assert_eq!(joint.sites.supplies[1].eligible,2);
    assert_eq!(joint.workforce.candidates.iter().map(Vec::len).collect::<Vec<_>>(),[1,1]);
    let workers:BTreeSet<_>=joint.selection.workers.assignments.iter().map(|a|a.supply_id).collect();
    assert_eq!(workers.len(),2);
    for a in &joint.selection.materials.assignments {
        let local=joint.inventory_for(a.demand_index)?;
        assert_eq!(local.origin,joint.sites.task_origins[a.demand_index]);
        assert!(local.locations.contains_key(&a.supply_id));assert_eq!(local.anchor,c.anchor);
        assert_eq!(local.source_digest,state.source_digest()?);
    }
    assert_eq!(state.snapshot().expect("snapshot").anchor(),c.anchor);Ok(())
}

#[test]
fn one_stack_reachable_from_multiple_sites_is_still_one_stack()->Result<()> {
    let(state,c)=setup(1,0,false)?;let tasks=[task("a",2,1),task("b",3,1)];
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&sites(),10_000_000)?;
    assert_eq!(joint.sites.supplies.len(),1);assert_eq!(joint.sites.supplies[0].eligible,3);
    assert_eq!(joint.sites.supplies[0].units,1);assert_eq!(joint.selection.task_mask,2);
    assert_eq!(joint.selection.materials.allocated_units,1);
    assert_eq!(joint.selection.rejected[0].shortage.deficit,1);Ok(())
}

#[test]
fn stock_at_the_wrong_site_cannot_support_the_task()->Result<()> {
    let(state,c)=setup(2,0,true)?;let tasks=[task("a",1,1),task("b",100,1)];
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&sites(),10_000_000)?;
    assert_eq!(joint.selection.task_mask,1);
    assert!(joint.selection.rejected.iter().any(|r|r.task_mask==2));
    assert_eq!(joint.sites.supplies[0].eligible,1);Ok(())
}

#[test]
fn reserve_support_stays_at_default_origin_and_competes_globally()->Result<()> {
    let(state,c)=setup(1,1,true)?;let tasks=[task("a",100,1),task("b",1,1)];
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[material("buffer",1)],&sites(),10_000_000)?;
    assert_eq!(joint.selection.task_mask,2);assert_eq!(joint.selection.materials.allocated_units,2);
    let reserve=joint.material_owners.iter().position(|o|*o==RESERVE_OWNER).expect("reserve owner");
    assert_eq!(joint.inventory_for(reserve)?.origin,WEST);
    assert_eq!(joint.sites.material_origins,[WEST,EAST,WEST]);
    let ids:BTreeSet<_>=joint.selection.materials.assignments.iter().map(|a|a.supply_id).collect();
    assert_eq!(ids.len(),2);Ok(())
}

#[test]
fn unreachable_remote_stock_cannot_repair_a_hard_reserve_shortfall()->Result<()> {
    let(state,c)=setup(1,100,true)?;let tasks=[task("a",1,1),task("b",100,1)];
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[material("buffer",2)],&sites(),10_000_000)?;
    assert_eq!(joint.selection.task_mask,0);
    let cut=joint.selection.materials.shortage.as_ref().expect("reserve shortage");
    assert_eq!((cut.required_units,cut.eligible_units,cut.deficit),(2,1,1));
    assert!(cut.demand_indices.iter().all(|i|joint.material_owners[*i]==RESERVE_OWNER));Ok(())
}

#[test]
fn task_order_and_explicit_default_sites_do_not_change_the_model()->Result<()> {
    let(state,c)=setup(2,2,false)?;let tasks=[task("a",2,1),task("b",3,1)];
    let first=portfolio::plan_with_reserves(&state,&c,WEST,&tasks,&[material("buffer",1)],10_000_000)?;
    let explicit=BTreeMap::from([("a".into(),WEST),("b".into(),WEST)]);
    let same=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[material("buffer",1)],&explicit,10_000_000)?;
    assert_eq!(first,same);
    let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&sites(),10_000_000)?;
    let reversed=[tasks[1].clone(),tasks[0].clone()];
    assert_eq!(joint,portfolio::plan_at_sites(&state,&c,WEST,&reversed,&[],&sites(),10_000_000)?);Ok(())
}

#[test]
fn malformed_sites_and_budget_or_authority_failures_return_no_plan()->Result<()> {
    let(state,c)=setup(2,2,true)?;let tasks=[task("a",1,1),task("b",100,1)];
    for overrides in [BTreeMap::from([("missing".into(),EAST)]),BTreeMap::from([("a".into(),[2,0,5])]),
        BTreeMap::from([("a".into(),[99,0,5])]),BTreeMap::from([("a".into(),[32768,0,5])])] {
        assert!(portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&overrides,10_000_000).is_err());
    }
    assert!(portfolio::plan_at_sites(&state,&c,WEST,&tasks,&[],&sites(),1).is_err());
    let mut denied=c.clone();denied.grants.clear();
    assert!(matches!(portfolio::plan_at_sites(&state,&denied,WEST,&tasks,&[],&sites(),10_000_000),Err(e)if e.code==ErrorCode::CapabilityDenied));
    assert_eq!(state.snapshot().expect("snapshot").anchor(),c.anchor);Ok(())
}

#[test]
fn small_site_portfolios_match_independent_capacity_enumeration()->Result<()> {
    for separated in [false,true] {for west in 0..=3 {for east in 0..=3 {
        let(state,c)=setup(west,east,separated)?;
        for a in 1..=2u64 {for b in 1..=2u64 {for reserve in 0..=1u64 {
            let tasks=[task("a",2,a),task("b",3,b)];
            let reserves=if reserve==0 {vec![]}else{vec![material("buffer",reserve)]};
            let joint=portfolio::plan_at_sites(&state,&c,WEST,&tasks,&reserves,&sites(),10_000_000)?;
            let feasible=|mask:u16| {
                let x=if mask&1!=0 {a}else{0};let y=if mask&2!=0 {b}else{0};
                if separated {x+reserve<=u64::from(west)&&y<=u64::from(east)}
                else {x+y+reserve<=u64::from(west+east)}
            };
            let expected=(0..=3u16).filter(|m|feasible(*m)).max_by_key(|m|
                u32::from(m&1!=0)*2+u32::from(m&2!=0)*3);
            assert_eq!(joint.selection.task_mask,expected.unwrap_or(0));
            assert_eq!(joint.selection.materials.shortage.is_none(),expected.is_some());
        }}}
    }}}
    Ok(())
}
