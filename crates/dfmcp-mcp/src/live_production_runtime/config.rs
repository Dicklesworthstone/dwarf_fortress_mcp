//! Fixed operator configuration for the combined, unadmitted development owner.
use super::input_config::{self, InputCheckpoint, InputConfig, InputFloor};
use super::manager_config::{
    self, ManagerConfig, ManagerPrerequisiteCheckpoint, ManagerRecoveryFloor,
};
use super::workshop_config::{self, WorkshopConfig};
use crate::manager_appointment_server::{ManagerProjectHandoff, ProjectManagerConfig};
use crate::semantic_workforce_runtime::{denied, digest, invalid};
use crate::workshop_construction_server::WorkshopProjectHandoff;
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::live_dispatch::durable;
use dfmcp_adapter::workshop_construction::WorkshopBinding;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor, Result, StateAnchor};
use dfmcp_intent::plan_archive::PlanArchiveSeal;
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION";
const CONFIG: &str = "DFMCP_COMBINED_PRODUCTION_CONFIG";
const WORKFORCE_CONFIG: &str = "DFMCP_SEMANTIC_WORKFORCE_CONFIG";
const ORDER_CONFIG: &str = "DFMCP_SEMANTIC_WORK_ORDER_CONFIG";
const ALLOWED: &[&str] = &[
    OPT_IN,
    CONFIG,
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE",
    WORKFORCE_CONFIG,
    "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR",
    "DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN",
    "DFMCP_SEMANTIC_WORKFORCE_TOKEN",
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS",
    ORDER_CONFIG,
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
    "DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
    "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
    "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
];
const MAX_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Recovery {
    pub(crate) seal: PlanArchiveSeal,
    pub(crate) anchor: StateAnchor,
    pub(crate) identity_digest: Digest32,
    pub(crate) frontier: durable::Recovery,
    pub(crate) goal_history_present: bool,
    pub(crate) clock_journal_id: Option<Digest32>,
    pub(crate) inventory_clock_journal_id: Option<Digest32>,
    pub(crate) portfolio: Option<[Option<PortfolioRecovery>; 4]>,
    pub(crate) workshop_prerequisite_digest: Option<Digest32>,
    pub(super) workshop_prerequisite_floor: Option<WorkshopRecoveryFloor>,
    pub(crate) manager_prerequisite_digest: Option<Digest32>,
    pub(super) manager_prerequisite_floor: Option<ManagerRecoveryFloor>,
    pub(crate) input_supply_digest: Option<Digest32>,
    pub(super) input_supply_floor: Option<InputFloor>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkshopRecoveryFloor {
    pub(super) tick: u64,
    pub(super) generation: u64,
    pub(super) next_building_id: u32,
    pub(super) fenced: bool,
}

impl WorkshopRecoveryFloor {
    fn validate(self, original_generation: u64, authority_tick_floor: u64) -> Result<()> {
        if self.tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || self.generation < original_generation
            || self.generation == u64::MAX
            || (!self.fenced && self.generation != original_generation)
            || self.next_building_id > i32::MAX as u32
            || authority_tick_floor < self.tick
        {
            return Err(denied(
                "workshop recovery must retain legal consumed source floors and sticky continuity loss",
            ));
        }
        Ok(())
    }
}

fn workshop_floor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<WorkshopRecoveryFloor>, D::Error> {
    WorkshopRecoveryFloor::deserialize(deserializer).map(Some)
}

/// Externally retained pre-plan rejection evidence. Its witness must match the
/// newly verified donor before the runtime consumes these monotone floors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WorkshopPrerequisiteCheckpoint {
    pub(super) origin_witness: Digest32,
    pub(super) floor: WorkshopRecoveryFloor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkshopCheckpointInput {
    origin_witness: String,
    tick: u64,
    generation: u64,
    next_building_id: u32,
    fenced: bool,
}

fn workshop_checkpoint<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<WorkshopPrerequisiteCheckpoint>, D::Error> {
    let input = WorkshopCheckpointInput::deserialize(deserializer)?;
    let origin_witness = digest(&input.origin_witness).map_err(serde::de::Error::custom)?;
    Ok(Some(WorkshopPrerequisiteCheckpoint {
        origin_witness,
        floor: WorkshopRecoveryFloor {
            tick: input.tick,
            generation: input.generation,
            next_building_id: input.next_building_id,
            fenced: input.fenced,
        },
    }))
}

#[derive(Clone, Copy)]
pub(crate) struct PortfolioRecovery {
    pub(crate) step_id: u32,
    pub(crate) goal_history_present: bool,
    pub(crate) clock_journal_id: Option<Digest32>,
    pub(crate) inventory_clock_journal_id: Option<Digest32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClockConfig {
    pub(crate) generation: u64,
    pub(crate) journal_id: Option<Digest32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockInput {
    generation: u64,
    #[serde(default, deserialize_with = "journal_identity")]
    journal_id: Option<String>,
}
fn journal_identity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
impl ClockInput {
    fn validate(self, initialize: bool) -> Result<ClockConfig> {
        if self.generation == 0
            || self.generation == u64::MAX
            || initialize == self.journal_id.is_some()
        {
            return Err(invalid(
                "combined clock requires an exact incarnation and a retained journal identity only when reopening",
            ));
        }
        Ok(ClockConfig {
            generation: self.generation,
            journal_id: self.journal_id.as_deref().map(digest).transpose()?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    directory: PathBuf,
    initialize: bool,
    recovery: Option<RecoveryInput>,
    clock: Option<ClockInput>,
    inventory_clock: Option<ClockInput>,
    workshop_prerequisite: Option<workshop_config::Input>,
    #[serde(default, deserialize_with = "workshop_checkpoint")]
    workshop_prerequisite_checkpoint: Option<WorkshopPrerequisiteCheckpoint>,
    manager_prerequisite: Option<manager_config::Input>,
    #[serde(default, deserialize_with = "manager_config::checkpoint")]
    manager_prerequisite_checkpoint: Option<ManagerPrerequisiteCheckpoint>,
    #[serde(default, deserialize_with = "input_config::configuration")]
    input_supply: Option<InputConfig>,
    #[serde(default, deserialize_with = "input_config::checkpoint")]
    input_supply_checkpoint: Option<InputCheckpoint>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    plan_digest: String,
    object_digest: String,
    identity_digest: String,
    anchor: AnchorInput,
    frontier: FrontierInput,
    #[serde(default)]
    goal_history_present: bool,
    clock_journal_id: Option<String>,
    inventory_clock_journal_id: Option<String>,
    portfolio: Option<Vec<PortfolioRecoveryInput>>,
    #[serde(default, deserialize_with = "journal_identity")]
    workshop_prerequisite_digest: Option<String>,
    #[serde(default, deserialize_with = "workshop_floor")]
    workshop_prerequisite_floor: Option<WorkshopRecoveryFloor>,
    #[serde(default, deserialize_with = "journal_identity")]
    manager_prerequisite_digest: Option<String>,
    #[serde(default, deserialize_with = "manager_config::recovery_floor")]
    manager_prerequisite_floor: Option<ManagerRecoveryFloor>,
    #[serde(default, deserialize_with = "journal_identity")]
    input_supply_digest: Option<String>,
    #[serde(default, deserialize_with = "input_config::recovery_floor")]
    input_supply_floor: Option<InputFloor>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioRecoveryInput {
    step_id: u32,
    goal_history_present: bool,
    clock_journal_id: Option<String>,
    inventory_clock_journal_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorInput {
    fortress_id: String,
    epoch: u64,
    sequence: u64,
    tick: u64,
    state_hash: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontierInput {
    journal_id: String,
    head: String,
    authority_tick_floor: u64,
}

pub(crate) struct Config {
    pub(crate) directory: PathBuf,
    pub(crate) initialize: bool,
    pub(crate) recovery: Option<Recovery>,
    pub(crate) clock: Option<ClockConfig>,
    pub(crate) inventory_clock: Option<ClockConfig>,
    pub(super) workshop_prerequisite: Option<WorkshopConfig>,
    pub(super) workshop_prerequisite_checkpoint: Option<WorkshopPrerequisiteCheckpoint>,
    pub(super) manager_prerequisite: Option<ManagerConfig>,
    pub(super) manager_prerequisite_checkpoint: Option<ManagerPrerequisiteCheckpoint>,
    pub(super) input_supply: Option<InputConfig>,
    pub(super) input_supply_checkpoint: Option<InputCheckpoint>,
    pub(crate) workforce: workforce::native::NativeShell,
    pub(crate) orders: orders::native::NativeShell,
    pub(super) project_binding: Option<ProjectProductionBinding>,
    pub(super) project_manager_handoff: Option<ManagerProjectHandoff>,
    pub(super) project_recovery: Option<ProjectRecovery>,
    check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    runtime_check: fn() -> Result<()>,
    parent_configuration_digest: Option<Digest32>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectProductionBinding {
    pub(super) workshop_origin: Digest32,
    pub(super) deadline_tick: u64,
    pub(super) authority_expires_tick: u64,
}

/// A parent-retained historical pin. It never restores a review or source read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProjectClockRecovery {
    pub(super) journal_id: Digest32,
    pub(super) head: Digest32,
    pub(super) authority_tick_floor: u64,
    pub(super) high_source_sequence: u64,
}

impl ProjectClockRecovery {
    pub(super) fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "journal_id": self.journal_id.to_hex(),
            "head": self.head.to_hex(),
            "authority_tick_floor": self.authority_tick_floor,
            "high_source_sequence": self.high_source_sequence,
        })
    }
}

/// Independent historical custody for each immutable original order slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProjectPortfolioClockRecovery {
    pub(super) step_id: u32,
    pub(super) clock: Option<ProjectClockRecovery>,
    pub(super) inventory_clock: Option<ProjectClockRecovery>,
}

impl ProjectPortfolioClockRecovery {
    pub(super) fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "step_id": self.step_id,
            "clock": self.clock.map(ProjectClockRecovery::to_json),
            "inventory_clock": self.inventory_clock.map(ProjectClockRecovery::to_json),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectRecovery {
    pub(super) workforce_journal: Digest32,
    pub(super) order_journal: Digest32,
    pub(super) observation_tick_floor: u64,
    pub(super) clock: Option<ProjectClockRecovery>,
    pub(super) inventory_clock: Option<ProjectClockRecovery>,
    pub(super) portfolio_clocks: Option<[Option<ProjectPortfolioClockRecovery>; 4]>,
    pub(super) manager_origin: Option<Digest32>,
    pub(super) publication_pending: bool,
}

#[path = "project_recovery.rs"]
mod project_recovery;
pub(crate) use project_recovery::ProjectProductionIntent;

#[path = "project_manager_config.rs"]
mod project_manager;

/// Closed child configuration facts needed before the staged parent opens either owner.
pub(crate) struct ParentMetadata {
    pub(crate) directory: PathBuf,
    pub(crate) initialize: bool,
    pub(crate) has_original: bool,
    pub(crate) clock: bool,
    pub(crate) inventory_clock: bool,
    pub(crate) manager_directory: Option<PathBuf>,
    pub(crate) workshop_checkpoint: bool,
    pub(crate) authority_tick_floor: u64,
}

fn family_environment() -> Result<[String; 3]> {
    if std::env::var(OPT_IN).ok().as_deref() != Some("1")
        || std::env::var("DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE")
            .ok()
            .as_deref()
            != Some("1")
        || std::env::var("DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS")
            .ok()
            .as_deref()
            != Some("1")
    {
        return Err(denied(
            "combined and both family development opt-ins are required",
        ));
    }
    configuration_strings()
}

fn environment() -> Result<[String; 3]> {
    let raw = family_environment()?;
    let (_, _, _, clock, inventory_clock, workshop, _, manager, _, _, _) = parse(&raw[0])?;
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if !allowed_key(
            &key,
            clock.is_some(),
            inventory_clock.is_some(),
            workshop.is_some(),
            manager.is_some(),
        ) {
            return Err(denied(
                "combined development rejects other DFMCP state and unconfigured native credentials",
            ));
        }
    }
    Ok(raw)
}

fn configuration_strings() -> Result<[String; 3]> {
    let read = |name| -> Result<String> {
        let raw = std::env::var(name)
            .map_err(|_| denied("required combined operator configuration is missing"))?;
        if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
            return Err(invalid(
                "combined operator configuration exceeds its fixed bound",
            ));
        }
        Ok(raw)
    };
    Ok([read(CONFIG)?, read(WORKFORCE_CONFIG)?, read(ORDER_CONFIG)?])
}

fn parent_fingerprint(raw: &[String; 3]) -> Digest32 {
    let mut bytes = b"dfmcp.workshop-production-child-configurations/1".to_vec();
    for value in raw {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    Digest32::of_bytes(&bytes)
}

/// Decode the original closed input before inserting an internal donor. In
/// particular, a Value round trip must not conceal duplicate operator fields.
fn parent_configuration(raw: &str, workshop: Option<&serde_json::Value>) -> Result<String> {
    let _ = decode_input(raw)?;
    let mut value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|_| invalid("invalid closed combined operator configuration"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("combined configuration must be an object"))?;
    if object.contains_key("workshop_prerequisite") {
        return Err(denied(
            "staged production obtains its Workshop prerequisite only from its verified original",
        ));
    }
    if let Some(workshop) = workshop {
        object.insert("workshop_prerequisite".to_owned(), workshop.clone());
        serde_json::to_string(&value)
            .map_err(|_| invalid("could not encode the verified Workshop prerequisite"))
    } else {
        Ok(raw.to_owned())
    }
}

fn allowed_key(
    key: &str,
    clock: bool,
    inventory_clock: bool,
    workshop: bool,
    manager: bool,
) -> bool {
    match key {
        workshop_config::TOKEN => workshop,
        manager_config::TOKEN => manager,
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK" | "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN" => clock,
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"
        | "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN" => inventory_clock,
        _ => !key.starts_with("DFMCP_") || ALLOWED.contains(&key),
    }
}

fn matches_digests(raw: &[String; 3], expected: [Digest32; 3]) -> bool {
    raw.each_ref()
        .map(|value| Digest32::of_bytes(value.as_bytes()))
        == expected
}

type Parsed = (
    PathBuf,
    bool,
    Option<Recovery>,
    Option<ClockConfig>,
    Option<ClockConfig>,
    Option<WorkshopConfig>,
    Option<WorkshopPrerequisiteCheckpoint>,
    Option<ManagerConfig>,
    Option<ManagerPrerequisiteCheckpoint>,
    Option<InputConfig>,
    Option<InputCheckpoint>,
);

fn parse(raw: &str) -> Result<Parsed> {
    parse_with_workshop(raw, None, false)
}

fn decode_input(raw: &str) -> Result<Input> {
    if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
        return Err(invalid("combined operator configuration exceeds 16 KiB"));
    }
    let input: Input = serde_json::from_str(raw)
        .map_err(|_| invalid("invalid closed combined operator configuration"))?;
    if input.schema != "dfmcp.combined-production-development/1"
        || !input.directory.is_absolute()
        || input.directory.as_os_str().len() > 3800
        || input
            .directory
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        || (input.initialize && input.recovery.is_some())
    {
        return Err(invalid(
            "invalid combined schema, custody directory or recovery mode",
        ));
    }
    Ok(input)
}

fn decode_recovery(
    r: RecoveryInput,
    has_clock: bool,
    has_inventory_clock: bool,
) -> Result<Recovery> {
    let fortress = r
        .anchor
        .fortress_id
        .parse::<u64>()
        .map_err(|_| invalid("invalid recovery fortress"))?;
    if fortress == 0
        || fortress.to_string() != r.anchor.fortress_id
        || r.anchor.epoch >= u64::from(u32::MAX)
    {
        return Err(invalid("invalid combined recovery anchor"));
    }
    let frontier = durable::Recovery {
        journal_id: digest(&r.frontier.journal_id)?,
        head: digest(&r.frontier.head)?,
        authority_tick_floor: r.frontier.authority_tick_floor,
    };
    frontier.validate()?;
    let portfolio = r.portfolio.map(|entries| {
        if entries.is_empty() || entries.len() > 4 {
            return Err(invalid("portfolio recovery requires one to four complete original order slots"));
        }
        let mut result = [None; 4];
        let first = entries[0].step_id;
        if first > 1 {
            return Err(invalid("portfolio recovery must start with its original order step zero or one"));
        }
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.step_id != first + index as u32 {
                return Err(invalid("portfolio recovery slots must name each original order in order"));
            }
            let value = PortfolioRecovery { step_id: entry.step_id, goal_history_present: entry.goal_history_present,
                clock_journal_id: entry.clock_journal_id.as_deref().map(digest).transpose()?,
                inventory_clock_journal_id: entry.inventory_clock_journal_id.as_deref().map(digest).transpose()? };
            if value.clock_journal_id.is_some() != has_clock
                || value.inventory_clock_journal_id.is_some() != has_inventory_clock {
                return Err(denied("every portfolio slot must retain its independently configured clock custody"));
            }
            result[index] = Some(value);
        }
        Ok(result)
    }).transpose()?;
    Ok(Recovery {
        seal: PlanArchiveSeal {
            plan_digest: digest(&r.plan_digest)?,
            object_digest: digest(&r.object_digest)?,
        },
        identity_digest: digest(&r.identity_digest)?,
        anchor: StateAnchor {
            fortress_id: FortressId::new(fortress),
            cursor: ObservationCursor {
                epoch: r.anchor.epoch,
                sequence: r.anchor.sequence,
            },
            tick: GameTick(r.anchor.tick),
            state_hash: digest(&r.anchor.state_hash)?,
        },
        frontier,
        goal_history_present: r.goal_history_present,
        clock_journal_id: r.clock_journal_id.as_deref().map(digest).transpose()?,
        inventory_clock_journal_id: r
            .inventory_clock_journal_id
            .as_deref()
            .map(digest)
            .transpose()?,
        portfolio,
        workshop_prerequisite_digest: r
            .workshop_prerequisite_digest
            .as_deref()
            .map(digest)
            .transpose()?,
        workshop_prerequisite_floor: r.workshop_prerequisite_floor,
        manager_prerequisite_digest: r
            .manager_prerequisite_digest
            .as_deref()
            .map(digest)
            .transpose()?,
        manager_prerequisite_floor: r.manager_prerequisite_floor,
        input_supply_digest: r.input_supply_digest.as_deref().map(digest).transpose()?,
        input_supply_floor: r.input_supply_floor,
    })
}

fn parse_with_workshop(
    raw: &str,
    handoff: Option<&WorkshopProjectHandoff>,
    project: bool,
) -> Result<Parsed> {
    parse_with_project_sources(raw, handoff, None, false, project)
}

fn parse_with_project_sources(
    raw: &str,
    handoff: Option<&WorkshopProjectHandoff>,
    manager_handoff: Option<&ManagerProjectHandoff>,
    owns_manager: bool,
    project: bool,
) -> Result<Parsed> {
    let input = decode_input(raw)?;
    let clock = input
        .clock
        .map(|clock| clock.validate(input.initialize))
        .transpose()?;
    let inventory_clock = input
        .inventory_clock
        .map(|clock| clock.validate(input.initialize))
        .transpose()?;
    if project && input.workshop_prerequisite.is_some() {
        return Err(denied(
            "project Workshop donor must come from the verified completed child handoff",
        ));
    }
    let workshop_prerequisite = match handoff {
        Some(handoff) => Some(WorkshopConfig::from_project_handoff(
            handoff,
            &input.directory,
        )?),
        None => input
            .workshop_prerequisite
            .map(|workshop| workshop.validate(&input.directory))
            .transpose()?,
    };
    let mut workshop_prerequisite_checkpoint = input.workshop_prerequisite_checkpoint;
    if let Some(handoff) = handoff {
        let seed = WorkshopPrerequisiteCheckpoint {
            origin_witness: handoff.origin_witness(),
            floor: WorkshopRecoveryFloor {
                tick: handoff.high_tick(),
                generation: handoff.high_generation(),
                next_building_id: handoff.high_next_building_id(),
                fenced: false,
            },
        };
        if let Some(recovery) = &input.recovery {
            let retained = recovery
                .workshop_prerequisite_floor
                .ok_or_else(|| denied("project recovery lost its Workshop source floors"))?;
            require_project_floor(retained, seed.floor)?;
        } else if let Some(retained) = workshop_prerequisite_checkpoint {
            if retained.origin_witness != seed.origin_witness {
                return Err(denied(
                    "project checkpoint belongs to another Workshop origin",
                ));
            }
            require_project_floor(retained.floor, seed.floor)?;
        } else {
            // The completed child has already consumed these source facts.
            // Restore them before the production owner can contact any native
            // source, even on its first opening and before an original exists.
            workshop_prerequisite_checkpoint = Some(seed);
        }
    }
    let (manager_prerequisite, manager_prerequisite_checkpoint) = project_manager::resolve(
        input.manager_prerequisite,
        input.manager_prerequisite_checkpoint,
        input.recovery.as_ref(),
        &input.directory,
        manager_handoff,
        owns_manager,
    )?;
    let input_supply = input.input_supply;
    let input_supply_checkpoint = input.input_supply_checkpoint;
    if let Some(checkpoint) = input_supply_checkpoint {
        if input_supply.is_none() || input.recovery.is_some() {
            return Err(denied(
                "input supply checkpoint requires its configured pre-plan owner",
            ));
        }
        checkpoint.floor.validate(0, checkpoint.floor.tick)?;
    }
    if let (Some(workshop), Some(manager)) = (&workshop_prerequisite, &manager_prerequisite)
        && !distinct_directories([&workshop.directory, &manager.directory])
    {
        return Err(denied(
            "workshop and manager donors require disjoint private custody",
        ));
    }
    if let Some(checkpoint) = workshop_prerequisite_checkpoint {
        let workshop = workshop_prerequisite.as_ref().ok_or_else(|| {
            denied("pre-plan workshop checkpoint requires its exact configured donor")
        })?;
        if input.recovery.is_some() {
            return Err(denied(
                "pre-plan workshop checkpoint cannot compete with complete original recovery",
            ));
        }
        checkpoint
            .floor
            .validate(workshop.binding.generation(), checkpoint.floor.tick)?;
    }
    if let Some(checkpoint) = manager_prerequisite_checkpoint {
        let manager = manager_prerequisite.as_ref().ok_or_else(|| {
            denied("pre-plan manager checkpoint requires its exact configured donor")
        })?;
        if input.recovery.is_some() {
            return Err(denied(
                "pre-plan manager checkpoint cannot compete with complete original recovery",
            ));
        }
        checkpoint
            .floor
            .validate(manager.binding.generation, checkpoint.floor.tick)?;
    }
    let recovery = input
        .recovery
        .map(|r| decode_recovery(r, clock.is_some(), inventory_clock.is_some()))
        .transpose()?;
    if let Some(recovery) = recovery {
        if recovery.input_supply_digest.is_some() != input_supply.is_some()
            || recovery.input_supply_floor.is_some() != input_supply.is_some()
        {
            return Err(denied(
                "combined input supply recovery requires its exact original marker and complete source floors",
            ));
        }
        if let Some(floor) = recovery.input_supply_floor {
            floor.validate(0, recovery.frontier.authority_tick_floor)?;
        }
        if recovery.workshop_prerequisite_digest.is_some() != workshop_prerequisite.is_some()
            || recovery.workshop_prerequisite_floor.is_some() != workshop_prerequisite.is_some()
        {
            return Err(denied(
                "combined recovery requires the exact workshop prerequisite association and consumed floors",
            ));
        }
        if let (Some(workshop), Some(floor)) =
            (&workshop_prerequisite, recovery.workshop_prerequisite_floor)
        {
            floor.validate(
                workshop.binding.generation(),
                recovery.frontier.authority_tick_floor,
            )?;
        }
        if recovery.manager_prerequisite_digest.is_some() != manager_prerequisite.is_some()
            || recovery.manager_prerequisite_floor.is_some() != manager_prerequisite.is_some()
        {
            return Err(denied(
                "combined recovery requires the exact manager prerequisite association and consumed floors",
            ));
        }
        if let (Some(manager), Some(floor)) =
            (&manager_prerequisite, recovery.manager_prerequisite_floor)
        {
            floor.validate(
                manager.binding.generation,
                recovery.frontier.authority_tick_floor,
            )?;
        }
        if let Some(entries) = recovery.portfolio {
            let first =
                entries[0].ok_or_else(|| invalid("first portfolio recovery slot absent"))?;
            if first.goal_history_present != recovery.goal_history_present
                || first.clock_journal_id != recovery.clock_journal_id
                || first.inventory_clock_journal_id != recovery.inventory_clock_journal_id
            {
                return Err(denied(
                    "first portfolio slot differs from original legacy custody paths",
                ));
            }
        }
        if recovery.clock_journal_id != clock.and_then(|clock| clock.journal_id) {
            return Err(denied(
                "combined clock recovery must name its exact independently retained journal",
            ));
        }
        if recovery.inventory_clock_journal_id != inventory_clock.and_then(|clock| clock.journal_id)
        {
            return Err(denied(
                "combined inventory clock recovery must name its exact independently retained journal",
            ));
        }
    }
    Ok((
        input.directory,
        input.initialize,
        recovery,
        clock,
        inventory_clock,
        workshop_prerequisite,
        workshop_prerequisite_checkpoint,
        manager_prerequisite,
        manager_prerequisite_checkpoint,
        input_supply,
        input_supply_checkpoint,
    ))
}

fn require_project_floor(
    retained: WorkshopRecoveryFloor,
    seed: WorkshopRecoveryFloor,
) -> Result<()> {
    if retained.tick < seed.tick
        || retained.generation < seed.generation
        || retained.next_building_id < seed.next_building_id
    {
        return Err(denied(
            "project recovery rolled back a consumed Workshop source floor",
        ));
    }
    Ok(())
}

fn distinct_directories<const N: usize>(paths: [&Path; N]) -> bool {
    (0..paths.len()).all(|i| {
        ((i + 1)..paths.len())
            .all(|j| !paths[i].starts_with(paths[j]) && !paths[j].starts_with(paths[i]))
    })
}

fn matching_sources(
    wa: &workforce::policy::SourceAcceptance,
    oa: &orders::policy::SourceAcceptance,
) -> bool {
    wa.world_folder == oa.world_folder
        && wa.site_id == oa.site_id
        && wa.df_version == oa.df_version
        && wa.dfhack_version == oa.dfhack_version
        && wa.citizen_bridge_version == oa.citizen_bridge_version
        && wa.citizen_generation == oa.citizen_generation
}

/// Configuration-only description for the parent ledger. It contains no
/// credentials, live handles, authority grants, or current source evidence.
#[derive(Clone, Debug)]
pub(crate) struct ProjectProductionPolicy {
    identity: Digest32,
    directory: PathBuf,
    workforce_directory: PathBuf,
    order_directory: PathBuf,
    initialize: bool,
    workforce: workforce::policy::SourceAcceptance,
    orders: orders::policy::SourceAcceptance,
}

impl ProjectProductionPolicy {
    pub(crate) fn identity(&self) -> Digest32 {
        self.identity
    }
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn workforce_directory(&self) -> &Path {
        &self.workforce_directory
    }
    pub(crate) fn order_directory(&self) -> &Path {
        &self.order_directory
    }
    pub(crate) fn initializing(&self) -> bool {
        self.initialize
    }
    pub(crate) fn matches_workshop(&self, workshop: &WorkshopBinding) -> Result<()> {
        if !WorkshopConfig::binding_matches_sources(workshop, &self.workforce, &self.orders) {
            return Err(denied(
                "project Workshop and production fortress or software pins differ",
            ));
        }
        Ok(())
    }
}

fn project_credentials(clock: bool, inventory_clock: bool, manager: bool) -> Result<()> {
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        let own = key.starts_with("DFMCP_COMBINED_PRODUCTION_")
            || key.starts_with("DFMCP_SEMANTIC_WORKFORCE_")
            || key.starts_with("DFMCP_SEMANTIC_WORK_ORDER_");
        if own && !allowed_key(&key, clock, inventory_clock, true, manager) {
            return Err(denied(
                "project production rejects unconfigured native credentials and child state",
            ));
        }
    }
    for flag in [
        "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR",
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
    ] {
        if std::env::var_os(flag).is_some_and(|value| value.to_str() != Some("1")) {
            return Err(denied(
                "project effect enablement must be absent or exactly 1",
            ));
        }
    }
    Ok(())
}

/// Only the externally retained restart custody changes between openings.
/// The parent must compare every omitted identity with its checkpoint before
/// opening the child; this digest is never a substitute for that comparison.
fn project_policy_identity(raw: &[String; 3]) -> Result<Digest32> {
    let mut bytes = b"dfmcp-workshop-stock-project-production-policy/1\0".to_vec();
    for (index, source) in raw.iter().enumerate() {
        let mut value: serde_json::Value = serde_json::from_str(source)
            .map_err(|_| invalid("invalid checked project source policy"))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid("project source policy must be an object"))?;
        object.remove("initialize");
        object.remove("recovery");
        if index == 0 {
            for field in [
                "workshop_prerequisite_checkpoint",
                "manager_prerequisite_checkpoint",
                "input_supply_checkpoint",
            ] {
                object.remove(field);
            }
        } else if index == 1 {
            object.remove("journal_id");
        }
        for clock in ["clock", "inventory_clock"] {
            if let Some(fields) = object
                .get_mut(clock)
                .and_then(serde_json::Value::as_object_mut)
            {
                fields.remove("journal_id");
            }
        }
        // serde_json's checked values retain every source pin and scope. Sort
        // object members explicitly so harmless JSON key order on a restart
        // cannot change this policy identity, even with preserve_order enabled.
        fn ordered(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<()> {
            match value {
                serde_json::Value::Object(object) => {
                    output.push(b'{');
                    let sorted: std::collections::BTreeMap<_, _> = object.iter().collect();
                    for (index, (key, value)) in sorted.into_iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        serde_json::to_writer(&mut *output, key)
                            .map_err(|_| invalid("project policy key encoding failed"))?;
                        output.push(b':');
                        ordered(value, output)?;
                    }
                    output.push(b'}');
                }
                serde_json::Value::Array(values) => {
                    output.push(b'[');
                    for (index, value) in values.iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        ordered(value, output)?;
                    }
                    output.push(b']');
                }
                _ => serde_json::to_writer(output, value)
                    .map_err(|_| invalid("project policy value encoding failed"))?,
            }
            Ok(())
        }
        let mut encoded = Vec::new();
        ordered(&value, &mut encoded)?;
        bytes.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&encoded);
    }
    Ok(Digest32::of_bytes(&bytes))
}

