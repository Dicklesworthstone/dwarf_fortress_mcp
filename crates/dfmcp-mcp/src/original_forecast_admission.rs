//! Shared exact custody for already committed complete-project forecasts.
//! Admission borrows retained originals and never loads archives, restores
//! handles, publishes observations or creates a new wall-time allowance.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_core::{
    ActionId, CommitState, DfmcpError, Digest32, ErrorCode, OperationContext, Result, SessionId,
    StateAnchor,
};
use dfmcp_world::{PredicateTruth, WorldSnapshot};
use serde_json::{Value, json};

use super::super::{
    CarriedStep, LabSession, anchor_json,
    objectives::Objective,
    selected_plan_wait::{PlanSetSource, SelectedPlanWait},
};
use super::{ReservePursuit, exhausted, invalid, model, remaining};

/// One admitted set shared by the original query and portfolio comparisons.
/// Fixed custody excludes the current world, which every private source check
/// admits again. It includes the complete retained goal book and all owned
/// competing work; callers must not add those same archives or monitors twice.
pub(in crate::server) struct AdmittedProjectSet<'a> {
    session: SessionId,
    anchor: StateAnchor,
    requested: Vec<Digest32>,
    pub(in crate::server) originals: Vec<model::Original>,
    pub(in crate::server) projects: Vec<model::Project>,
    pub(in crate::server) carried: Vec<CarriedStep>,
    pub(in crate::server) owned_actions: &'a [ActionId],
    /// Complete selected live/recovered and competing action domain. Drained
    /// committed stages still count even when absent from open progression.
    pub(in crate::server) action_steps: u64,
    pub(in crate::server) fixed_records: u64,
    pub(in crate::server) fixed_bytes: u64,
    /// The full root/retained-stage proof domain plus the closed-model semantic
    /// allowance, conservatively charged before each selected goal evaluation.
    pub(in crate::server) predicate_node_bound: u64,
    pub(in crate::server) recovered_predicate_node_bound: u64,
    goals: BTreeMap<Digest32, &'a Objective>,
    pub(super) source: PlanSetSource,
    pub(super) project_goal_node_bound: u64,
    pub(super) recovered_goals: BTreeSet<Digest32>,
    pub(super) recovered_sources: Vec<Value>,
    pub(super) recovered_steps: usize,
    pub(super) verified_recovered_steps: usize,
    pub(super) unfinished_recovered_steps: usize,
    pub(super) project_snapshot_count: usize,
    pub(super) project_records: u64,
    pub(super) project_bytes: u64,
    pub(super) fixed_shape: (u64, u64),
    pub(super) carried_custody_records: u64,
    pub(super) carried_custody_bytes: u64,
    pub(super) next_wait_digests: Vec<String>,
    pub(super) reserve_endpoint_pursuits: Vec<ReservePursuit>,
}

