//! Complete-plan custody around the existing single-placement control boundary.
use super::{error, exhausted, presentation};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildInventory, BuildStage};
use dfmcp_adapter::build_placement::{BuildBinding, BuildPlan, BuildSelection};
use dfmcp_adapter::furniture_batch::store::{BatchStore, MAX_STORE_BYTES};
use dfmcp_adapter::furniture_batch::{BatchDefinition, FurniturePlan};
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result};
use serde_json::{Value, json};

pub(super) type Parent = BatchStore<PrivateBuildFile>;
// Separately reserved from the child journal/native operation. Each boundary
// consumes one whole-parent read; an unexpectedly long sequence fails closed.
pub(super) const GUARD_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn verify(
    parent: &mut Parent,
    view: &BuildInventory,
    c: &OperationContext,
) -> Result<()> {
    parent.verify(c)?;
    parent.definition().audit(view, parent.stopped())?;
    Ok(())
}

pub(super) fn next(parent: &Parent, view: &BuildInventory) -> Result<(String, BuildSelection)> {
    let definition = parent.definition();
    let progress = definition.audit(view, parent.stopped())?;
    let step = progress
        .next_step
        .as_deref()
        .and_then(|name| definition.plan().step(name))
        .ok_or_else(|| {
            error(
                ErrorCode::Conflict,
                "the original batch has no unblocked next step",
            )
        })?;
    Ok((definition.key(step), step.selection))
}

pub(super) fn seal(
    parent: &Parent,
    plan: &BuildPlan,
    head: Digest32,
    policy: Digest32,
) -> Digest32 {
    let mut bytes = b"dfmcp-furniture-batch-mcp-review/1\0".to_vec();
    bytes.extend_from_slice(parent.definition().id().as_bytes());
    bytes.extend_from_slice(head.as_bytes());
    bytes.extend_from_slice(policy.as_bytes());
    bytes.extend_from_slice(&plan.canonical_bytes());
    Digest32::of_bytes(&bytes)
}

/// Bound the entire future inventory before parent creation or preparation.
/// 32 KiB is reserved for a full maximum native record and capture rendered as
/// hex, source references, policy (including 32 protected regions), one pending
/// child, and the common Agent Turn. Batch rows contain receipt identities,
/// never copies of full native records; those remain available through get.
pub(super) fn reserve_output(plan: &FurniturePlan) -> Result<()> {
    let rows = plan.steps().iter().map(|step| json!({
        "name":step.name,"idempotency_key":format!("fb-{}-{}", "f".repeat(64), step.name),
        "native_plan_digest":"f".repeat(64),"state":"cancel_requested", "outcome":"indeterminate",
        "receipt_digest":"f".repeat(64),"insertion":{"building_id":u32::MAX,"job_id":u32::MAX}
    })).collect::<Vec<_>>();
    let bound = plan.canonical_bytes().len() + json!(rows).to_string().len() + 2048 + 32768;
    if bound as u64 > presentation::OUTPUT_BYTES {
        return Err(exhausted());
    }
    Ok(())
}

