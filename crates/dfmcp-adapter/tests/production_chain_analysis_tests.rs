//! Public production-chain analysis against the existing coherent wire fixture.
#[path = "support/production_spatial.rs"]
mod fixture;

use dfmcp_adapter::live_operations::{LiveOperationsState, OperationsProfile};
use dfmcp_adapter::live_spatial::{
    LiveSpatialState, SpatialStateView, citizens::LiveSpatialCitizenState,
};
use dfmcp_adapter::operations_analysis::production_chain::{
    ProductionResource, plan_production_chain,
};
use dfmcp_adapter::operations_analysis::{
    self as analysis, MAX_ANALYSIS_WORK, MaterialDemand, OperationsStateView,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, EntityId, ErrorCode, GameTick,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{BuildingKind, ProductionQuota, ProductionRecipe};

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InvalidRequest,
        "production-chain fixture missing",
    )
}
fn context<S: OperationsStateView>(state: &S) -> Result<OperationContext> {
    let anchor = state.operations_snapshot().ok_or_else(missing)?.anchor();
    Ok(OperationContext {
        session_id: SessionId::new(98125),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 16 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_wall_millis: 60_000,
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
fn resources() -> Vec<ProductionResource> {
    [("raw", "item_type_3"), ("product", "item_type_999")]
        .into_iter()
        .map(|(key, kind)| ProductionResource {
            key: key.to_owned(),
            item_types: vec![kind.to_owned()],
            subtype: None,
            material_type: None,
            material_index: None,
        })
        .collect()
}
fn quota(key: &str, amount: u32) -> ProductionQuota {
    ProductionQuota {
        item_token: key.to_owned(),
        minimum_stock: amount,
    }
}
fn recipe() -> ProductionRecipe {
    ProductionRecipe {
        output_token: "product".to_owned(),
        output_batch_size: 1,
        input_tokens: vec![("raw".to_owned(), 1)],
        job_token: "DeclaredTransformation".to_owned(),
        workshop: BuildingKind::Workshop("DeclaredWorkshop".to_owned()),
    }
}
fn state() -> Result<LiveSpatialCitizenState> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(fixture::observation(3, 2, 7, false)?)?;
    Ok(state)
}

#[test]
fn chain_uses_the_existing_conservative_stock_policy_and_preserves_final_reserves() -> Result<()> {
    let state = state()?;
    let c = context(&state)?;
    let before = state.snapshot().cloned();
    let report = plan_production_chain(
        &state,
        &c,
        &resources(),
        &[quota("product", 4), quota("raw", 4)],
        &[recipe()],
        MAX_ANALYSIS_WORK,
    )?;
    let raw = report
        .resources
        .iter()
        .find(|r| r.resource.key == "raw")
        .ok_or_else(missing)?;
    assert_eq!(raw.stock_units, 7);
    assert_eq!(raw.eligible_items, 1);
    assert_eq!(raw.examples.len(), 1);
    assert_eq!(raw.examples[0].generation, 1);
    assert_eq!(report.plan.shortages().len(), 1);
    assert_eq!(report.plan.shortages()[0].item_token, "raw");
    assert_eq!(report.plan.shortages()[0].missing_units, 1);
    let supply = analysis::plan_inventory(
        &state,
        &c,
        &[MaterialDemand {
            key: "raw".to_owned(),
            units: 1_000,
            item_types: vec!["item_type_3".to_owned()],
            subtype: None,
            material_type: None,
            material_index: None,
        }],
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(u64::from(raw.stock_units), supply.candidate_stack_units);
    assert_eq!(report.excluded_items, supply.excluded_items);
    assert_eq!(report.anchor, c.anchor);
    assert_eq!(
        report.source_digest,
        SpatialStateView::source_digest(&state)?
    );
    assert_eq!(state.snapshot(), before.as_ref());
    Ok(())
}

#[test]
fn sealed_profiles_share_quantities_but_never_source_or_model_identity() -> Result<()> {
    let observation = fixture::observation(3, 2, 7, false)?;
    let mut operations = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    operations.publish(observation.spatial().operations().clone())?;
    let mut spatial = LiveSpatialState::default();
    spatial.publish(observation.spatial().clone())?;
    let mut citizens = LiveSpatialCitizenState::default();
    citizens.publish(observation)?;
    let quotas = [quota("product", 4)];
    let recipes = [recipe()];
    let a = plan_production_chain(
        &operations,
        &context(&operations)?,
        &resources(),
        &quotas,
        &recipes,
        MAX_ANALYSIS_WORK,
    )?;
    let b = plan_production_chain(
        &spatial,
        &context(&spatial)?,
        &resources(),
        &quotas,
        &recipes,
        MAX_ANALYSIS_WORK,
    )?;
    let c = plan_production_chain(
        &citizens,
        &context(&citizens)?,
        &resources(),
        &quotas,
        &recipes,
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(a.plan, b.plan);
    assert_eq!(b.plan, c.plan);
    assert_ne!(a.anchor, b.anchor);
    assert_ne!(b.source_digest, c.source_digest);
    assert_ne!(a.model_digest, b.model_digest);
    assert_ne!(b.model_digest, c.model_digest);
    Ok(())
}

#[test]
fn overlapping_aliases_are_refused_even_when_no_item_matches() -> Result<()> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(fixture::observation(3, 0, 0, true)?)?;
    let c = context(&state)?;
    for variant in 0..4 {
        let mut definitions = resources();
        definitions[1].item_types = definitions[0].item_types.clone();
        match variant {
            1 => definitions[1].subtype = Some(7),
            2 => definitions[1].material_type = Some(3),
            3 => {
                definitions[1].material_type = Some(3);
                definitions[1].material_index = Some(2);
            }
            _ => {}
        }
        assert!(
            plan_production_chain(
                &state,
                &c,
                &definitions,
                &[quota("raw", 1)],
                &[],
                MAX_ANALYSIS_WORK
            )
            .is_err_and(|e| e.code == ErrorCode::InvalidRequest)
        );
    }
    let mut definitions = resources();
    definitions[1].item_types = definitions[0].item_types.clone();
    definitions[0].material_type = Some(1);
    definitions[1].material_type = Some(2);
    let report = plan_production_chain(
        &state,
        &c,
        &definitions,
        &[quota("raw", 1), quota("product", 1)],
        &[],
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(report.plan.shortages().len(), 2); // Empty complete roster, not unknown stock.
    Ok(())
}

#[test]
fn all_declared_references_and_duplicate_outputs_are_checked() -> Result<()> {
    let state = state()?;
    let c = context(&state)?;
    assert!(
        plan_production_chain(
            &state,
            &c,
            &resources(),
            &[quota("absent", 1)],
            &[],
            MAX_ANALYSIS_WORK
        )
        .is_err_and(|e| e.code == ErrorCode::InvalidRequest)
    );
    let mut bad = recipe();
    bad.input_tokens[0].0 = "absent".to_owned();
    assert!(
        plan_production_chain(
            &state,
            &c,
            &resources(),
            &[quota("product", 1)],
            &[bad],
            MAX_ANALYSIS_WORK
        )
        .is_err_and(|e| e.code == ErrorCode::InvalidRequest)
    );
    assert!(
        plan_production_chain(
            &state,
            &c,
            &resources(),
            &[quota("product", 1)],
            &[recipe(), recipe()],
            MAX_ANALYSIS_WORK
        )
        .is_err_and(|e| e.code == ErrorCode::InvalidRequest)
    );
    let mut bad = recipe();
    bad.workshop = BuildingKind::Custom("not-a-workshop".to_owned());
    assert!(
        plan_production_chain(
            &state,
            &c,
            &resources(),
            &[quota("raw", 0)],
            &[bad],
            MAX_ANALYSIS_WORK
        )
        .is_err_and(|e| e.code == ErrorCode::InvalidRequest)
    );
    Ok(())
}

#[test]
fn model_identity_is_canonical_and_binds_recipe_semantics() -> Result<()> {
    let state = state()?;
    let c = context(&state)?;
    let mut declaration = recipe();
    declaration.input_tokens = vec![("raw".to_owned(), 2)];
    let a = plan_production_chain(
        &state,
        &c,
        &resources(),
        &[quota("product", 2)],
        &[declaration.clone()],
        MAX_ANALYSIS_WORK,
    )?;
    let mut reversed = resources();
    reversed.reverse();
    declaration.input_tokens = vec![("raw".to_owned(), 1), ("raw".to_owned(), 1)];
    let b = plan_production_chain(
        &state,
        &c,
        &reversed,
        &[quota("product", 1), quota("product", 2)],
        &[declaration.clone()],
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(a.model_digest, b.model_digest);
    assert_eq!(a.plan, b.plan);
    declaration.job_token = "DifferentDeclaredJob".to_owned();
    let changed = plan_production_chain(
        &state,
        &c,
        &resources(),
        &[quota("product", 2)],
        &[declaration],
        MAX_ANALYSIS_WORK,
    )?;
    assert_ne!(a.model_digest, changed.model_digest);
    Ok(())
}

#[test]
fn item_examples_retain_generation_history_after_id_reuse() -> Result<()> {
    let mut state = state()?;
    let old = plan_production_chain(
        &state,
        &context(&state)?,
        &resources(),
        &[quota("product", 1)],
        &[recipe()],
        MAX_ANALYSIS_WORK,
    )?;
    state.publish(fixture::observation(4, 0, 0, true)?)?;
    state.publish(fixture::observation(5, 2, 7, false)?)?;
    let current = plan_production_chain(
        &state,
        &context(&state)?,
        &resources(),
        &[quota("product", 1)],
        &[recipe()],
        MAX_ANALYSIS_WORK,
    )?;
    assert_ne!(old.model_digest, current.model_digest);
    let raw = current
        .resources
        .iter()
        .find(|r| r.resource.key == "raw")
        .ok_or_else(missing)?;
    assert_eq!(raw.examples.len(), 1);
    assert_eq!(raw.examples[0].generation, 2);
    assert_eq!(old.plan.requirements(), current.plan.requirements());
    Ok(())
}

#[test]
fn authority_anchor_scan_cancellation_expiry_and_work_refusals_are_read_only() -> Result<()> {
    let state = state()?;
    let original = context(&state)?;
    let before = state.snapshot().cloned();
    for case in 0..8 {
        let mut c = original.clone();
        match case {
            0 => c.grants.clear(),
            1 => c.cancellation_requested = true,
            2 => c.anchor.cursor.sequence += 1,
            3 => c.budget.max_entities = 1,
            4 => c.grants[0].scope.entity_ids = [EntityId::new(9)].into_iter().collect(),
            5 => c.grants[0].expires_at_tick = Some(GameTick(0)),
            6 => c.grants[0].remaining_uses = Some(0),
            _ => c.budget.max_wall_millis = 0,
        }
        assert!(
            plan_production_chain(
                &state,
                &c,
                &resources(),
                &[quota("product", 4)],
                &[recipe()],
                MAX_ANALYSIS_WORK
            )
            .is_err()
        );
    }
    assert!(
        plan_production_chain(
            &state,
            &original,
            &resources(),
            &[quota("product", 4)],
            &[recipe()],
            1
        )
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(state.snapshot(), before.as_ref());
    Ok(())
}