impl AdmittedProjectSet<'_> {
    pub(in crate::server) fn requested_digests(&self) -> &[Digest32] {
        &self.requested
    }

    /// Reusing admitted custody cannot switch its session or actual source.
    /// Modeled descendants are inspected separately by inspect_structure.
    pub(in crate::server) fn check_source(
        &self,
        snapshot: &WorldSnapshot,
        context: &OperationContext,
        started: Instant,
    ) -> Result<()> {
        remaining(context, started)?;
        if context.session_id != self.session
            || context.anchor != self.anchor
            || snapshot.anchor() != self.anchor
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "joint forecast lost its admitted session source",
            ));
        }
        Ok(())
    }

    pub(in crate::server) fn evaluate(
        &self,
        digest: Digest32,
        snapshot: &WorldSnapshot,
    ) -> Result<(PredicateTruth, PredicateTruth)> {
        let original = self
            .goals
            .get(&digest)
            .ok_or_else(|| invalid("selected original goal disappeared during private forecast"))?;
        if self.recovered_goals.contains(&digest) {
            original.forecast_recovered_truth_at(snapshot)
        } else {
            original.forecast_truth_at(snapshot)
        }
    }

    /// Retain authenticated historical structural charges on every descendant
    /// inspection. The same request start is used by the original query and by
    /// each alternative of a portfolio comparison.
    pub(in crate::server) fn inspect_structure(
        &self,
        snapshot: &WorldSnapshot,
        context: &OperationContext,
        started: Instant,
    ) -> Result<()> {
        if !self.recovered_goals.is_empty() {
            let shape = super::super::query_budget::source_shape(
                snapshot,
                &remaining(context, started)?,
                Instant::now(),
            )?;
            if self.fixed_shape.0.saturating_add(shape.0) > 16 * 1024 * 1024
                || self.fixed_shape.1.saturating_add(shape.1) > 4_000_000
            {
                return Err(exhausted(
                    "recovered forecast exceeds its complete structural source byte or work bound",
                ));
            }
        }
        Ok(())
    }

    pub(in crate::server) fn describe(&self) -> Value {
        json!({
            "scope":"selected_committed_complete_projects",
            "requested_plan_digests":self.requested.iter().map(|digest| digest.to_hex()).collect::<Vec<_>>(),
            "actual_plan_digests":self.originals.iter().map(|original| original.actual.to_hex()).collect::<Vec<_>>(),
            "owned_live_action_count":self.owned_actions.len(),
            "selected_and_competing_action_steps":self.action_steps,
            "recovered_observation_steps":self.carried.len(),
            "retained_source_records":self.source.records,
            "retained_source_bytes":self.source.bytes,
            "retained_source_snapshots":self.source.snapshots,
            "fixed_custody_records":self.fixed_records,
            "fixed_custody_bytes":self.fixed_bytes,
            "predicate_node_bound":self.predicate_node_bound,
            "recovered_predicate_nodes_per_evaluation":self.recovered_predicate_node_bound,
            "additional_original_snapshots":self.project_snapshot_count,
            "additional_original_records":self.project_records,
            "additional_original_bytes":self.project_bytes,
            "recovered_original_steps":self.recovered_steps,
            "verified_recovered_original_steps":self.verified_recovered_steps,
            "selected_recovered_observation_steps":self.unfinished_recovered_steps,
            "fixed_structural_bytes":self.fixed_shape.0,
            "fixed_structural_work":self.fixed_shape.1,
            "query_archive_reads":0,
            "dispatch_handles_restored":false,
            "committed_stages_recommitted":false,
        })
    }
}

/// Admit ordinary roots and their exact retained complete-project lineages.
/// Public callers select at most eight originals; a candidate portfolio uses
/// at most seven here so the candidate and selected peers fit the same model.
pub(in crate::server) fn admit_complete_projects<'a>(
    session: &'a LabSession,
    digests: &[Digest32],
    context: &OperationContext,
    started: Instant,
) -> Result<AdmittedProjectSet<'a>> {
    remaining(context, started)?;
    if context.anchor != session.adapter.snapshot().anchor()
        || context.session_id != session.session_id
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "joint forecast lost its current session source",
        ));
    }
    if digests.is_empty() || digests.len() > 8 {
        return Err(invalid(
            "complete-project admission requires one to eight originals",
        ));
    }
    admit_originals(session, digests, context, started, true)
}

fn admit_live_actions(
    session: &LabSession,
    selected: &super::super::plan_cancellation::Selection,
    action_union: &mut BTreeSet<ActionId>,
    in_continued_project: bool,
    predecessor: bool,
) -> Result<()> {
    for action in &selected.actions {
        let receipt = session
            .adapter
            .action_receipt(*action)
            .ok_or_else(|| invalid("forecast original action disappeared"))?;
        if (!receipt.state.is_terminal()
            || !session.adapter.action_work_state(*action)?.is_quiescent())
            && !session.open_actions.contains(action)
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "unfinished forecast original is absent from the owning progression set",
            ));
        }
        if in_continued_project {
            let work = session.adapter.action_work_state(*action)?;
            if !matches!(
                receipt.state,
                CommitState::Prepared
                    | CommitState::AppliedAwaitingVerification
                    | CommitState::Verified
            ) || matches!(work, dfmcp_intent::EffectWorkState::Unknown { .. })
            {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "complete-project continuation requires uninterrupted, known original action custody",
                ));
            }
            if predecessor && (receipt.state != CommitState::Verified || !work.is_quiescent()) {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "complete-project predecessors must already have Verified, physically quiet actions",
                ));
            }
        }
        action_union.insert(*action);
    }
    Ok(())
}

