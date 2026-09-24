//! Receipt-verified, foreground-only recovery for the control/1.7 pause family.
//!
//! A known native key can name only a prepare, or an interrupted commit. Neither
//! is proof of non-application. Only a complete identity-bound terminal receipt
//! may resolve an attempted effect. The reconciliation transport has no mutation
//! methods; one-shot execution uses its own explicitly separate source trait.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};

use crate::control_effect_journal::{
    ControlEffectJournal, DurablePauseRecord, EffectJournalStorage,
};
use crate::live_control_rpc::PauseEffect;

#[path = "pause_commit.rs"]
mod execution;
pub use execution::{PauseCommitOutcome, PauseCommitSource, commit_once};

pub const MAX_RECONCILIATION_EFFECTS: usize = 16;

fn rejected(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}

fn identity_bytes(domain: &[u8], key: &str, plan: Digest32, generation: u64) -> Vec<u8> {
    let mut bytes = domain.to_vec();
    bytes.extend_from_slice(&generation.to_be_bytes());
    bytes.extend_from_slice(key.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(plan.as_bytes());
    bytes
}

fn token_digest(key: &str, plan: Digest32, generation: u64, tick: u64, paused: bool) -> Digest32 {
    let mut bytes = identity_bytes(b"dfmcp-control-token-v2\0", key, plan, generation);
    bytes.extend_from_slice(&tick.to_be_bytes());
    bytes.push(u8::from(paused));
    Digest32::of_bytes(&bytes)
}

fn receipt_digest(record: &DurablePauseRecord, tick: u64) -> Digest32 {
    let mut bytes = identity_bytes(
        b"dfmcp-control-receipt-v2\0",
        &record.idempotency_key,
        record.plan_digest,
        record.bridge_generation,
    );
    bytes.push(u8::from(record.desired_paused));
    bytes.extend_from_slice(&tick.to_be_bytes());
    Digest32::of_bytes(&bytes)
}

/// Validate the native seal before persisting a newly prepared effect.
/// This is an identity checksum, not authentication or production admission.
pub fn validate_prepare_reply(
    key: &str,
    plan: Digest32,
    paused: bool,
    tick: u64,
    effect: &PauseEffect,
) -> Result<[u8; 16]> {
    if key.is_empty()
        || key.len() > 512
        || key.chars().any(char::is_control)
        || effect.bridge_generation == 0
        || effect.bridge_generation == u64::MAX
        || effect.known
        || effect.applied
        || !effect.receipt_digest.is_empty()
    {
        return Err(rejected(
            "invalid new pause preparation identity or outcome",
        ));
    }
    let token: [u8; 16] = effect
        .prepare_token
        .as_slice()
        .try_into()
        .map_err(|_| rejected("pause prepare token must contain exactly 16 bytes"))?;
    let expected = token_digest(key, plan, effect.bridge_generation, tick, paused);
    if token.as_slice() != &expected.as_bytes()[..16] {
        return Err(rejected(
            "pause prepare token is not bound to the requested identity",
        ));
    }
    Ok(token)
}

/// Missing or lost-generation evidence returns None, never a not-applied proof.
fn verified_receipt(record: &DurablePauseRecord, effect: &PauseEffect) -> Result<Option<Digest32>> {
    if effect.bridge_generation != record.bridge_generation || !effect.known {
        return Ok(None);
    }
    if !effect.prepare_token.is_empty()
        && effect.prepare_token.as_slice() != record.prepare_token.as_slice()
    {
        return Err(rejected(
            "pause reconciliation token names another prepared effect",
        ));
    }
    if effect.receipt_digest.is_empty() {
        if effect.applied {
            return Err(rejected("applied pause effect has no terminal receipt"));
        }
        return Ok(None);
    }
    let digest: [u8; 32] = effect
        .receipt_digest
        .as_slice()
        .try_into()
        .map_err(|_| rejected("pause terminal receipt must contain exactly 32 bytes"))?;
    let digest = Digest32::from_bytes(digest);
    if effect.observed_tick < record.expected_game_tick
        || effect.applied != (effect.paused == record.desired_paused)
        || digest != receipt_digest(record, effect.observed_tick)
    {
        return Err(rejected(
            "pause terminal receipt or observed outcome disagrees with durable identity",
        ));
    }
    Ok(Some(digest))
}

/// Validate a complete native result before adding terminal journal evidence.
/// Invalid evidence leaves the existing attempt unresolved; no commit is sent.
pub fn reconcile_reply<S: EffectJournalStorage>(
    journal: &mut ControlEffectJournal<S>,
    key: &str,
    plan: Digest32,
    effect: &PauseEffect,
    context: &OperationContext,
) -> Result<DurablePauseRecord> {
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
    if journal.read_only() {
        return Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            "offline recovery cannot reconcile or write evidence",
        ));
    }
    let _ = journal.records(context)?;
    let record = journal.lookup(key).cloned().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InvalidRequest, "unknown durable pause effect")
    })?;
    if record.plan_digest != plan {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "pause effect plan digest disagrees",
        ));
    }
    if !record.state.reconciliation_required() {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "only unresolved commit attempts may be reconciled",
        ));
    }
    match verified_receipt(&record, effect)? {
        None => journal.mark_indeterminate(key, plan, context),
        Some(receipt) => journal.record_reconciliation(
            key,
            plan,
            effect.bridge_generation,
            true,
            effect.applied,
            effect.paused,
            effect.observed_tick,
            Some(receipt),
            context,
        ),
    }
}

