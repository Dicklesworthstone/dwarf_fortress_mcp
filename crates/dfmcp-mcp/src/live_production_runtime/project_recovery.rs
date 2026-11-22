//! Parent-held original and source-frontier checks before child custody opens.
//!
//! This is recovery metadata only. It restores no preparation, current sample,
//! worker choice, inventory proof, clock permission or replacement original.
use super::*;
use serde_json::Value;

const SCHEMA: &str = "dfmcp.project-production-recovery/1";
const MAX_REFERENCE_BYTES: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    schema: String,
    recovery: Option<RecoveryInput>,
    native_custody: NativeCustody,
    observation_tick_floor: u64,
    clock: Option<ClockPin>,
    inventory_clock: Option<ClockPin>,
    workshop_prerequisite: Option<WorkshopSource>,
    manager_prerequisite: Option<ManagerSource>,
    input_supply: Option<InputSource>,
    original_goal_proven: bool,
    preparation_restored: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCustody {
    workforce_journal_id: String,
    order_journal_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockPin {
    journal_id: String,
    head: String,
    authority_tick_floor: u64,
    high_source_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkshopSource {
    #[serde(deserialize_with = "workshop_checkpoint")]
    checkpoint: Option<WorkshopPrerequisiteCheckpoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagerSource {
    #[serde(deserialize_with = "manager_config::checkpoint")]
    checkpoint: Option<ManagerPrerequisiteCheckpoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputSource {
    #[serde(deserialize_with = "input_config::checkpoint")]
    checkpoint: Option<InputCheckpoint>,
}

fn retained(value: &Value) -> Result<Retained> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("project production recovery is not serializable"))?;
    if bytes.len() > MAX_REFERENCE_BYTES {
        return Err(invalid("project production recovery exceeds its compact bound"));
    }
    let retained: Retained = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("invalid closed project production recovery reference"))?;
    if retained.schema != SCHEMA
        || retained.original_goal_proven
        || retained.preparation_restored
        || retained.observation_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
    {
        return Err(denied("project recovery is historical custody, not restored authority or proof"));
    }
    Ok(retained)
}

fn same_digest(current: Digest32, prior: &str) -> Result<()> {
    if current == digest(prior)? {
        Ok(())
    } else {
        Err(denied("project recovery changed an acknowledged original identity"))
    }
}

fn same_optional_digest(current: Option<Digest32>, prior: Option<&str>) -> Result<()> {
    if current == prior.map(digest).transpose()? {
        Ok(())
    } else {
        Err(denied("project recovery changed an acknowledged original association"))
    }
}

fn monotone(current: &[u64], prior: &[u64], fenced: bool, prior_fenced: bool) -> Result<()> {
    if current.len() != prior.len()
        || current.iter().zip(prior).any(|(now, before)| now < before)
        || (prior_fenced && !fenced)
    {
        return Err(denied("project recovery regressed a source floor or cleared a sticky fence"));
    }
    Ok(())
}

fn workshop_floor_at_least(current: WorkshopRecoveryFloor, prior: WorkshopRecoveryFloor) -> Result<()> {
    monotone(
        &[current.tick, current.generation, u64::from(current.next_building_id)],
        &[prior.tick, prior.generation, u64::from(prior.next_building_id)],
        current.fenced,
        prior.fenced,
    )
}

fn manager_floor_at_least(current: ManagerRecoveryFloor, prior: ManagerRecoveryFloor) -> Result<()> {
    let counters = |floor: ManagerRecoveryFloor| [
        floor.tick, floor.generation, floor.sequence,
        u64::from(floor.next_event), u64::from(floor.history_count),
        u64::from(floor.group_next_position), u64::from(floor.group_next_assignment),
        u64::from(floor.civ_next_position), u64::from(floor.civ_next_assignment),
    ];
    monotone(&counters(current), &counters(prior), current.fenced, prior.fenced)
}

fn input_floor_at_least(current: InputFloor, prior: InputFloor) -> Result<()> {
    let counters = |floor: InputFloor| [
        floor.tick, floor.generation, u64::from(floor.next_job_id),
        u64::from(floor.next_building_id), u64::from(floor.next_item_id),
    ];
    monotone(&counters(current), &counters(prior), current.fenced, prior.fenced)
}

fn optional_floor<T: Copy>(
    current: Option<T>,
    prior: Option<T>,
    check: impl FnOnce(T, T) -> Result<()>,
) -> Result<()> {
    match (current, prior) {
        (Some(current), Some(prior)) => check(current, prior),
        (None, None) => Ok(()),
        _ => Err(denied("project recovery added or removed an original source owner")),
    }
}

fn original(current: Recovery, prior: &RecoveryInput, observation_tick_floor: u64) -> Result<()> {
    same_digest(current.seal.plan_digest, &prior.plan_digest)?;
    same_digest(current.seal.object_digest, &prior.object_digest)?;
    same_digest(current.identity_digest, &prior.identity_digest)?;
    same_digest(current.anchor.state_hash, &prior.anchor.state_hash)?;
    if prior.portfolio.is_some()
        || current.portfolio.is_some()
        || current.anchor.fortress_id.get().to_string() != prior.anchor.fortress_id
        || current.anchor.cursor.epoch != prior.anchor.epoch
        || current.anchor.cursor.sequence != prior.anchor.sequence
        || current.anchor.tick.get() != prior.anchor.tick
        || (prior.goal_history_present && !current.goal_history_present)
        || current.frontier.authority_tick_floor < prior.frontier.authority_tick_floor
        || current.frontier.authority_tick_floor < observation_tick_floor
    {
        return Err(denied("project recovery changed its original anchor, goal history or authority floor"));
    }
    // Exact parent head is the replay starting point. The actual coordinator
    // subsequently verifies that its durable file is a descendant of this pin.
    same_digest(current.frontier.journal_id, &prior.frontier.journal_id)?;
    same_digest(current.frontier.head, &prior.frontier.head)?;
    same_optional_digest(current.clock_journal_id, prior.clock_journal_id.as_deref())?;
    same_optional_digest(current.inventory_clock_journal_id, prior.inventory_clock_journal_id.as_deref())?;
    same_optional_digest(current.workshop_prerequisite_digest, prior.workshop_prerequisite_digest.as_deref())?;
    same_optional_digest(current.manager_prerequisite_digest, prior.manager_prerequisite_digest.as_deref())?;
    same_optional_digest(current.input_supply_digest, prior.input_supply_digest.as_deref())?;
    optional_floor(current.workshop_prerequisite_floor, prior.workshop_prerequisite_floor, workshop_floor_at_least)?;
    optional_floor(current.manager_prerequisite_floor, prior.manager_prerequisite_floor, manager_floor_at_least)?;
    optional_floor(current.input_supply_floor, prior.input_supply_floor, input_floor_at_least)
}

fn clock_pin(current: Option<ClockConfig>, prior: Option<&ClockPin>) -> Result<Option<ProjectClockRecovery>> {
    match (current, prior) {
        (Some(current), Some(prior)) => {
            let journal_id = digest(&prior.journal_id)?;
            let head = digest(&prior.head)?;
            if current.journal_id != Some(journal_id)
                || prior.authority_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            {
                return Err(denied("project recovery changed its clock custodian or source floor"));
            }
            Ok(Some(ProjectClockRecovery {
                journal_id,
                head,
                authority_tick_floor: prior.authority_tick_floor,
                high_source_sequence: prior.high_source_sequence,
            }))
        }
        (None, None) => Ok(None),
        _ => Err(denied("project recovery cannot hide or add a clock custodian")),
    }
}

impl Config {
    /// Validate all caller-selected recovery identities before Runtime::open
    /// can touch child files or native sources. Actual file ancestry is checked
    /// by each durable owner after it acquires its original exclusive custody.
    pub(crate) fn validate_project_recovery(&mut self, reference: &Value) -> Result<()> {
        self.current()?;
        let binding = self.project_binding
            .ok_or_else(|| denied("project recovery requires the verified Workshop handoff"))?;
        if self.initialize {
            return Err(denied("acknowledged project child custody cannot be initialized again"));
        }
        let prior = retained(reference)?;
        match (self.recovery, prior.recovery.as_ref()) {
            (Some(current), Some(prior_original)) => {
                original(current, prior_original, prior.observation_tick_floor)?;
            }
            (None, None) => {}
            _ => return Err(denied("project recovery cannot adopt or discard an unacknowledged original")),
        }
        let workforce_journal = digest(&prior.native_custody.workforce_journal_id)?;
        let order_journal = digest(&prior.native_custody.order_journal_id)?;
        // Native workforce configuration has an independently retained journal
        // identity. The order owner exposes its identity only after local open,
        // so retain that expectation privately for the actual-owner check.
        let raw = family_environment()?;
        let workforce: Value = serde_json::from_str(&raw[1])
            .map_err(|_| invalid("invalid retained workforce configuration"))?;
        let configured_workforce = workforce.get("journal_id").and_then(Value::as_str)
            .ok_or_else(|| denied("project recovery lost its workforce journal identity"))?;
        same_digest(workforce_journal, configured_workforce)?;
        let clock = clock_pin(self.clock, prior.clock.as_ref())?;
        let inventory_clock = clock_pin(self.inventory_clock, prior.inventory_clock.as_ref())?;
        let mut observation_tick_floor = prior.observation_tick_floor
            .max(self.recovery.map_or(0, |recovery| recovery.frontier.authority_tick_floor));
        let mut source_floor = clock.map_or(0, |pin| pin.authority_tick_floor)
            .max(inventory_clock.map_or(0, |pin| pin.authority_tick_floor));

        let workshop = prior.workshop_prerequisite.as_ref().and_then(|source| source.checkpoint);
        if self.workshop_prerequisite.is_some() != workshop.is_some() {
            return Err(denied("project recovery lost its Workshop source checkpoint"));
        }
        if let Some(previous) = workshop {
            if previous.origin_witness != binding.workshop_origin {
                return Err(denied("project recovery changed its verified Workshop donor"));
            }
            let current = if let Some(recovery) = self.recovery {
                recovery.workshop_prerequisite_floor
                    .ok_or_else(|| denied("project recovery lost Workshop rejection floors"))?
            } else {
                let checkpoint = self.workshop_prerequisite_checkpoint
                    .ok_or_else(|| denied("project recovery requires its latest pre-plan Workshop checkpoint"))?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan Workshop checkpoint changed its donor"));
                }
                checkpoint.floor
            };
            workshop_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }

        let manager = prior.manager_prerequisite.as_ref().and_then(|source| source.checkpoint);
        if self.manager_prerequisite.is_some() != manager.is_some() {
            return Err(denied("project recovery lost its Manager source checkpoint"));
        }
        if let Some(previous) = manager {
            let current = if let Some(recovery) = self.recovery {
                recovery.manager_prerequisite_floor
                    .ok_or_else(|| denied("project recovery lost Manager rejection floors"))?
            } else {
                let checkpoint = self.manager_prerequisite_checkpoint
                    .ok_or_else(|| denied("project recovery requires its latest pre-plan Manager checkpoint"))?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan Manager checkpoint changed its donor"));
                }
                checkpoint.floor
            };
            manager_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }

        let input = prior.input_supply.as_ref().and_then(|source| source.checkpoint);
        if self.input_supply.is_some() != input.is_some() {
            return Err(denied("project recovery lost its input-supply source checkpoint"));
        }
        if let Some(previous) = input {
            let current = if let Some(recovery) = self.recovery {
                // The planned input origin is its exact original binding; the
                // pre-plan origin is the independently pinned reserve policy.
                if Some(previous.origin_witness) != recovery.input_supply_digest {
                    return Err(denied("project recovery changed its original input binding"));
                }
                recovery.input_supply_floor
                    .ok_or_else(|| denied("project recovery lost input rejection floors"))?
            } else {
                let checkpoint = self.input_supply_checkpoint
                    .ok_or_else(|| denied("project recovery requires its latest pre-plan input checkpoint"))?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan input checkpoint changed its policy"));
                }
                checkpoint.floor
            };
            input_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }
        if prior.observation_tick_floor < source_floor {
            return Err(denied("project recovery observation floor precedes retained source evidence"));
        }
        self.current()?;
        self.project_recovery = Some(ProjectRecovery {
            workforce_journal,
            order_journal,
            observation_tick_floor,
            clock,
            inventory_clock,
            manager_origin: manager.map(|checkpoint| checkpoint.origin_witness),
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "project_recovery_tests.rs"]
mod tests;
