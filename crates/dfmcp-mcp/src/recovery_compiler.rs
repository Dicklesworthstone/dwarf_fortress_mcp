//! Reconstruct retained plan identities without granting legacy dispatch authority.
//!
//! Old sources did not record a compensation compiler revision. Only an exact
//! match to their complete sealed digest selects that historical interpretation.
//! Fresh intake, rebase and continuation always use the current planner.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result};
use dfmcp_intent::{Intent, PreparedPlan, StaticPlanner};
use dfmcp_world::WorldSnapshot;

pub(super) fn prepare_exact(
    snapshot: &WorldSnapshot,
    intent: &Intent,
    context: &OperationContext,
    expected: Digest32,
) -> Result<PreparedPlan> {
    let planner = StaticPlanner::default();
    let current = planner.prepare_laboratory(snapshot, intent, context);
    if let Ok(plan) = &current
        && plan.digest == expected
    {
        return current;
    }
    // This is a closed, two-version reconstruction, not a best-effort replan.
    // Neither candidate is accepted merely because it can be compiled.
    if let Ok(plan) = planner.prepare_laboratory_legacy_compensation(snapshot, intent, context)
        && plan.digest == expected
    {
        return Ok(plan);
    }
    Err(DfmcpError::new(
        ErrorCode::CorruptLedger,
        "the original source does not reproduce its complete sealed plan digest with a supported compiler revision",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId,
        ObservationCursor, RequestId, RiskTier, SessionId, WorkBudget,
    };
    use dfmcp_intent::{Action, Constraint, RequestedAction};
    use dfmcp_world::{Predicate, WorldGraph};

    fn fixture() -> (WorldSnapshot, Intent, OperationContext) {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(10),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = Intent {
            id: IntentId::new(17),
            anchor: snapshot.anchor(),
            summary: "retain a no-op setter alongside a future goal".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![RequestedAction {
                action: Action::Pause { paused: true },
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };
        let context = OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(17),
            anchor: snapshot.anchor(),
            budget: WorkBudget::default(),
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        };
        (snapshot, intent, context)
    }

    #[test]
    fn recovers_each_exact_compiler_seal_without_upgrading_the_original() -> Result<()> {
        let (snapshot, intent, context) = fixture();
        let planner = StaticPlanner::default();
        let current = planner.prepare_laboratory(&snapshot, &intent, &context)?;
        let historical =
            planner.prepare_laboratory_legacy_compensation(&snapshot, &intent, &context)?;
        assert_ne!(current.digest, historical.digest);
        assert!(current.steps[0].compensation.is_none());
        assert_eq!(
            historical.steps[0].compensation,
            Some(Action::Pause { paused: false })
        );
        assert_eq!(
            prepare_exact(&snapshot, &intent, &context, current.digest)?,
            current
        );
        assert_eq!(
            prepare_exact(&snapshot, &intent, &context, historical.digest)?,
            historical
        );
        Ok(())
    }

    #[test]
    fn altered_source_and_unrecognized_seals_never_select_a_fallback() -> Result<()> {
        let (snapshot, mut intent, context) = fixture();
        let original = StaticPlanner::default()
            .prepare_laboratory_legacy_compensation(&snapshot, &intent, &context)?;
        intent.summary.push_str(" changed");
        assert_eq!(
            prepare_exact(&snapshot, &intent, &context, original.digest)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CorruptLedger)
        );
        assert_eq!(
            prepare_exact(
                &snapshot,
                &intent,
                &context,
                Digest32::of_bytes(b"unrecognized")
            )
            .err()
            .map(|error| error.code),
            Some(ErrorCode::CorruptLedger)
        );
        Ok(())
    }
}
