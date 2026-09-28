//! Retain a complete allocation request and its historical source through placement.
//!
//! The constructor derives its report from one sealed, published operations/1.4
//! state. The caller supplies the endpoint only after authenticated acquisition;
//! this pure module does not authenticate a socket. Decoding retained bytes is
//! evidence replay, never a new observation, reservation or dispatch permit.
//! Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Instant;

use crate::build_placement::{BuildBinding, BuildCapture, BuildItem, BuildKind, BuildSelection};
use crate::furniture_allocation::{Candidate, Kind};
use crate::furniture_batch::{FurniturePlan, FurnitureStep};
use crate::furniture_supply::{self, Report};
use crate::live_operations::{LiveOperationsState, OperationsProfile, item_entity_id};
use crate::operations_analysis::AnalysisHandle;
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor};

mod request;
pub mod rpc;
mod wire;
pub use request::{FurnitureRequest, MAX_REQUEST_BYTES};

pub const MAX_HANDOFF_BYTES: usize = 24 * 1024;
pub const PROFILE: &str = "operations/1.4";
pub const POLICY: &str = "dfmcp.furniture-handoff/1";
/// Conservative allowance for the payload and bounded source-digest copies,
/// reserved before encoding or source hash validation allocates their buffers.
pub const ALLOCATION_BYTE_RESERVE: u64 =
    3 * OperationsProfile::PagedV1_4.maximum_bytes() as u64 + MAX_HANDOFF_BYTES as u64;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn require(value: bool, message: &str) -> Result<()> {
    if value { Ok(()) } else { Err(invalid(message)) }
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "complete furniture handoff exceeds allowance")
}
fn digest(bytes: &[u8]) -> Digest32 {
    let mut domain = b"dfmcp-furniture-handoff/1\0".to_vec();
    domain.extend_from_slice(bytes);
    Digest32::of_bytes(&domain)
}
fn build_kind(kind: Kind) -> BuildKind {
    match kind { Kind::Bed => BuildKind::Bed, Kind::Chair => BuildKind::Chair, Kind::Table => BuildKind::Table }
}

/// Historical operations source. Its canonical cursor is not a native furniture
/// sequence; the two plugin generations are independent namespaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub endpoint: SocketAddr,
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub capture_digest: Digest32,
    pub capture_bytes: u32,
    pub next_job_id: u32,
    pub next_building_id: u32,
    pub next_item_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedItem {
    pub slot: String,
    pub candidate: Candidate,
    pub native_type: u32,
    pub handle: AnalysisHandle,
    pub distance: u32,
}

/// A shortage contains the entire report and no executable subset or handoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocationOutcome {
    pub report: Report,
    pub handoff: Option<Handoff>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handoff {
    request: FurnitureRequest,
    plan: FurniturePlan,
    source: Source,
    items: Vec<SelectedItem>,
    canonical: Vec<u8>,
    digest: Digest32,
}

struct Work<'a> {
    context: &'a OperationContext,
    started: Instant,
    maximum: u64,
    used: u64,
}
impl Work<'_> {
    fn charge(&mut self, count: u64) -> Result<()> {
        self.used = self.used.checked_add(count).ok_or_else(exhausted)?;
        if self.used > self.maximum || self.started.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis) {
            return Err(exhausted());
        }
        self.context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn remaining(&self) -> Result<OperationContext> {
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).map_err(|_| exhausted())?;
        let mut context = self.context.clone();
        context.budget.max_wall_millis = context.budget.max_wall_millis.checked_sub(elapsed).filter(|n| *n > 0).ok_or_else(exhausted)?;
        Ok(context)
    }
}

