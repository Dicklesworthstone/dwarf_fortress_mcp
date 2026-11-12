//! Fixed operator configuration for the combined, unadmitted development owner.
use super::manager_config::{
    self, ManagerConfig, ManagerPrerequisiteCheckpoint, ManagerRecoveryFloor,
};
use super::workshop_config::{self, WorkshopConfig};
use crate::semantic_workforce_runtime::{denied, digest, invalid};
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::live_dispatch::durable;
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
    pub(crate) workforce: workforce::native::NativeShell,
    pub(crate) orders: orders::native::NativeShell,
    check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    runtime_check: fn() -> Result<()>,
}

fn environment() -> Result<[String; 3]> {
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
    let raw = [read(CONFIG)?, read(WORKFORCE_CONFIG)?, read(ORDER_CONFIG)?];
    let (_, _, _, clock, inventory_clock, workshop, _, manager, _) = parse(&raw[0])?;
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

fn parse(
    raw: &str,
) -> Result<(
    PathBuf,
    bool,
    Option<Recovery>,
    Option<ClockConfig>,
    Option<ClockConfig>,
    Option<WorkshopConfig>,
    Option<WorkshopPrerequisiteCheckpoint>,
    Option<ManagerConfig>,
    Option<ManagerPrerequisiteCheckpoint>,
)> {
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
    let clock = input
        .clock
        .map(|clock| clock.validate(input.initialize))
        .transpose()?;
    let inventory_clock = input
        .inventory_clock
        .map(|clock| clock.validate(input.initialize))
        .transpose()?;
    let workshop_prerequisite = input
        .workshop_prerequisite
        .map(|workshop| workshop.validate(&input.directory))
        .transpose()?;
    let workshop_prerequisite_checkpoint = input.workshop_prerequisite_checkpoint;
    let manager_prerequisite = input
        .manager_prerequisite
        .map(|manager| manager.validate(&input.directory))
        .transpose()?;
    let manager_prerequisite_checkpoint = input.manager_prerequisite_checkpoint;
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
        .map(|r| {
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
                    if value.clock_journal_id.is_some() != clock.is_some()
                        || value.inventory_clock_journal_id.is_some() != inventory_clock.is_some() {
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
            })
        })
        .transpose()?;
    if let Some(recovery) = recovery {
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
    ))
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

impl Config {
    pub(crate) fn from_environment(runtime_check: fn() -> Result<()>) -> Result<Self> {
        runtime_check()?;
        let raw = environment()?;
        let digests = raw
            .each_ref()
            .map(|value| Digest32::of_bytes(value.as_bytes()));
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
        ) = parse(&raw[0])?;
        let check: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            let current = environment()?;
            if !matches_digests(&current, digests) {
                return Err(denied(
                    "combined operator configuration changed; close this session",
                ));
            }
            Ok(())
        });
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
            workforce,
            orders,
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
