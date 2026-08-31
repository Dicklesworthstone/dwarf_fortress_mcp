#![forbid(unsafe_code)]

//! Dynamic Labor Specialization and Mental Health Optimization Engine.
//!
//! WP-PLN-03: Optimizes dwarf labor assignments based on skill rankings and stress levels,
//! relieving traumatized dwarves (>800 stress) and prioritizing master craftsdwarves.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};

use crate::action::Action;

/// Threshold above which dwarves receive mental health relief and are excused from heavy labor.
pub const HIGH_STRESS_THRESHOLD: u32 = 800;

/// Profile of a citizen dwarf for labor allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DwarfLaborProfile {
    pub id: EntityId,
    pub skills: BTreeMap<String, u32>, // e.g. "MINING" -> 15 (Legendary)
    pub stress_level: u32,             // 0 - 1000
    pub assigned_labors: BTreeSet<String>,
}

/// Dynamic Labor Specialization Engine.
#[derive(Clone, Debug, Default)]
pub struct LaborAllocator;

impl LaborAllocator {
    /// Optimize labor assignments for a roster of citizen dwarves.
    pub fn optimize_roster(&self, roster: &[DwarfLaborProfile]) -> Result<Vec<Action>> {
        let mut actions = Vec::new();
        let mut seen = BTreeSet::new();

        for dwarf in roster {
            if dwarf.id == EntityId::NIL || !seen.insert(dwarf.id) || dwarf.stress_level > 1_000 {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "labor roster contains a nil/duplicate ID or out-of-range stress value",
                ));
            }
            if dwarf
                .skills
                .iter()
                .any(|(skill, rating)| skill.trim().is_empty() || *rating > 20)
                || dwarf
                    .assigned_labors
                    .iter()
                    .any(|labor| labor.trim().is_empty())
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "labor profile contains an empty token or out-of-range skill rating",
                ));
            }
            // 1. High-stress mental health guard
            if dwarf.stress_level >= HIGH_STRESS_THRESHOLD {
                let heavy_labors = [
                    "MINING",
                    "MASONRY",
                    "WEAPONSMITHING",
                    "ARMORING",
                    "CARPENTRY",
                ];
                let to_disable: Vec<String> = dwarf
                    .assigned_labors
                    .iter()
                    .filter(|l| heavy_labors.contains(&l.as_str()))
                    .cloned()
                    .collect();

                for labor in to_disable {
                    actions.push(Action::SetLabor {
                        units: vec![dwarf.id],
                        labor,
                        enabled: false,
                    });
                }
                continue;
            }

            // 2. Skill specialization: assign top-tier skills
            for (skill, &rating) in &dwarf.skills {
                if rating >= 10 && !dwarf.assigned_labors.contains(skill) {
                    actions.push(Action::SetLabor {
                        units: vec![dwarf.id],
                        labor: skill.clone(),
                        enabled: true,
                    });
                }
            }
        }

        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_stress_dwarf_relieved_from_heavy_labor() -> Result<()> {
        let allocator = LaborAllocator;
        let dwarf = DwarfLaborProfile {
            id: EntityId::new(1),
            skills: BTreeMap::new(),
            stress_level: 850, // High stress
            assigned_labors: BTreeSet::from(["MINING".to_owned(), "HAULING".to_owned()]),
        };

        let actions = allocator.optimize_roster(&[dwarf])?;
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            Action::SetLabor {
                units,
                labor,
                enabled: false,
            } if units == &vec![EntityId::new(1)] && labor == "MINING"
        ));
        Ok(())
    }

    #[test]
    fn test_master_craftsdwarf_assigned_labor() -> Result<()> {
        let allocator = LaborAllocator;
        let mut skills = BTreeMap::new();
        skills.insert("WEAPONSMITHING".to_owned(), 15);

        let dwarf = DwarfLaborProfile {
            id: EntityId::new(2),
            skills,
            stress_level: 100, // Happy
            assigned_labors: BTreeSet::new(),
        };

        let actions = allocator.optimize_roster(&[dwarf])?;
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            Action::SetLabor {
                labor,
                enabled: true,
                ..
            } if labor == "WEAPONSMITHING"
        ));
        Ok(())
    }
}
