//! Immutable association between a complete original production plan and one
//! verified selected order. This is historical custody, never current evidence
//! or source/clock authority. Embeddings must still recheck all predecessors and
//! original custody at every native effect boundary.
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, GameTick, Result, StateAnchor, StepId};
use dfmcp_intent::PreparedPlan;
use dfmcp_intent::plan_archive::PlanArchiveSeal;

use crate::order_run::FortressIdentity;
use crate::semantic_work_orders::{NativeOrderCreation, SemanticWorkOrderResult};
use crate::work_order_control::CreationState;
use crate::work_orders::WorkOrderPlan;

use super::{ProductionLayout, validate_plan};

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn denied(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CapabilityDenied, message)
}

/// Authority-free original association shared by independently owned clocks.
/// It cannot renew a deadline, qualify a source or restore a dispatch permit.
#[derive(Clone, Debug)]
pub struct SelectedOrderClockCustody {
    identity_bytes: Vec<u8>,
    original_anchor: StateAnchor,
    step_id: StepId,
    review_seal: Digest32,
    native_plan: WorkOrderPlan,
    native_order_id: u32,
    deadline_tick: GameTick,
}

impl SelectedOrderClockCustody {
    pub fn new(
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
        creation_journal: Digest32,
        result: &SemanticWorkOrderResult,
    ) -> Result<Self> {
        let review = result.review();
        let selected = review.selected_step();
        let independent_order = selected.depends_on.is_empty();
        if validate_plan(plan)? != archive
            || plan != review.original_plan()
            || creation_journal == Digest32::ZERO
            || review.seal() == Digest32::ZERO
            || plan.steps.iter().find(|step| step.id == review.step_id()) != Some(selected)
            || (plan.steps.len() < 2 && !independent_order)
            || result.native_record().state() != CreationState::Created
            || result.native_record().plan() != review.native_plan()
            || selected.idempotency_key != review.native_plan().key()
        {
            return Err(corrupt(
                "selected clock target differs from its complete original parent or verified custody",
            ));
        }
        if independent_order {
            let layout = ProductionLayout::from_plan(plan)?;
            if layout.existing_labor().is_none() || layout.order_ordinal(selected.id).is_none() {
                return Err(corrupt(
                    "dependency-free selected clock requires the exact existing-worker production layout",
                ));
            }
        }
        let NativeOrderCreation::Inserted {
            native_order_id,
            receipt,
        } = result.creation_result()
        else {
            return Err(denied(
                "selected clock requires the original verified Created receipt",
            ));
        };
        let native = review.native_plan();
        let before = native.observation();
        let effect = result.native_record().effect();
        if *receipt == Digest32::ZERO
            || *native_order_id > i32::MAX as u32
            || *native_order_id != before.next_order_id()
            || effect.created_order_id() != Some(*native_order_id)
            || effect.receipt() != Some(*receipt)
        {
            return Err(corrupt(
                "selected clock insertion differs from its original allocation and receipt",
            ));
        }
        let fortress = FortressIdentity::new(before.world_folder(), before.site_id())?;
        if fortress.fortress_id() != plan.anchor.fortress_id {
            return Err(corrupt("selected clock belongs to another fortress"));
        }
        let deadline = selected
            .obligation
            .as_ref()
            .ok_or_else(|| denied("selected clock requires the original order obligation"))?
            .deadline_tick
            .get();
        if deadline <= plan.anchor.tick.get() {
            return Err(corrupt(
                "selected clock deadline precedes its original observation",
            ));
        }
        let step_bytes = review.step_id().get().to_be_bytes();
        let order_bytes = native_order_id.to_be_bytes();
        let review_seal = review.seal();
        let fields: [&[u8]; 8] = [
            plan.digest.as_bytes(),
            archive.object_digest.as_bytes(),
            creation_journal.as_bytes(),
            &step_bytes,
            selected.idempotency_key.as_bytes(),
            review_seal.as_bytes(),
            receipt.as_bytes(),
            &order_bytes,
        ];
        let mut identity_bytes = Vec::new();
        for field in fields {
            identity_bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
            identity_bytes.extend_from_slice(field);
        }
        Ok(Self {
            identity_bytes,
            original_anchor: plan.anchor,
            step_id: review.step_id(),
            review_seal,
            native_plan: native.clone(),
            native_order_id: *native_order_id,
            deadline_tick: GameTick(deadline),
        })
    }

    /// The established length-prefixed fields, without a clock-family domain:
    /// parent plan, archive object, creation journal, StepId, key, paired review,
    /// Created receipt and allocated order ID. Consumers supply their own domain.
    pub fn identity_bytes(&self) -> &[u8] {
        &self.identity_bytes
    }
    pub const fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }
    pub const fn step_id(&self) -> StepId {
        self.step_id
    }
    pub const fn review_seal(&self) -> Digest32 {
        self.review_seal
    }
    pub fn native_plan(&self) -> &WorkOrderPlan {
        &self.native_plan
    }
    pub const fn native_order_id(&self) -> u32 {
        self.native_order_id
    }
    pub const fn deadline_tick(&self) -> GameTick {
        self.deadline_tick
    }
}
