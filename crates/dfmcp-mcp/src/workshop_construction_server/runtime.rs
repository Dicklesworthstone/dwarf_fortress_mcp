//! One original workshop goal. Operator configuration supplies sources and authority;
//! tool arguments select only an observed offer and its exact review.
#[path = "clock.rs"]
mod clock;
#[path = "material_selection.rs"]
mod material_selection;
#[path = "tree_material.rs"]
mod tree_material;
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::build_placement::rpc::BuildCancellation;
use dfmcp_adapter::workshop_construction::{
    FortressIdentity, WorkshopBinding, WorkshopCapture, WorkshopGoal, WorkshopItem, WorkshopPlan,
    WorkshopSelection, WorkshopTile,
    acknowledgement::{WorkshopAcknowledgements, open_private_acknowledgements},
    journal::{
        WorkshopGuard, WorkshopJournal, WorkshopMode, WorkshopPin, WorkshopStage,
        open_private_workshop,
    },
    rpc::{WorkshopRpc, WorkshopSource},
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    LeaseId, LeaseManager, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, StateAnchor, WorkBudget, cuboids_intersect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) const RESPONSE_BYTES: u64 = 128 * 1024;
pub(super) const VIEW_BYTES: u64 = 32 * 1024 * 1024;
const INSPECT_BYTES: u64 = 4 * 1024 * 1024;
const CONNECT_BYTES: u64 = 1024 * 1024;
const CONFIG: &str = "DFMCP_WORKSHOP_MCP_CONFIG";
const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20";
const TOKEN: &str = "DFMCP_WORKSHOP_TOKEN";
const BUILD: &str = "DFMCP_WORKSHOP_ALLOW_BUILD";
pub(super) fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn denied(message: &str) -> DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "workshop request work or output bound exhausted",
    )
}
pub(super) fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("digest requires 64 lowercase hex characters"));
    }
    Digest32::from_hex(raw)
        .filter(|d| *d != Digest32::ZERO)
        .ok_or_else(|| invalid("invalid digest"))
}
pub(super) fn validate_environment() -> Result<()> {
    if std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        return Err(denied(
            "unadmitted workshop wrapper refuses production protocol and provenance",
        ));
    }
    if std::env::var(OPT_IN).ok().as_deref() != Some("1") {
        return Err(denied(
            "explicit workshop/1.20 development opt-in is required",
        ));
    }
    if std::env::var_os(BUILD).is_some_and(|v| v.to_str() != Some("1")) {
        return Err(denied(
            "workshop build permission must be absent or exactly 1",
        ));
    }
    let tree_material = std::env::var(tree_material::OPT_IN).ok().as_deref() == Some("1");
    let clock = std::env::var(clock::OPT_IN).ok().as_deref() == Some("1");
    if std::env::var_os(clock::ALLOW).is_some_and(|v| v.to_str() != Some("1")) {
        return Err(denied(
            "workshop clock permission must be absent or exactly 1",
        ));
    }
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("DFMCP_")
            && ![CONFIG, OPT_IN, TOKEN, BUILD].contains(&key.as_ref())
            && !(tree_material
                && [tree_material::OPT_IN, tree_material::TOKEN].contains(&key.as_ref()))
            && !(clock && [clock::OPT_IN, clock::TOKEN, clock::ALLOW].contains(&key.as_ref()))
        {
            return Err(denied(
                "workshop profile refuses production launch and other DFMCP profile state",
            ));
        }
    }
    Ok(())
}
pub(super) fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current()
        .ok_or_else(|| denied("workshop I/O requires its owned runtime context"))?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "workshop request cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(denied("inherited context denies workshop I/O"));
    }
    Ok(())
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinInput {
    journal_id: String,
    head: String,
    frames: u32,
}
impl PinInput {
    fn decode(&self) -> Result<WorkshopPin> {
        Ok(WorkshopPin {
            journal_id: digest(&self.journal_id)?,
            head: digest(&self.head)?,
            frames: self.frames,
        })
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    directory: PathBuf,
    endpoint: String,
    initialize: bool,
    acknowledged: Option<PinInput>,
    world_folder: String,
    site_id: u32,
    df_version: String,
    dfhack_version: String,
    generation: u64,
    recovery_generation: Option<u64>,
    map_dimensions: [u32; 3],
    item_id: Option<u32>,
    tree_material_source: Option<tree_material::Input>,
    clock: Option<clock::Input>,
    center: [u32; 3],
    scope: [u32; 6],
    #[serde(default)]
    protected: Vec<[u32; 6]>,
    authority_expires_tick: u64,
    lease_ticks: u64,
    checkpoint: String,
}
#[derive(Clone)]
pub(super) struct Config {
    input: Input,
    endpoint: SocketAddr,
    selection: Option<WorkshopSelection>,
    tree_material: Option<tree_material::Config>,
    clock: Option<clock::Config>,
    scope: MapCuboid,
    protected: Vec<MapCuboid>,
    identity: FortressIdentity,
    pin: Option<WorkshopPin>,
    hash: Digest32,
}
fn area(raw: [u32; 6], dimensions: [u32; 3]) -> Result<MapCuboid> {
    if (0..3).any(|i| raw[i] > raw[i + 3] || raw[i + 3] >= dimensions[i]) {
        return Err(invalid("scope must be ordered and inside the pinned map"));
    }
    MapCuboid::new(
        MapCoord::new(raw[0] as i32, raw[1] as i32, raw[2] as i32),
        MapCoord::new(raw[3] as i32, raw[4] as i32, raw[5] as i32),
    )
}
impl Config {
    fn view_bytes(&self) -> u64 {
        if self.clock.is_some() {
            VIEW_BYTES
        } else {
            INSPECT_BYTES
        }
    }
    pub(super) fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > 16 * 1024 {
            return Err(invalid("workshop operator configuration exceeds 16 KiB"));
        }
        let i: Input = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed workshop operator configuration"))?;
        let path = i
            .directory
            .to_str()
            .ok_or_else(|| invalid("custody path must be UTF-8"))?;
        if !["dfmcp.workshop-mcp-development/1", tree_material::SCHEMA].contains(&i.schema.as_str())
            || !i.directory.is_absolute()
            || path.len() < 2
            || path.len() > 3800
            || path[1..]
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.chars().any(char::is_control))
            || i.initialize == i.acknowledged.is_some()
            || i.protected.len() > 32
            || i.generation == 0
            || i.generation == u64::MAX
            || i.map_dimensions.iter().any(|n| *n == 0 || *n > 32768)
            || i.recovery_generation
                .is_some_and(|g| g < i.generation || g == u64::MAX)
            || i.authority_expires_tick == 0
            || !(1..=1200).contains(&i.lease_ticks)
            || !["required", "disposable-fortress-no-checkpoint"].contains(&i.checkpoint.as_str())
            || [&i.df_version, &i.dfhack_version]
                .iter()
                .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        {
            return Err(invalid(
                "invalid workshop schema, initialization, source pins, scope or authority interval",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&i.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != i.endpoint {
            return Err(invalid("canonical IPv4 loopback endpoint required"));
        }
        let selection = i
            .item_id
            .map(|item| WorkshopSelection::new(item, i.center))
            .transpose()?;
        // Validate the fixed footprint even when a donor supplies the material.
        WorkshopSelection::new(0, i.center)?;
        let tree_material = match (&i.schema[..], &i.tree_material_source, selection) {
            ("dfmcp.workshop-mcp-development/1", None, Some(_)) => None,
            (tree_material::SCHEMA, Some(source), None) => {
                Some(tree_material::Config::parse(source.clone(), &i)?)
            }
            _ => {
                return Err(invalid(
                    "workshop schema must select exactly one material policy",
                ));
            }
        };
        if i.center[0] + 1 >= i.map_dimensions[0]
            || i.center[1] + 1 >= i.map_dimensions[1]
            || i.center[2] >= i.map_dimensions[2]
        {
            return Err(invalid("workshop footprint exceeds pinned map"));
        }
        let scope = area(i.scope, i.map_dimensions)?;
        let protected = i
            .protected
            .iter()
            .map(|p| area(*p, i.map_dimensions))
            .collect::<Result<Vec<_>>>()?;
        let identity = FortressIdentity::new(&i.world_folder, i.site_id)?;
        let pin = i.acknowledged.as_ref().map(PinInput::decode).transpose()?;
        let clock = i
            .clock
            .clone()
            .map(|input| clock::Config::parse(input, &i, endpoint))
            .transpose()?;
        Ok(Self {
            input: i,
            endpoint,
            selection,
            tree_material,
            clock,
            scope,
            protected,
            identity,
            pin,
            hash: Digest32::of_bytes(raw.as_bytes()),
        })
    }
    pub(super) fn environment() -> Result<Self> {
        validate_environment()?;
        let config = Self::parse(
            &std::env::var(CONFIG)
                .map_err(|_| denied("operator workshop configuration is missing"))?,
        )?;
        config.material_environment()?;
        config.clock_environment()?;
        Ok(config)
    }
    fn material_environment(&self) -> Result<()> {
        let opted_in = std::env::var(tree_material::OPT_IN).ok().as_deref() == Some("1");
        if opted_in != self.tree_material.is_some()
            || (self.tree_material.is_none() && std::env::var_os(tree_material::TOKEN).is_some())
        {
            return Err(denied(
                "Tree material import requires its exact independent development opt-in and configuration",
            ));
        }
        Ok(())
    }
    fn current(&self) -> Result<()> {
        runtime_io()?;
        validate_environment()?;
        self.material_environment()?;
        self.clock_environment()?;
        let raw =
            std::env::var(CONFIG).map_err(|_| denied("operator workshop configuration removed"))?;
        if raw.len() > 16 * 1024 || Digest32::of_bytes(raw.as_bytes()) != self.hash {
            return Err(denied(
                "operator workshop source or authority changed; preserve and close the original session",
            ));
        }
        Ok(())
    }
    fn clock_environment(&self) -> Result<()> {
        let opted_in = std::env::var(clock::OPT_IN).ok().as_deref() == Some("1");
        if opted_in != self.clock.is_some()
            || (self.clock.is_none()
                && [clock::TOKEN, clock::ALLOW]
                    .iter()
                    .any(|key| std::env::var_os(key).is_some()))
        {
            return Err(denied(
                "workshop clock requires its independent opt-in and exact operator configuration",
            ));
        }
        Ok(())
    }
    fn binding(&self, b: &WorkshopBinding) -> Result<()> {
        if b.endpoint() != self.endpoint
            || b.generation() != self.input.generation
            || b.fortress() != &self.identity
            || b.dimensions() != self.input.map_dimensions
            || b.df_version() != self.input.df_version
            || b.dfhack_version() != self.input.dfhack_version
        {
            return Err(denied(
                "workshop original binding differs from operator source pins",
            ));
        }
        Ok(())
    }
    fn capture(&self, c: &WorkshopCapture) -> Result<()> {
        if c.fortress() != &self.identity
            || c.generation() != self.input.generation
            || c.dimensions() != self.input.map_dimensions
            || c.selection().center() != self.input.center
            || self
                .selection
                .is_some_and(|selection| c.selection() != selection)
        {
            return Err(denied(
                "workshop observation differs from exact original source or selection",
            ));
        }
        Ok(())
    }
    fn source(&self, c: &OperationContext, mode: WorkshopMode) -> Result<WorkshopRpc> {
        self.current()?;
        let secret = std::env::var(TOKEN)
            .map_err(|_| denied("workshop credential unavailable"))?
            .into_bytes();
        let pinned = secret.clone();
        let config = self.clone();
        let mut source = WorkshopRpc::connect(
            self.endpoint,
            secret,
            nonce(c)?,
            self.identity.clone(),
            c,
            BuildCancellation::with_check(Arc::new(runtime_io)),
            Box::new(move |build| {
                config.current()?;
                if build
                    && (mode != WorkshopMode::Control
                        || !config.input.initialize
                        || std::env::var(BUILD).ok().as_deref() != Some("1"))
                {
                    return Err(denied(
                        "current independent workshop build permission denied",
                    ));
                }
                if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&pinned) {
                    return Err(denied("workshop credential changed"));
                }
                Ok(())
            }),
        )?;
        let m = source.manifest();
        let generation = if mode == WorkshopMode::Control {
            self.input.generation
        } else {
            self.input
                .recovery_generation
                .unwrap_or(self.input.generation)
        };
        if m.generation != generation
            || m.df_version != self.input.df_version
            || m.dfhack_version != self.input.dfhack_version
        {
            source.fence();
            return Err(denied(
                "workshop connection differs from exact current software/incarnation pins",
            ));
        }
        Ok(source)
    }
}
fn nonce(context: &OperationContext) -> Result<[u8; 32]> {
    runtime_io()?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.budget.max_bytes < 32 {
        return Err(exhausted());
    }
    let mut out = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut out))
        .map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "workshop entropy unavailable",
            )
        })?;
    if out == [0; 32] {
        return Err(denied("invalid workshop entropy"));
    }
    Ok(out)
}

