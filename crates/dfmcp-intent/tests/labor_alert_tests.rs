#![forbid(unsafe_code)]

//! Integration tests for WP-PLN-03 Dynamic Labor Specialization & Civilian Alert FSM.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{EntityId, Result};
use dfmcp_intent::alert_fsm::{CivilianAlertFsm, ThreatLevel};
use dfmcp_intent::labor::{DwarfLaborProfile, LaborAllocator};

#[test]
fn test_labor_and_alert_orchestration() -> Result<()> {
    let allocator = LaborAllocator;
    let mut fsm = CivilianAlertFsm::new(EntityId::new(100), vec![EntityId::new(200)])?;

    let mut dwarf_skills = BTreeMap::new();
    dwarf_skills.insert("MINING".to_owned(), 15);

    let dwarf1 = DwarfLaborProfile {
        id: EntityId::new(1),
        skills: dwarf_skills,
        stress_level: 200,
        assigned_labors: BTreeSet::new(),
    };

    let actions = allocator.optimize_roster(&[dwarf1])?;
    assert_eq!(actions.len(), 1);

    // Threat escalation
    let alert_actions = fsm.transition_to(ThreatLevel::EmergencyLockdown, &[EntityId::new(1)])?;
    assert_eq!(alert_actions.len(), 2);
    fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
    assert_eq!(fsm.current_level(), ThreatLevel::EmergencyLockdown);

    Ok(())
}
