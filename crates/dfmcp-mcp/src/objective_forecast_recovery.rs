//! Source custody captured while durable recovery already reads the exact seal.
//! A forecast consumes this immutable witness without opening the store again.

use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveredProjectSource {
    pub(crate) anchor: StateAnchor,
    pub(crate) ledger: crate::lab_world::ProductionGoalLedger,
    pub(crate) records: u64,
    pub(crate) bytes: u64,
    pub(crate) structural_bytes: u64,
    pub(crate) structural_work: u64,
}

impl RecoveredProjectSource {
    pub(super) fn capture(
        snapshot: &dfmcp_world::WorldSnapshot,
        plan: &PreparedPlan,
        context: &OperationContext,
    ) -> Result<Self> {
        let started = std::time::Instant::now();
        let (structural_bytes, structural_work) =
            super::super::query_budget::source_shape(snapshot, context, started)?;
        if !snapshot.hash_is_valid() || snapshot.anchor() != plan.anchor {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered project source does not match its exact original seal",
            ));
        }
        let ledger = crate::lab_world::ProductionGoalLedger::capture(snapshot)?;
        ledger.validate_plan(plan)?;
        let records = [
            snapshot.graph.entities.len(),
            snapshot.graph.edges.len(),
            snapshot.graph.chunks.len(),
            snapshot.graph.events.len(),
        ]
        .into_iter()
        .fold(0u64, |total, count| total.saturating_add(count as u64));
        let bytes = snapshot.canonical_bytes().len() as u64;
        if records > u64::from(MAX_LAB_BUDGET.max_entities)
            || bytes > MAX_LAB_BUDGET.max_bytes.min(16 * 1024 * 1024)
            || started.elapsed() >= std::time::Duration::from_millis(context.budget.max_wall_millis)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "recovered project source exceeds its complete archive bound",
            ));
        }
        Ok(Self {
            anchor: snapshot.anchor(),
            ledger,
            records,
            bytes,
            structural_bytes,
            structural_work,
        })
    }
}

impl Objective {
    /// The exact recovery verifier checks every historical receipt and current
    /// effect identity. An unfinished tip uses only an already-retained
    /// temporal monitor; predecessors remain Verified and physically quiet.
    pub(in super::super) fn forecast_recovered_project(
        &self,
        session: &LabSession,
        predecessor: bool,
    ) -> Result<(
        RecoveredProjectSource,
        Vec<super::super::selected_plan_wait::RecoveredStepCustody>,
        Value,
    )> {
        let custody = self.selected_wait_recovery(session)?;
        let plan = self.resource_plan().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered project seal is missing",
            )
        })?;
        let source = self
            .recovered_project_source
            .as_ref()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "recovered complete-project forecast lacks its verified sealed-source witness",
                )
            })?
            .as_ref()
            .map_err(Clone::clone)?;
        if source.anchor != plan.anchor || source.anchor.state_hash != self.sealed_state_hash {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered forecast source witness changed its original anchor",
            ));
        }
        source.ledger.validate_plan(plan)?;
        source.ledger.validate_current(session.adapter.snapshot())?;
        let steps = custody.forecast_census(session, plan, predecessor)?;
        if steps.len() != plan.steps.len() || steps.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "recovered complete-project forecast requires its complete original step census",
            ));
        }
        Ok((*source, steps, custody.evidence_json()?))
    }

    /// Admit the complete retained proof owner before cloning any unfinished
    /// row. Competing recovered originals constrain the same observation loop;
    /// they receive neither new plans nor restored dispatch authority.
    pub(in super::super) fn forecast_carried_progress(
        &self,
        session: &LabSession,
    ) -> Result<(Vec<super::super::CarriedStep>, usize)> {
        let custody = self.selected_wait_recovery(session)?;
        let plan = self.resource_plan().ok_or_else(|| {
            DfmcpError::new(ErrorCode::CorruptLedger, "recovered progression seal is missing")
        })?;
        custody.forecast_census(session, plan, false)?;
        let rows = session.carried.iter()
            .filter(|row| row.plan_digest == plan.digest && !row.is_final())
            .cloned()
            .collect();
        Ok((rows, custody.bytes()?))
    }

    /// Admission above is repeated against the private descendant's accepted
    /// recovery lineage. Archived proof is not a fresh stability observation.
    pub(in super::super) fn forecast_recovered_truth_at(
        &self,
        snapshot: &dfmcp_world::WorldSnapshot,
    ) -> Result<(PredicateTruth, PredicateTruth)> {
        if self.originating_process || self.original_handles_live {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "recovered forecast cannot substitute for live original custody",
            ));
        }
        let plan = self.resource_plan().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered project seal is missing",
            )
        })?;
        let active = self.active_observation_anchor.ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::Conflict,
                "recovered project lineage is unavailable",
            )
        })?;
        self.recovered_wait_custody
            .as_ref()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "recovered project action custody is unavailable",
                )
            })?
            .validate(plan, active, snapshot.anchor())?;
        let source = self
            .recovered_project_source
            .as_ref()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "recovered project source witness is unavailable",
                )
            })?
            .as_ref()
            .map_err(Clone::clone)?;
        if source.anchor != plan.anchor || source.anchor.state_hash != self.sealed_state_hash {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered forecast no longer binds its exact archived source",
            ));
        }
        source.ledger.validate_plan(plan)?;
        source.ledger.validate_current(snapshot)?;
        self.forecast_truth_at_source(snapshot)
    }
}