pub(super) fn display(parent: &Parent, view: Option<&BuildInventory>, verified: bool) -> Value {
    let definition = parent.definition();
    let progress = view.and_then(|v| definition.audit(v, parent.stopped()).ok());
    let verified = verified && progress.is_some();
    let rows = definition
        .plan()
        .ordered_steps()
        .map(|step| {
            let key = definition.key(step);
            let row = progress
                .as_ref()
                .and_then(|p| p.rows.iter().find(|r| r.step == step.name));
            let entry = view.and_then(|v| v.entry(&key));
            json!({"name":step.name,"idempotency_key":key,
            "native_plan_digest":row.and_then(|r|r.native_plan_digest).map(|v|v.to_string()),
            "state":row.map_or("unverified",|r|r.state),"outcome":row.and_then(|r|r.outcome),
            "receipt_digest":entry.and_then(|e|e.native()).map(|n|n.receipt().to_string()),
            "insertion":entry.and_then(|e|e.native()).and_then(|n|n.insertion())
                .map(|i|json!({"building_id":i.building_id(),"job_id":i.job_id()}))})
        })
        .collect::<Vec<_>>();
    let next = progress.as_ref().filter(|_| verified).and_then(|p| p.next_step.as_deref())
        .and_then(|name|definition.plan().step(name)).map(|step|json!({
            "name":step.name,"idempotency_key":definition.key(step),"selection":presentation::selection(step.selection)}));
    // Decoding is only a presentation projection of already validated canonical
    // bytes, never the parser that accepts a client's complete-plan intent.
    let plan: Value =
        serde_json::from_slice(definition.plan().canonical_bytes()).unwrap_or(Value::Null);
    let mut result = json!({"schema":"dfmcp.furniture-batch-mcp/1","batch_id":definition.id().to_string(),
        "plan_digest":definition.plan().digest().to_string(),"plan":plan,
        "journal_id":definition.journal_id().to_string(),"head":view.map(|v|v.head.to_string()),
        "inventory_verified":verified,"historical_evidence_only":true,
        "stopped":parent.durable_stopped(),"advancement_fenced":parent.is_fenced(),
        "status":if verified {progress.as_ref().map_or("unverified",|p|p.status)} else {"unverified"},
        "placed":progress.as_ref().map(|p|p.placed),"total":definition.plan().steps().len(),
        "pending_step":progress.as_ref().and_then(|p|p.pending_step.as_deref()),
        "steps":rows,"next":next,"atomic":false,"construction_completion_proven":false,
        "retry_permitted":false,"max_native_commit_calls_per_request":1,
        "fresh_observation_and_review_per_step":true,"reopening_restores_dispatch_permission":false});
    if let Some(handoff) = definition.handoff() {
        result["allocation"] = super::allocation::summary(handoff);
    }
    result
}

pub(super) fn matches(definition: &BatchDefinition, binding: &BuildBinding) -> Result<()> {
    if definition.binding() != binding {
        return Err(error(
            ErrorCode::CorruptLedger,
            "batch source differs from the original child journal",
        ));
    }
    Ok(())
}

pub(super) struct Guard<'a, G> {
    pub parent: &'a mut Option<Parent>,
    pub view: &'a BuildInventory,
    pub key: Option<&'a str>,
    pub runtime: &'a mut G,
    pub bytes: u64,
}
impl<G: BuildGuard> BuildGuard for Guard<'_, G> {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        context: &OperationContext,
    ) -> Result<()> {
        self.runtime
            .check(stage, binding, plan, selection, context)?;
        if let Some(parent) = self.parent.as_mut() {
            // Recovery deliberately uses the original child journal without
            // this guard. Losing the parent cannot erase a pending native key.
            if !matches!(
                stage,
                BuildStage::Connect
                    | BuildStage::Observe
                    | BuildStage::Prepare
                    | BuildStage::Commit
            ) {
                return Ok(());
            }
            self.bytes = self
                .bytes
                .checked_sub(MAX_STORE_BYTES as u64)
                .ok_or_else(exhausted)?;
            let mut current = context.clone();
            current.budget.max_bytes = MAX_STORE_BYTES as u64;
            parent.verify(&current)?;
            matches(parent.definition(), binding)?;
            let key = self
                .key
                .ok_or_else(|| error(ErrorCode::InvalidRequest, "batch step identity missing"))?;
            parent.definition().validate_next(
                self.view,
                parent.stopped(),
                key,
                selection,
                plan.map(|p| p.before()),
            )?;
            if plan.is_some_and(|plan| plan.key() != key) {
                return Err(error(
                    ErrorCode::Conflict,
                    "batch preparation changed its original step key",
                ));
            }
        }
        Ok(())
    }
}
