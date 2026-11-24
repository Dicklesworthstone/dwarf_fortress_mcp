//! One request-owned meter for a bounded set of discarded laboratory forecasts.
//! Canonical-source work is measured; retained adapter history is not a heap-size
//! measurement. No counter here represents actual game-time advancement.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
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
    owned_progression: bool,
    competing_actions: u64,
    custody_records: u64,
    custody_bytes: u64,
    forked_custody_records: u64,
    forked_custody_bytes: u64,
    generated_custody_records: u64,
    generated_custody_bytes: u64,
    predicate_nodes: u64,
    portfolio_candidate: Option<Digest32>,
    portfolio_stages: BTreeMap<Digest32, u64>,
    selected_peer_project_copies: u64,
    hypothetical_competing_actions: u64,
    hypothetical_peer_stages: u64,
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
            owned_progression: false,
            competing_actions: 0,
            custody_records: 0,
            custody_bytes: 0,
            forked_custody_records: 0,
            forked_custody_bytes: 0,
            generated_custody_records: 0,
            generated_custody_bytes: 0,
            predicate_nodes: 0,
            portfolio_candidate: None,
            portfolio_stages: BTreeMap::new(),
            selected_peer_project_copies: 0,
            hypothetical_competing_actions: 0,
            hypothetical_peer_stages: 0,
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

    pub(crate) fn begin_candidate(
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
        if self.candidates >= MAX_CANDIDATES
            || actions.saturating_add(self.competing_actions)
                > u64::from(current.budget.max_actions)
        {
            return Err(exhausted(
                "comparison exceeds its aggregate candidate or action-step budget",
            ));
        }
        self.candidates += 1;
        self.actions = actions;
        Ok(())
    }

    /// Each candidate copies the same authenticated peer custody onto its own
    /// discarded world. Charge the complete competing action domain per model
    /// and retain its canonical custody footprint for every source clone/read.
    pub(crate) fn admit_owned_candidate(
        &mut self,
        actions: u64,
        custody_records: u64,
        custody_bytes: u64,
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        let competing = self.competing_actions.saturating_add(actions);
        if self.actions.saturating_add(competing) > u64::from(current.budget.max_actions)
            || custody_records > u64::from(current.budget.max_entities)
            || custody_bytes > current.budget.max_bytes.min(16 * 1024 * 1024)
        {
            return Err(exhausted(
                "owned candidate forecasts exceed their complete competing action or custody budget",
            ));
        }
        self.owned_progression = true;
        self.competing_actions = competing;
        self.custody_records = custody_records;
        self.custody_bytes = custody_bytes;
        Ok(())
    }

    pub(crate) fn inspect_owned_source(
        &self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        let anchor = snapshot.anchor();
        if !self.owned_progression
            || anchor.fortress_id != self.current.anchor.fortress_id
            || anchor.cursor.epoch != self.current.anchor.cursor.epoch
            || anchor.cursor.sequence < self.current.anchor.cursor.sequence
            || anchor.tick < self.current.anchor.tick
            || anchor.tick.0 > self.horizon_tick
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "owned candidate model source is outside its original epoch and common horizon",
            ));
        }
        let source = self.inspect_source(snapshot, current)?;
        let records = source["records"]
            .as_u64()
            .ok_or_else(|| exhausted("model source record accounting is missing"))?;
        let bytes = source["canonical_bytes"]
            .as_u64()
            .ok_or_else(|| exhausted("model source byte accounting is missing"))?;
        if records.saturating_add(self.custody_records) > u64::from(current.budget.max_entities)
            || bytes.saturating_add(self.custody_bytes)
                > current.budget.max_bytes.min(16 * 1024 * 1024)
        {
            return Err(exhausted(
                "owned candidate model exceeds its complete current source and retained custody allowance",
            ));
        }
        Ok(())
    }

    pub(crate) fn before_predicates(
        &mut self,
        nodes: u64,
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        let total = self.predicate_nodes.saturating_add(nodes);
        if total > 4_000_000 {
            return Err(exhausted(
                "owned candidate forecasts exceed their shared predicate-work bound",
            ));
        }
        self.predicate_nodes = total;
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

    /// A portfolio keeps an independent four-stage allowance for the proposed
    /// project and each explicitly selected committed root. Committed history
    /// has already been charged as competing custody; only new successors add
    /// stage and action costs here.
    pub(crate) fn select_owned_projects(
        &mut self,
        candidate: Digest32,
        peers: &[Digest32],
        current: &OperationContext,
    ) -> Result<()> {
        self.check(current)?;
        if !self.project_mode
            || self.current_project_stages != 1
            || peers.is_empty()
            || peers.len() > 7
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "portfolio forecasts require one candidate and one to seven selected roots",
            ));
        }
        let mut stages = BTreeMap::from([(candidate, 1)]);
        for peer in peers {
            if stages.insert(*peer, 1).is_some() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "portfolio forecasts cannot duplicate candidate or selected root identities",
                ));
            }
        }
        self.portfolio_candidate = Some(candidate);
        self.portfolio_stages = stages;
        self.selected_peer_project_copies = self
            .selected_peer_project_copies
            .saturating_add(peers.len() as u64);
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

    /// Future candidate stages retain their exact archive on the same private
    /// model. Charge it and every competing action before the adapter prepares
    /// or commits; the next candidate starts with its own admitted custody.
    pub(crate) fn admit_owned_project_stage(
        &mut self,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        archive_bytes: u64,
        current: &OperationContext,
    ) -> Result<()> {
        self.inspect_owned_source(snapshot, current)?;
        if self
            .actions
            .saturating_add(plan.steps.len() as u64)
            .saturating_add(self.competing_actions)
            > u64::from(current.budget.max_actions)
        {
            return Err(exhausted(
                "owned project stages exceed the complete candidate and competing action budget",
            ));
        }
        let generated_records = self
            .generated_custody_records
            .saturating_add(plan.steps.len() as u64);
        let generated_bytes = self.generated_custody_bytes.saturating_add(archive_bytes);
        if self
            .forked_records
            .saturating_add(self.forked_custody_records)
            .saturating_add(generated_records)
            > u64::from(current.budget.max_entities)
            || self
                .forked_bytes
                .saturating_add(self.forked_custody_bytes)
                .saturating_add(generated_bytes)
                > current.budget.max_bytes
        {
            return Err(exhausted(
                "owned project archives exceed the shared copied and generated custody budget",
            ));
        }
        self.admit_project_stage(snapshot, plan, current)?;
        self.generated_custody_records = generated_records;
        self.generated_custody_bytes = generated_bytes;
        self.custody_records = self.custody_records.saturating_add(plan.steps.len() as u64);
        self.custody_bytes = self.custody_bytes.saturating_add(archive_bytes);
        self.inspect_owned_source(snapshot, current)
    }

    /// Candidate and selected-peer successors share all aggregate allowances,
    /// while each exact root retains its own bounded stage count. All authority
    /// checks have already succeeded at the modeled frontier before this call.
    pub(crate) fn admit_owned_portfolio_stage(
        &mut self,
        original: Digest32,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        archive_bytes: u64,
        current: &OperationContext,
    ) -> Result<()> {
        self.inspect_owned_source(snapshot, current)?;
        let stages = self
            .portfolio_stages
            .get(&original)
            .copied()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "portfolio successor does not belong to an admitted project",
                )
            })?;
        if plan.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "portfolio stage differs from its exact private source",
            ));
        }
        let added = plan.steps.len() as u64;
        if stages >= MAX_PROJECT_STAGES
            || self
                .actions
                .saturating_add(self.competing_actions)
                .saturating_add(added)
                > u64::from(current.budget.max_actions)
        {
            return Err(exhausted(
                "portfolio stages exceed their per-root or complete shared action allowance",
            ));
        }
        let generated_records = self.generated_custody_records.saturating_add(added);
        let generated_bytes = self.generated_custody_bytes.saturating_add(archive_bytes);
        if self
            .forked_records
            .saturating_add(self.forked_custody_records)
            .saturating_add(generated_records)
            > u64::from(current.budget.max_entities)
            || self
                .forked_bytes
                .saturating_add(self.forked_custody_bytes)
                .saturating_add(generated_bytes)
                > current.budget.max_bytes
        {
            return Err(exhausted(
                "portfolio archives exceed the shared copied and generated custody budget",
            ));
        }
        if self.portfolio_candidate == Some(original) {
            self.actions = self.actions.saturating_add(added);
            self.current_project_stages += 1;
        } else {
            self.competing_actions = self.competing_actions.saturating_add(added);
            self.hypothetical_competing_actions =
                self.hypothetical_competing_actions.saturating_add(added);
            self.hypothetical_peer_stages += 1;
        }
        self.portfolio_stages.insert(original, stages + 1);
        self.project_stages += 1;
        self.generated_custody_records = generated_records;
        self.generated_custody_bytes = generated_bytes;
        self.custody_records = self.custody_records.saturating_add(added);
        self.custody_bytes = self.custody_bytes.saturating_add(archive_bytes);
        self.inspect_owned_source(snapshot, current)
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
        let forked_custody_records = self
            .forked_custody_records
            .saturating_add(self.custody_records);
        let maximum_forks = if self.project_mode && self.portfolio_candidate.is_none() {
            MAX_PROJECT_FORKS
        } else {
            MAX_SOURCE_FORKS
        };
        if self.source_forks >= maximum_forks
            || forked_records
                .saturating_add(forked_custody_records)
                .saturating_add(self.generated_custody_records)
                > u64::from(current.budget.max_entities)
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
        let forked_custody_bytes = self.forked_custody_bytes.saturating_add(self.custody_bytes);
        if forked_bytes
            .saturating_add(forked_custody_bytes)
            .saturating_add(self.generated_custody_bytes)
            > current.budget.max_bytes
        {
            return Err(exhausted(
                "comparison exceeds its aggregate forked-source byte budget",
            ));
        }
        self.source_forks += 1;
        self.forked_records = forked_records;
        self.forked_bytes = forked_bytes;
        self.forked_custody_records = forked_custody_records;
        self.forked_custody_bytes = forked_custody_bytes;
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

    pub(crate) fn before_poll(&mut self, current: &OperationContext) -> Result<()> {
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
        if self.owned_progression {
            work["progression_scope"] = json!("owned_session");
            work["competing_action_steps"] = json!(self.competing_actions);
            work["forked_retained_custody_records"] = json!(self.forked_custody_records);
            work["forked_retained_custody_bytes"] = json!(self.forked_custody_bytes);
            if self.project_mode {
                work["generated_project_custody_records"] = json!(self.generated_custody_records);
                work["generated_project_custody_bytes"] = json!(self.generated_custody_bytes);
            }
            work["predicate_node_visits_bound"] = json!(self.predicate_nodes);
            work["limits"]["complete_candidate_and_competing_steps"] =
                json!(self.current.budget.max_actions);
            work["limits"]["predicate_node_visits_bound"] = json!(4_000_000);
            work["source_accounting"] = json!(
                "canonical world and exact retained original/monitor custody copied for each discarded candidate; combined source and custody totals share negotiated record/byte limits, not a measured heap-size total"
            );
            if self.project_mode {
                work["source_accounting"] = json!(
                    "canonical world and retained original/monitor custody copied into each private candidate, plus every generated future plan archive; copied and generated totals share the aggregate record/byte allowance, and each modeled source retains all current custody"
                );
            }
            work["slice_accounting"] = json!(
                "attempted shared foreground observation rounds, including zero-time polls; simulated_game_ticks counts only successful advances"
            );
        }
        if self.portfolio_candidate.is_some() {
            work["selected_peer_project_copies"] = json!(self.selected_peer_project_copies);
            work["hypothetical_competing_action_steps"] =
                json!(self.hypothetical_competing_actions);
            work["hypothetical_peer_stages"] = json!(self.hypothetical_peer_stages);
            work["limits"]["selected_peers_per_candidate"] = json!(7);
            work["limits"]["source_forks"] = json!(MAX_SOURCE_FORKS);
            work["stage_accounting"] = json!(
                "candidate initial and all hypothetical candidate/selected-peer stages; committed peer history is charged once per candidate in competing action and source custody totals"
            );
        }
        work
    }
}