impl Config {
    pub(crate) fn from_environment(runtime_check: fn() -> Result<()>) -> Result<Self> {
        runtime_check()?;
        let raw = environment()?;
        let digests = raw
            .each_ref()
            .map(|value| Digest32::of_bytes(value.as_bytes()));
        let parsed = parse(&raw[0])?;
        let check: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            let current = environment()?;
            if !matches_digests(&current, digests) {
                return Err(denied(
                    "combined operator configuration changed; close this session",
                ));
            }
            Ok(())
        });
        Self::from_checked_inputs(parsed, check, runtime_check)
    }

    /// Parse and cross-check every future production pin without opening a
    /// custody file, credential, or native connection. The parent uses this
    /// before starting Workshop work and retains the stable policy identity.
    pub(crate) fn validate_project_environment(
        parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        handoff: Option<&WorkshopProjectHandoff>,
    ) -> Result<ProjectProductionPolicy> {
        Self::validate_resource_project_environment(
            parent_guard,
            runtime_check,
            handoff,
            None,
            None,
        )
    }

    /// Validate the optional Manager prefix without inventing its future
    /// appointment, ACK or donor. The actual handoff is required on activation.
    pub(crate) fn validate_resource_project_environment(
        parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        handoff: Option<&WorkshopProjectHandoff>,
        manager: Option<&ProjectManagerConfig>,
        manager_handoff: Option<&ManagerProjectHandoff>,
    ) -> Result<ProjectProductionPolicy> {
        if manager_handoff.is_some() && manager.is_none() {
            return Err(denied(
                "a project Manager donor requires its original Manager policy",
            ));
        }
        let (config, raw) = Self::project_configuration(
            parent_guard,
            runtime_check,
            handoff,
            manager_handoff,
            manager.is_some(),
        )?;
        let directory = |raw: &str| -> Result<PathBuf> {
            let value: serde_json::Value = serde_json::from_str(raw)
                .map_err(|_| invalid("invalid checked production policy"))?;
            value["directory"]
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| invalid("checked child policy lacks its custody directory"))
        };
        let policy = ProjectProductionPolicy {
            identity: project_policy_identity(&raw)?,
            directory: config.directory.clone(),
            workforce_directory: directory(&raw[1])?,
            order_directory: directory(&raw[2])?,
            initialize: config.initialize,
            workforce: workforce::Shell::acceptance(&config.workforce).clone(),
            orders: orders::Shell::acceptance(&config.orders).clone(),
        };
        if let Some(manager) = manager {
            policy.matches_manager(manager.project_binding())?;
        }
        config.current()?;
        Ok(policy)
    }

    /// Only the typed completed Workshop handoff can supply this donor. The
    /// parent owns the closed union of environment keys; this owner still pins
    /// every byte of its own three source configurations and all child guards.
    pub(crate) fn from_project_environment(
        parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        handoff: &WorkshopProjectHandoff,
    ) -> Result<Self> {
        Self::from_resource_project_environment(parent_guard, runtime_check, handoff, None)
    }

    /// Import only the original Manager descriptor retained by the parent.
    /// The existing production owner reopens its donor Offline and independently
    /// qualifies current readiness at every protected production boundary.
    pub(crate) fn from_resource_project_environment(
        parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        handoff: &WorkshopProjectHandoff,
        manager_handoff: Option<&ManagerProjectHandoff>,
    ) -> Result<Self> {
        if manager_handoff.is_some_and(|manager| {
            manager.project_deadline_tick() != handoff.project_deadline_tick()
        }) {
            return Err(denied(
                "project Manager and Workshop original deadlines differ",
            ));
        }
        Self::project_configuration(
            parent_guard,
            runtime_check,
            Some(handoff),
            manager_handoff,
            manager_handoff.is_some(),
        )
        .map(|(config, _)| config)
    }

    fn project_configuration(
        parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        handoff: Option<&WorkshopProjectHandoff>,
        manager_handoff: Option<&ManagerProjectHandoff>,
        owns_manager: bool,
    ) -> Result<(Self, [String; 3])> {
        runtime_check()?;
        parent_guard()?;
        let raw = family_environment()?;
        let parsed =
            parse_with_project_sources(&raw[0], handoff, manager_handoff, owns_manager, true)?;
        if parsed
            .2
            .is_some_and(|recovery| recovery.portfolio.is_some())
            && (parsed.3.is_some() || parsed.4.is_some())
        {
            return Err(denied(
                "project portfolio recovery cannot adopt production clock custody",
            ));
        }
        let conditional = (
            parsed.3.is_some(),
            parsed.4.is_some(),
            owns_manager || parsed.7.is_some(),
        );
        let digests = raw
            .each_ref()
            .map(|value| Digest32::of_bytes(value.as_bytes()));
        let check: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            parent_guard()?;
            let current = family_environment()?;
            if !matches_digests(&current, digests) {
                return Err(denied(
                    "project production source configuration changed; close this session",
                ));
            }
            project_credentials(conditional.0, conditional.1, conditional.2)
        });
        check()?;
        let mut config = Self::from_checked_inputs(parsed, check, runtime_check)?;
        if config.initialize
            && (!workforce::Shell::labor_enabled(&config.workforce)
                || !orders::Shell::production_enabled(&config.orders))
        {
            return Err(denied(
                "project production initialization requires both explicit family authorities",
            ));
        }
        config.project_binding = handoff.map(|handoff| ProjectProductionBinding {
            workshop_origin: handoff.origin_witness(),
            deadline_tick: handoff.project_deadline_tick(),
            authority_expires_tick: handoff.authority_expires_tick(),
        });
        config.project_manager_handoff = manager_handoff.cloned();
        Ok((config, raw))
    }

    /// The fixed staged parent owns the union allowlist and pins all five raw
    /// configurations. Native family readers retain their own exact checks.
    pub(crate) fn from_parent_environment(
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        verified_workshop: Option<&serde_json::Value>,
    ) -> Result<Self> {
        runtime_check()?;
        parent_check()?;
        let raw = configuration_strings()?;
        let fingerprint = parent_fingerprint(&raw);
        let digests = raw
            .each_ref()
            .map(|value| Digest32::of_bytes(value.as_bytes()));
        let configuration = parent_configuration(&raw[0], verified_workshop)?;
        let check: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            parent_check()?;
            if !matches_digests(&configuration_strings()?, digests) {
                return Err(denied(
                    "staged production child configuration changed; close this session",
                ));
            }
            Ok(())
        });
        Self::from_configuration(&configuration, check, runtime_check, Some(fingerprint))
    }

    pub(crate) fn parent_configuration_digest(&self) -> Option<Digest32> {
        self.parent_configuration_digest
    }

    pub(crate) fn require_parent_workshop(
        &self,
        binding: &dfmcp_adapter::workshop_construction::WorkshopBinding,
        directory: &Path,
    ) -> Result<()> {
        if !self.workshop_prerequisite.as_ref().is_some_and(|workshop| {
            &workshop.binding == binding && workshop.directory.as_path() == directory
        }) {
            return Err(denied(
                "staged production donor differs from its owned Workshop source or custody",
            ));
        }
        Ok(())
    }

    pub(crate) fn parent_allowed_key(
        key: &str,
        clock: bool,
        inventory_clock: bool,
        manager: bool,
    ) -> bool {
        allowed_key(key, clock, inventory_clock, true, manager)
    }

    pub(crate) fn parent_metadata(raw: &str) -> Result<ParentMetadata> {
        // This validates the closed operator input before handoff is available.
        // Complete original recovery is validated by the ordinary parser after
        // the independently verified original donor has been injected locally.
        parent_configuration(raw, None)?;
        let input = decode_input(raw)?;
        let clock = input
            .clock
            .map(|c| c.validate(input.initialize))
            .transpose()?;
        let inventory_clock = input
            .inventory_clock
            .map(|c| c.validate(input.initialize))
            .transpose()?;
        let manager = input
            .manager_prerequisite
            .map(|m| m.validate(&input.directory))
            .transpose()?;
        let authority_tick_floor = [
            input
                .recovery
                .as_ref()
                .map(|r| r.frontier.authority_tick_floor),
            input.workshop_prerequisite_checkpoint.map(|c| c.floor.tick),
            input.manager_prerequisite_checkpoint.map(|c| c.floor.tick),
            input.input_supply_checkpoint.map(|c| c.floor.tick),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);
        Ok(ParentMetadata {
            directory: input.directory,
            initialize: input.initialize,
            has_original: input.recovery.is_some(),
            clock: clock.is_some(),
            inventory_clock: inventory_clock.is_some(),
            manager_directory: manager.map(|m| m.directory),
            workshop_checkpoint: input.workshop_prerequisite_checkpoint.is_some(),
            authority_tick_floor,
        })
    }

    fn from_configuration(
        raw: &str,
        check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
        parent_configuration_digest: Option<Digest32>,
    ) -> Result<Self> {
        let mut config = Self::from_checked_inputs(parse(raw)?, check, runtime_check)?;
        config.parent_configuration_digest = parent_configuration_digest;
        Ok(config)
    }

    fn from_checked_inputs(
        parsed: Parsed,
        check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        runtime_check: fn() -> Result<()>,
    ) -> Result<Self> {
        let (
            directory,
            initialize,
            recovery,
            clock,
            inventory_clock,
            workshop_prerequisite,
            workshop_prerequisite_checkpoint,
            manager_prerequisite,
            manager_prerequisite_checkpoint,
            input_supply,
            input_supply_checkpoint,
        ) = parsed;
        let w = workforce::native::Config::from_combined_environment(
            Arc::clone(&check),
            runtime_check,
        )?;
        let o = match (clock, inventory_clock) {
            (Some(clock), None) => orders::native::Config::from_combined_clock_environment(
                Arc::clone(&check),
                runtime_check,
                clock.generation,
                clock.journal_id,
            )?,
            (None, None) => orders::native::Config::from_combined_environment(
                Arc::clone(&check),
                runtime_check,
            )?,
            (clock, inventory_clock) => orders::native::Config::from_combined_clocks_environment(
                Arc::clone(&check),
                runtime_check,
                clock.map(|clock| (clock.generation, clock.journal_id)),
                inventory_clock.map(|clock| (clock.generation, clock.journal_id)),
            )?,
        };
        if !distinct_directories([&directory, w.custody_directory(), o.custody_directory()])
            || workshop_prerequisite.as_ref().is_some_and(|workshop| {
                !distinct_directories([
                    &directory,
                    w.custody_directory(),
                    o.custody_directory(),
                    &workshop.directory,
                ])
            })
            || manager_prerequisite.as_ref().is_some_and(|manager| {
                !distinct_directories([
                    &directory,
                    w.custody_directory(),
                    o.custody_directory(),
                    &manager.directory,
                ])
            })
            || w.endpoint() != o.endpoint()
            || w.initializing() != initialize
            || o.initializing() != initialize
        {
            return Err(denied(
                "combined family custody, initialization or endpoint differs",
            ));
        }
        let workforce = workforce::native::NativeShell::from_combined_config(w)?;
        let orders = orders::native::NativeShell::from_combined_config(o)?;
        let wa = workforce::Shell::acceptance(&workforce);
        let oa = orders::Shell::acceptance(&orders);
        if !matching_sources(wa, oa)
            || workshop_prerequisite
                .as_ref()
                .is_some_and(|workshop| !workshop.matching_sources(wa, oa))
            || manager_prerequisite
                .as_ref()
                .is_some_and(|manager| !manager.matching_sources(wa, oa))
            || recovery.is_some_and(|r| r.anchor.fortress_id != wa.fortress())
        {
            return Err(denied(
                "combined family citizen source or fortress pins differ",
            ));
        }
        runtime_check()?;
        check()?;
        Ok(Self {
            directory,
            initialize,
            recovery,
            clock,
            inventory_clock,
            workshop_prerequisite,
            workshop_prerequisite_checkpoint,
            manager_prerequisite,
            manager_prerequisite_checkpoint,
            input_supply,
            input_supply_checkpoint,
            workforce,
            orders,
            project_binding: None,
            project_manager_handoff: None,
            project_recovery: None,
            parent_configuration_digest: None,
            check,
            runtime_check,
        })
    }
    pub(crate) fn current(&self) -> Result<()> {
        (self.runtime_check)()?;
        (self.check)()
    }
    pub(super) fn read_guard(&self) -> Arc<dyn Fn() -> Result<()> + Send + Sync> {
        let runtime_check = self.runtime_check;
        let check = Arc::clone(&self.check);
        Arc::new(move || {
            runtime_check()?;
            check()
        })
    }
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manager_config_tests.rs"]
mod manager_tests;

#[cfg(test)]
#[path = "project_config_tests.rs"]
mod project_tests;

#[cfg(test)]
#[path = "parent_config_tests.rs"]
mod parent_tests;
