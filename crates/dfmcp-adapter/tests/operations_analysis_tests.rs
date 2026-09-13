#![forbid(unsafe_code)]

use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_operations::{JobItemAttachment, LiveBuilding, LiveItem,
    LiveOperationsObservation, LiveOperationsState, building_entity_id, item_entity_id};
use dfmcp_adapter::operations_analysis::{DiagnosisScope, MaterialDemand,
    MAX_ANALYSIS_WORK, diagnose_production, plan_inventory};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, EntityId, ErrorCode,
    MapCoord, OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget};

fn item(id: u32, name: &str, units: u32) -> LiveItem {
    LiveItem { native_id: id, item_type: 1, type_key: name.to_owned(), subtype: -1,
        material_type: 0, material_index: 1, stack_size: units,
        raw_position: MapCoord::new(1, 2, 3), flags: 64,
        container_native_id: None, holder_building_native_id: None }
}
fn observation() -> LiveOperationsObservation {
    let mut stock = item(10, "BAR", 5); stock.container_native_id = Some(20); stock.flags = 0;
    let mut container = item(20, "BIN", 1); container.flags |= 1;
    let mut first = LiveJob { native_id: 1, job_type: 5, type_key: "ConstructBuilding".to_owned(),
        reaction: String::new(), suspended: true, repeating: false, position: MapCoord::new(1,2,3),
        worker_native_id: None, holder_native_id: Some(0), completion_timer: -1,
        attached_item_count: 2, required_item_filter_count: 1 };
    let mut second = first.clone(); second.native_id = 2; second.suspended = false;
    second.holder_native_id = None; second.worker_native_id = Some(7); second.attached_item_count = 1;
    first.repeating = true;
    LiveOperationsObservation {
        jobs: LiveJobObservation { bridge_generation: 7, df_version: "df-test".to_owned(),
            dfhack_version: "dfhack-test".to_owned(), year: 105, year_tick: 3, paused: true,
            site_id: 1, world_folder: "region1".to_owned(), next_job_id: 3, jobs: vec![first, second] },
        next_building_id: 1, next_item_id: 21,
        buildings: vec![LiveBuilding { native_id: 0, building_type: 1, type_key: "Workshop".to_owned(),
            x1: 0, y1: 0, x2: 2, y2: 2, z: 3, build_stage: 1, max_build_stage: 3 }],
        items: vec![stock, item(11,"BAR",3), item(12,"BOULDER",4), container],
        attachments: vec![JobItemAttachment { job_native_id: 1, item_native_id: 10, role: 0, filter_index: 0 },
            JobItemAttachment { job_native_id: 1, item_native_id: 10, role: 1, filter_index: 0 },
            JobItemAttachment { job_native_id: 2, item_native_id: 10, role: 0, filter_index: 0 }],
    }
}
fn fixture(value: LiveOperationsObservation) -> Result<(LiveOperationsState, OperationContext)> {
    let mut state = LiveOperationsState::default(); state.publish(value)?;
    let snapshot = state.snapshot().ok_or_else(|| dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation,"fixture"))?;
    let context = OperationContext { session_id: SessionId::new(17), request_id: RequestId::new(1),
        anchor: snapshot.anchor(), budget: WorkBudget { max_entities: 4096, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query,
            scope: CapabilityScope { fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false };
    Ok((state, context))
}
fn demand(key: &str, units: u64, types: &[&str]) -> MaterialDemand {
    MaterialDemand { key: key.to_owned(), units, item_types: types.iter().map(|s| (*s).to_owned()).collect(),
        subtype: None, material_type: None, material_index: None }
}

#[test]
fn diagnostics_join_holders_containers_and_shared_inputs_without_duplicate_counts() -> Result<()> {
    let (state, context) = fixture(observation())?;
    let result = diagnose_production(&state, &context, DiagnosisScope::default(), MAX_ANALYSIS_WORK)?;
    assert_eq!(result.jobs_considered, 2); assert_eq!(result.jobs_with_findings, 2);
    let first = &result.rows[0];
    assert_eq!(first.job.entity_id, EntityId::new(3));
    assert_eq!(first.holder.map(|h| h.entity_id), Some(building_entity_id(0)));
    assert_eq!(first.holder_stage, Some((1, 3)));
    assert_eq!(first.attachment_records, 2); assert_eq!(first.distinct_attached_items, 1);
    assert_eq!(first.filters_without_indexed_attachments, 0);
    assert!(first.direct_item_flags.is_empty());
    assert_eq!(first.container_item_flags.get("forbidden"), Some(&1));
    assert_eq!(first.shared_attached_items, 1); assert_eq!(first.affected_item_count, 1);
    assert_eq!(first.affected_item_examples[0].entity_id, item_entity_id(10));
    assert!(first.findings.contains(&"holder_under_construction"));
    assert_eq!(result.anchor, context.anchor);
    assert_eq!(result.source_digest, observation().source_digest()?);
    Ok(())
}

#[test]
fn allocation_excludes_actual_attachments_and_container_flags_not_just_item_flags() -> Result<()> {
    let (state, context) = fixture(observation())?;
    let result = plan_inventory(&state, &context, &[demand("bars", 4, &["BAR"])], MAX_ANALYSIS_WORK)?;
    assert_eq!(result.candidate_items, 1); assert_eq!(result.candidate_stack_units, 3);
    assert_eq!(result.allocation.allocated_units, 3);
    assert_eq!(result.allocation.shortage.as_ref().map(|s|s.deficit), Some(1));
    assert_eq!(result.excluded_items.get("forbidden_in_chain"), Some(&2));
    assert_eq!(result.unmatched_items, 1);
    let mut value = observation(); value.items[3].flags = 64;
    let (state, context) = fixture(value)?;
    let result = plan_inventory(&state, &context, &[demand("bars", 8, &["BAR"])], MAX_ANALYSIS_WORK)?;
    assert_eq!(result.excluded_items.get("attached_to_job_in_chain"), Some(&1));
    assert_eq!(result.allocation.allocated_units, 3);
    Ok(())
}

#[test]
fn explicit_overlapping_demands_share_one_integral_supply_model() -> Result<()> {
    let mut value = observation(); value.jobs.jobs.clear(); value.attachments.clear();
    value.items = vec![item(10,"BAR",2), item(11,"BOULDER",2)];
    let (state, context) = fixture(value)?;
    let input = [demand("specific", 2, &["BAR"]), demand("flexible", 3, &["BOULDER","BAR","BAR"])];
    let result = plan_inventory(&state, &context, &input, MAX_ANALYSIS_WORK)?;
    assert_eq!(result.allocation.requested_units, 5); assert_eq!(result.allocation.allocated_units, 4);
    assert_eq!(result.allocation.cut_capacity, 4);
    assert_eq!(result.demands[0].key,"flexible");
    assert_eq!(result.demands[0].item_types, vec!["BAR","BOULDER"]);
    let mut reversed = input.to_vec(); reversed.reverse();
    assert_eq!(result, plan_inventory(&state, &context, &reversed, MAX_ANALYSIS_WORK)?);
    Ok(())
}

#[test]
fn raw_material_and_subtype_selectors_are_exact_not_display_name_guesses() -> Result<()> {
    let mut value = observation(); value.jobs.jobs.clear(); value.attachments.clear();
    value.items = vec![item(10,"BAR",2), item(11,"BAR",5)];
    value.items[1].material_index = 2; value.items[1].subtype = 9;
    let (state, context) = fixture(value)?;
    let mut request = demand("copper_model",7,&["BAR"]); request.material_type=Some(0); request.material_index=Some(1);
    let result = plan_inventory(&state,&context,&[request.clone()],MAX_ANALYSIS_WORK)?;
    assert_eq!(result.allocation.allocated_units,2);
    request.material_index=Some(2); request.subtype=Some(9);
    assert_eq!(plan_inventory(&state,&context,&[request],MAX_ANALYSIS_WORK)?.allocation.allocated_units,5);
    Ok(())
}

#[test]
fn clear_jobs_are_not_promoted_to_ready_and_scope_is_generation_checked() -> Result<()> {
    let mut value=observation(); value.jobs.jobs.truncate(1); value.jobs.jobs[0].suspended=false;
    value.jobs.jobs[0].worker_native_id=Some(2); value.jobs.jobs[0].holder_native_id=None;
    value.jobs.jobs[0].attached_item_count=0; value.jobs.jobs[0].required_item_filter_count=0; value.attachments.clear();
    let (state,ctx)=fixture(value)?;
    let result=diagnose_production(&state,&ctx,DiagnosisScope::default(),MAX_ANALYSIS_WORK)?;
    assert_eq!(result.jobs_considered,1); assert_eq!(result.jobs_with_findings,0); assert!(result.rows.is_empty());
    let scope=DiagnosisScope {include_clear_jobs:true,..DiagnosisScope::default()};
    assert!(diagnose_production(&state,&ctx,scope,MAX_ANALYSIS_WORK)?.rows[0].findings.is_empty());
    let scope=DiagnosisScope {job:Some((EntityId::new(3),2)),..scope};
    assert!(matches!(diagnose_production(&state,&ctx,scope,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::Conflict));
    let scope=DiagnosisScope {job:Some((item_entity_id(10),1)),..scope};
    assert!(matches!(diagnose_production(&state,&ctx,scope,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::InvalidRequest));
    Ok(())
}

#[test]
fn holder_focus_narrows_jobs_without_weakening_authority_scope() -> Result<()> {
    let (state,mut ctx)=fixture(observation())?;
    let scope=DiagnosisScope {holder:Some((building_entity_id(0),1)),..DiagnosisScope::default()};
    let result=diagnose_production(&state,&ctx,scope,MAX_ANALYSIS_WORK)?;
    assert_eq!(result.jobs_considered,1); assert_eq!(result.rows[0].native_job_id,1);
    ctx.grants[0].scope.entity_ids.insert(EntityId::new(3));
    assert!(matches!(diagnose_production(&state,&ctx,scope,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::CapabilityDenied));
    Ok(())
}

#[test]
fn deep_container_chains_are_linear_and_policy_rejections_are_explicit() -> Result<()> {
    let mut value=observation();value.jobs.jobs.clear();value.attachments.clear();value.items.clear();value.next_item_id=201;
    for id in 1..=200 {
        let mut entry=item(id,"WOOD",1);
        entry.container_native_id=(id<200).then_some(id+1);
        if id==200 {entry.flags|=1;}
        value.items.push(entry);
    }
    let (state,ctx)=fixture(value)?;
    let result=plan_inventory(&state,&ctx,&[demand("logs",1,&["WOOD"])],MAX_ANALYSIS_WORK)?;
    assert_eq!(result.excluded_items.get("forbidden_in_chain"),Some(&200));
    assert_eq!(result.candidate_items,0);assert!(result.work_units<1500);
    Ok(())
}

#[test]
fn authority_anchor_scan_work_and_input_failures_do_not_return_partial_analysis() -> Result<()> {
    let (state,ctx)=fixture(observation())?;
    let request=[demand("bars",2,&["BAR"])];
    let mut changed=ctx.clone();changed.grants.clear();
    assert!(matches!(plan_inventory(&state,&changed,&request,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::CapabilityDenied));
    changed=ctx.clone();changed.cancellation_requested=true;
    assert!(matches!(plan_inventory(&state,&changed,&request,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::CancellationRequested));
    changed=ctx.clone();changed.anchor.cursor.sequence+=1;
    assert!(matches!(plan_inventory(&state,&changed,&request,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::StaleAnchor));
    changed=ctx.clone();changed.budget.max_entities=1;
    assert!(matches!(plan_inventory(&state,&changed,&request,MAX_ANALYSIS_WORK),Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert!(matches!(plan_inventory(&state,&ctx,&request,1),Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert!(matches!(diagnose_production(&state,&ctx,DiagnosisScope::default(),1),Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert!(plan_inventory(&state,&ctx,&[request[0].clone(),request[0].clone()],MAX_ANALYSIS_WORK).is_err());
    let mut bad=request[0].clone();bad.material_index=Some(2);
    assert!(plan_inventory(&state,&ctx,&[bad],MAX_ANALYSIS_WORK).is_err());
    Ok(())
}
