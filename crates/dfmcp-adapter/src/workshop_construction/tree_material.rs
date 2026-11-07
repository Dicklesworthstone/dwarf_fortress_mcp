//! Bounded Tree/1.21 material discovery followed by independent Workshop/1.20
//! qualification. Neither a handle nor imported history grants build authority.
use super::*;
use crate::tree_harvest::{Progress, journal::TreeMaterialOrigin};
use dfmcp_core::{
    Capability, GameTick, MapCoord, MapCuboid, OperationContext, RiskTier, SessionId,
};
use std::time::{Duration, Instant};

/// One observed log, offered only under a complete retained discovery witness.
/// The native selector is for the embedding, never an MCP argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopMaterialOffer {
    handle: Digest32,
    selection: WorkshopSelection,
    position: [u32; 3],
    material: i32,
    material_index: i32,
}
impl WorkshopMaterialOffer {
    pub const fn handle(&self) -> Digest32 {
        self.handle
    }
    pub const fn selection(&self) -> WorkshopSelection {
        self.selection
    }
    pub const fn position(&self) -> [u32; 3] {
        self.position
    }
    pub const fn material(&self) -> i32 {
        self.material
    }
    pub const fn material_index(&self) -> i32 {
        self.material_index
    }
}

/// Fresh visible ground supply associated with one acknowledged original Tree
/// selector. It is not a Tree completion sample, causal output attribution,
/// reservation, native preparation, or a new canonical world observation.
/// The host retains source custody and independently pins each native family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopMaterialDiscovery {
    session: SessionId,
    origin: Digest32,
    pin: journal::WorkshopPin,
    witness: Digest32,
    progress: Progress,
    offers: Vec<WorkshopMaterialOffer>,
}

fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn stale(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(ErrorCode::BudgetExceeded, "Tree material discovery exceeds bounded request work")
}
fn authorize(
    source: &FortressIdentity,
    dimensions: [u32; 3],
    tick: u64,
    bytes: u64,
    context: &OperationContext,
) -> Result<OperationContext> {
    context.budget.validate()?;
    if context.cancellation_requested {
        return Err(error(ErrorCode::CancellationRequested, "Tree material discovery cancelled"));
    }
    if context.budget.max_bytes < bytes
        || context.budget.max_wall_millis == 0
        || context.budget.max_wall_millis > 60000
    {
        return Err(exhausted());
    }
    if context.anchor.fortress_id != source.fortress_id() {
        return Err(denied("material discovery belongs to another fortress"));
    }
    if tick < context.anchor.tick.get() {
        return Err(stale("material discovery predates the current authority frontier"));
    }
    let scope = MapCuboid::new(
        MapCoord::new(0, 0, 0),
        MapCoord::new(
            dimensions[0] as i32 - 1,
            dimensions[1] as i32 - 1,
            dimensions[2] as i32 - 1,
        ),
    )?;
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick);
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(scope))?;
    current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], Some(scope))?;
    Ok(current)
}
fn finish(start: Instant, context: &OperationContext) -> Result<()> {
    if start.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted());
    }
    Ok(())
}

impl WorkshopMaterialDiscovery {
    pub const MAX_OFFERS: usize = 64;

