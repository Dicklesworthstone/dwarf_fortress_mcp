#![forbid(unsafe_code)]

mod action;
pub mod alert_fsm;
pub mod blueprint;
pub mod civilian_protection;
pub mod effect_quiescence;
pub mod effects;
pub mod execution;
pub mod labor;
pub mod labor_execution;
pub mod labor_observation;
pub mod logistics;
pub mod obligation;
mod plan;
pub mod plan_archive;
mod planner;

pub use action::{
    Action, ActionScope, BuildingKind, DigMode, MaterialSelector, WorkOrderCondition,
};
pub use alert_fsm::{
    AlertDecision, AlertObservation, AlertRecoveryPolicy, CivilianAlertFsm, ThreatAssessment,
    ThreatLevel,
};
pub use blueprint::{BlueprintPlanner, BlueprintTemplate, HazardAssessment};
pub use civilian_protection::{CivilianProtectionPlan, ProtectionProgress};
pub use effect_quiescence::{EffectWorkState, inspect_effect_work};
pub use labor::{
    DwarfLaborProfile, HIGH_STRESS_THRESHOLD, LaborAllocation, LaborAllocationLimits,
    LaborAllocator, LaborDemand, LaborStaffing,
};
pub use labor_execution::LaborExecutionPlan;
pub use logistics::{
    InventoryStockpile, ProductionLogisticsCompiler, ProductionPlan, ProductionPlanningLimits,
    ProductionQuota, ProductionRecipe, ProductionRequirement, ProductionShortage, ProductionStep,
};
pub use obligation::{
    BoundedObligation, DrainProgressCertificate, ObligationRuntime, ObligationStatus,
    RecoveredObligation,
};
pub use plan::derive_step_idempotency_key;
pub use plan::{Constraint, Intent, ObligationSpec, PlanStep, PreparedPlan, RequestedAction};
pub use planner::{PlanPolicy, StaticPlanner};
