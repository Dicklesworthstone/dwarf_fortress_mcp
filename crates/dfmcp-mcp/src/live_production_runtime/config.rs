//! Fixed operator configuration for the combined, unadmitted development owner.
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
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if !allowed_key(&key) {
            return Err(denied(
                "combined development rejects other DFMCP environment state",
            ));
        }
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
    Ok([read(CONFIG)?, read(WORKFORCE_CONFIG)?, read(ORDER_CONFIG)?])
}

fn allowed_key(key: &str) -> bool {
    !key.starts_with("DFMCP_") || ALLOWED.contains(&key)
}

fn matches_digests(raw: &[String; 3], expected: [Digest32; 3]) -> bool {
    raw.each_ref()
        .map(|value| Digest32::of_bytes(value.as_bytes()))
        == expected
}

fn parse(raw: &str) -> Result<(PathBuf, bool, Option<Recovery>, Option<ClockConfig>)> {
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
            })
        })
        .transpose()?;
    if let Some(recovery) = recovery
        && recovery.clock_journal_id != clock.and_then(|clock| clock.journal_id)
    {
        return Err(denied(
            "combined clock recovery must name its exact independently retained journal",
        ));
    }
    Ok((input.directory, input.initialize, recovery, clock))
}

fn distinct_directories(paths: [&Path; 3]) -> bool {
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
        let (directory, initialize, recovery, clock) = parse(&raw[0])?;
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
        let o = match clock {
            Some(clock) => orders::native::Config::from_combined_clock_environment(
                Arc::clone(&check),
                runtime_check,
                clock.generation,
                clock.journal_id,
            )?,
            None => orders::native::Config::from_combined_environment(
                Arc::clone(&check),
                runtime_check,
            )?,
        };
        if !distinct_directories([&directory, w.custody_directory(), o.custody_directory()])
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
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
