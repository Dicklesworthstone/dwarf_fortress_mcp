#![forbid(unsafe_code)]

mod action;
pub mod alert_fsm;
pub mod blueprint;
pub mod labor;
pub mod logistics;
pub mod obligation;
mod plan;
mod planner;

pub use action::{
    Action, ActionScope, BuildingKind, DigMode, MaterialSelector, WorkOrderCondition,
};
pub use alert_fsm::{CivilianAlertFsm, ThreatLevel};
pub use blueprint::{BlueprintPlanner, BlueprintTemplate, HazardAssessment};
pub use labor::{DwarfLaborProfile, HIGH_STRESS_THRESHOLD, LaborAllocator};
pub use logistics::{InventoryStockpile, ProductionLogisticsCompiler, ProductionRecipe};
pub use obligation::{
    BoundedObligation, DrainProgressCertificate, ObligationRuntime, ObligationStatus,
};
pub use plan::{Constraint, Intent, ObligationSpec, PlanStep, PreparedPlan, RequestedAction};
pub use planner::{PlanPolicy, StaticPlanner};
