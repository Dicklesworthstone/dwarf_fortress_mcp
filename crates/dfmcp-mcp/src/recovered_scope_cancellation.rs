//! Bounded selection of original work across durable recovery.
//!
//! Recovery discards live action handles. Automatic selection must inspect the
//! original sources and current physical work instead of treating an empty live
//! handle list, or a cached terminal receipt, as proof that nothing remains.

use std::collections::BTreeSet;

use super::super as server;
use super::*;

pub(super) struct Discovery {
    pub(super) digests: Vec<Digest32>,
    pub(super) inspected_bytes: u64,
    pub(super) inspected_records: u64,
}

/// Recovery originals precede every current-process dispatch: their seals are
/// in an earlier accepted observation epoch. Within that history, use original
/// cursor order and the sealed digest as the canonical tie-break, never the
/// journal map's incidental digest order or the cached carried-work status.
pub(super) fn discover_recovered(session: &LabSession, started: Instant, all: bool) -> Result<Discovery> {
    if session.carried.is_empty() {
        return Ok(Discovery { digests: Vec::new(), inspected_bytes: 0, inspected_records: 0 });
    }
    session.budget.validate()?;
    let context = remaining_context(session, started)?;
    server::authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    server::authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    let snapshot = session.adapter.snapshot();
    let source = server::query_budget::admit_sources(&[snapshot], &context)?;
    let source_bytes = source["canonical_bytes"].as_u64().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "original selection source size missing")
    })?;
    let source_records = source["records"].as_u64().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "original selection source count missing")
    })?;
    let mut records = (session.carried.len() as u64)
        .saturating_add(session.objectives.len() as u64)
        .saturating_add(session.open_actions.len() as u64);
    let maximum = session.budget.max_bytes.min(MAX_SOURCE_BYTES);
    // Fixed-size selection metadata: digest, full original anchor and step
    // identity. Variable source text and typed archives are charged separately
    // before visiting any original action or hashing its recursive seal.
    let mut bytes = records.saturating_mul(192);
    let check = |bytes: u64, records: u64| -> Result<()> {
        check_time(session, started)?;
        if source_bytes.saturating_add(bytes) >= maximum
            || source_records.saturating_add(records) > u64::from(session.budget.max_entities)
        {
            return Err(exhausted("complete recovered-original selection exceeds its source budget"));
        }
        Ok(())
    };
    check(bytes, records)?;
    let digests = session.carried.iter().map(|row| row.plan_digest).collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(digests.len());
    for digest in digests {
        check(bytes, records)?;
        let entry = original(session, digest)?.resource_goal(session);
        let plan = entry.plan.ok_or_else(|| refused("an original work owner has no verified sealed plan"))?;
        let anchor = entry.sealed_anchor.ok_or_else(|| refused("an original work owner has no exact sealed anchor"))?;
        if entry.owned_by_current_session
            || session.commit_receipts.contains_key(&digest.to_hex())
            || plan.digest != digest
            || plan.anchor != anchor
            || anchor.fortress_id != snapshot.fortress_id
            || anchor.cursor.epoch >= snapshot.cursor.epoch
        {
            return Err(refused("recovered work has ambiguous live custody or an invalid original observation epoch"));
        }
        bytes = bytes.saturating_add(entry.summary.len() as u64)
            .saturating_add(entry.request_json.map_or(0, str::len) as u64);
        records = records.saturating_add(plan.steps.len() as u64);
        check(bytes, records)?;
        let (archive, _) = plan_archive::encode_with_check(
            plan,
            plan_archive::ArchiveLimits {
                max_bytes: ((maximum - source_bytes - bytes) as usize).min(plan_archive::MAX_ARCHIVE_BYTES),
                max_steps: MAX_STEPS,
                ..plan_archive::ArchiveLimits::default()
            },
            || check_time(session, started),
        )?;
        bytes = bytes.saturating_add(archive.len() as u64);
        check(bytes, records)?;
        if !plan.digest_is_valid() || !plan.id_is_valid() {
            return Err(refused("an original work source no longer matches its sealed identity"));
        }
        // Verify every candidate seal before its anchor can influence order.
        // A corrupted older source must not sort behind a valid newer source.
        ordered.push((anchor.cursor.epoch, anchor.cursor.sequence, digest));
    }
    ordered.sort_unstable();
    let mut pending_originals = Vec::new();
    for (_, _, digest) in ordered {
        let entry = original(session, digest)?.resource_goal(session);
        let plan = entry.plan.ok_or_else(|| refused("the original work source became unavailable"))?;
        let mut seen = BTreeSet::new();
        let mut pending = false;
        for row in session.carried.iter().filter(|row| row.plan_digest == digest) {
            check(bytes, records)?;
            let id = row.step.ok_or_else(|| refused("recovered work has no exact original step"))?;
            let step = plan.steps.iter().find(|step| step.id == id)
                .ok_or_else(|| refused("recovered work names a foreign original step"))?;
            if !seen.insert(id) {
                return Err(refused("recovered work has duplicate original step custody"));
            }
            let not_dispatched = row.state == "not_dispatched";
            if not_dispatched && !server::goal_custody::effect_identity_absent(step, snapshot) {
                return Err(refused("current physical work contradicts original nondispatch custody"));
            }
            let work = dfmcp_intent::inspect_effect_work(
                snapshot, &step.action, &step.idempotency_key, !not_dispatched,
            )?;
            let terminal = matches!(row.state.as_str(), "verified" | "failed" | "cancelled"
                | "compensated" | "not_dispatched" | "abandoned");
            pending |= !terminal || !work.is_quiescent();
        }
        // For an active pursuit, even a quiet skip requires its complete
        // authenticated action census. Archived goal truth or a cached final
        // disposition cannot conceal a missing or contradictory receipt.
        if entry.withdrawal.is_none() && entry.restore_abandoned_anchor.is_none()
            && !entry.abandonment_publication_pending
        {
            let custody = original(session, digest)?.selected_wait_recovery(session)?;
            bytes = bytes.saturating_add(custody.bytes()? as u64);
            check(bytes, records)?;
            pending = custody.drain_census(session, plan)?.iter().any(needs_drain);
        }
        if pending {
            // Selection never falls through an older unavailable original to
            // whichever newer original happens to be cancellable. Full exact
            // custody, current scope and effect authority are checked next.
            pending_originals.push(digest);
            if !all {
                return Ok(Discovery {
                    digests: pending_originals,
                    inspected_bytes: source_bytes.saturating_add(bytes),
                    inspected_records: source_records.saturating_add(records),
                });
            }
        }
    }
    check(bytes, records)?;
    Ok(Discovery {
        digests: pending_originals,
        inspected_bytes: source_bytes.saturating_add(bytes),
        inspected_records: source_records.saturating_add(records),
    })
}

pub(crate) fn oldest_open_plan(session: &mut LabSession, mode: CancelMode) -> String {
    let started = Instant::now();
    match discover_recovered(session, started, false) {
        Ok(oldest) => {
            let Some(digest) = oldest.digests.first().copied() else {
                return server::drain_oldest_live_at(session, mode, started,
                    oldest.inspected_bytes, oldest.inspected_records);
            };
            let selected = match select_at(session, digest, mode, started,
                oldest.inspected_bytes, oldest.inspected_records) {
                Ok(selected) => selected,
                Err(error) => return server::drain_refusal_payload(session, "oldest_open_plan", &[], &error, None),
            };
            cancel_selected(session, mode, "oldest_open_plan", selected)
        }
        Err(error) => server::drain_refusal_payload(session, "oldest_open_plan", &[], &error, None),
    }
}
