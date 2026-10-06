#![forbid(unsafe_code)]

//! Allocate distinct furniture items from one complete, published operations view.
//! The conservative supply policy does not establish native placement eligibility,
//! route access, reservations, or future availability. Keep the enclosing source
//! and entity generations; never reproject an embedded capture into another world.
//! Beads: df-dfhack-bridge-plane-c-pic.3 / df-dfhack-bridge-plane-c-pic.4.

use crate::furniture_allocation::{self as allocation, Allocation, Candidate, Kind, Request};
use crate::live_operations::item_entity_id;
use crate::operations_analysis::{AnalysisHandle, OperationsStateView};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const SUPPLY_POLICY: &str = "direct-ground-unattached-singleton-furniture/1";
pub const MAX_WORK: u64 = crate::operations_analysis::MAX_ANALYSIS_WORK;
const MAX_ITEMS: usize = 65_536;
const MAX_ATTACHMENTS: usize = 65_536;
const CHECK_INTERVAL: u64 = 256;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}
fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

struct Work<'a> {
    started: Instant,
    wall_millis: u64,
    used: u64,
    maximum: u64,
    check: Option<&'a mut dyn FnMut() -> Result<()>>,
}
impl<'a> Work<'a> {
    fn new(maximum: u64, wall_millis: u64) -> Result<Self> {
        if maximum == 0 || maximum > MAX_WORK || wall_millis == 0 {
            return Err(exhausted("invalid furniture allocation work allowance"));
        }
        Ok(Self {
            started: Instant::now(),
            wall_millis,
            used: 0,
            maximum,
            check: None,
        })
    }

    fn with_check(
        maximum: u64,
        wall_millis: u64,
        check: &'a mut dyn FnMut() -> Result<()>,
    ) -> Result<Self> {
        let mut work = Self::new(maximum, wall_millis)?;
        work.check = Some(check);
        work.checkpoint()?;
        Ok(work)
    }

