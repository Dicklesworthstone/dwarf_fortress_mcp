//! Public adapter APIs: identical analysis rules, distinct coherent identities.
#[path="support/production_spatial.rs"]
mod fixture;
use dfmcp_adapter::live_operations::{LiveOperationsState,OperationsProfile,item_entity_id};
use dfmcp_adapter::live_spatial::{LiveSpatialState,SpatialStateView,citizens::LiveSpatialCitizenState};
use dfmcp_adapter::operations_analysis::{self as analysis,DiagnosisScope,MaterialDemand,OperationsStateView,MAX_ANALYSIS_WORK};
use dfmcp_core::{Capability,CapabilityGrant,CapabilityScope,DfmcpError,EntityId,ErrorCode,
    OperationContext,RequestId,Result,RiskTier,SessionId,WorkBudget};

fn invalid() -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest,"missing production test projection") }
fn context<S:OperationsStateView>(state:&S) -> Result<OperationContext> {
    let anchor=state.operations_snapshot().ok_or_else(invalid)?.anchor();
    Ok(OperationContext {session_id:SessionId::new(98123),request_id:RequestId::new(1),anchor,
        budget:WorkBudget {max_entities:100_000,max_bytes:16*1024*1024,max_output_tokens:65536,
            max_wall_millis:60_000,..WorkBudget::default()},
        grants:vec![CapabilityGrant {capability:Capability::Query,
            scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}],cancellation_requested:false})
}
fn demands() -> Vec<MaterialDemand> {
    ["a","b"].into_iter().map(|key|MaterialDemand {key:key.into(),units:4,item_types:vec!["item_type_3".into()],
        subtype:None,material_type:None,material_index:None}).collect()
}

#[test]
fn operations_spatial_and_citizen_profiles_share_rules_without_sharing_source_identity() -> Result<()> {
    let observation=fixture::observation(3,2,7,false)?;
    let mut old=LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    old.publish(observation.spatial().operations().clone())?;
    let mut map=LiveSpatialState::default();map.publish(observation.spatial().clone())?;
    let mut citizens=LiveSpatialCitizenState::default();citizens.publish(observation)?;
    let a=analysis::diagnose_production(&old,&context(&old)?,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    let b=analysis::diagnose_production(&map,&context(&map)?,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    let c=analysis::diagnose_production(&citizens,&context(&citizens)?,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    assert_eq!(a.rows,b.rows);assert_eq!(a.rows,c.rows);assert_eq!(a.finding_counts,c.finding_counts);
    assert_ne!(a.anchor,b.anchor);assert_ne!(b.anchor,c.anchor);
    assert_ne!(a.source_digest,b.source_digest);assert_ne!(b.source_digest,c.source_digest);
    assert_eq!(c.source_digest,SpatialStateView::source_digest(&citizens)?);
    let supply_a=analysis::plan_inventory(&old,&context(&old)?,&demands(),MAX_ANALYSIS_WORK)?;
    let supply_b=analysis::plan_inventory(&citizens,&context(&citizens)?,&demands(),MAX_ANALYSIS_WORK)?;
    assert_eq!(supply_a.allocation,supply_b.allocation);assert_eq!(supply_a.excluded_items,supply_b.excluded_items);
    assert_eq!(supply_b.anchor,c.anchor);assert_eq!(supply_b.source_digest,c.source_digest);
    Ok(())
}

#[test]
fn recycled_jobs_holders_and_items_retain_the_full_spatial_generation_history() -> Result<()> {
    let mut state=LiveSpatialCitizenState::default();state.publish(fixture::observation(3,1,7,false)?)?;
    let old=analysis::diagnose_production(&state,&context(&state)?,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    let old_job=old.rows.first().ok_or_else(invalid)?.job;
    state.publish(fixture::observation(4,0,0,true)?)?;
    state.publish(fixture::observation(5,1,7,false)?)?;
    let c=context(&state)?;
    assert!(matches!(analysis::diagnose_production(&state,&c,DiagnosisScope {job:Some((old_job.entity_id,old_job.generation)),
        ..DiagnosisScope::default()},MAX_ANALYSIS_WORK),Err(e)if e.code==ErrorCode::Conflict));
    let report=analysis::diagnose_production(&state,&c,DiagnosisScope {job:Some((old_job.entity_id,2)),
        ..DiagnosisScope::default()},MAX_ANALYSIS_WORK)?;
    let row=report.rows.first().ok_or_else(invalid)?;
    assert_eq!(row.job.generation,2);assert_eq!(row.holder.map(|h|h.generation),Some(2));
    assert!(!row.affected_item_examples.is_empty());assert!(row.affected_item_examples.iter().all(|h|h.generation==2));
    assert_eq!(state.snapshot().and_then(|s|s.graph.entities.get(&item_entity_id(32))).map(|e|e.generation),Some(2));
    assert_eq!(c.anchor.cursor.epoch,old.anchor.cursor.epoch);
    Ok(())
}

#[test]
fn conservative_supply_excludes_inherited_flags_and_never_double_counts_a_stack() -> Result<()> {
    let mut state=LiveSpatialCitizenState::default();state.publish(fixture::observation(3,2,7,false)?)?;
    let c=context(&state)?;
    let diagnosis=analysis::diagnose_production(&state,&c,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    assert!(diagnosis.rows.iter().all(|row|row.container_item_flags.get("forbidden")==Some(&1)));
    assert!(diagnosis.rows.iter().all(|row|row.shared_attached_items==1));
    let supply=analysis::plan_inventory(&state,&c,&demands(),MAX_ANALYSIS_WORK)?;
    assert_eq!(supply.candidate_items,1);assert_eq!(supply.candidate_stack_units,7);
    assert_eq!(supply.allocation.requested_units,8);assert_eq!(supply.allocation.allocated_units,7);
    assert_eq!(supply.allocation.assignments.iter().map(|a|a.units).sum::<u64>(),7);
    assert!(supply.allocation.assignments.iter().all(|a|a.supply_id==item_entity_id(32).get()));
    assert_eq!(supply.allocation.shortage.as_ref().map(|cut|cut.deficit),Some(1));
    Ok(())
}

#[test]
fn full_projection_authority_anchor_cancellation_and_work_limits_are_checked() -> Result<()> {
    let mut state=LiveSpatialCitizenState::default();state.publish(fixture::observation(3,1,7,false)?)?;
    let original=context(&state)?;let before=state.snapshot().cloned();
    for case in 0..5 {
        let mut c=original.clone();
        match case {
            0=>c.grants.clear(),1=>c.cancellation_requested=true,
            2=>c.anchor.cursor.sequence+=1,
            3=>c.budget.max_entities=1,
            _=>c.grants[0].scope.entity_ids=vec![EntityId::new(9)],
        }
        assert!(analysis::diagnose_production(&state,&c,DiagnosisScope::default(),MAX_ANALYSIS_WORK).is_err());
        assert!(analysis::plan_inventory(&state,&c,&demands(),MAX_ANALYSIS_WORK).is_err());
    }
    assert!(matches!(analysis::diagnose_production(&state,&original,DiagnosisScope::default(),1),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert!(matches!(analysis::plan_inventory(&state,&original,&demands(),1),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert_eq!(state.snapshot(),before.as_ref());
    Ok(())
}
