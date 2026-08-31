#![forbid(unsafe_code)]

//! Adapter-neutral owner for bounded read-only sessions.
//!
//! A `ReadSession` owns the semantic request sequence, negotiated grants,
//! budget, bootstrap anchor, and adapter. Transport identity contributes
//! nothing. The session can observe, query, and diagnose through the ordinary
//! `GameAdapter` contract, but it has no prepare/commit/cancel/checkpoint or
//! restore methods.

use dfmcp_adapter::{
    AdapterHealth, GameAdapter, ObservationFrame, ObservationRequest, QueryRequest, QueryResponse,
};
use dfmcp_core::{
    CapabilityGrant, DfmcpError, ErrorCode, FortressId, OperationContext, RequestId, Result,
    SessionId, StateAnchor, WorkBudget,
};

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadSessionMetadata {
    pub session_id: SessionId,
    pub fortress_id: FortressId,
    pub budget: WorkBudget,
    pub grants: Vec<CapabilityGrant>,
}

pub struct ReadSession<A> {
    metadata: ReadSessionMetadata,
    adapter: A,
    bootstrap_anchor: StateAnchor,
    next_request_id: u128,
    cancellation_requested: bool,
}

impl<A: GameAdapter> ReadSession<A> {
    pub fn new(
        metadata: ReadSessionMetadata,
        adapter: A,
        bootstrap_anchor: StateAnchor,
    ) -> Result<Self> {
        if metadata.session_id == SessionId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "read session identity must not be zero",
            ));
        }
        if metadata.fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "read session fortress identity must not be zero",
            ));
        }
        metadata.budget.validate()?;
        if bootstrap_anchor.fortress_id != metadata.fortress_id {
            return Err(error(
                ErrorCode::InvalidRequest,
                "read session bootstrap anchor belongs to a different fortress",
            ));
        }
        if metadata.grants.is_empty() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "read session requires at least one explicit capability grant",
            ));
        }
        for grant in &metadata.grants {
            if grant
                .scope
                .fortress_id
                .is_some_and(|fortress| fortress != metadata.fortress_id)
            {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "read session grant is scoped to a different fortress",
                ));
            }
        }
        Ok(Self {
            metadata,
            adapter,
            bootstrap_anchor,
            next_request_id: 1,
            cancellation_requested: false,
        })
    }

    #[must_use]
    pub fn metadata(&self) -> &ReadSessionMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    #[must_use]
    pub fn current_anchor(&self) -> StateAnchor {
        self.adapter
            .current_anchor()
            .unwrap_or(self.bootstrap_anchor)
    }

    #[must_use]
    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }

    pub fn request_cancellation(&mut self) {
        self.cancellation_requested = true;
    }

    pub fn clear_cancellation(&mut self) {
        self.cancellation_requested = false;
    }

    fn next_context(&mut self) -> Result<OperationContext> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "read session request identity space is exhausted",
            )
        })?;
        Ok(OperationContext {
            session_id: self.metadata.session_id,
            request_id: RequestId::new(request_id),
            anchor: self.current_anchor(),
            budget: self.metadata.budget,
            grants: self.metadata.grants.clone(),
            cancellation_requested: self.cancellation_requested,
        })
    }

    pub fn observe(
        &mut self,
        request: &ObservationRequest,
    ) -> Result<ReadSessionResult<ObservationFrame>> {
        let context = self.next_context()?;
        let value = self.adapter.observe(request, &context)?;
        Ok(ReadSessionResult { context, value })
    }

    pub fn query(
        &mut self,
        request: &QueryRequest,
    ) -> Result<ReadSessionResult<QueryResponse>> {
        let context = self.next_context()?;
        let value = self.adapter.query(request, &context)?;
        Ok(ReadSessionResult { context, value })
    }

    pub fn health(&mut self) -> Result<ReadSessionResult<AdapterHealth>> {
        let context = self.next_context()?;
        let value = self.adapter.health(&context)?;
        Ok(ReadSessionResult { context, value })
    }

    pub fn into_adapter(self) -> A {
        self.adapter
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadSessionResult<T> {
    pub context: OperationContext,
    pub value: T,
}

#[cfg(test)]
mod tests {
    use dfmcp_adapter::{InterestSet, ObservationPayload, Projection};
    use dfmcp_core::{
        Capability, CapabilityScope, Digest32, GameTick, ObservationCursor, RiskTier,
    };
    use dfmcp_lab::MemoryAdapter;
    use dfmcp_world::{WorldGraph, WorldSnapshot};

    use super::*;

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(7),
            GameTick::new(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        )
    }

    fn grant(capability: Capability) -> CapabilityGrant {
        CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(FortressId::new(7)),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }
    }

    fn session(grants: Vec<CapabilityGrant>) -> Result<ReadSession<MemoryAdapter>> {
        let bootstrap_anchor = StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick::new(0),
            state_hash: Digest32::ZERO,
        };
        ReadSession::new(
            ReadSessionMetadata {
                session_id: SessionId::new(11),
                fortress_id: FortressId::new(7),
                budget: WorkBudget::CONSERVATIVE_DEFAULT,
                grants,
            },
            MemoryAdapter::new(snapshot()),
            bootstrap_anchor,
        )
    }

    fn observation_request() -> ObservationRequest {
        ObservationRequest {
            since: None,
            projection: Projection::Summary,
            interest: InterestSet::default(),
            max_entities: WorkBudget::CONSERVATIVE_DEFAULT.max_entities,
            max_bytes: WorkBudget::CONSERVATIVE_DEFAULT.max_bytes,
            max_output_tokens: WorkBudget::CONSERVATIVE_DEFAULT.max_output_tokens,
            continuation: None,
        }
    }

    #[test]
    fn request_ids_are_owned_and_monotonic() -> Result<()> {
        let mut session = session(vec![grant(Capability::Observe)])?;
        let first = session.observe(&observation_request())?;
        let second = session.observe(&observation_request())?;
        assert_eq!(first.context.request_id, RequestId::new(1));
        assert_eq!(second.context.request_id, RequestId::new(2));
        assert!(matches!(first.value.payload, ObservationPayload::Snapshot(_)));
        Ok(())
    }

    #[test]
    fn missing_observe_grant_fails_without_transport_inference() -> Result<()> {
        let mut session = session(vec![grant(Capability::Doctor)])?;
        let failure = session
            .observe(&observation_request())
            .err()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "read session inferred observe authority",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::CapabilityDenied);
        Ok(())
    }

    #[test]
    fn cancellation_is_carried_in_the_next_operation_context() -> Result<()> {
        let mut session = session(vec![grant(Capability::Observe)])?;
        session.request_cancellation();
        let failure = session
            .observe(&observation_request())
            .err()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "cancelled read session unexpectedly observed",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::CancellationRequested);
        Ok(())
    }
}
