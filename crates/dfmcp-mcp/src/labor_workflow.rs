//! Versioned request decoding only; allocation and execution semantics live in
//! dfmcp-intent. Model skills and stress never become canonical observations.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_intent::{
    DwarfLaborProfile, LaborAllocationLimits, LaborAllocator, LaborDemand, RequestedAction,
};
use serde::{Deserialize, Deserializer, de::{MapAccess, Visitor}};

const MAX_ROSTER: usize = 64;
const MAX_DEMANDS: usize = 16;
const MAX_PROFILE_TOKENS: usize = 256;
const MAX_COMPILED_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(tag = "workflow", deny_unknown_fields)]
enum Workflow {
    #[serde(rename = "labor_allocation_v1")]
    LaborAllocation {
        roster: Vec<Profile>,
        demands: Vec<Demand>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    id: String,
    #[serde(deserialize_with = "unique_skills")]
    skills: BTreeMap<String, u32>,
    stress_level: u32,
    assigned_labors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Demand {
    labor: String,
    skill: String,
    workers: u32,
    minimum_skill: u32,
    priority: u8,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn unique_skills<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<BTreeMap<String, u32>, D::Error> {
    struct Skills;
    impl<'de> Visitor<'de> for Skills {
        type Value = BTreeMap<String, u32>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("at most 256 distinct skill keys and unsigned ratings")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut skills = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, u32>()? {
                if skills.len() == MAX_PROFILE_TOKENS || skills.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate skill or too many skill entries"));
                }
            }
            Ok(skills)
        }
    }
    deserializer.deserialize_map(Skills)
}

fn entity(raw: &str) -> Result<EntityId> {
    if raw.is_empty() || raw.len() > 20 || raw.starts_with('0') || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("workforce IDs must be nonzero canonical decimal u64 strings"));
    }
    raw.parse::<u64>().map(EntityId::new).map_err(|_| invalid("workforce ID exceeds u64"))
}

pub(super) fn parse(raw: &str) -> Result<Vec<RequestedAction>> {
    if raw.len() > super::MAX_ACTIONS_JSON_BYTES {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "labor workflow exceeds the action request byte limit"));
    }
    let Workflow::LaborAllocation { roster, demands } = serde_json::from_str(raw)
        .map_err(|e| invalid(format!("invalid labor_allocation_v1 request: {e}")))?;
    if roster.is_empty() || roster.len() > MAX_ROSTER || demands.is_empty() || demands.len() > MAX_DEMANDS {
        return Err(invalid("labor workflow requires 1..64 citizens and 1..16 demands"));
    }
    let mut rows = Vec::with_capacity(roster.len());
    for row in roster {
        if row.assigned_labors.len() > MAX_PROFILE_TOKENS {
            return Err(invalid("too many declared labor assignments"));
        }
        let count = row.assigned_labors.len();
        let assigned_labors: BTreeSet<_> = row.assigned_labors.into_iter().collect();
        if assigned_labors.len() != count {
            return Err(invalid("duplicate declared labor assignment"));
        }
        rows.push(DwarfLaborProfile {
            id: entity(&row.id)?, skills: row.skills, stress_level: row.stress_level, assigned_labors,
        });
    }
    let roles: Vec<_> = demands.into_iter().map(|d| LaborDemand {
        labor: d.labor, skill: d.skill, workers: d.workers,
        minimum_skill: d.minimum_skill, priority: d.priority,
    }).collect();
    let limits = LaborAllocationLimits {
        max_roster: MAX_ROSTER, max_demands: MAX_DEMANDS, max_positions: MAX_ROSTER,
        max_actions: super::MAX_LAB_STEPS, ..LaborAllocationLimits::default()
    };
    let compiled = LaborAllocator.compile_execution(&rows, &roles, limits)?;
    if compiled.encoded_bytes() > MAX_COMPILED_BYTES {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "labor workflow expands beyond its compiled byte bound"));
    }
    compiled.into_requested_actions()
}