    /// Consume one authentic same-incarnation Tree progress read from the exact
    /// retained original. A failed or incomplete supply observation grants no
    /// offer. Later observations must also respect the importing host's highest
    /// consumed tick, sequence and item horizon, including rejected reads.
    pub fn new(
        origin: &TreeMaterialOrigin,
        progress: Progress,
        center: [u32; 3],
        context: &OperationContext,
    ) -> Result<Self> {
        let start = Instant::now();
        let current = authorize(
            &origin.binding().source,
            origin.binding().dimensions,
            progress.tick,
            64 * 1024,
            context,
        )?;
        if Progress::decode(progress.bytes(), &origin.original().plan)? != progress {
            return Err(stale("Tree supply fields differ from their canonical capture"));
        }
        if progress.generation != origin.binding().generation
            || progress.source != origin.binding().source
            || progress.dimensions != origin.binding().dimensions
            || progress.tick < origin.high_tick()
            || progress.sequence < origin.high_sequence()
            || progress.next_item < origin.high_item()
        {
            return Err(stale("Tree supply changed source or regressed below retained original history"));
        }
        require(
            progress.supply_observed() && progress.wood.len() <= Self::MAX_OFFERS,
            "fresh Tree read has no complete paused ground-WOOD supply offer",
        )?;
        let first = progress.wood.first().ok_or_else(invalid)?;
        let selected = WorkshopSelection::new(first.id, center)?;
        let [x, y, z] = selected.center();
        require(
            x + 1 < progress.dimensions[0]
                && y + 1 < progress.dimensions[1]
                && z < progress.dimensions[2],
            "material discovery workshop footprint exceeds the pinned map",
        )?;
        let mut identity = b"dfmcp-workshop-tree-material-discovery/1\0".to_vec();
        identity.extend_from_slice(origin.witness().as_bytes());
        identity.extend_from_slice(Digest32::of_bytes(progress.bytes()).as_bytes());
        identity.extend_from_slice(&context.session_id.get().to_be_bytes());
        put(&mut identity, center);
        let witness = Digest32::of_bytes(&identity);
        let offers = progress
            .wood
            .iter()
            .map(|wood| {
                let selection = WorkshopSelection::new(wood.id, center)?;
                let mut identity = b"dfmcp-workshop-tree-material-offer/1\0".to_vec();
                identity.extend_from_slice(witness.as_bytes());
                identity.extend_from_slice(&wood.id.to_be_bytes());
                Ok(WorkshopMaterialOffer {
                    handle: Digest32::of_bytes(&identity),
                    selection,
                    position: wood.position,
                    material: wood.material,
                    material_index: wood.material_index,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        finish(start, &current)?;
        Ok(Self {
            session: context.session_id,
            origin: origin.witness(),
            pin: origin.pin(),
            witness,
            progress,
            offers,
        })
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }
    pub const fn origin_witness(&self) -> Digest32 {
        self.origin
    }
    pub const fn tree_pin(&self) -> journal::WorkshopPin {
        self.pin
    }
    pub fn progress(&self) -> &Progress {
        &self.progress
    }
    pub fn offers(&self) -> &[WorkshopMaterialOffer] {
        &self.offers
    }

    /// Resolve only a handle in this exact retained bounded offer set.
    pub fn resolve(&self, handle: Digest32) -> Result<WorkshopSelection> {
        Ok(self.offer(handle)?.selection)
    }

    fn offer(&self, handle: Digest32) -> Result<&WorkshopMaterialOffer> {
        self.offers
            .iter()
            .find(|offer| offer.handle == handle)
            .ok_or_else(|| stale("material handle is absent from the retained Tree discovery"))
    }

    /// Independently inspect the selected log with Workshop/1.20 before its
    /// ordinary prepare/revalidate/commit owner is entered. The embedding must
    /// separately verify that connection's software and generation pins; Tree
    /// and Workshop generations are independent and are never equated here.
    /// The original Workshop Intent becomes the immutable selection latch.
    pub fn qualify(
        &self,
        handle: Digest32,
        capture: &WorkshopCapture,
        context: &OperationContext,
    ) -> Result<WorkshopSelection> {
        let start = Instant::now();
        let current = authorize(
            &self.progress.source,
            self.progress.dimensions,
            capture.tick(),
            16 * 1024,
            context,
        )?;
        if context.session_id != self.session {
            return Err(denied("material offer belongs to another discovery session"));
        }
        let offer = self.offer(handle)?;
        if capture.selection() != offer.selection
            || capture.fortress() != &self.progress.source
            || capture.dimensions() != self.progress.dimensions
            || capture.tick() != self.progress.tick
        {
            return Err(stale("Tree supply and selected workshop do not share one paused source frontier"));
        }
        let WorkshopItem::Visible(item) = capture.item() else {
            return Err(stale("selected Tree material is no longer independently visible"));
        };
        if item.position() != offer.position
            || item.material() != offer.material
            || item.material_index() != offer.material_index
            || !capture.eligible()
        {
            return Err(stale("selected Tree material or workshop eligibility changed before preparation"));
        }
        finish(start, &current)?;
        Ok(offer.selection)
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "tree_material_tests.rs"]
mod tests;