pub(super) fn admit_originals<'a>(
    session: &'a LabSession,
    digests: &[Digest32],
    context: &OperationContext,
    started: Instant,
    complete_project: bool,
) -> Result<AdmittedProjectSet<'a>> {
    let members = SelectedPlanWait::set_members(session, digests, started)?;
    let mut source = SelectedPlanWait::admit_retained_set_source(&members, session)?;
    let mut project_lineages = BTreeMap::new();
    let mut complete_custody = None;
    if complete_project {
        let mut custody_digests = digests.to_vec();
        for member in &members {
            remaining(context, started)?;
            let original = session
                .objectives
                .iter()
                .find(|original| original.plan_digest == member.actual_digest())
                .ok_or_else(|| invalid("forecast original goal disappeared"))?;
            let lineage = original.forecast_project_source(session)?;
            custody_digests.extend(lineage.1.iter().map(|(digest, _)| *digest));
            project_lineages.insert(member.actual_digest(), lineage);
        }
        if custody_digests.len() > digests.len() {
            let admitted = SelectedPlanWait::set_members(session, &custody_digests, started)?;
            source = SelectedPlanWait::admit_retained_set_source(&admitted, session)?;
            complete_custody = Some(admitted);
        }
    }
    let mut originals = Vec::with_capacity(members.len());
    let mut goals = BTreeMap::new();
    let mut action_union = BTreeSet::new();
    let mut projects = Vec::new();
    let mut project_snapshots = Vec::new();
    let mut recovered_goals = BTreeSet::new();
    let mut recovered_sources = Vec::new();
    let mut recovered_records = 0u64;
    let mut recovered_bytes = 0u64;
    let mut recovered_steps = 0usize;
    let mut verified_recovered_steps = 0usize;
    let mut unfinished_recovered_steps = 0usize;
    let mut recovered_digests = BTreeSet::new();
    let mut recovered_shape = (0u64, 0u64);
    let mut reserve_snapshots = Vec::new();
    let mut next_wait_digests = Vec::new();
    let mut reserve_endpoint_pursuits = Vec::new();
    let mut admitted_snapshots = BTreeSet::from([context.anchor.state_hash]);
    for member in complete_custody.as_deref().unwrap_or(&members) {
        if let Some(original) = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == member.actual_digest())
            && let Some((anchor, _)) = original.selected_wait_reserve_observation
        {
            if admitted_snapshots.insert(anchor.state_hash) {
                let snapshot = session
                    .history
                    .get(&anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == anchor)
                    .ok_or_else(|| invalid("forecast reserve source disappeared"))?;
                reserve_snapshots.push(snapshot);
            }
        }
    }
    for member in &members {
        remaining(context, started)?;
        let original = session
            .objectives
            .iter()
            .find(|original| original.plan_digest == member.actual_digest())
            .ok_or_else(|| invalid("forecast original goal disappeared"))?;
        // A present live receipt remains authoritative: malformed live custody
        // cannot fall back to recovery. The default current-stage route keeps
        // its existing live-only behavior.
        let recovery = if complete_project
            && !session
                .commit_receipts
                .contains_key(&member.requested_digest().to_hex())
        {
            let plan = original
                .resource_plan()
                .ok_or_else(|| invalid("recovered project seal disappeared"))?;
            let predecessor = project_lineages
                .get(&member.actual_digest())
                .is_some_and(|(_, descendants)| !descendants.is_empty());
            let (source, census, evidence) =
                original.forecast_recovered_project(session, predecessor)?;
            let verified = census.iter().filter(|row| row.state == "verified").count();
            verified_recovered_steps = verified_recovered_steps.saturating_add(verified);
            unfinished_recovered_steps =
                unfinished_recovered_steps.saturating_add(census.len() - verified);
            recovered_digests.insert(member.actual_digest());
            let recovered = model::RecoveredOriginal::new(
                plan,
                census,
                evidence,
                session.adapter.snapshot(),
                &session.carried,
            )?;
            recovered_steps = recovered_steps.saturating_add(plan.steps.len());
            recovered_goals.insert(member.actual_digest());
            Some((source, recovered))
        } else {
            None
        };
        let selected = if recovery.is_some() {
            super::super::plan_cancellation::Selection {
                requested: member.requested_digest(),
                actual: member.actual_digest(),
                plan_id: original
                    .resource_plan()
                    .ok_or_else(|| invalid("recovered project seal disappeared"))?
                    .id
                    .to_string(),
                actions: Vec::new(),
            }
        } else {
            super::super::plan_cancellation::select(session, member.requested_digest())?
        };
        let continued = project_lineages
            .get(&selected.actual)
            .is_some_and(|(_, descendants)| !descendants.is_empty());
        if recovery.is_none() {
            admit_live_actions(session, &selected, &mut action_union, continued, continued)?;
        }
        original.selected_wait_truth(session)?;
        original.selected_wait_stage_truth(session)?;
        if complete_project {
            let (project_source, descendants) = project_lineages
                .remove(&selected.actual)
                .ok_or_else(|| invalid("complete-project lineage disappeared"))?;
            let plan = original
                .resource_plan()
                .ok_or_else(|| invalid("project original seal disappeared"))?;
            let (mut project, ledger) = if let Some((archived, recovered)) = recovery {
                if admitted_snapshots.insert(archived.anchor.state_hash) {
                    recovered_records = recovered_records.saturating_add(archived.records);
                    recovered_bytes = recovered_bytes.saturating_add(archived.bytes);
                    recovered_shape.0 = recovered_shape.0.saturating_add(archived.structural_bytes);
                    recovered_shape.1 = recovered_shape.1.saturating_add(archived.structural_work);
                    recovered_sources.push(json!({
                        "anchor":anchor_json(&archived.anchor),
                        "records":archived.records,"canonical_bytes":archived.bytes,
                        "structural_bytes":archived.structural_bytes,"structural_work":archived.structural_work,
                        "verification":"exact_sealed_snapshot_during_durable_recovery",
                        "query_archive_reads":0,
                    }));
                }
                (
                    model::Project::new(selected.actual, project_source, archived.ledger)
                        .with_recovery(recovered)?,
                    archived.ledger,
                )
            } else {
                let sealed = session
                    .history
                    .get(&plan.anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == plan.anchor)
                    .ok_or_else(|| {
                        invalid("complete_project requires the exact retained original source")
                    })?;
                // Admit exact source bytes before checking the original ledger
                // or preparing any future stage. No historical hydration.
                if admitted_snapshots.insert(sealed.state_hash) {
                    project_snapshots.push(sealed);
                }
                super::super::query_budget::admit_sources(
                    &[sealed],
                    &remaining(context, started)?,
                )?;
                let ledger = crate::lab_world::ProductionGoalLedger::capture(sealed)?;
                ledger.validate_plan(plan)?;
                ledger.validate_current(session.adapter.snapshot())?;
                (
                    model::Project::new(selected.actual, project_source, ledger),
                    ledger,
                )
            };
            let root_reserve = original.selected_wait_reserve_horizon()?;
            next_wait_digests.push(descendants.last().map_or_else(
                || member.requested_digest().to_hex(),
                |(digest, _)| digest.to_hex(),
            ));
            if let Some(until) = root_reserve {
                reserve_endpoint_pursuits.push(ReservePursuit {
                    plan_digest: selected.actual.to_hex(),
                    wait_digest: member.requested_digest().to_hex(),
                    reserve_until_tick: until,
                    sample_retained: original.selected_wait_reserve_observation.is_some(),
                });
            }
            for (index, (digest, parent)) in descendants.iter().enumerate() {
                remaining(context, started)?;
                let objective = session
                    .objectives
                    .iter()
                    .find(|objective| objective.plan_digest == *digest)
                    .ok_or_else(|| invalid("retained project stage disappeared"))?;
                objective.selected_wait_truth(session)?;
                objective.selected_wait_stage_truth(session)?;
                let reserve_until = objective.selected_wait_reserve_horizon()?;
                if reserve_until != root_reserve {
                    return Err(invalid(
                        "continuation changed the original absolute reserve horizon",
                    ));
                }
                if let Some(until) = reserve_until {
                    reserve_endpoint_pursuits.push(ReservePursuit {
                        plan_digest: digest.to_hex(),
                        wait_digest: digest.to_hex(),
                        reserve_until_tick: until,
                        sample_retained: objective.selected_wait_reserve_observation.is_some(),
                    });
                }
                let plan = objective
                    .resource_plan()
                    .ok_or_else(|| invalid("retained continuation seal disappeared"))?;
                let retained_original = model::Original {
                    requested: *digest,
                    actual: *digest,
                    actions: Vec::new(),
                    reserve_until,
                    reserve_observation: objective
                        .selected_wait_reserve_observation
                        .map(|(_, truth)| truth),
                    can_continue: true,
                };
                if !session.commit_receipts.contains_key(&digest.to_hex()) {
                    let (archived, census, evidence) = objective
                        .forecast_recovered_project(session, index + 1 < descendants.len())?;
                    let verified = census.iter().filter(|row| row.state == "verified").count();
                    verified_recovered_steps = verified_recovered_steps.saturating_add(verified);
                    unfinished_recovered_steps =
                        unfinished_recovered_steps.saturating_add(census.len() - verified);
                    recovered_digests.insert(*digest);
                    let recovered = model::RecoveredOriginal::new(
                        plan,
                        census,
                        evidence,
                        session.adapter.snapshot(),
                        &session.carried,
                    )?;
                    recovered_steps = recovered_steps.saturating_add(plan.steps.len());
                    recovered_goals.insert(*digest);
                    if admitted_snapshots.insert(archived.anchor.state_hash) {
                        recovered_records = recovered_records.saturating_add(archived.records);
                        recovered_bytes = recovered_bytes.saturating_add(archived.bytes);
                        recovered_shape.0 =
                            recovered_shape.0.saturating_add(archived.structural_bytes);
                        recovered_shape.1 =
                            recovered_shape.1.saturating_add(archived.structural_work);
                        recovered_sources.push(json!({
                            "anchor":anchor_json(&archived.anchor),
                            "records":archived.records,"canonical_bytes":archived.bytes,
                            "structural_bytes":archived.structural_bytes,"structural_work":archived.structural_work,
                            "verification":"exact_sealed_snapshot_during_durable_recovery",
                            "query_archive_reads":0,
                        }));
                    }
                    project.retain_recovered_stage(
                        plan.clone(),
                        *parent,
                        retained_original,
                        recovered,
                        session.adapter.snapshot(),
                    )?;
                    continue;
                }
                let retained = super::super::plan_cancellation::select(session, *digest)?;
                if retained.actual != *digest {
                    return Err(invalid(
                        "continuation custody cannot alias another sealed stage",
                    ));
                }
                admit_live_actions(
                    session,
                    &retained,
                    &mut action_union,
                    true,
                    index + 1 < descendants.len(),
                )?;
                let sealed = session
                    .history
                    .get(&plan.anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == plan.anchor)
                    .ok_or_else(|| {
                        invalid("complete_project requires each exact retained stage source")
                    })?;
                super::super::query_budget::admit_sources(
                    &[sealed],
                    &remaining(context, started)?,
                )?;
                ledger.validate_current(sealed)?;
                crate::lab_world::ProductionGoalLedger::capture(sealed)?.validate_plan(plan)?;
                if admitted_snapshots.insert(sealed.state_hash) {
                    project_snapshots.push(sealed);
                }
                project.retain_stage(
                    plan.clone(),
                    *parent,
                    model::Original {
                        actions: retained.actions,
                        ..retained_original
                    },
                    sealed,
                )?;
            }
            projects.push(project);
        }
        goals.insert(selected.actual, original);
        originals.push(model::Original {
            requested: member.requested_digest(),
            actual: member.actual_digest(),
            actions: selected.actions,
            reserve_until: original.selected_wait_reserve_horizon()?,
            reserve_observation: original
                .selected_wait_reserve_observation
                .map(|(_, truth)| truth),
            can_continue: original.selected_wait_can_continue(),
        });
    }
    let mut owned_plans = BTreeSet::new();
    let mut receipt_bytes = 0u64;
    for id in &session.open_actions {
        remaining(context, started)?;
        action_union.insert(*id);
        let commit = session
            .adapter
            .action_plan_receipt(*id)
            .ok_or_else(|| invalid("current session progression lost an original commit"))?;
        if owned_plans.insert(commit.plan_digest) {
            let selected =
                super::super::plan_cancellation::select_for_census(session, commit.plan_digest)?;
            if !selected.actions.contains(id) {
                return Err(invalid(
                    "current progression action has inconsistent owned custody",
                ));
            }
            receipt_bytes = receipt_bytes.saturating_add(
                session
                    .commit_receipts
                    .get(&commit.plan_digest.to_hex())
                    .map_or(0, String::len) as u64,
            );
        }
    }
    let mut carried_progress = Vec::new();
    let mut carried_custody_bytes = 0u64;
    let mut carried_custody_records = 0u64;
    let mut additional_carried_bytes = 0u64;
    // Every real foreground wait includes recovered peer cadence, including
    // when the selected original still owns live action handles. Admit the
    // same passive monitors for both scopes; selection remains live-only for
    // current_stage and never gains recovered dispatch authority.
    let carried_originals: BTreeSet<_> = session
        .carried
        .iter()
        .filter(|row| !row.is_final())
        .map(|row| row.plan_digest)
        .collect();
    for digest in carried_originals {
        remaining(context, started)?;
        let owner = session
            .objectives
            .iter()
            .find(|original| original.plan_digest == digest)
            .ok_or_else(|| invalid("competing recovered work lost its exact original owner"))?;
        let (rows, custody_bytes) = owner.forecast_carried_progress(session)?;
        if !recovered_digests.contains(&digest) {
            carried_custody_bytes = carried_custody_bytes.saturating_add(custody_bytes as u64);
            carried_custody_records = carried_custody_records
                .saturating_add(owner.resource_plan().map_or(0, |plan| plan.steps.len()) as u64);
        }
        if recovered_goals.is_empty() {
            for row in &rows {
                additional_carried_bytes =
                    additional_carried_bytes.saturating_add(row.to_json().to_string().len() as u64);
            }
        }
        carried_progress.extend(rows);
        if carried_progress.len() > context.budget.max_actions as usize {
            return Err(exhausted(
                "competing recovered monitors exceed the shared action budget",
            ));
        }
    }
    let recovered_peer_steps = carried_progress
        .iter()
        .filter(|row| !recovered_digests.contains(&row.plan_digest))
        .count();
    let recovered_predicate_bound = model::recovered_predicate_node_bound(&carried_progress)?;
    if action_union
        .len()
        .saturating_add(recovered_steps)
        .saturating_add(recovered_peer_steps)
        > context.budget.max_actions as usize
    {
        return Err(exhausted(
            "selected and competing original actions exceed the shared action budget",
        ));
    }
    let current_source = super::super::query_budget::admit_sources(
        &[session.adapter.snapshot()],
        &remaining(context, started)?,
    )?;
    let current_records = current_source["records"]
        .as_u64()
        .ok_or_else(|| invalid("forecast source record accounting missing"))?;
    let current_bytes = current_source["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("forecast source byte accounting missing"))?;
    let project_sources = if project_snapshots.is_empty() {
        json!({"records":0,"canonical_bytes":0})
    } else {
        super::super::query_budget::admit_sources(
            &project_snapshots,
            &remaining(context, started)?,
        )?
    };
    let project_records = project_sources["records"]
        .as_u64()
        .ok_or_else(|| invalid("project source record accounting missing"))?
        .saturating_add(recovered_records);
    let project_bytes = project_sources["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("project source byte accounting missing"))?
        .saturating_add(recovered_bytes);
    // Archived shapes have already passed the same iterative inspector during
    // recovery. Combine their authenticated bounds with all retained original
    // and reserve sources; every private current source joins this union below.
    let mut fixed_shape = recovered_shape;
    if !recovered_goals.is_empty() {
        for snapshot in project_snapshots.iter().chain(&reserve_snapshots) {
            let shape = super::super::query_budget::source_shape(
                snapshot,
                &remaining(context, started)?,
                Instant::now(),
            )?;
            fixed_shape.0 = fixed_shape.0.saturating_add(shape.0);
            fixed_shape.1 = fixed_shape.1.saturating_add(shape.1);
        }
    }
    let fixed_records = source
        .records
        .saturating_sub(current_records)
        .saturating_add(project_records)
        .saturating_add(carried_custody_records)
        .saturating_add(session.open_actions.len() as u64);
    let fixed_bytes = source
        .bytes
        .saturating_sub(current_bytes)
        .saturating_add(receipt_bytes)
        .saturating_add(project_bytes)
        .saturating_add(carried_custody_bytes)
        .saturating_add(additional_carried_bytes);
    let project_goal_node_bound = if complete_project {
        // One extra semantic goal and one possible remaining-reserve pass,
        // using the same closed-model bound as selected original inspection.
        super::super::selected_plan_wait::SEMANTIC_GOAL_NODE_RESERVE * 2
    } else {
        0
    };
    let predicate_node_bound = source
        .predicate_node_bound
        .saturating_add(project_goal_node_bound);
    let action_steps = action_union
        .len()
        .saturating_add(recovered_steps)
        .saturating_add(recovered_peer_steps) as u64;
    Ok(AdmittedProjectSet {
        session: context.session_id,
        anchor: context.anchor,
        requested: digests.to_vec(),
        originals,
        projects,
        carried: carried_progress,
        owned_actions: &session.open_actions,
        action_steps,
        fixed_records,
        fixed_bytes,
        predicate_node_bound,
        recovered_predicate_node_bound: recovered_predicate_bound,
        goals,
        source,
        project_goal_node_bound,
        recovered_goals,
        recovered_sources,
        recovered_steps,
        verified_recovered_steps,
        unfinished_recovered_steps,
        project_snapshot_count: project_snapshots.len(),
        project_records,
        project_bytes,
        fixed_shape,
        carried_custody_records,
        carried_custody_bytes,
        next_wait_digests,
        reserve_endpoint_pursuits,
    })
}