struct Guard {
    config: Config,
    selection: Option<WorkshopSelection>,
    acks: Arc<Mutex<WorkshopAcknowledgements<PrivateBuildFile>>>,
    leases: LeaseManager,
    lease: Option<LeaseId>,
    session: SessionId,
}
impl WorkshopGuard for Guard {
    fn acknowledge(&mut self, pin: WorkshopPin, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        self.acks
            .try_lock()
            .map_err(|_| denied("independent workshop acknowledgement busy or poisoned"))?
            .acknowledge(pin, c)?;
        self.config.current()
    }
    fn check(
        &mut self,
        stage: WorkshopStage,
        binding: &WorkshopBinding,
        selection: WorkshopSelection,
        item: Option<[u32; 3]>,
        c: &OperationContext,
    ) -> Result<()> {
        self.config.current()?;
        self.config.binding(binding)?;
        if c.session_id != self.session || self.selection != Some(selection) {
            return Err(denied(
                "workshop guard belongs to another session/selection",
            ));
        }
        self.acks
            .try_lock()
            .map_err(|_| denied("workshop acknowledgement busy or poisoned"))?
            .verify(c)?;
        if matches!(
            stage,
            WorkshopStage::Observe | WorkshopStage::Prepare | WorkshopStage::Commit
        ) {
            let [x1, y1, x2, y2, z] = selection.footprint();
            let footprint = area([x1, y1, z, x2, y2, z], self.config.input.map_dimensions)?;
            let p =
                item.ok_or_else(|| denied("original workshop material position unavailable"))?;
            let material = area(
                [p[0], p[1], p[2], p[0], p[1], p[2]],
                self.config.input.map_dimensions,
            )?;
            if !self.config.scope.contains_cuboid(footprint)
                || !self.config.scope.contains_cuboid(material)
                || self
                    .config
                    .protected
                    .iter()
                    .any(|r| cuboids_intersect(r, &footprint) || cuboids_intersect(r, &material))
            {
                return Err(denied(
                    "workshop footprint or exact material exceeds host scope/protected regions",
                ));
            }
            self.leases.verify_exclusive_spatial(
                self.lease
                    .ok_or_else(|| denied("original workshop lease unavailable"))?,
                self.session,
                footprint,
                c.anchor.tick,
            )?;
            self.leases.verify_exclusive_spatial(
                self.lease
                    .ok_or_else(|| denied("original workshop lease unavailable"))?,
                self.session,
                material,
                c.anchor.tick,
            )?;
        }
        if matches!(stage, WorkshopStage::Prepare | WorkshopStage::Commit) {
            if self.config.input.checkpoint != "disposable-fortress-no-checkpoint" {
                return Err(denied(
                    "checkpoint required; this profile has no checkpoint provider",
                ));
            }
            if std::env::var(BUILD).ok().as_deref() != Some("1") {
                return Err(denied("workshop build permission revoked"));
            }
        }
        Ok(())
    }
}
struct Work {
    context: OperationContext,
    start: Instant,
    bytes: u64,
}
impl Work {
    fn new(c: &OperationContext, view_bytes: u64) -> Result<Self> {
        c.budget.validate()?;
        let bytes = c
            .budget
            .max_bytes
            .checked_sub(RESPONSE_BYTES + view_bytes)
            .ok_or_else(exhausted)?;
        Ok(Self {
            context: c.clone(),
            start: Instant::now(),
            bytes,
        })
    }
    fn current(&self, bytes: u64) -> Result<OperationContext> {
        let mut c = self.context.clone();
        c.budget.max_wall_millis = c
            .budget
            .max_wall_millis
            .checked_sub(self.start.elapsed().as_millis() as u64)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    fn advance_authorization_tick(&mut self, tick: u64) {
        self.context.anchor.tick = GameTick(tick.max(self.context.anchor.tick.get()));
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        self.current(bytes)
    }
}
pub(super) enum Action {
    Inspect,
    Observe,
    Plan(String),
    Commit { seal: String, confirmed: bool },
    Wait,
    WaitClock,
    Cancel,
    CancelClock,
    Unavailable,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    kind: String,
    expected_observation: String,
    material: String,
}
pub(super) struct Runtime {
    pub(super) session: SessionId,
    pub(super) mode: WorkshopMode,
    pub(super) budget: WorkBudget,
    config: Config,
    journal: WorkshopJournal<PrivateBuildFile>,
    guard: Guard,
    source: Option<WorkshopRpc>,
    observation: Option<WorkshopCapture>,
    tree_material: Option<tree_material::Owner>,
    clock: Option<clock::Owner>,
    pub(super) anchor: Option<StateAnchor>,
    authority_tick: u64,
    next_request: u64,
}
fn pin_json(pin: WorkshopPin) -> Value {
    json!({"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames})
}
fn anchor(c: &WorkshopCapture) -> StateAnchor {
    StateAnchor {
        fortress_id: c.fortress_id(),
        cursor: ObservationCursor {
            epoch: 0,
            sequence: c.sequence(),
        },
        tick: GameTick(c.tick()),
        state_hash: c.witness(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn material(c: &WorkshopCapture) -> String {
    format!(
        "workshop-material:{}:{}",
        c.selection().item_id(),
        c.witness().to_hex()
    )
}
fn seal(id: Digest32, p: &WorkshopPlan) -> Digest32 {
    let mut data = b"dfmcp-workshop-mcp-review/1\0".to_vec();
    data.extend_from_slice(id.as_bytes());
    data.extend_from_slice(&p.canonical_bytes());
    Digest32::of_bytes(&data)
}
impl Runtime {
    fn grants(config: &Config, control: bool, clock_control: bool) -> Vec<CapabilityGrant> {
        let mut capabilities = vec![Capability::Query, Capability::Observe];
        if control {
            capabilities.extend([Capability::Plan, Capability::Construct]);
        }
        if clock_control {
            if !control {
                capabilities.push(Capability::Plan);
            }
            capabilities.push(Capability::ControlClock);
        }
        capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(config.identity.fortress_id()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: Some(GameTick(config.input.authority_expires_tick)),
                remaining_uses: None,
            })
            .collect()
    }
    pub(super) fn open(
        config: Config,
        mode: WorkshopMode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        config.current()?;
        let control = mode == WorkshopMode::Control
            && config.input.initialize
            && std::env::var(BUILD).ok().as_deref() == Some("1");
        if config.input.initialize && !control {
            return Err(denied(
                "new workshop custody requires explicit control authority",
            ));
        }
        let context = OperationContext {
            session_id: session,
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: config.identity.fortress_id(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            grants: Self::grants(
                &config,
                control,
                config
                    .clock
                    .as_ref()
                    .is_some_and(|clock| clock.control(mode)),
            ),
            budget,
            cancellation_requested: false,
        };
        let mut work = Work::new(&context, config.view_bytes())?;
        let mut tree_material = if config.input.initialize {
            config
                .tree_material
                .clone()
                .map(|source| tree_material::Owner::open(source, &mut work))
                .transpose()?
        } else {
            // The sealed workshop original is sufficient for reconciliation.
            // Reopening must not discover another material or contact its donor.
            None
        };
        let mut acks = open_private_acknowledgements(
            &config.input.directory.join("workshop.ack"),
            &work.reserve(1024 * 1024)?,
            mode,
            config.input.initialize,
            config.pin,
        )?;
        let mut observation = None;
        let expected = if config.input.initialize {
            let selection = match &mut tree_material {
                Some(donor) => donor.refresh(&config, &mut work)?,
                None => config
                    .selection
                    .ok_or_else(|| invalid("workshop material policy has no selection"))?,
            };
            let mut source = config.source(&work.reserve(CONNECT_BYTES)?, mode)?;
            let c = work.reserve(CONNECT_BYTES)?;
            let observed = source.observe(
                selection,
                &c,
                std::time::Duration::from_millis(c.budget.max_wall_millis),
            )?;
            config.capture(&observed)?;
            work.advance_authorization_tick(observed.tick());
            if let Some(donor) = &mut tree_material {
                donor.check_capture_clock(&observed)?;
                donor.verify(&mut work)?;
            }
            if !observed.paused() {
                return Err(denied("workshop initialization requires a paused source"));
            }
            let b = WorkshopBinding::new(
                config.endpoint,
                &config.input.df_version,
                &config.input.dfhack_version,
                &observed,
            )?;
            observation = Some(observed);
            Some(b)
        } else {
            None
        };
        let opening = work.reserve(32 * 1024 * 1024)?;
        let mut journal = open_private_workshop(
            &config.input.directory.join("workshop.journal"),
            &opening,
            mode,
            expected,
            if config.input.initialize {
                nonce(&opening)?
            } else {
                [0; 32]
            },
            acks.latest(),
        )?;
        config.binding(journal.binding())?;
        let selection = if let Some(observed) = &observation {
            Some(observed.selection())
        } else if let Some(selection) = config.selection {
            Some(selection)
        } else {
            let inventory = journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
            if inventory.entries().len() > 1 {
                return Err(denied("workshop consumer owns exactly one original goal"));
            }
            inventory
                .entries()
                .first()
                .map(|entry| entry.plan().before().selection())
        };
        if selection.is_some_and(|selection| selection.center() != config.input.center) {
            return Err(denied(
                "retained workshop selection differs from the fixed host footprint",
            ));
        }
        if mode != WorkshopMode::Offline {
            acks.acknowledge(journal.pin(), &work.reserve(1024 * 1024)?)?;
        }
        let acks = Arc::new(Mutex::new(acks));
        let mut leases = LeaseManager::new();
        let lease = if control {
            Some(leases.acquire_spatial_lease(
                session,
                config.scope,
                true,
                GameTick(observation.as_ref().map_or(0, WorkshopCapture::tick)),
                config.input.lease_ticks,
            )?)
        } else {
            None
        };
        let current_anchor = observation.as_ref().map(anchor);
        let authority_tick = work.context.anchor.tick.get().max(journal.high_tick());
        let guard = Guard {
            config: config.clone(),
            selection,
            acks,
            leases,
            lease,
            session,
        };
        work.advance_authorization_tick(authority_tick);
        let clock = if config.clock.is_some() {
            Some(clock::Owner::open(&config, mode, &mut work)?)
        } else {
            None
        };
        Ok(Self {
            session,
            mode,
            budget,
            config,
            journal,
            guard,
            source: None,
            observation,
            tree_material,
            clock,
            anchor: current_anchor,
            authority_tick,
            next_request: 2,
        })
    }
    pub(super) fn context(&mut self) -> Result<OperationContext> {
        let request = self.next_request;
        self.next_request = self.next_request.checked_add(1).ok_or_else(exhausted)?;
        // This clock is an authority floor only. The observed native anchor
        // retained in self.anchor and exposed in source_json remains unchanged.
        let mut authority_anchor = self.anchor.unwrap_or(StateAnchor {
            fortress_id: self.config.identity.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(self.journal.high_tick()),
            state_hash: Digest32::ZERO,
        });
        authority_anchor.tick = GameTick(authority_anchor.tick.get().max(self.authority_tick));
        if let Some(donor) = &self.tree_material {
            authority_anchor.tick = GameTick(authority_anchor.tick.get().max(donor.high_tick()));
        }
        if let Some(clock) = &self.clock {
            authority_anchor.tick = GameTick(authority_anchor.tick.get().max(clock.high_tick()));
        }
        Ok(OperationContext {
            session_id: self.session,
            request_id: RequestId::new(u128::from(request)),
            anchor: authority_anchor,
            budget: self.budget,
            grants: Self::grants(
                &self.config,
                self.mode == WorkshopMode::Control && self.config.input.initialize,
                self.config
                    .clock
                    .as_ref()
                    .is_some_and(|clock| clock.control(self.mode)),
            ),
            cancellation_requested: false,
        })
    }
    fn view(&mut self, c: &OperationContext, fresh: bool) -> Result<Value> {
        self.config.current()?;
        let mut authority = c.clone();
        authority.anchor.tick = GameTick(authority.anchor.tick.get().max(self.authority_tick));
        if let Some(donor) = &self.tree_material {
            authority.anchor.tick = GameTick(authority.anchor.tick.get().max(donor.high_tick()));
        }
        let c = &authority;
        if c.budget.max_bytes < self.config.view_bytes() {
            return Err(exhausted());
        }
        let mut inventory_context = c.clone();
        inventory_context.budget.max_bytes = INSPECT_BYTES;
        let inv = self.journal.inventory(&inventory_context)?;
        if inv.entries().len() > 1 {
            return Err(denied("workshop consumer owns exactly one original goal"));
        }
        if self.mode != WorkshopMode::Offline {
            self.guard
                .acknowledge(self.journal.pin(), &inventory_context)?;
        } else {
            self.guard
                .acks
                .try_lock()
                .map_err(|_| denied("workshop acknowledgement busy"))?
                .verify(&inventory_context)?;
        }
        let original=inv.entries().first().map(|e| {
            let p=e.plan(); let receipt=e.native();
            json!({"key":p.key(),"plan_digest":p.digest().to_hex(),"review_seal":seal(inv.journal_id,p).to_hex(),
                "kind":"Workshop/Carpenters","custom_type":-1,"footprint":p.before().selection().footprint(),
                "source":source_json(p.before()),"material":material(p.before()),
                "canonical_plan_hex":hex(&p.canonical_bytes()),"state":e.state().as_str(),
                "native_phase":receipt.map(|r|r.phase().as_str()),"native_receipt":receipt.map(|r|r.receipt().to_hex()),
                "native_record_hex":receipt.map(|r|hex(r.canonical_bytes())),
                "building_id":receipt.and_then(|r|r.insertion()).map(|i|i.building_id()),
                "job_id":receipt.and_then(|r|r.insertion()).map(|i|i.job_id()),
                "placed_at_anchor":e.placed(),"dispatch_started":e.dispatch_started(),"unresolved":e.unresolved(),
                "prepared_in_this_connection":self.journal.has_permit(p.key()) && self.source.is_some(),
                "historical_goal":e.goal().map(WorkshopGoal::as_str),"samples":e.samples(),
                "sample_generation":e.latest_sample().map(|s|s.generation()),"sample_tick":e.latest_sample().map(|s|s.tick()),
                "sample_hex":e.latest_sample().map(|s|hex(s.canonical_bytes())),
                "high_tick":e.high_tick(),"high_generation":e.high_generation(),
                "fresh_goal":if fresh { e.goal().map(WorkshopGoal::as_str) } else {None},
                "fresh_completion_proven":fresh && e.goal_complete()})
        });
        let clock = self.clock_view(&inv, c, fresh)?;
        Ok(
            json!({"ok":true,"mode":self.mode.as_str(),"source_profile":"workshop/1.20",
            "runtime_admitted":false,"automatic_redispatch":false,
            "clock_advanced":if self.clock.is_some() {Value::Null}else{json!(false)},
            "journal_fenced":self.journal.is_fenced(),"acknowledged":pin_json(self.journal.pin()),
            "retained_state_currently_verified":true,"original":original,
            "observation":self.observation.as_ref().map(source_json),"current_goal_available":fresh,
            "material_policy":if self.config.tree_material.is_some() {"tree_supply"} else {"operator_pinned"},
            "tree_material":self.tree_material.as_ref().map(|owner|owner.view(&self.config)),
            "clock":clock,
            "manager_activation_available":false,"finite_bed_output_proven":false}),
        )
    }
    pub(super) fn inspect(&mut self, c: &OperationContext) -> Result<Value> {
        self.view(c, false)
    }
    fn sample_after_clock(&mut self, work: &mut Work) -> Result<bool> {
        let mut source_entered = false;
        let result = (|| {
            let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
            let entry = inventory.entries().first().ok_or_else(|| {
                denied("original workshop unavailable after clock reconciliation")
            })?;
            if !entry.placed() || entry.unresolved() {
                return Err(denied(
                    "only the exact resolved original workshop can be sampled after the clock",
                ));
            }
            if entry.goal_complete() {
                return Ok(false);
            }
            self.source = None;
            self.observation = None;
            let connect = work.reserve(CONNECT_BYTES)?;
            source_entered = true;
            let mut source = self.config.source(&connect, WorkshopMode::Recover)?;
            source.require_progress_at_or_after(self.authority_tick);
            let sampled = self.journal.sample(
                &mut source,
                entry.plan().key(),
                entry.plan().digest(),
                &work.reserve(work.bytes)?,
                &mut self.guard,
            )?;
            self.authority_tick = self.authority_tick.max(sampled.high_tick());
            work.advance_authorization_tick(self.authority_tick);
            let fresh = sampled.samples() > entry.samples();
            if fresh && let Some(progress) = sampled.latest_sample() {
                self.anchor = Some(StateAnchor {
                    fortress_id: progress.fortress().fortress_id(),
                    cursor: ObservationCursor {
                        epoch: 0,
                        sequence: self.next_request,
                    },
                    tick: GameTick(progress.tick()),
                    state_hash: progress.witness(),
                });
            }
            Ok(fresh)
        })();
        if result.is_err()
            && source_entered
            && let Some(clock) = &mut self.clock
        {
            clock.fence_advancement();
        }
        result
    }
    pub(super) fn perform(&mut self, action: Action, c: &OperationContext) -> Result<Value> {
        let mut workshop_source_entered = false;
        let result = (|| {
            self.config.current()?;
            let mut work = Work::new(c, self.config.view_bytes())?;
            work.advance_authorization_tick(self.authority_tick);
            if let Some(donor) = &self.tree_material {
                work.advance_authorization_tick(donor.high_tick());
            }
            let mut fresh = false;
            match action {
                Action::Inspect => {}
                Action::Unavailable => {
                    return Err(denied(
                        "this isolated profile has no checkpoint, restore, manager or activation provider",
                    ));
                }
                Action::Observe => {
                    if self.mode != WorkshopMode::Control
                        || !self
                            .journal
                            .inventory(&work.reserve(INSPECT_BYTES)?)?
                            .entries()
                            .is_empty()
                    {
                        return Err(denied(
                            "an original goal cannot be replaced by another observation or key",
                        ));
                    }
                    self.observe_material(&mut work)?;
                }
                Action::Plan(raw) => {
                    if clock::is_request(&raw)? {
                        self.clock_action(clock::Action::Plan(&raw), &mut work)?;
                        return self.view(&work.current(self.config.view_bytes())?, false);
                    }
                    if raw.len() > 2048 {
                        return Err(exhausted());
                    }
                    let request: PlanInput = serde_json::from_str(&raw)
                        .map_err(|_| invalid("invalid closed original workshop request"))?;
                    let inv = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
                    if !inv.entries().is_empty() {
                        return Err(denied(
                            "original workshop goal already retained; inspect or recover it",
                        ));
                    }
                    if self.mode != WorkshopMode::Control || !self.config.input.initialize {
                        return Err(denied("recovery cannot grant fresh construction authority"));
                    }
                    self.select_material(&request, &mut work)?;
                    let before = self
                        .observation
                        .as_ref()
                        .ok_or_else(|| invalid("observe the original workshop offer first"))?;
                    if !before.workers().iter().any(|w| {
                        w.eligible()
                            && self.config.scope.contains(MapCoord::new(
                                w.position()[0] as i32,
                                w.position()[1] as i32,
                                w.position()[2] as i32,
                            ))
                    }) {
                        return Err(denied(
                            "no observed eligible carpenter within the host scope",
                        ));
                    }
                    let p = WorkshopPlan::new(
                        &format!("workshop-{}", inv.journal_id.to_hex()),
                        before.clone(),
                    )?;
                    let mut source = self
                        .config
                        .source(&work.reserve(CONNECT_BYTES)?, self.mode)?;
                    let current = work.reserve(work.bytes)?;
                    self.journal
                        .prepare(&mut source, &p, &current, &mut self.guard)?;
                    self.source = Some(source);
                    // The immutable Workshop original now owns the chosen item.
                    // Its independent native qualification ends the donor handoff.
                    self.tree_material = None;
                }
                Action::Commit {
                    seal: requested,
                    confirmed,
                } => {
                    let requested_digest = digest(&requested)?;
                    if self.clock.as_ref().and_then(clock::Owner::prepared_seal)
                        == Some(requested_digest)
                    {
                        self.clock_action(
                            clock::Action::Commit {
                                seal: requested_digest,
                                confirmed,
                            },
                            &mut work,
                        )?;
                        return self.view(&work.current(self.config.view_bytes())?, false);
                    }
                    let inv = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
                    let e = inv
                        .entries()
                        .first()
                        .ok_or_else(|| invalid("no original workshop review"))?;
                    if !confirmed {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "review and confirm the exact original workshop seal",
                        ));
                    }
                    if requested_digest != seal(inv.journal_id, e.plan()) {
                        return Err(error(
                            ErrorCode::Conflict,
                            "workshop review seal differs from original goal",
                        ));
                    }
                    let source = self.source.as_mut().ok_or_else(|| {
                        denied("original prepared connection lost; use read-only recovery")
                    })?;
                    let current = work.reserve(work.bytes)?;
                    self.journal.commit(
                        source,
                        e.plan().key(),
                        e.plan().digest(),
                        &current,
                        &mut self.guard,
                    )?;
                    self.source = None;
                    self.observation = None;
                }
                Action::Wait | Action::Cancel => {
                    if self.mode == WorkshopMode::Offline {
                        return Err(denied(
                            "offline workshop mode cannot contact native sources",
                        ));
                    }
                    let cancel = matches!(action, Action::Cancel);
                    if !cancel {
                        self.clock_allows_goal_sample(&mut work)?;
                    }
                    let inv = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
                    let e = inv
                        .entries()
                        .first()
                        .ok_or_else(|| invalid("no original workshop goal to recover"))?;
                    self.source = None;
                    self.observation = None;
                    let connect = work.reserve(CONNECT_BYTES)?;
                    workshop_source_entered = true;
                    let mut source = self.config.source(&connect, WorkshopMode::Recover)?;
                    source.require_progress_at_or_after(self.authority_tick);
                    let current = work.reserve(work.bytes)?;
                    if e.needs_reconciliation() || cancel {
                        self.journal.recover(
                            &mut source,
                            e.plan().key(),
                            e.plan().digest(),
                            cancel,
                            &current,
                            &mut self.guard,
                        )?;
                    } else if e.placed() && !e.goal_complete() {
                        let sampled = self.journal.sample(
                            &mut source,
                            e.plan().key(),
                            e.plan().digest(),
                            &current,
                            &mut self.guard,
                        )?;
                        fresh = sampled.samples() > e.samples();
                        if fresh && let Some(p) = sampled.latest_sample() {
                            self.anchor = Some(StateAnchor {
                                fortress_id: p.fortress().fortress_id(),
                                cursor: ObservationCursor {
                                    epoch: 0,
                                    sequence: self.next_request,
                                },
                                tick: GameTick(p.tick()),
                                state_hash: p.witness(),
                            });
                        }
                    }
                }
                Action::WaitClock | Action::CancelClock => {
                    let cancel = matches!(action, Action::CancelClock);
                    let settled = self.clock_action(
                        if cancel {
                            clock::Action::Cancel
                        } else {
                            clock::Action::Wait
                        },
                        &mut work,
                    )?;
                    if settled && !cancel {
                        fresh = self.sample_after_clock(&mut work)?;
                    }
                }
            }
            self.authority_tick = self.authority_tick.max(self.journal.high_tick());
            self.view(&work.current(self.config.view_bytes())?, fresh)
        })();
        if let Some(donor) = &self.tree_material {
            self.authority_tick = self.authority_tick.max(donor.high_tick());
        }
        self.authority_tick = self.authority_tick.max(self.journal.high_tick());
        if let Some(clock) = &self.clock {
            self.authority_tick = self.authority_tick.max(clock.high_tick());
        }
        if result.is_err() {
            if workshop_source_entered && let Some(clock) = &mut self.clock {
                clock.fence_advancement();
            }
            self.withdraw();
        }
        result
    }
    pub(super) fn withdraw(&mut self) {
        self.observation = None;
        self.anchor = None;
        self.source = None;
        if let Some(donor) = &mut self.tree_material {
            donor.withdraw();
        }
        if let Some(clock) = &mut self.clock {
            clock.withdraw();
        }
        self.journal.abandon();
    }
    pub(super) fn failure_reference(&self) -> Value {
        json!({"journal_pin_unverified":pin_json(self.journal.pin()),"original_custody_currently_verified":false,
            "original":self.journal.unverified_original_reference().map(|e|json!({"key":e.plan().key(),
                "plan_digest":e.plan().digest().to_hex(),"review_seal":seal(self.journal.pin().journal_id,e.plan()).to_hex(),
                "state":e.state().as_str(),"dispatch_started":e.dispatch_started(),"unresolved":e.unresolved(),
                "native_receipt":e.native().map(|r|r.receipt().to_hex()),"current_goal":null,"currently_verified":false})),
            "current_goal_available":false,"automatic_redispatch":false,
            "clock":self.clock.as_ref().map(clock::Owner::reference),
            "recovery":"Keep both original workshop.journal and independent workshop.ack, plus the externally retained acknowledgement. Reopen recover/offline; never construct a replacement key."})
    }
}
pub(super) fn source_json(c: &WorkshopCapture) -> Value {
    let item = match c.item() {
        WorkshopItem::Visible(v) => {
            json!({"presence":"visible","position":v.position(),"native_type":v.native_type(),"material":v.material(),"material_index":v.material_index(),"available_at_anchor":c.item().available()})
        }
        WorkshopItem::Missing => json!({"presence":"absent","available_at_anchor":null}),
        WorkshopItem::Hidden => json!({"presence":"hidden","available_at_anchor":null}),
    };
    json!({"witness":c.witness().to_hex(),"profile":"workshop/1.20","generation":c.generation(),"sequence":c.sequence(),"tick":c.tick(),
        "world_folder":c.folder(),"site_id":c.site(),"map_dimensions":c.dimensions(),"paused_at_anchor":c.paused(),
        "selection":{"kind":"carpenters","item_id":c.selection().item_id(),"center":c.selection().center(),"footprint":c.selection().footprint()},
        "material_handle":material(c),"selected_material":item,"material_inside_footprint":c.item_inside_footprint(),
        "tiles":c.tiles().iter().map(|t|match t {
            WorkshopTile::Visible(v)=>json!({"presence":"visible","visible":true,"shape":v.shape(),"liquid_depth":v.liquid(),"dry":t.dry(),"empty_floor":t.empty_floor(),"building_occupied_at_anchor":v.occupied()}),
            WorkshopTile::Missing=>json!({"presence":"missing","visible":false,"dry":null,"empty_floor":null,"building_occupied_at_anchor":null}),
            WorkshopTile::Hidden=>json!({"presence":"hidden","visible":false,"dry":null,"empty_floor":null,"building_occupied_at_anchor":null}),
        }).collect::<Vec<_>>(),
        "worker_census_complete":c.census_complete(),"workers":c.workers().iter().map(|w|json!({"id":w.id(),"position":w.position(),"visible":w.visible(),"adult":w.adult(),"sane":w.sane(),"carpenter_labor_enabled":w.carpenter_labor_enabled(),"labor_valid":w.labor_valid(),"job_available":w.job_available(),"walk_group_connected":w.reachable(),"eligible_at_anchor":w.eligible()})).collect::<Vec<_>>(),
        "assignment_or_scheduling_proven":false,"eligible_at_anchor":c.eligible(),
        "blockers":c.blockers().iter().map(|b|b.as_str()).collect::<Vec<_>>(),"canonical_capture_hex":hex(c.canonical_bytes())})
}
