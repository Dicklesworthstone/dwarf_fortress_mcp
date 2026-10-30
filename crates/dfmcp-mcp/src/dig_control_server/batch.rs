//! Sparse original mining projects around the unchanged one-attempt dig owner.
use super::{error, exhausted, presentation};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::store::{BatchStore, MAX_STORE_BYTES, Recovery};
use dfmcp_adapter::dig_blueprint::{BatchProgress, BlueprintPart, Layout};
use dfmcp_adapter::dig_control_policy::{DigCheckpointPolicy, DigControlPolicy};
use dfmcp_adapter::dig_designation::journal::session::{DigSessionGuard, DigSessionView};
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigGuard, DigRecord, DigStage};
use dfmcp_adapter::dig_designation::{DigObservation, DigPlan, DigRegion};
use dfmcp_core::{
    Capability, Digest32, ErrorCode, OperationContext, Result, RiskTier, cuboids_intersect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) const GUARD_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const PUBLICATION_BYTES: u64 = 8 * MAX_STORE_BYTES as u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub path: PathBuf,
    pub initialize: bool,
    pub recovery: Option<Recovery>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigInput {
    path: String,
    initialize: bool,
    recovery: Option<RecoveryInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    blueprint_id: String,
    head: String,
}
impl Config {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 8192 {
            return Err(exhausted());
        }
        let value: ConfigInput = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid operator blueprint configuration",
            )
        })?;
        if value.path.len() > 4096
            || !value.path.starts_with('/')
            || value.path.contains('\0')
            || value.path[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || value.initialize == value.recovery.is_some()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "blueprint initialization or exact recovery configuration differs",
            ));
        }
        let recovery = value
            .recovery
            .map(|r| {
                let blueprint_id = presentation::digest(&r.blueprint_id)?;
                let head = presentation::digest(&r.head)?;
                if blueprint_id == Digest32::ZERO || head == Digest32::ZERO {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "reserved blueprint recovery identity",
                    ));
                }
                Ok(Recovery { blueprint_id, head })
            })
            .transpose()?;
        Ok(Self {
            path: value.path.into(),
            initialize: value.initialize,
            recovery,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintInput {
    schema: String,
    parts: Vec<PartInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartInput {
    region: RegionInput,
    shape: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionInput {
    origin: [u32; 3],
    size: [u32; 3],
}
pub(super) fn parse(raw: &str) -> Result<Layout> {
    if raw.is_empty() || raw.len() > 16_384 {
        return Err(exhausted());
    }
    let value: BlueprintInput = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed floor blueprint"))?;
    if value.schema != "dfmcp.excavation-blueprint/1"
        || value.parts.is_empty()
        || value.parts.len() > 32
        || value.parts.iter().any(|p| p.shape != "floor")
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "blueprint requires one to 32 exact floor parts",
        ));
    }
    Layout::new(
        value
            .parts
            .into_iter()
            .map(|p| BlueprintPart {
                origin: p.region.origin,
                size: p.region.size,
            })
            .collect(),
    )
}

/// Preflight every future shared block before publishing the parent. Each
/// subsequent native preparation still repeats current authority and policy.
pub(super) fn policy(
    layout: &Layout,
    policy: &DigControlPolicy,
    c: &OperationContext,
) -> Result<()> {
    c.authorize(
        Capability::Plan,
        RiskTier::Guarded,
        &[],
        Some(policy.binding().scope()),
    )?;
    c.authorize(
        Capability::Designate,
        RiskTier::Guarded,
        &[],
        Some(policy.binding().scope()),
    )?;
    if policy.checkpoint_policy() != DigCheckpointPolicy::DisposableFortress {
        return Err(error(
            ErrorCode::CheckpointRequired,
            "blueprint designation needs the explicit operator checkpoint exception",
        ));
    }
    for region in layout.regions() {
        if !policy.binding().scope().contains_cuboid(region.halo())
            || !policy
                .binding()
                .scope()
                .contains_cuboid(region.write_area())
            || policy
                .protected_areas()
                .iter()
                .any(|p| cuboids_intersect(p, &region.write_area()))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "a future blueprint halo or shared write block violates operator scope",
            ));
        }
    }
    Ok(())
}

