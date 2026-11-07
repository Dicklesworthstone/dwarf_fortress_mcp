//! Exact original inventory-goal association for separately reviewed Run/1.13
//! stability advancement. A retired order is never reconstructed from inventory.
use super::bounded_goal_clock::{self, GoalClockTarget};
use super::{denied, invalid};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan};
use dfmcp_adapter::semantic_work_orders::{
    NativeOrderCreation, ObservedInventoryGoal, SemanticWorkOrderResult,
    SemanticWorkOrderReview,
};
use dfmcp_core::{Capability, Digest32, ErrorCode, FortressId, OperationContext, Result, RiskTier};
use dfmcp_intent::PreparedPlan;
use dfmcp_intent::plan_archive::PlanArchiveSeal;

pub use bounded_goal_clock::{ClockRunView, ClockView, MAX_RUNS};
pub type Clock<J> = bounded_goal_clock::Clock<J, ClockTarget>;

fn corrupt(message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn stale(message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(ErrorCode::StaleAnchor, message)
}
fn hash(domain: &[u8], fields: &[&[u8]]) -> Digest32 {
    let mut bytes = domain.to_vec();
    bytes.push(0);
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    Digest32::of_bytes(&bytes)
}

#[derive(Clone, Debug)]
pub struct RunRequest(bounded_goal_clock::RunRequest);
impl RunRequest {
    pub fn parse(raw: &str) -> Result<Self> {
        bounded_goal_clock::RunRequest::parse(raw, ClockTarget::REQUEST_KIND).map(Self)
    }
    pub(crate) fn inner(&self) -> &bounded_goal_clock::RunRequest {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ClockTarget {
    digest: Digest32,
    original_seal: Digest32,
    fortress: FortressId,
    epoch: u64,
    original_tick: u64,
    deadline: u64,
}
impl ClockTarget {
    pub fn new(
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
        creation_journal: Digest32,
        review: &SemanticWorkOrderReview,
        creation: &NativeOrderCreation,
    ) -> Result<Self> {
        if ObservedInventoryGoal::from_plan(plan).is_none() {
            return Err(denied(
                "inventory clock requires the exact original inventory objective",
            ));
        }
        if plan != review.original_plan()
            || !plan.digest_is_valid()
            || !plan.id_is_valid()
            || archive.plan_digest != plan.digest
            || archive.object_digest == Digest32::ZERO
            || creation_journal == Digest32::ZERO
        {
            return Err(corrupt(
                "inventory clock target differs from original plan custody",
            ));
        }
        let NativeOrderCreation::Inserted {
            native_order_id,
            receipt,
        } = creation
        else {
            return Err(denied(
                "inventory clock requires the original verified Created receipt",
            ));
        };
        let before = review.native_plan().observation();
        if *receipt == Digest32::ZERO
            || *native_order_id > i32::MAX as u32
            || *native_order_id != before.next_order_id()
        {
            return Err(corrupt("inventory clock insertion identity is invalid"));
        }
        let fortress =
            dfmcp_adapter::workforce_control::fortress_id(before.world_folder(), before.site_id());
        if fortress == FortressId::NIL || fortress != plan.anchor.fortress_id {
            return Err(corrupt(
                "inventory clock original belongs to another fortress",
            ));
        }
        let deadline = plan
            .steps
            .first()
            .and_then(|step| step.obligation.as_ref())
            .ok_or_else(|| invalid("original inventory obligation is absent"))?
            .deadline_tick
            .get();
        if deadline <= plan.anchor.tick.get() {
            return Err(corrupt(
                "inventory clock deadline precedes its original observation",
            ));
        }
        let digest = hash(
            b"dfmcp-semantic-inventory-clock-target/1",
            &[
                plan.digest.as_bytes(),
                archive.object_digest.as_bytes(),
                creation_journal.as_bytes(),
                review.seal().as_bytes(),
                receipt.as_bytes(),
                &native_order_id.to_be_bytes(),
            ],
        );
        Ok(Self {
            digest,
            original_seal: review.seal(),
            fortress,
            epoch: plan.anchor.cursor.epoch,
            original_tick: plan.anchor.tick.get(),
            deadline,
        })
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }

    /// Bind inventory stability to the exact selected order in its complete
    /// original parent. The verified Created receipt remains historical custody
    /// after normal queue retirement; it does not establish current inventory.
    /// The combined owner must independently recheck all predecessor custody
    /// and fresh qualified goal evidence at every native clock effect boundary.
    pub fn for_selected_original(
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
        creation_journal: Digest32,
        result: &SemanticWorkOrderResult,
    ) -> Result<Self> {
        let custody = super::clock::ClockTarget::for_selected_original(
            plan,
            archive,
            creation_journal,
            result,
        )?;
        let review = result.review();
        if ObservedInventoryGoal::from_selected_plan(plan, review.step_id()).is_none() {
            return Err(denied(
                "selected inventory clock requires the exact original inventory objective",
            ));
        }
        let deadline = review
            .selected_step()
            .obligation
            .as_ref()
            .ok_or_else(|| corrupt("selected inventory clock obligation is absent"))?
            .deadline_tick
            .get();
        Ok(Self {
            digest: hash(
                b"dfmcp-combined-inventory-clock-target/1",
                &[custody.digest().as_bytes()],
            ),
            original_seal: review.seal(),
            fortress: plan.anchor.fortress_id,
            epoch: plan.anchor.cursor.epoch,
            original_tick: plan.anchor.tick.get(),
            deadline,
        })
    }
}
impl GoalClockTarget for ClockTarget {
    const REQUEST_KIND: &'static str = "inventory_run";
    const CANDIDATE_KEY: &'static str = "inventory-clock-candidate";
    const PURPOSE: &'static str = "advance_original_inventory_stability";
    fn digest(&self) -> Digest32 {
        self.digest
    }
    fn original_paired_seal(&self) -> Digest32 {
        self.original_seal
    }
    fn original_tick(&self) -> u64 {
        self.original_tick
    }
    fn context(&self, c: &OperationContext) -> Result<()> {
        if c.anchor.fortress_id != self.fortress
            || c.anchor.cursor.epoch != self.epoch
            || c.anchor.tick.get() < self.original_tick
        {
            return Err(stale(
                "inventory clock cannot cross original fortress, epoch or tick floor",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn capture(&self, capture: &RunObservation) -> Result<u64> {
        let tick = capture
            .tick()
            .filter(|tick| *tick >= self.original_tick)
            .ok_or_else(|| stale("inventory clock source lacks the original tick floor"))?;
        if !capture.eligible() {
            return Err(stale("inventory clock requires a loaded paused source"));
        }
        Ok(tick)
    }
    fn plan(&self, plan: &RunPlan) -> Result<()> {
        let tick = self.capture(plan.before())?;
        if tick
            .checked_add(u64::from(plan.spec().game_ticks()))
            .is_none_or(|end| end > self.deadline)
            || plan.key() != self.key(plan.digest())
        {
            return Err(corrupt(
                "inventory clock history changes original target or deadline",
            ));
        }
        Ok(())
    }
    fn key(&self, plan: Digest32) -> String {
        let digest = hash(
            b"dfmcp-semantic-inventory-clock-key/1",
            &[self.digest.as_bytes(), plan.as_bytes()],
        );
        format!("inventory-clock-{}", digest.to_hex())
    }
    fn seal(&self, journal: Digest32, plan: &RunPlan) -> Digest32 {
        hash(
            b"dfmcp-semantic-inventory-clock-review/1",
            &[
                self.digest.as_bytes(),
                journal.as_bytes(),
                &plan.canonical_bytes(),
            ],
        )
    }
}

#[cfg(test)]
#[path = "inventory_clock_target_tests.rs"]
mod tests;
