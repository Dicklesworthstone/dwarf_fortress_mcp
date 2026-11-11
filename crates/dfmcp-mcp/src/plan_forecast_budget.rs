//! One request-owned meter for a bounded set of discarded laboratory forecasts.
//! Canonical-source work is measured; retained adapter history is not a heap-size
//! measurement. No counter here represents actual game-time advancement.

use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};

const MAX_CANDIDATES: u64 = 4;
const MAX_SOURCE_FORKS: u64 = MAX_CANDIDATES * 2;
pub(crate) const MAX_PROJECT_STAGES: u64 = 4;
const MAX_PROJECT_FORKS: u64 = MAX_CANDIDATES * (MAX_PROJECT_STAGES + 1);

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(crate) struct ForecastBudget {
    current: OperationContext,
    started: Instant,
    horizon_ticks: u64,
    horizon_tick: u64,
    source_work: Option<Value>,
    candidates: u64,
    source_forks: u64,
    forked_records: u64,
    forked_bytes: u64,
    actions: u64,
    polls: u64,
    ticks: u64,
    slices: u64,
    project_mode: bool,
    project_stages: u64,
    current_project_stages: u64,
}

impl ForecastBudget {
    pub(crate) fn new(
        current: &OperationContext,
        started: Instant,
        horizon_ticks: u64,
    ) -> Result<Self> {
        if horizon_ticks > current.budget.max_game_ticks {
            return Err(exhausted(
                "forecast horizon exceeds the shared simulated-tick budget",
            ));
        }
        let horizon_tick = current
            .anchor
            .tick
            .0
            .checked_add(horizon_ticks)
            .ok_or_else(|| exhausted("forecast horizon exceeds the game-tick identity space"))?;
        let result = Self {
            current: current.clone(),
            started,
            horizon_ticks,
            horizon_tick,
            source_work: None,
            candidates: 0,
            source_forks: 0,
            forked_records: 0,
            forked_bytes: 0,
            actions: 0,
            polls: 0,
            ticks: 0,
            slices: 0,
            project_mode: false,
            project_stages: 0,
            current_project_stages: 0,
        };
        result.check(current)?;
        Ok(result)
    }

    /// The original comparison profile is unchanged. Project comparisons
    /// explicitly opt into bounded later stages while retaining one owner,
    /// common horizon and aggregate action/tick/slice/byte budget.
    pub(crate) fn new_project(
        current: &OperationContext,
        started: Instant,
        horizon_ticks: u64,
    ) -> Result<Self> {
        let mut budget = Self::new(current, started, horizon_ticks)?;
        budget.project_mode = true;
        Ok(budget)
    }

