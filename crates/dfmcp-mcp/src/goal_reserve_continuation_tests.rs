//! Future reserve planning preserves its absolute horizon through new pursuits.

use super::*;
use crate::lab_world::scenario_snapshot;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, RequestId, SessionId, WorkBudget};
use dfmcp_lab::MemoryAdapter;

fn world() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_provisions", FortressId::new(781), false)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}

fn source(until: u64) -> Result<ProductionContinuation> {
    let request = ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": 60}, {"item": "FOOD", "minimum": 65}],
            "reserve_until_tick": until,
        })
        .to_string(),
    )?;
    let root = Digest32::of_bytes(b"original explicit reserve goal");
    ProductionContinuation::new(root, root, request)
}

#[test]
fn reserve_source_reopens_to_the_exact_original_compiler_plan_and_seal() -> Result<()> {
    let snapshot = world()?;
    let original = source(4801)?;
    let bytes = original.canonical_json();
    let reopened = ProductionContinuation::parse(&bytes)?;
    assert_eq!(original, reopened);
    assert_eq!(reopened.canonical_json(), bytes);
    assert_eq!(reopened.version, SourceVersion::ConsumptionReserveV1);
    assert_eq!(reopened.reserve_until_tick(), Some(4801));
    let mut expected_digest = b"dfmcp-production-continuation-source/3\0".to_vec();
    expected_digest.extend_from_slice(bytes.as_bytes());
    assert_eq!(
        original.source_digest(),
        Digest32::of_bytes(&expected_digest)
    );
    let first = original.compile(IntentId::new(19), &snapshot, "future reserve")?;
    let replayed = reopened.compile(IntentId::new(19), &snapshot, "future reserve")?;
    assert_eq!(first, replayed);
    let planner = StaticPlanner::default();
    assert_eq!(
        planner.prepare_laboratory(&snapshot, &first.0, &context(&snapshot))?,
        planner.prepare_laboratory(&snapshot, &replayed.0, &context(&snapshot))?,
    );
    assert_eq!(
        first.1["continuation"]["source_schema"],
        RESERVE_SOURCE_SCHEMA
    );
    assert_eq!(first.1["continuation"]["reserve_until_tick"], 4801);
    assert_eq!(
        first.1["continuation"]["continuous_maintenance_proven"],
        false
    );
    Ok(())
}

#[test]
fn archive_generation_cannot_downgrade_or_invent_an_explicit_horizon() -> Result<()> {
    let original = source(4801)?;
    let raw: Value = serde_json::from_str(&original.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for schema in [LEGACY_SOURCE_SCHEMA, SOURCE_SCHEMA] {
        let mut changed = raw.clone();
        changed["schema"] = json!(schema);
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    let mut missing = raw.clone();
    missing["production"]
        .as_object_mut()
        .ok_or_else(|| invalid("fixture production missing"))?
        .remove("reserve_until_tick");
    assert!(ProductionContinuation::parse(&missing.to_string()).is_err());
    let mut old_generation = raw;
    old_generation["production"]["planner"] = json!("consumption_aware_v1");
    assert!(ProductionContinuation::parse(&old_generation.to_string()).is_err());
    assert!(
        ProductionContinuation::from_parts(
            original.parent,
            original.root,
            original
                .request
                .clone()
                .with_planner(ProductionPlanner::ConsumptionAwareV1),
            SourceVersion::ConsumptionAwareV1,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn subsequent_pursuits_keep_the_absolute_tick_and_flat_original_lineage() -> Result<()> {
    let original = source(4801)?;
    let mut current = original.clone();
    for ordinal in 0u64..8 {
        let parent = Digest32::of_bytes(&ordinal.to_le_bytes());
        current = current.next(parent)?;
        assert_eq!(current.parent, parent);
        assert_eq!(current.root, original.root);
        assert_eq!(current.request, original.request);
        assert_eq!(current.reserve_until_tick(), Some(4801));
        assert_eq!(current.version, SourceVersion::ConsumptionReserveV1);
        assert_eq!(
            current.canonical_json().len(),
            original.canonical_json().len()
        );
        current = ProductionContinuation::parse(&current.canonical_json())?;
    }
    let different_horizon = source(4802)?;
    assert!(!original.same_original_request(&different_horizon));
    assert_ne!(original.source_digest(), different_horizon.source_digest());
    Ok(())
}

#[test]
fn later_source_tick_reduces_remaining_reserve_time_without_renewing_it() -> Result<()> {
    let source = source(4801)?;
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(1200)?;
    let (_, analysis) =
        source.compile(IntentId::new(20), adapter.snapshot(), "continue reserve")?;
    assert_eq!(analysis["consumption"]["source_tick"], 1201);
    assert_eq!(analysis["consumption"]["reserve_until_tick"], 4801);
    assert_eq!(analysis["consumption"]["reserve_horizon_ticks"], 3600);
    assert_eq!(
        analysis["continuation"]["original_absolute_horizon_preserved"],
        true
    );
    adapter.advance_ticks(3600)?;
    assert!(
        source
            .compile(IntentId::new(21), adapter.snapshot(), "expired reserve")
            .is_err()
    );
    let next = source.next(Digest32::of_bytes(b"new parent after expiry"))?;
    assert_eq!(next.reserve_until_tick(), Some(4801));
    assert!(
        next.compile(IntentId::new(22), adapter.snapshot(), "cannot renew")
            .is_err()
    );
    Ok(())
}
