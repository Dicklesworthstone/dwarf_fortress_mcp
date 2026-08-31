#![forbid(unsafe_code)]

//! Civilian Alert and Burrow Evacuation Finite State Machine.
//!
//! WP-PLN-03: Automates fortress defense lockdowns during sieges or forgotten beast attacks,
//! confining civilians to subterranean burrows and mobilizing military squads.

use dfmcp_core::{EntityId, Result};

use crate::action::Action;

/// Current fortress defensive readiness threat level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreatLevel {
    Peace,
    Cautious,
    EmergencyLockdown,
    Recovering,
}

/// Fortress Civilian Alert and Burrow Orchestrator.
#[derive(Clone, Debug)]
pub struct CivilianAlertFsm {
    current_level: ThreatLevel,
    safe_burrow_id: EntityId,
    military_squad_ids: Vec<EntityId>,
}

impl CivilianAlertFsm {
    #[must_use]
    pub fn new(safe_burrow_id: EntityId, military_squad_ids: Vec<EntityId>) -> Self {
        Self {
            current_level: ThreatLevel::Peace,
            safe_burrow_id,
            military_squad_ids,
        }
    }

    /// Current alert level.
    #[must_use]
    pub const fn current_level(&self) -> ThreatLevel {
        self.current_level
    }

    /// Safe burrow entity ID.
    #[must_use]
    pub const fn safe_burrow_id(&self) -> EntityId {
        self.safe_burrow_id
    }

    /// Military squad entity IDs.
    #[must_use]
    pub fn military_squad_ids(&self) -> &[EntityId] {
        &self.military_squad_ids
    }

    /// Transition to a new threat level and emit required containment / mobilization actions.
    pub fn transition_to(
        &mut self,
        new_level: ThreatLevel,
        civilian_ids: &[EntityId],
    ) -> Result<Vec<Action>> {
        if self.current_level == new_level {
            return Ok(Vec::new()); // No transition needed
        }

        let mut actions = Vec::new();

        match (self.current_level, new_level) {
            (_, ThreatLevel::EmergencyLockdown) => {
                // 1. Confine all civilians to safe burrow
                actions.push(Action::SetBurrowMembership {
                    units: civilian_ids.to_vec(),
                    burrow: self.safe_burrow_id,
                    assigned: true,
                });

                // 2. Set defensive standing orders
                actions.push(Action::SetStandingOrder {
                    key: "CIVILIAN_BURROW_RESTRICTION".to_owned(),
                    value: "ACTIVE".to_owned(),
                });
            }
            (ThreatLevel::EmergencyLockdown, ThreatLevel::Recovering)
            | (ThreatLevel::EmergencyLockdown, ThreatLevel::Peace)
            | (ThreatLevel::Recovering, ThreatLevel::Peace) => {
                // Release civilians from emergency burrow
                actions.push(Action::SetBurrowMembership {
                    units: civilian_ids.to_vec(),
                    burrow: self.safe_burrow_id,
                    assigned: false,
                });

                actions.push(Action::SetStandingOrder {
                    key: "CIVILIAN_BURROW_RESTRICTION".to_owned(),
                    value: "INACTIVE".to_owned(),
                });
            }
            _ => {}
        }

        self.current_level = new_level;
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_lockdown_and_recovery_cycle() -> Result<()> {
        let mut fsm = CivilianAlertFsm::new(EntityId::new(10), vec![EntityId::new(20)]);
        let civilians = vec![EntityId::new(1), EntityId::new(2)];

        // Transition: Peace -> EmergencyLockdown
        let actions_lockdown = fsm.transition_to(ThreatLevel::EmergencyLockdown, &civilians)?;
        assert_eq!(actions_lockdown.len(), 2);
        assert_eq!(fsm.current_level(), ThreatLevel::EmergencyLockdown);

        // Transition: EmergencyLockdown -> Peace
        let actions_recovery = fsm.transition_to(ThreatLevel::Peace, &civilians)?;
        assert_eq!(actions_recovery.len(), 2);
        assert_eq!(fsm.current_level(), ThreatLevel::Peace);

        Ok(())
    }
}