pub(super) fn verify<S: EffectJournalStorage>(
    parent: &mut BatchStore<S>,
    view: &DigSessionView,
    records: &[DigRecord],
    c: &OperationContext,
) -> Result<BatchProgress> {
    parent.verify(c)?;
    parent.definition().audit(view, records, parent.stopped())
}
pub(super) fn seal<S>(
    parent: &BatchStore<S>,
    plan: &DigPlan,
    head: Digest32,
    policy: Digest32,
) -> Digest32
where
    S: EffectJournalStorage,
{
    let mut bytes = b"dfmcp-dig-blueprint-mcp-review/1\0".to_vec();
    bytes.extend_from_slice(parent.definition().id().as_bytes());
    bytes.extend_from_slice(parent.head().as_bytes());
    bytes.extend_from_slice(head.as_bytes());
    bytes.extend_from_slice(policy.as_bytes());
    bytes.extend_from_slice(&plan.canonical_bytes());
    Digest32::of_bytes(&bytes)
}
pub(super) fn recovery(value: Recovery) -> Value {
    json!({"blueprint_id": value.blueprint_id.to_string(), "head": value.head.to_string()})
}
pub(super) fn preview(layout: &Layout) -> Value {
    json!({"schema":"dfmcp.dig-blueprint-layout/1", "layout_digest":layout.digest().to_string(),
        "blueprint_digest":layout.blueprint_digest().to_string(), "target_tiles":layout.targets().len(),
        "step_count":layout.regions().len(), "first_region":layout.regions().first().map(|r|r.coordinates()),
        "unselected_tiles_designated":false,"terrain_observed":false,"mutation_authority_granted":false,
        "excavation_completion_proven":false})
}
pub(super) fn reference<S: EffectJournalStorage>(parent: &BatchStore<S>) -> Value {
    json!({"schema":"dfmcp.dig-blueprint-mcp/1", "blueprint_id":parent.definition().id().to_string(),
        "journal_id":parent.definition().journal_id().to_string(), "recovery":recovery(parent.recovery()),
        "layout_digest":parent.definition().layout().digest().to_string(),
        "blueprint_digest":parent.definition().layout().blueprint_digest().to_string(),
        "target_tiles":parent.definition().layout().targets().len(),"total":parent.definition().layout().regions().len(),
        "inventory_verified":false,"status":"unverified", "stopped":parent.durable_stopped(),
        "advancement_fenced":parent.is_fenced(),"historical_evidence_only":true,
        "excavation_completion_proven":false,"retry_commit_permitted":false,
        "reopening_restores_dispatch_permission":false})
}
pub(super) fn display<S: EffectJournalStorage>(
    parent: &BatchStore<S>,
    progress: &BatchProgress,
    view: &DigSessionView,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    if !(1..=16).contains(&limit) || offset >= progress.total {
        return Err(error(
            ErrorCode::InvalidRequest,
            "blueprint page is outside the original layout",
        ));
    }
    let definition = parent.definition();
    let end = offset.saturating_add(limit).min(progress.total);
    let rows = progress.rows.iter().skip(offset).take(limit).map(|row| {
        json!({"index":row.index,"region":definition.layout().regions()[row.index].coordinates(),
            "idempotency_key":row.key,"native_plan_digest":row.native_plan_digest.map(|d|d.to_string()),
            "state":row.state,"outcome":row.outcome,"receipt_digest":row.receipt.map(|d|d.to_string())})
    }).collect::<Vec<_>>();
    let next = progress.next_index.map(|index| -> Result<Value> { Ok(json!({"index":index,
        "region":definition.layout().regions()[index].coordinates(), "idempotency_key":definition.key(index)?}))
    }).transpose()?;
    let mut result = reference(parent);
    result["inventory_verified"] = json!(true);
    result["status"] = json!(progress.status);
    result["designated"] = json!(progress.designated);
    result["pending_step"] = json!(progress.pending_index);
    result["journal_head"] = json!(view.head.to_string());
    result["steps"] = json!(rows);
    result["offset"] = json!(offset);
    result["next"] = json!(next);
    result["next_query"] = if end < progress.total {
        json!({"mode":"blueprint","blueprint_id":definition.id().to_string(),"expected_head":view.head.to_string(),"expected_parent_head":parent.head().to_string(),"offset":end,"limit":limit})
    } else {
        Value::Null
    };
    result["atomic"] = json!(false);
    result["max_native_commit_calls_per_request"] = json!(1);
    result["fresh_observation_and_review_per_step"] = json!(true);
    Ok(result)
}

/// Parent checks add constraints to the existing runtime guard. The caller
/// reserves this owner's work separately from native/journal work. Recovery and
/// retirement use the underlying guard directly so lost parent custody cannot
/// erase an already pending native operation.
pub(super) struct Guard<'a, S, G> {
    pub parent: &'a mut Option<BatchStore<S>>,
    pub inventory: Option<&'a (DigSessionView, Vec<DigRecord>)>,
    pub key: Option<&'a str>,
    pub runtime: &'a mut G,
    pub bytes: u64,
}
impl<S: EffectJournalStorage, G: DigSessionGuard> Guard<'_, S, G> {
    fn validate(
        &mut self,
        binding: Option<&DigBinding>,
        region: DigRegion,
        capture: Option<&DigObservation>,
        c: &OperationContext,
    ) -> Result<()> {
        if let Some(parent) = self.parent.as_mut() {
            self.bytes = self
                .bytes
                .checked_sub(MAX_STORE_BYTES as u64)
                .ok_or_else(exhausted)?;
            let mut current = c.clone();
            current.budget.max_bytes = MAX_STORE_BYTES as u64;
            parent.verify(&current)?;
            if binding.is_some_and(|b| b != parent.definition().binding()) {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "blueprint source binding changed",
                ));
            }
            let (view, records) = self.inventory.ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "complete blueprint prefix is absent",
                )
            })?;
            let key = self.key.ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "original blueprint step key is absent",
                )
            })?;
            parent.definition().validate_next(
                view,
                records,
                parent.stopped(),
                key,
                region,
                capture,
            )?;
        }
        Ok(())
    }
}
impl<S: EffectJournalStorage, G: DigSessionGuard> DigGuard for Guard<'_, S, G> {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, c: &OperationContext) -> Result<()> {
        self.runtime.check(stage, plan, c)?;
        if matches!(stage, DigStage::Prepare | DigStage::Commit) {
            if self.parent.is_some()
                && (plan.allow_hidden_neighbors() || self.key.is_some_and(|key| key != plan.key()))
            {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "blueprint child changes its original key or hidden-neighbor policy",
                ));
            }
            self.validate(None, plan.before().region(), Some(plan.before()), c)?;
        }
        Ok(())
    }
}
impl<S: EffectJournalStorage, G: DigSessionGuard> DigSessionGuard for Guard<'_, S, G> {
    fn connect(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.runtime.connect(binding, region, c)?;
        self.validate(Some(binding), region, None, c)
    }
    fn observe(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.runtime.observe(binding, region, c)?;
        self.validate(Some(binding), region, None, c)
    }
}