    pub(crate) fn check(&self, current: &OperationContext) -> Result<()> {
        for capability in [Capability::Plan, Capability::Observe, Capability::Query] {
            current.authorize(capability, RiskTier::ReadOnly, &[], None)?;
        }
        if current.session_id != self.current.session_id
            || current.request_id != self.current.request_id
            || current.anchor != self.current.anchor
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "comparison forecasts must retain one current owner, request and source anchor",
            ));
        }
        if current.grants != self.current.grants {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "comparison authority changed during forecast evaluation",
            ));
        }
        if current.budget != self.current.budget {
            return Err(exhausted(
                "comparison budget changed during forecast evaluation",
            ));
        }
        if self.started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
            return Err(exhausted(
                "comparison forecasts exhausted their shared wall-time budget",
            ));
        }
        Ok(())
    }

    /// The owner admits its immutable current source before compiling any
    /// candidate. Repeated forks reuse these exact original-anchor measurements.
    pub(crate) fn admit_source(
        &mut self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
    ) -> Result<Value> {
        self.check(current)?;
        if snapshot.anchor() != current.anchor {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "comparison source differs from the owned current anchor",
            ));
        }
        if let Some(work) = &self.source_work {
            return Ok(work.clone());
        }
        let work = self.inspect_source(snapshot, current)?;
        self.source_work = Some(work.clone());
        Ok(work)
    }

    fn inspect_source(
        &self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
    ) -> Result<Value> {
        self.check(current)?;
        let elapsed = self.started.elapsed().as_nanos().div_ceil(1_000_000);
        if elapsed >= u128::from(current.budget.max_wall_millis) {
            return Err(exhausted(
                "comparison source exhausted its shared wall-time budget",
            ));
        }
        let mut remaining = current.clone();
        remaining.budget.max_wall_millis -= elapsed as u64;
        let work = super::super::query_budget::admit_sources(&[snapshot], &remaining)?;
        self.check(current)?;
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "forecast source has an invalid canonical hash",
            ));
        }
        self.check(current)?;
        Ok(work)
    }

    pub(super) fn begin_candidate(
        &mut self,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        current: &OperationContext,
    ) -> Result<()> {
        self.admit_source(snapshot, current)?;
        if plan.anchor != current.anchor {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "candidate plan differs from the comparison source",
            ));
        }
        let actions = self.actions.saturating_add(plan.steps.len() as u64);
        if self.candidates >= MAX_CANDIDATES || actions > u64::from(current.budget.max_actions) {
            return Err(exhausted(
                "comparison exceeds its aggregate candidate or action-step budget",
            ));
        }
        self.candidates += 1;
        self.actions = actions;
        Ok(())
    }

    pub(crate) fn begin_project(
        &mut self,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        current: &OperationContext,
    ) -> Result<()> {
        if !self.project_mode {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "complete-project forecasts require their explicit shared budget profile",
            ));
        }
        self.begin_candidate(snapshot, plan, current)?;
        self.current_project_stages = 1;
        self.project_stages += 1;
        Ok(())
    }

    /// Derived sources remain private descendants of this forecast's source.
    /// Inspection keeps the original real owner/context; only model execution
    /// binds the derived anchor. No simulated observation becomes authority.
    pub(crate) fn inspect_project_source(
        &self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        let anchor = snapshot.anchor();
        if !self.project_mode
            || self.current_project_stages == 0
            || anchor.fortress_id != self.current.anchor.fortress_id
            || anchor.cursor.epoch != self.current.anchor.cursor.epoch
            || anchor.cursor.sequence < self.current.anchor.cursor.sequence
            || anchor.tick < self.current.anchor.tick
            || anchor.tick.0 > self.horizon_tick
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "project model source is outside its owned original epoch and common horizon",
            ));
        }
        self.inspect_source(snapshot, current).map(|_| ())
    }

    pub(crate) fn admit_project_stage(
        &mut self,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        current: &OperationContext,
    ) -> Result<()> {
        self.inspect_project_source(snapshot, current)?;
        if plan.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "project stage plan differs from its exact derived model source",
            ));
        }
        let actions = self.actions.saturating_add(plan.steps.len() as u64);
        if self.current_project_stages >= MAX_PROJECT_STAGES
            || actions > u64::from(current.budget.max_actions)
        {
            return Err(exhausted(
                "project stages exceed the aggregate action or four-stage bound",
            ));
        }
        self.current_project_stages += 1;
        self.project_stages += 1;
        self.actions = actions;
        Ok(())
    }

    pub(crate) fn before_fork(
        &mut self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        let records = [
            snapshot.graph.entities.len(),
            snapshot.graph.edges.len(),
            snapshot.graph.chunks.len(),
            snapshot.graph.events.len(),
        ]
        .into_iter()
        .fold(0_u64, |total, count| total.saturating_add(count as u64));
        let forked_records = self.forked_records.saturating_add(records);
        let maximum_forks = if self.project_mode {
            MAX_PROJECT_FORKS
        } else {
            MAX_SOURCE_FORKS
        };
        if self.source_forks >= maximum_forks
            || forked_records > u64::from(current.budget.max_entities)
        {
            return Err(exhausted(
                "comparison exceeds its aggregate source-fork record budget",
            ));
        }
        let work = if snapshot.anchor() == self.current.anchor {
            self.admit_source(snapshot, current)?
        } else {
            self.inspect_source(snapshot, current)?
        };
        let bytes = work["canonical_bytes"].as_u64().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "forecast source byte accounting is missing",
            )
        })?;
        let forked_bytes = self.forked_bytes.saturating_add(bytes);
        if forked_bytes > current.budget.max_bytes {
            return Err(exhausted(
                "comparison exceeds its aggregate forked-source byte budget",
            ));
        }
        self.source_forks += 1;
        self.forked_records = forked_records;
        self.forked_bytes = forked_bytes;
        Ok(())
    }

    pub(crate) fn before_advance(&mut self, ticks: u64, current: &OperationContext) -> Result<()> {
        self.check(current)?;
        let total = self
            .ticks
            .checked_add(ticks)
            .ok_or_else(|| exhausted("comparison simulated-tick counter exhausted"))?;
        if total > current.budget.max_game_ticks || self.slices >= super::MAX_FORECAST_SLICES {
            return Err(exhausted(
                "comparison exceeds its aggregate simulated-tick or slice budget",
            ));
        }
        self.slices += 1;
        Ok(())
    }

    pub(crate) fn advanced(&mut self, ticks: u64) {
        // before_advance admitted this exact sequential model call. A failed
        // advance is still an attempted slice but cannot claim elapsed ticks.
        self.ticks += ticks;
    }

    pub(super) fn before_poll(&mut self, current: &OperationContext) -> Result<()> {
        self.check(current)?;
        let maximum =
            u64::from(current.budget.max_actions).saturating_mul(super::MAX_FORECAST_SLICES);
        if self.polls >= maximum {
            return Err(exhausted(
                "comparison exceeds its aggregate action-poll budget",
            ));
        }
        self.polls += 1;
        Ok(())
    }

    pub(crate) const fn horizon_tick(&self) -> u64 {
        self.horizon_tick
    }
    pub(crate) const fn horizon_ticks(&self) -> u64 {
        self.horizon_ticks
    }

    pub(crate) fn work_json(&self) -> Value {
        let mut work = json!({
            "candidates":self.candidates, "source_forks":self.source_forks,
            "forked_source_records":self.forked_records,
            "forked_source_canonical_bytes":self.forked_bytes,
            "candidate_action_steps":self.actions, "action_polls":self.polls,
            "simulated_game_ticks":self.ticks, "simulation_slices":self.slices,
            "common_horizon_ticks":self.horizon_ticks, "common_horizon_tick":self.horizon_tick,
            "limits":{"candidates":MAX_CANDIDATES,"source_forks":MAX_SOURCE_FORKS,
                "forked_source_records":self.current.budget.max_entities,
                "forked_source_canonical_bytes":self.current.budget.max_bytes,
                "candidate_action_steps":self.current.budget.max_actions,
                "action_polls":u64::from(self.current.budget.max_actions).saturating_mul(super::MAX_FORECAST_SLICES),
                "simulated_game_ticks":self.current.budget.max_game_ticks,
                "simulation_slices":super::MAX_FORECAST_SLICES,
                "wall_millis":self.current.budget.max_wall_millis},
            "source_accounting":"canonical snapshot records and bytes copied into discarded model forks; retained adapter plans, actions, checkpoints and journal are not a measured heap-byte count",
            "actual_game_ticks_advanced":0,
            "slice_accounting":"attempted bounded model advances; simulated_game_ticks counts only successful advances",
        });
        if self.project_mode {
            work["project_stages"] = json!(self.project_stages);
            work["limits"]["stages_per_project"] = json!(MAX_PROJECT_STAGES);
            work["limits"]["source_forks"] = json!(MAX_PROJECT_FORKS);
        }
        work
    }
}