    fn within_allowance(&self) -> Result<()> {
        if self.used > self.maximum
            || self.started.elapsed().as_millis() >= u128::from(self.wall_millis)
        {
            return Err(exhausted(
                "furniture allocation exhausted its shared work or wall allowance",
            ));
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<()> {
        self.within_allowance()?;
        if let Some(check) = self.check.as_mut() {
            check()?;
        }
        // Checking the owner is work too; it cannot renew this deadline.
        self.within_allowance()
    }

    fn charge(&mut self) -> Result<()> {
        self.used = self
            .used
            .checked_add(1)
            .ok_or_else(|| exhausted("furniture allocation work overflow"))?;
        self.within_allowance()?;
        if self.used.is_multiple_of(CHECK_INTERVAL) {
            self.checkpoint()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEvidence {
    pub candidate: Candidate,
    pub handle: AnalysisHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub request: Request,
    pub allocation: Allocation,
    /// Only assigned items or members of the complete shortage witness.
    pub items: BTreeMap<u32, ItemEvidence>,
    /// One disjoint primary classification per observed item.
    pub item_counts: BTreeMap<&'static str, u64>,
    pub observed_items: usize,
    pub candidate_count: usize,
    pub work_units: u64,
}

pub fn plan<S: OperationsStateView + ?Sized>(
    state: &S,
    context: &OperationContext,
    folder: &str,
    site: u32,
    requested: &Request,
    maximum_work: u64,
) -> Result<Report> {
    plan_with_check(
        state,
        context,
        folder,
        site,
        requested,
        maximum_work,
        &mut || Ok(()),
    )
}

/// Plan under the live foreground owner's cancellation and authority checks.
/// The callback may only further restrict the context's Query authority. It is
/// checked before source work, at phase boundaries, at most every 256 charged
/// work units during scans/matching, and before returning the complete result.
/// Bounded hashing and sorting remain synchronous, not hard-preemptible work.
/// Failure returns no partial assignment or shortage report. The legacy entry
/// point retains identical model, digest and work-accounting semantics.
pub fn plan_with_check<S: OperationsStateView + ?Sized>(
    state: &S,
    context: &OperationContext,
    folder: &str,
    site: u32,
    requested: &Request,
    maximum_work: u64,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Report> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut work = Work::with_check(maximum_work, context.budget.max_wall_millis, check)?;
    if folder.is_empty() || folder.len() > 512 || folder.contains('\0') || site > i32::MAX as u32 {
        return Err(invalid("invalid requested furniture fortress identity"));
    }
    let observation = state.operations_observation().ok_or_else(|| {
        invalid("furniture allocation requires a complete published operations capture")
    })?;
    let snapshot = state
        .operations_snapshot()
        .ok_or_else(|| invariant("furniture allocation projection is absent"))?;
    if context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "furniture allocation names another observation",
        ));
    }
    if observation.jobs.world_folder != folder
        || i64::from(observation.jobs.site_id) != i64::from(site)
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "furniture request belongs to another fortress",
        ));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize
        || observation.items.len() > MAX_ITEMS
        || observation.attachments.len() > MAX_ATTACHMENTS
    {
        return Err(exhausted(
            "furniture allocation exceeds its complete inventory scan bound",
        ));
    }
    work.checkpoint()?;
    if !snapshot.hash_is_valid() {
        return Err(invariant("furniture allocation source hash is invalid"));
    }
    let source_digest = state.operations_source_digest()?;
    work.checkpoint()?;
    work.charge()?;
    let request = allocation::normalize(requested, &mut || work.charge())?;
    let mut excluded = BTreeSet::new();
    for &id in &request.excluded_items {
        work.charge()?;
        excluded.insert(id);
    }
    let mut attached = BTreeSet::new();
    for attachment in &observation.attachments {
        work.charge()?;
        attached.insert(attachment.item_native_id);
    }
    let mut containing_items = BTreeSet::new();
    for item in &observation.items {
        work.charge()?;
        if let Some(container) = item.container_native_id {
            containing_items.insert(container);
        }
    }
    let mut candidates = Vec::new();
    let mut item_counts = BTreeMap::new();
    for item in &observation.items {
        work.charge()?;
        let kind = match item.type_key.as_str() {
            "BED" => Some(Kind::Bed),
            "CHAIR" => Some(Kind::Chair),
            "TABLE" => Some(Kind::Table),
            _ => None,
        };
        let p = item.raw_position;
        let reason = if kind.is_none() {
            Some("unsupported_furniture_type")
        } else if item.flags != 64 {
            Some("not_exclusively_ground_flags")
        } else if item.container_native_id.is_some() {
            Some("contained_item")
        } else if item.holder_building_native_id.is_some() {
            Some("building_held_item")
        } else if containing_items.contains(&item.native_id) {
            Some("contains_observed_items")
        } else if attached.contains(&item.native_id) {
            Some("job_attached_item")
        } else if item.stack_size != 1 {
            Some("not_singleton")
        } else if item.material_type < 0 {
            Some("unknown_material")
        } else if [p.x, p.y, p.z].iter().any(|&n| !(0..=32_767).contains(&n)) {
            Some("unestablished_bounded_position")
        } else if excluded.contains(&item.native_id) {
            Some("explicitly_excluded_item")
        } else {
            None
        };
        if let Some(reason) = reason {
            *item_counts.entry(reason).or_insert(0) += 1;
            continue;
        }
        let kind = kind.ok_or_else(|| invariant("admitted furniture kind disappeared"))?;
        candidates.push(Candidate {
            native_id: item.native_id,
            kind,
            position: [p.x as u32, p.y as u32, p.z as u32],
            material_type: item.material_type,
            material_index: item.material_index,
            subtype: item.subtype,
        });
        *item_counts.entry("candidate_furniture").or_insert(0) += 1;
    }
    work.checkpoint()?;
    let allocation = allocation::allocate(&request, &candidates, &mut || work.charge())?;
    work.checkpoint()?;
    let mut named = BTreeSet::new();
    for assignment in &allocation.assignments {
        work.charge()?;
        named.insert(assignment.item_id);
    }
    if let Some(shortage) = &allocation.shortage {
        for &id in &shortage.candidate_items {
            work.charge()?;
            named.insert(id);
        }
    }
    let mut items = BTreeMap::new();
    for candidate in &candidates {
        work.charge()?;
        if !named.contains(&candidate.native_id) {
            continue;
        }
        let id = item_entity_id(candidate.native_id);
        let entity =
            snapshot.graph.entities.get(&id).ok_or_else(|| {
                invariant("allocated furniture lacks its original canonical entity")
            })?;
        items.insert(
            candidate.native_id,
            ItemEvidence {
                candidate: *candidate,
                handle: AnalysisHandle {
                    entity_id: id,
                    generation: entity.generation,
                    revision: entity.revision,
                },
            },
        );
    }
    if items.len() != named.len() {
        return Err(invariant(
            "furniture allocation lost original item evidence",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    work.charge()?;
    work.checkpoint()?;
    Ok(Report {
        anchor: snapshot.anchor(),
        source_digest,
        request,
        allocation,
        items,
        item_counts,
        observed_items: observation.items.len(),
        candidate_count: candidates.len(),
        work_units: work.used,
    })
}

#[cfg(test)]
#[path = "furniture_supply_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "furniture_supply/check_tests.rs"]
mod check_tests;