impl Handoff {
    /// Derive a complete assignment from this exact canonical state. No caller
    /// report, source digest, or selected-item projection is accepted as input.
    pub fn allocate(
        state: &LiveOperationsState,
        context: &OperationContext,
        endpoint: SocketAddr,
        request: &FurnitureRequest,
        maximum_work: u64,
    ) -> Result<AllocationOutcome> {
        context.budget.validate()?;
        require(maximum_work > 0 && maximum_work <= furniture_supply::MAX_WORK, "invalid furniture handoff work ceiling")?;
        let mut work = Work { context, started: Instant::now(), maximum: maximum_work, used: 0 };
        work.charge(1)?;
        if context.budget.max_bytes < ALLOCATION_BYTE_RESERVE {
            return Err(exhausted());
        }
        require(state.profile() == OperationsProfile::PagedV1_4, "furniture handoff requires complete operations/1.4")?;
        require(endpoint.is_ipv4() && endpoint.ip().is_loopback() && endpoint.port() != 0, "furniture handoff requires IPv4 loopback")?;
        let observed = state.observation().ok_or_else(|| invalid("no published furniture inventory"))?;
        let snapshot = state.snapshot().ok_or_else(|| invalid("no published furniture source anchor"))?;
        require(snapshot.anchor() == context.anchor && snapshot.hash_is_valid(), "furniture handoff names another published anchor")?;
        require(observed.jobs.world_folder == request.folder() && i64::from(observed.jobs.site_id) == i64::from(request.site()), "furniture request source differs")?;
        if snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        // Reject enum aliasing anywhere in the full roster, including irrelevant
        // items. A selected semantic key must have one observed native type.
        let mut types = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for item in &observed.items {
            work.charge(1)?;
            if types.insert(item.item_type, &item.type_key).is_some_and(|key| key != &item.type_key)
                || keys.insert(&item.type_key, item.item_type).is_some_and(|value| value != item.item_type)
            {
                return Err(invalid("inconsistent native furniture item type catalog"));
            }
        }
        // The profile's complete byte ceiling was reserved before encoding.
        // Existing strict codec bounds this temporary payload at 16 MiB.
        let capture = observed.encode_profile(OperationsProfile::PagedV1_4)?;
        work.charge(capture.len().div_ceil(64 * 1024) as u64)?;
        let remaining = work.maximum.checked_sub(work.used).filter(|n| *n > 0).ok_or_else(exhausted)?;
        let mut report = furniture_supply::plan(state, &work.remaining()?, request.folder(), request.site(), request.request(), remaining)?;
        work.charge(report.work_units)?;
        let handoff = if report.allocation.shortage.is_some() {
            require(report.allocation.assignments.is_empty(), "shortage exposed a partial furniture assignment")?;
            None
        } else {
            let source = Source {
                endpoint,
                generation: observed.jobs.bridge_generation,
                df_version: observed.jobs.df_version.clone(),
                dfhack_version: observed.jobs.dfhack_version.clone(),
                anchor: report.anchor,
                source_digest: report.source_digest,
                capture_digest: Digest32::of_bytes(&capture),
                capture_bytes: capture.len() as u32,
                next_job_id: observed.jobs.next_job_id,
                next_building_id: observed.next_building_id,
                next_item_id: observed.next_item_id,
            };
            let mut items = Vec::with_capacity(report.allocation.assignments.len());
            for assignment in &report.allocation.assignments {
                work.charge(1)?;
                let selected = report.items.get(&assignment.item_id).ok_or_else(|| invalid("allocated furniture evidence missing"))?;
                let index = observed.items.binary_search_by_key(&assignment.item_id, |item| item.native_id).map_err(|_| invalid("allocated furniture not in original source"))?;
                let native_type = u32::try_from(observed.items[index].item_type).map_err(|_| invalid("invalid selected native furniture type"))?;
                items.push(SelectedItem { slot: assignment.slot.clone(), candidate: selected.candidate, native_type, handle: selected.handle, distance: assignment.distance });
            }
            Some(Self::assemble(request.clone(), source, items)?)
        };
        work.charge(1)?;
        report.work_units = work.used;
        Ok(AllocationOutcome { report, handoff })
    }

