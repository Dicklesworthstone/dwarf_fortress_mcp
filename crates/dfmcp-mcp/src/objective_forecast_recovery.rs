//! Source custody captured while durable recovery already reads the exact seal.
//! A forecast consumes this immutable witness without opening the store again.

use super::*;

/// Complete sealed-source custody for an already committed original. Raw
/// excavation and harvesting need no production ledger. This witness never
/// retains a mutable snapshot or restores a dispatch handle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveredOriginalSource {
    pub(crate) anchor: StateAnchor,
    pub(crate) records: u64,
    pub(crate) bytes: u64,
    pub(crate) structural_bytes: u64,
    pub(crate) structural_work: u64,
    plan_digest: Digest32,
    plan_object_digest: Digest32,
}

impl RecoveredOriginalSource {
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
                "recovered original source does not match its exact sealed anchor",
            ));
        }
        let records = [
            snapshot.graph.entities.len(),
            snapshot.graph.edges.len(),
            snapshot.graph.chunks.len(),
            snapshot.graph.events.len(),
        ]
        .into_iter()
        .fold(0u64, |total, count| total.saturating_add(count as u64));
        let bytes = snapshot.canonical_bytes().len() as u64;
        let (_, seal) = dfmcp_intent::plan_archive::encode(
            plan,
            dfmcp_intent::plan_archive::ArchiveLimits {
                max_steps: 64,
                ..dfmcp_intent::plan_archive::ArchiveLimits::default()
            },
        )?;
        if records > u64::from(MAX_LAB_BUDGET.max_entities)
            || bytes > MAX_LAB_BUDGET.max_bytes.min(16 * 1024 * 1024)
            || started.elapsed() >= std::time::Duration::from_millis(context.budget.max_wall_millis)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "recovered original source exceeds its complete archive bound",
            ));
        }
        Ok(Self {
            anchor: snapshot.anchor(),
            records,
            bytes,
            structural_bytes,
            structural_work,
            plan_digest: plan.digest,
            plan_object_digest: seal.object_digest,
        })
    }

    /// Admission binds the whole typed object, including fields outside the
    /// semantic plan digest. The owning immutable Objective is then borrowed
    /// throughout private evaluation; there is no archive read or hydration.
    fn validate_plan(&self, plan: &PreparedPlan) -> Result<()> {
        let (_, seal) = dfmcp_intent::plan_archive::encode(
            plan,
            dfmcp_intent::plan_archive::ArchiveLimits {
                max_steps: 64,
                ..dfmcp_intent::plan_archive::ArchiveLimits::default()
            },
        )?;
        if self.anchor != plan.anchor
            || self.plan_digest != plan.digest
            || self.plan_object_digest != seal.object_digest
        {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered forecast source lost its exact typed original",
            ));
        }
        Ok(())
    }
}

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
        source: RecoveredOriginalSource,
        context: &OperationContext,
        started: std::time::Instant,
    ) -> Result<Self> {
        source.validate_plan(plan)?;
        if snapshot.anchor() != source.anchor {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered project source changed its admitted sealed world",
            ));
        }
        let ledger = crate::lab_world::ProductionGoalLedger::capture(snapshot)?;
        ledger.validate_plan(plan)?;
        if started.elapsed() >= std::time::Duration::from_millis(context.budget.max_wall_millis) {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "recovered project source exhausted its complete capture deadline",
            ));
        }
        Ok(Self {
            anchor: source.anchor,
            ledger,
            records: source.records,
            bytes: source.bytes,
            structural_bytes: source.structural_bytes,
            structural_work: source.structural_work,
        })
    }
}

impl Objective {
    /// Current-stage prediction needs the exact original and source census,
    /// without assuming that a raw action belongs to a production project.
    pub(in super::super) fn forecast_recovered_current_stage(
        &self,
        session: &LabSession,
    ) -> Result<(
        RecoveredOriginalSource,
        Vec<super::super::selected_plan_wait::RecoveredStepCustody>,
        Value,
    )> {
        let custody = self.selected_wait_recovery(session)?;
        let plan = self.resource_plan().ok_or_else(|| {
            DfmcpError::new(ErrorCode::CorruptLedger, "recovered original seal is missing")
        })?;
        let source = self.recovered_forecast_source.as_ref().ok_or_else(|| {
            DfmcpError::new(ErrorCode::Conflict, "recovered original source witness is unavailable")
        })?.as_ref().map_err(Clone::clone)?;
        source.validate_plan(plan)?;
        if source.anchor.state_hash != self.sealed_state_hash {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered original source witness changed its sealed world",
            ));
        }
        let steps = custody.forecast_census(session, plan, false)?;
        if steps.len() != plan.steps.len() || steps.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "recovered forecast requires the complete original step census",
            ));
        }
        Ok((*source, steps, custody.evidence_json()?))
    }

    /// The admitted Objective stays immutable while its current truth is
    /// evaluated on descendants of the accepted recovery source.
    pub(in super::super) fn forecast_recovered_current_truth_at(
        &self,
        snapshot: &dfmcp_world::WorldSnapshot,
    ) -> Result<(PredicateTruth, PredicateTruth)> {
        if self.originating_process || self.original_handles_live {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "recovered forecast cannot replace live original custody",
            ));
        }
        let plan = self.resource_plan().ok_or_else(|| {
            DfmcpError::new(ErrorCode::CorruptLedger, "recovered original seal is missing")
        })?;
        let active = self.active_observation_anchor.ok_or_else(|| {
            DfmcpError::new(ErrorCode::Conflict, "recovered original lineage is unavailable")
        })?;
        self.recovered_wait_custody.as_ref().ok_or_else(|| {
            DfmcpError::new(ErrorCode::Conflict, "recovered original custody is unavailable")
        })?.validate(plan, active, snapshot.anchor())?;
        let source = self.recovered_forecast_source.as_ref().ok_or_else(|| {
            DfmcpError::new(ErrorCode::Conflict, "recovered original source witness is unavailable")
        })?.as_ref().map_err(Clone::clone)?;
        if source.anchor != plan.anchor
            || source.plan_digest != plan.digest
            || source.anchor.state_hash != self.sealed_state_hash
        {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered current-stage forecast lost its admitted original source",
            ));
        }
        self.forecast_truth_at_source(snapshot)
    }

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

#[cfg(test)]
#[path = "recovered_forecast_source_tests.rs"]
mod tests;