/// A deliberately read-only transport interface: recovery cannot call CommitPause.
pub trait PauseReconciliationSource {
    fn query_effect(
        &mut self,
        record: &DurablePauseRecord,
        remaining: Duration,
        context: &OperationContext,
    ) -> Result<PauseEffect>;
}

#[derive(Clone, Debug)]
pub struct ReconciliationItem {
    pub record: DurablePauseRecord,
    pub queried: bool,
    pub deferred: bool,
    pub error: Option<ErrorCode>,
}

#[derive(Clone, Debug)]
pub struct ReconciliationBatch {
    pub items: Vec<ReconciliationItem>,
    pub head_before: Digest32,
    pub head_after: Digest32,
    pub stopped: Option<ErrorCode>,
}

/// Validate the entire bounded selection before any transport or journal write.
/// Canonical key order makes equivalent selections execute deterministically.
pub fn select_effects<S: EffectJournalStorage>(
    journal: &mut ControlEffectJournal<S>,
    keys: &[String],
    context: &OperationContext,
) -> Result<Vec<DurablePauseRecord>> {
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
    if journal.read_only() {
        return Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            "offline recovery cannot contact a bridge",
        ));
    }
    if keys.is_empty()
        || keys.len() > MAX_RECONCILIATION_EFFECTS
        || keys.len() as u64 > u64::from(context.budget.max_entities)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "reconciliation selects 1..16 effects within the entity budget",
        ));
    }
    if keys
        .iter()
        .any(|key| key.is_empty() || key.len() > 512 || key.chars().any(char::is_control))
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "invalid reconciliation idempotency key",
        ));
    }
    let ordered: BTreeSet<_> = keys.iter().collect();
    if ordered.len() != keys.len() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "duplicate reconciliation idempotency key",
        ));
    }
    let _ = journal.records(context)?;
    ordered
        .into_iter()
        .map(|key| {
            journal.lookup(key).cloned().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "reconciliation selection contains an unknown effect",
                )
            })
        })
        .collect()
}

/// One bounded foreground pass. Prepared/terminal effects never contact the bridge.
/// The first failure or deadline stops further queries; completed evidence is not undone.
pub fn reconcile_batch<S: EffectJournalStorage, Q: PauseReconciliationSource>(
    journal: &mut ControlEffectJournal<S>,
    source: &mut Q,
    keys: &[String],
    context: &OperationContext,
) -> Result<ReconciliationBatch> {
    let started = Instant::now();
    reconcile_batch_with_clock(journal, source, keys, context, || started.elapsed())
}

fn reconcile_batch_with_clock<S: EffectJournalStorage, Q: PauseReconciliationSource>(
    journal: &mut ControlEffectJournal<S>,
    source: &mut Q,
    keys: &[String],
    context: &OperationContext,
    mut elapsed: impl FnMut() -> Duration,
) -> Result<ReconciliationBatch> {
    let selected = select_effects(journal, keys, context)?;
    let mut batch = ReconciliationBatch {
        items: Vec::with_capacity(selected.len()),
        head_before: journal.head(),
        head_after: journal.head(),
        stopped: None,
    };
    let budget = Duration::from_millis(context.budget.max_wall_millis);
    for record in selected {
        let mut item = ReconciliationItem {
            record,
            queried: false,
            deferred: false,
            error: None,
        };
        if item.record.state.reconciliation_required() {
            let remaining = budget
                .checked_sub(elapsed())
                .filter(|v| *v >= Duration::from_millis(1));
            if remaining.is_none() && batch.stopped.is_none() {
                batch.stopped = Some(ErrorCode::BudgetExceeded);
            }
            if batch.stopped.is_some() {
                item.deferred = true;
            } else if let Some(remaining) = remaining {
                // Recheck cancellation/authority before each effectful read.
                context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
                item.queried = true;
                let outcome = source
                    .query_effect(&item.record, remaining, context)
                    .and_then(|effect| {
                        reconcile_reply(
                            journal,
                            &item.record.idempotency_key,
                            item.record.plan_digest,
                            &effect,
                            context,
                        )
                    });
                match outcome {
                    Ok(updated) => item.record = updated,
                    Err(error) => {
                        // A failed sync may have written bytes. Never invent a terminal result.
                        item.error = Some(error.code);
                        batch.stopped = Some(error.code);
                    }
                }
            }
        }
        batch.items.push(item);
    }
    batch.head_after = journal.head();
    Ok(batch)
}

#[cfg(test)]
#[path = "pause_reconciliation_tests.rs"]
mod tests;
