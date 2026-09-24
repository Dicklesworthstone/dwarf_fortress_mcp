//! One explicitly authorized observation for a foreground condition wait.
//! The injected reader executes at most once and cannot receive mutation intent.

use dfmcp_adapter::{
    InterestSet, ObservationFrame, ObservationPayload, ObservationRequest, Projection,
};
use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use serde_json::{Value, json};

pub(super) struct RefreshedObservation {
    pub anchor: StateAnchor,
    pub summary: Value,
}

pub(super) fn once<F>(context: &OperationContext, read: F) -> Result<RefreshedObservation>
where
    F: FnOnce(&ObservationRequest, &OperationContext) -> Result<ObservationFrame>,
{
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    let request = ObservationRequest {
        since: Some(context.anchor.cursor),
        projection: Projection::Full,
        interest: InterestSet::default(),
        max_entities: context.budget.max_entities,
        max_bytes: context.budget.max_bytes,
        max_output_tokens: context.budget.max_output_tokens,
        continuation: None,
    };
    let frame = read(&request, context)?;
    if frame.truncated || frame.continuation.is_some() {
        return Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            "condition wait refuses a partial observation; no watch sample was published",
        ));
    }
    let (target, kind) = match &frame.payload {
        ObservationPayload::Snapshot(snapshot) => {
            if snapshot.graph.entities.len() > context.budget.max_entities as usize {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "condition wait observation exceeds its entity budget",
                ));
            }
            if !snapshot.hash_is_valid() {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "condition wait observation hash is invalid",
                ));
            }
            (snapshot.anchor(), "snapshot")
        }
        ObservationPayload::Heartbeat(anchor) if *anchor == context.anchor => {
            (*anchor, "heartbeat")
        }
        ObservationPayload::Heartbeat(_) | ObservationPayload::Delta(_) => {
            return Err(DfmcpError::new(
                ErrorCode::AdapterRejected,
                "condition wait requires a complete snapshot or an exact-basis heartbeat",
            ));
        }
    };
    if target.fortress_id != context.anchor.fortress_id {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "condition wait cannot switch fortress identity",
        ));
    }
    if frame.evidence.len() > 64
        || frame.warnings.len() > 32
        || frame.warnings.iter().any(|warning| warning.len() > 4096)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "condition wait observation evidence exceeds its bounds",
        ));
    }
    let summary = json!({"kind":kind,"basis":super::anchor_json(context.anchor),
        "target":super::anchor_json(target),"reset":target.cursor.epoch!=context.anchor.cursor.epoch,
        "read_calls":1,"advanced_game":false,"warnings":frame.warnings,
        "evidence":frame.evidence.iter().map(|evidence|evidence.digest.to_string()).collect::<Vec<_>>()});
    Ok(RefreshedObservation {
        anchor: target,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
        SessionId, WorkBudget,
    };
    use dfmcp_world::{WorldGraph, WorldSnapshot};

    fn snapshot(tick: u64, sequence: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor { epoch: 0, sequence },
            true,
            WorldGraph::default(),
        )
    }
    fn context() -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: snapshot(1, 0).anchor(),
            budget: WorkBudget::default(),
            grants: [Capability::Observe, Capability::Query]
                .into_iter()
                .map(|capability| CapabilityGrant {
                    capability,
                    scope: CapabilityScope {
                        fortress_id: Some(FortressId::new(1)),
                        ..CapabilityScope::default()
                    },
                    max_risk: RiskTier::ReadOnly,
                    expires_at_tick: None,
                    remaining_uses: None,
                })
                .collect(),
            cancellation_requested: false,
        }
    }
    fn frame(payload: ObservationPayload) -> ObservationFrame {
        ObservationFrame {
            payload,
            evidence: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
            continuation: None,
        }
    }

    #[test]
    fn read_permission_and_cancellation_are_checked_before_the_reader_runs() {
        for case in 0..3 {
            let mut ctx = context();
            match case {
                0 => ctx
                    .grants
                    .retain(|grant| grant.capability != Capability::Observe),
                1 => ctx
                    .grants
                    .retain(|grant| grant.capability != Capability::Query),
                _ => ctx.cancellation_requested = true,
            }
            let mut calls = 0;
            let result = once(&ctx, |_, _| {
                calls += 1;
                Ok(frame(ObservationPayload::Snapshot(snapshot(2, 1))))
            });
            assert!(result.is_err());
            assert_eq!(calls, 0);
        }
    }

    #[test]
    fn one_refresh_retains_bounds_basis_and_read_only_evidence() -> Result<()> {
        let ctx = context();
        let mut calls = 0;
        let result = once(&ctx, |request, seen| {
            calls += 1;
            assert_eq!(seen.anchor, ctx.anchor);
            assert_eq!(request.since, Some(ctx.anchor.cursor));
            assert_eq!(request.max_bytes, ctx.budget.max_bytes);
            assert_eq!(request.max_entities, ctx.budget.max_entities);
            assert_eq!(request.projection, Projection::Full);
            assert!(request.continuation.is_none());
            Ok(frame(ObservationPayload::Snapshot(snapshot(2, 1))))
        })?;
        assert_eq!(calls, 1);
        assert_eq!(result.anchor, snapshot(2, 1).anchor());
        assert_eq!(result.summary["read_calls"], 1);
        assert_eq!(result.summary["advanced_game"], false);
        assert_eq!(
            result.summary["basis"],
            super::super::anchor_json(ctx.anchor)
        );
        Ok(())
    }

    #[test]
    fn exact_heartbeats_do_not_invent_new_observations() -> Result<()> {
        let ctx = context();
        let result = once(&ctx, |_, _| {
            Ok(frame(ObservationPayload::Heartbeat(ctx.anchor)))
        })?;
        assert_eq!(result.anchor, ctx.anchor);
        assert_eq!(result.summary["kind"], "heartbeat");
        assert!(
            once(&ctx, |_, _| Ok(frame(ObservationPayload::Heartbeat(
                snapshot(2, 1).anchor()
            ))))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn partial_invalid_or_failed_reads_never_return_a_watch_sample() {
        let ctx = context();
        let mut partial = frame(ObservationPayload::Snapshot(snapshot(2, 1)));
        partial.truncated = true;
        assert!(once(&ctx, |_, _| Ok(partial)).is_err());
        let mut invalid = snapshot(2, 1);
        invalid.paused = false;
        assert!(
            once(&ctx, |_, _| Ok(frame(ObservationPayload::Snapshot(
                invalid
            ))))
            .is_err()
        );
        let mut foreign = snapshot(2, 1);
        foreign.fortress_id = FortressId::new(2);
        foreign.refresh_hash();
        assert!(
            once(&ctx, |_, _| Ok(frame(ObservationPayload::Snapshot(
                foreign
            ))))
            .is_err()
        );
        let mut calls = 0;
        let result = once(&ctx, |_, _| {
            calls += 1;
            Err(DfmcpError::new(ErrorCode::AdapterFailure, "lost read"))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
