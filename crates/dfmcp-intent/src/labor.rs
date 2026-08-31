#![forbid(unsafe_code)]

//! Dynamic Labor Specialization and Mental Health Optimization Engine.
//!
//! WP-PLN-03: Optimizes dwarf labor assignments based on skill rankings and stress levels,
//! relieving traumatized dwarves (>800 stress) and prioritizing master craftsdwarves.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::EntityId;

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
    #[must_use]
    pub fn optimize_roster(&self, roster: &[DwarfLaborProfile]) -> Vec<Action> {
        let mut actions = Vec::new();

        for dwarf in roster {
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

        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_stress_dwarf_relieved_from_heavy_labor() {
        let allocator = LaborAllocator;
        let dwarf = DwarfLaborProfile {
            id: EntityId::new(1),
            skills: BTreeMap::new(),
            stress_level: 850, // High stress
            assigned_labors: BTreeSet::from(["MINING".to_owned(), "HAULING".to_owned()]),
        };

        let actions = allocator.optimize_roster(&[dwarf]);
        assert_eq!(actions.len(), 1);
        if let Action::SetLabor {
            units,
            labor,
            enabled,
        } = &actions[0]
        {
            assert_eq!(units, &vec![EntityId::new(1)]);
            assert_eq!(labor, "MINING");
            assert!(!enabled);
        } else {
            assert!(false, "unexpected action");
        }
    }

    #[test]
    fn test_master_craftsdwarf_assigned_labor() {
        let allocator = LaborAllocator;
        let mut skills = BTreeMap::new();
        skills.insert("WEAPONSMITHING".to_owned(), 15);

        let dwarf = DwarfLaborProfile {
            id: EntityId::new(2),
            skills,
            stress_level: 100, // Happy
            assigned_labors: BTreeSet::new(),
        };

        let actions = allocator.optimize_roster(&[dwarf]);
        assert_eq!(actions.len(), 1);
        if let Action::SetLabor { labor, enabled, .. } = &actions[0] {
            assert_eq!(labor, "WEAPONSMITHING");
            assert!(enabled);
        } else {
            assert!(false, "unexpected action");
        }
    }
}