    fn assemble(request: FurnitureRequest, source: Source, items: Vec<SelectedItem>) -> Result<Self> {
        require(source.endpoint.is_ipv4() && source.endpoint.ip().is_loopback() && source.endpoint.port() != 0, "invalid retained allocation endpoint")?;
        require(source.generation > 0 && source.generation < u64::MAX, "invalid retained operations generation")?;
        for text in [&source.df_version, &source.dfhack_version] {
            require(!text.is_empty() && text.len() <= 128 && !text.contains('\0'), "invalid retained operations software")?;
        }
        let fortress = crate::build_placement::FortressIdentity::new(request.folder(), request.site())?;
        require(source.anchor.fortress_id == fortress.fortress_id()
            && source.anchor.tick.get() <= crate::build_placement::MAX_NATIVE_TICK
            && source.anchor.state_hash != Digest32::ZERO && source.source_digest != Digest32::ZERO && source.capture_digest != Digest32::ZERO
            && source.capture_bytes > 0 && source.capture_bytes as usize <= OperationsProfile::PagedV1_4.maximum_bytes(), "invalid retained operations source identity")?;
        require([source.next_job_id, source.next_building_id, source.next_item_id].iter().all(|n| *n <= i32::MAX as u32), "invalid retained allocation ID horizon")?;
        require(items.len() == request.request().slots.len(), "handoff omitted original furniture slots")?;
        let revision = source.anchor.cursor.sequence.checked_add(1)
            .ok_or_else(|| invalid("retained operations cursor revision overflow"))?;
        let mut steps = Vec::with_capacity(items.len());
        let mut types = BTreeMap::new();
        let mut kinds = BTreeMap::new();
        for (slot, chosen) in request.request().slots.iter().zip(&items) {
            let item = chosen.candidate;
            require(chosen.slot == slot.name && item.kind == slot.kind && item.native_id < source.next_item_id
                && chosen.native_type <= i32::MAX as u32 && item.subtype >= -1 && item.material_type >= 0 && item.material_index >= -1
                && item.position.iter().all(|n| *n < 32768)
                && chosen.handle.entity_id == item_entity_id(item.native_id) && chosen.handle.generation > 0
                && chosen.handle.revision == revision
                && !request.request().excluded_items.contains(&item.native_id), "invalid retained selected furniture identity")?;
            let distance = item.position[0].abs_diff(slot.target[0]) + item.position[1].abs_diff(slot.target[1]);
            require(item.position[2] == slot.target[2] && distance == chosen.distance && distance <= slot.max_distance
                && slot.material.is_none_or(|m| m == (item.material_type, item.material_index))
                && slot.subtype.is_none_or(|s| s == item.subtype), "retained furniture assignment violates original constraints")?;
            if types.insert(chosen.native_type, item.kind).is_some_and(|k| k != item.kind)
                || kinds.insert(item.kind, chosen.native_type).is_some_and(|t| t != chosen.native_type)
            { return Err(invalid("retained furniture type catalog changed")); }
            steps.push(FurnitureStep { name: slot.name.clone(), selection: BuildSelection::new(build_kind(slot.kind), item.native_id, slot.target)?, after: slot.after.clone() });
        }
        let plan = FurniturePlan::normalize(steps)?;
        let mut out = Self { request, plan, source, items, canonical: Vec::new(), digest: Digest32::ZERO };
        out.canonical = out.encode();
        require(out.canonical.len() <= MAX_HANDOFF_BYTES, "retained furniture handoff exceeds 24 KiB")?;
        out.digest = digest(&out.canonical);
        Ok(out)
    }

    pub fn request(&self) -> &FurnitureRequest { &self.request }
    pub fn plan(&self) -> &FurniturePlan { &self.plan }
    pub fn source(&self) -> &Source { &self.source }
    pub fn items(&self) -> &[SelectedItem] { &self.items }
    pub fn canonical_bytes(&self) -> &[u8] { &self.canonical }
    pub fn digest(&self) -> Digest32 { self.digest }

    pub fn validate_binding(&self, binding: &BuildBinding) -> Result<()> {
        require(binding.endpoint() == self.source.endpoint && binding.fortress().folder() == self.request.folder()
            && binding.fortress().site() == self.request.site() && binding.df_version() == self.source.df_version
            && binding.dfhack_version() == self.source.dfhack_version, "placement source differs from original allocation")?;
        // Cross-profile generations are not comparable. The complete placement
        // binding is separately frozen by BatchDefinition and checked per child.
        self.plan.validate_binding(binding)
    }

    /// Enforce the durable request on a fresh pre-placement capture. Native
    /// eligibility, lease, review and dispatch permission remain other checks.
    pub fn validate_capture(&self, binding: &BuildBinding, capture: &BuildCapture) -> Result<()> {
        self.validate_binding(binding)?;
        require(binding.capture_matches(capture) && capture.tick() >= self.source.anchor.tick.get()
            && capture.next_building_id() >= self.source.next_building_id && capture.next_job_id() >= self.source.next_job_id,
            "placement source or native clock/ID horizon predates allocation")?;
        let index = self.plan.steps().iter().position(|step| step.selection == capture.selection()).ok_or_else(|| invalid("selection is outside original allocation"))?;
        let slot = &self.request.request().slots[index];
        let chosen = &self.items[index];
        let BuildItem::Visible(item) = capture.item() else { return Err(invalid("allocated item is not currently visible")); };
        require(item.kind() == Some(build_kind(slot.kind)) && item.native_type() == chosen.native_type
            && item.material() == chosen.candidate.material_type && item.material_index() == chosen.candidate.material_index
            && item.subtype() == chosen.candidate.subtype, "allocated furniture item attributes changed")?;
        let position = item.position();
        require(position[2] == slot.target[2]
            && position[0].abs_diff(slot.target[0]) + position[1].abs_diff(slot.target[1]) <= slot.max_distance,
            "allocated furniture moved outside original distance constraint")
    }
}

#[cfg(test)]
mod tests;
