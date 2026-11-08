//! Closed host configuration and one foreground root-predicate obligation.
use dfmcp_adapter::build_placement::{
    journal::private_file::PrivateBuildFile, rpc::BuildCancellation,
};
use dfmcp_adapter::tree_harvest::{
    Capture, FortressIdentity, Plan, Progress,
    journal::{self, Journal, Mode, Original, State},
    rpc::{Binding, CONNECT_BYTES, TreeRpc},
};
use dfmcp_adapter::workshop_construction::{
    acknowledgement::WorkshopAcknowledgements, journal::WorkshopPin,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    LeaseId, LeaseManager, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
#[path = "clock.rs"]
mod clock;
pub(super) const RESPONSE_BYTES: u64 = 512 * 1024;
const CONFIG: &str = "DFMCP_TREE_HARVEST_MCP_CONFIG";
const OPT: &str = "DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21";
// ubs:ignore[rust.security.hardcoded-secrets] — environment-variable name; credentials come from the host.
const TOKEN: &str = "DFMCP_TREE_HARVEST_TOKEN";
const WRITE: &str = "DFMCP_TREE_HARVEST_ALLOW_DESIGNATE";
pub(super) fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn denied(message: &str) -> DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
pub(super) fn digest(s: &str) -> Result<Digest32> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(ErrorCode::InvalidRequest, "noncanonical tree digest"));
    }
    Digest32::from_hex(s).ok_or_else(|| error(ErrorCode::InvalidRequest, "invalid tree digest"))
}
pub(super) fn environment() -> Result<()> {
    if std::env::var(OPT).ok().as_deref() != Some("1")
        || std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        return Err(denied(
            "tree/1.21 requires isolated unadmitted opt-in and refuses production provenance",
        ));
    }
    if std::env::var_os(WRITE).is_some_and(|v| v.to_str() != Some("1")) {
        return Err(denied("tree designation gate must be absent or exactly 1"));
    }
    let clock = std::env::var(clock::OPT_IN).ok().as_deref() == Some("1");
    if std::env::var_os(clock::ALLOW).is_some_and(|v| v.to_str() != Some("1")) {
        return Err(denied("tree clock permission must be absent or exactly 1"));
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.starts_with("DFMCP_")
            && ![CONFIG, OPT, TOKEN, WRITE].contains(&name.as_ref())
            && !(clock && [clock::OPT_IN, clock::TOKEN, clock::ALLOW].contains(&name.as_ref()))
        {
            return Err(denied(
                "tree profile refuses other DFMCP and production launch state",
            ));
        }
    }
    Ok(())
}
pub(super) fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current()
        .ok_or_else(|| denied("tree I/O requires supervised runtime context"))?;
    cx.checkpoint()
        .map_err(|_| error(ErrorCode::CancellationRequested, "tree request cancelled"))?;
    if !cx.capabilities().io {
        return Err(denied("tree I/O capability withdrawn"));
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
    root: [u32; 3],
    read_scope: [u32; 6],
    scope: [u32; 6],
    protected: Vec<[u32; 6]>,
    authority_expires_tick: u64,
    lease_ticks: u64,
    checkpoint: String,
    clock: Option<clock::Input>,
}
#[derive(Clone)]
pub(super) struct Config {
    input: Input,
    binding: Binding,
    external: Option<WorkshopPin>,
    hash: Digest32,
    scope: MapCuboid,
    protected: Vec<MapCuboid>,
    clock: Option<clock::Config>,
}
fn area(raw: [u32; 6], d: [u32; 3]) -> Result<MapCuboid> {
    if (0..3).any(|i| raw[i] > raw[i + 3] || raw[i + 3] >= d[i]) {
        return Err(denied(
            "tree scope must be ordered and inside the pinned map",
        ));
    }
    MapCuboid::new(
        MapCoord::new(raw[0] as i32, raw[1] as i32, raw[2] as i32),
        MapCoord::new(raw[3] as i32, raw[4] as i32, raw[5] as i32),
    )
}
impl Config {
    pub(super) fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > 16384 {
            return Err(error(ErrorCode::InvalidRequest, "tree configuration bound"));
        }
        let i: Input = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed tree configuration",
            )
        })?;
        let path = i
            .directory
            .to_str()
            .ok_or_else(|| denied("tree custody path must be UTF-8"))?;
        let d = i.map_dimensions;
        if i.schema != "dfmcp.tree-harvest-mcp-development/1"
            || !i.directory.is_absolute()
            || path.len() < 2
            || path.len() > 3800
            || path[1..]
                .split('/')
                .any(|v| v.is_empty() || v == "." || v == ".." || v.chars().any(char::is_control))
            || i.initialize == i.acknowledged.is_some()
            || d.iter().any(|v| *v == 0 || *v > 32768)
            || (0..3).any(|n| i.root[n] >= d[n])
            || i.read_scope != [0, 0, 0, d[0] - 1, d[1] - 1, d[2] - 1]
            || i.protected.len() > 32
            || i.authority_expires_tick == 0
            || !(1..=1200).contains(&i.lease_ticks)
            || i.checkpoint != "disposable-fortress-no-checkpoint"
            || i.recovery_generation
                .is_some_and(|g| g < i.generation || g == u64::MAX)
        {
            return Err(denied(
                "tree configuration needs exact source/root, explicit registry read scope and disposable-fortress authority",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&i.endpoint)?;
        if endpoint.to_string() != i.endpoint {
            return Err(denied("canonical tree endpoint required"));
        }
        let scope = area(i.scope, d)?;
        let protected = i
            .protected
            .iter()
            .map(|p| area(*p, d))
            .collect::<Result<Vec<_>>>()?;
        let binding = Binding {
            endpoint,
            source: FortressIdentity::new(&i.world_folder, i.site_id)?,
            generation: i.generation,
            dimensions: d,
            scope,
            df_version: i.df_version.clone(),
            dfhack_version: i.dfhack_version.clone(),
        };
        binding.validate()?;
        let external = i
            .acknowledged
            .as_ref()
            .map(|p| {
                Ok(WorkshopPin {
                    journal_id: digest(&p.journal_id)?,
                    head: digest(&p.head)?,
                    frames: p.frames,
                })
            })
            .transpose()?;
        let clock = i
            .clock
            .clone()
            .map(|input| clock::Config::parse(input, &i, endpoint))
            .transpose()?;
        Ok(Self {
            input: i,
            binding,
            external,
            hash: Digest32::of_bytes(raw.as_bytes()),
            scope,
            protected,
            clock,
        })
    }
    pub(super) fn load() -> Result<Self> {
        environment()?;
        let config = Self::parse(
            &std::env::var(CONFIG).map_err(|_| denied("tree operator configuration missing"))?,
        )?;
        config.clock_environment()?;
        Ok(config)
    }
    pub(super) fn has_clock(&self) -> bool {
        self.clock.is_some()
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
                "tree clock requires its independent opt-in and exact operator configuration",
            ));
        }
        Ok(())
    }
    fn current(&self) -> Result<()> {
        runtime_io()?;
        environment()?;
        self.clock_environment()?;
        let raw = std::env::var(CONFIG).map_err(|_| denied("tree configuration withdrawn"))?;
        if Digest32::of_bytes(raw.as_bytes()) != self.hash {
            return Err(denied(
                "tree source or authority changed; original permission withdrawn",
            ));
        }
        Ok(())
    }
    fn connect(&self, c: &OperationContext, mode: Mode) -> Result<TreeRpc> {
        self.current()?;
        let token = std::env::var(TOKEN)
            .map_err(|_| denied("tree credential unavailable"))?
            .into_bytes();
        let pinned = token.clone();
        let config = self.clone();
        let mut binding = self.binding.clone();
        if mode != Mode::Control {
            binding.generation = self.input.recovery_generation.unwrap_or(binding.generation);
        }
        TreeRpc::connect(
            binding,
            token,
            nonce(c, self.scope)?,
            c,
            BuildCancellation::with_check(Arc::new(runtime_io)),
            Box::new(move |write| {
                config.current()?;
                if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&pinned) {
                    return Err(denied("tree credential changed"));
                }
                if write
                    && (mode != Mode::Control
                        || !config.input.initialize
                        || std::env::var(WRITE).ok().as_deref() != Some("1"))
                {
                    return Err(denied("tree designation permission denied"));
                }
                Ok(())
            }),
        )
    }
}
fn nonce(c: &OperationContext, area: MapCuboid) -> Result<[u8; 32]> {
    runtime_io()?;
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(area))?;
    if c.budget.max_bytes < 32 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "tree entropy budget exhausted",
        ));
    }
    let mut out = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut out))
        .map_err(|_| denied("tree entropy unavailable"))?;
    if out == [0; 32] {
        return Err(denied("invalid tree entropy"));
    }
    Ok(out)
}
fn context(
    config: &Config,
    mode: Mode,
    session: SessionId,
    budget: WorkBudget,
    request: u64,
    tick: u64,
    witness: Digest32,
) -> OperationContext {
    let mut capabilities = vec![Capability::Query, Capability::Observe];
    if mode == Mode::Control && config.input.initialize {
        capabilities.extend([Capability::Plan, Capability::Designate]);
    }
    let mut grants = capabilities
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(config.binding.source.fortress_id()),
                map_area: Some(
                    if matches!(capability, Capability::Query | Capability::Observe) {
                        MapCuboid {
                            min: MapCoord::new(0, 0, 0),
                            max: MapCoord::new(
                                (config.binding.dimensions[0] - 1) as i32,
                                (config.binding.dimensions[1] - 1) as i32,
                                (config.binding.dimensions[2] - 1) as i32,
                            ),
                        }
                    } else {
                        config.scope
                    },
                ),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Irreversible,
            expires_at_tick: if matches!(capability, Capability::Plan | Capability::Designate) {
                Some(GameTick(config.input.authority_expires_tick))
            } else {
                None
            },
            remaining_uses: None,
        })
        .collect::<Vec<_>>();
    // Named-fortress clock authority is separate from the scoped designation
    // lease and from Run/1.13's independent NIL-fortress native authority.
    if config
        .clock
        .as_ref()
        .is_some_and(|clock| clock.control(mode))
    {
        grants.extend(
            [Capability::Plan, Capability::ControlClock]
                .into_iter()
                .map(|capability| CapabilityGrant {
                    capability,
                    scope: CapabilityScope {
                        fortress_id: Some(config.binding.source.fortress_id()),
                        ..CapabilityScope::default()
                    },
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: Some(GameTick(config.input.authority_expires_tick)),
                    remaining_uses: None,
                }),
        );
    }
    OperationContext {
        session_id: session,
        request_id: RequestId::new(u128::from(request)),
        anchor: StateAnchor {
            fortress_id: config.binding.source.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(tick),
            state_hash: witness,
        },
        budget,
        grants,
        cancellation_requested: false,
    }
}
pub(super) enum Action {
    Observe,
    Plan(String),
    Commit(String, bool),
    Wait,
    Query,
    Cancel,
    ClockPlan(String),
    ClockCommit(String, bool),
    ClockWait,
    ClockCancel,
}
struct TurnWork {
    start: Instant,
    wall: u64,
    left: u64,
}
impl TurnWork {
    fn new(c: &OperationContext, start: Instant) -> Result<Self> {
        c.budget.validate()?;
        Ok(Self {
            start,
            wall: c.budget.max_wall_millis,
            left: c.budget.max_bytes,
        })
    }
    fn reserve(&mut self, c: &OperationContext, bytes: u64) -> Result<OperationContext> {
        self.left = self.left.checked_sub(bytes).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "tree request byte work exhausted",
            )
        })?;
        let mut current = c.clone();
        current.budget.max_wall_millis = self
            .wall
            .checked_sub(self.start.elapsed().as_millis() as u64)
            .filter(|n| *n > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "original tree request deadline exhausted",
                )
            })?;
        current.budget.max_bytes = bytes;
        Ok(current)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    kind: String,
    expected_observation: String,
    deadline_tick: u64,
}
pub(super) struct Runtime {
    config: Config,
    pub(super) session: SessionId,
    pub(super) mode: Mode,
    pub(super) budget: WorkBudget,
    journal: Journal<PrivateBuildFile>,
    acks: WorkshopAcknowledgements<PrivateBuildFile>,
    source: Option<TreeRpc>,
    observation: Option<Capture>,
    leases: LeaseManager,
    lease: Option<LeaseId>,
    next_request: u64,
    high_tick: u64,
    high_sequence: u64,
    clock: Option<clock::Owner>,
}
impl Runtime {
    pub(super) fn open(
        config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
        start: Instant,
    ) -> Result<(Self, Value)> {
        let initial = context(&config, mode, session, budget, 1, 0, Digest32::ZERO);
        let mut work = TurnWork::new(&initial, start)?;
        config.current()?;
        let mut observation = None;
        if config.input.initialize {
            if mode != Mode::Control {
                return Err(denied("new tree custody requires control"));
            }
            let mut source = config.connect(&work.reserve(&initial, CONNECT_BYTES)?, mode)?;
            let c = source.observe(config.input.root, &work.reserve(&initial, CONNECT_BYTES)?)?;
            config.binding.capture(&c)?;
            if !c.paused {
                return Err(denied("tree initialization requires paused source"));
            }
            observation = Some(c);
        }
        let current = context(
            &config,
            mode,
            session,
            budget,
            1,
            observation.as_ref().map_or(0, |c| c.tick),
            Digest32::ZERO,
        );
        let entropy = if config.input.initialize {
            nonce(&work.reserve(&current, 32)?, config.scope)?
        } else {
            [0; 32]
        };
        let (journal, acks) = journal::open_private(
            &config.input.directory,
            &work.reserve(&current, 32 * 1024 * 1024)?,
            mode,
            &config.binding,
            config.input.root,
            config.input.initialize,
            config.external,
            entropy,
        )?;
        let high_tick = journal
            .high_tick
            .max(observation.as_ref().map_or(0, |c| c.tick));
        let high_sequence = journal
            .high_sequence
            .max(observation.as_ref().map_or(0, |c| c.sequence));
        let mut leases = LeaseManager::new();
        let lease = if mode == Mode::Control && config.input.initialize {
            Some(leases.acquire_spatial_lease(
                session,
                config.scope,
                true,
                GameTick(high_tick),
                config.input.lease_ticks,
            )?)
        } else {
            None
        };
        let mut runtime = Self {
            config,
            session,
            mode,
            budget,
            journal,
            acks,
            source: None,
            observation,
            leases,
            lease,
            next_request: 2,
            high_tick,
            high_sequence,
            clock: None,
        };
        // Initial publication belongs to the same opening operation, not a renewed Query budget.
        let mut current = current;
        current.anchor.tick = GameTick(high_tick);
        if runtime.config.has_clock() {
            runtime.clock = Some(clock::Owner::open(
                &runtime.config,
                mode,
                &mut work,
                &current,
            )?);
        }
        let clock = runtime.clock_view(&mut work, &current, false)?;
        work.reserve(&current, 0)?;
        let mut view = runtime.view(false);
        view["clock"] = clock;
        Ok((runtime, view))
    }
    fn withdraw(&mut self) {
        self.observation = None;
        self.journal.abandon();
        if let Some(mut source) = self.source.take() {
            source.fence();
        }
        if let Some(clock) = &mut self.clock {
            clock.withdraw();
        }
    }
    pub(super) fn withdraw_permission(&mut self) {
        self.withdraw();
    }
    pub(super) fn last_request_id(&self) -> RequestId {
        RequestId::new(u128::from(self.next_request.saturating_sub(1)))
    }
    fn current(&mut self) -> Result<OperationContext> {
        let request = self.next_request;
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| denied("tree request identity exhausted"))?;
        self.config.current()?;
        self.budget.validate()?;
        Ok(context(
            &self.config,
            self.mode,
            self.session,
            self.budget,
            request,
            self.high_tick
                .max(self.journal.high_tick)
                .max(self.clock.as_ref().map_or(0, clock::Owner::high_tick)),
            self.observation
                .as_ref()
                .map_or(Digest32::ZERO, Capture::witness),
        ))
    }
    fn guard(&mut self, c: &OperationContext, write: bool) -> Result<()> {
        self.config.current()?;
        self.journal.verify(c)?;
        self.acks.verify(c)?;
        if write {
            if self.mode != Mode::Control
                || !self.config.input.initialize
                || std::env::var(WRITE).ok().as_deref() != Some("1")
            {
                return Err(denied("tree designation authority unavailable"));
            }
            let effect = if let Some(o) = &self.journal.original {
                o.plan.before.effect_scope()?
            } else {
                self.observation
                    .as_ref()
                    .ok_or_else(|| denied("tree effect geometry unavailable"))?
                    .effect_scope()?
            };
            if !self.config.scope.contains_cuboid(effect)
                || self
                    .config
                    .protected
                    .iter()
                    .any(|p| dfmcp_core::cuboids_intersect(p, &effect))
            {
                return Err(denied(
                    "tree effect exceeds scope or touches protected region",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Irreversible, &[], Some(effect))?;
            c.authorize(
                Capability::Designate,
                RiskTier::Irreversible,
                &[],
                Some(effect),
            )?;
            self.leases.verify_exclusive_spatial(
                self.lease
                    .ok_or_else(|| denied("original tree lease unavailable"))?,
                self.session,
                effect,
                c.anchor.tick,
            )?;
        }
        Ok(())
    }
    fn consume(&mut self, cap: &Capture) -> Result<()> {
        self.config.binding.capture(cap)?;
        if cap.root != self.config.input.root
            || cap.tick < self.high_tick
            || cap.sequence < self.high_sequence
        {
            return Err(denied("tree source/root/frontier regressed"));
        }
        self.high_tick = cap.tick;
        self.high_sequence = cap.sequence;
        Ok(())
    }
    fn sample_goal(&mut self, work: &mut TurnWork, c: &OperationContext) -> Result<bool> {
        if self.mode == Mode::Offline || self.journal.achieved_tick.is_some() {
            return Ok(false);
        }
        self.clock_allows_goal_sample(work, c)?;
        let original = self
            .journal
            .original
            .clone()
            .ok_or_else(|| denied("no original tree intent"))?;
        let mut current = c.clone();
        current.anchor.tick = GameTick(self.high_tick.max(self.journal.high_tick));
        self.journal
            .begin_sample(&work.reserve(&current, 32 * 1024 * 1024)?, &mut self.acks)?;
        let result = (|| {
            let mut source = self
                .config
                .connect(&work.reserve(&current, CONNECT_BYTES)?, self.mode)?;
            source.require_progress_at_or_after(self.high_tick.max(self.journal.high_tick));
            match source.progress(&original.plan, &work.reserve(&current, CONNECT_BYTES)?) {
                Ok(progress) => {
                    self.high_tick = self.high_tick.max(progress.tick);
                    self.high_sequence = self.high_sequence.max(progress.sequence);
                    current.anchor.tick = GameTick(self.high_tick);
                    self.journal.sample(
                        progress,
                        &work.reserve(&current, 32 * 1024 * 1024)?,
                        &mut self.acks,
                    )?;
                    Ok(true)
                }
                Err(cause) => {
                    if let Some(progress) = source.consumed_progress.take() {
                        self.high_tick = self.high_tick.max(progress.tick);
                        self.high_sequence = self.high_sequence.max(progress.sequence);
                        current.anchor.tick = GameTick(self.high_tick);
                        self.journal.consume_uncredited(
                            progress,
                            &work.reserve(&current, 32 * 1024 * 1024)?,
                            &mut self.acks,
                        )?;
                    }
                    Err(cause)
                }
            }
        })();
        if result.is_err()
            && let Some(clock) = &mut self.clock
        {
            clock.fence_advancement();
        }
        result
    }
    pub(super) fn perform(&mut self, action: Action) -> Result<Value> {
        // Fallible entry checks are inside withdrawal handling; restored bytes cannot revive a permit.
        let mut tree_source_entered = false;
        let result = (|| {
            let start = Instant::now();
            let mut c = self.current()?;
            let mut work = TurnWork::new(&c, start)?;
            self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, false)?;
            let mut fresh = false;
            match action {
                Action::Query => {}
                Action::Observe => {
                    if self.mode == Mode::Offline {
                        return Err(denied("offline tree custody cannot observe"));
                    }
                    self.clock_allows_goal_sample(&mut work, &c)?;
                    self.withdraw();
                    tree_source_entered = true;
                    let mut source = self
                        .config
                        .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                    let cap = source
                        .observe(self.config.input.root, &work.reserve(&c, CONNECT_BYTES)?)?;
                    self.consume(&cap)?;
                    self.observation = Some(cap);
                }
                Action::Plan(raw) => {
                    self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, true)?;
                    if raw.len() > 4096 {
                        return Err(denied("tree request bound"));
                    }
                    let request: PlanInput = serde_json::from_str(&raw)
                        .map_err(|_| denied("invalid closed tree plan request"))?;
                    if request.kind != "fell_one_visible_tree" {
                        return Err(denied("tree profile has one root-predicate intent"));
                    }
                    if let Some(original) = &self.journal.original {
                        if original.plan.before.witness() != digest(&request.expected_observation)?
                            || original.deadline_tick != request.deadline_tick
                        {
                            return Err(denied("original tree intent cannot be replaced"));
                        }
                    } else {
                        let observed = self
                            .observation
                            .clone()
                            .ok_or_else(|| denied("observe the original tree offer first"))?;
                        if observed.witness() != digest(&request.expected_observation)?
                            || request.deadline_tick > self.config.input.authority_expires_tick
                        {
                            return Err(denied("tree offer or deadline differs"));
                        }
                        let key = format!("tree-{}", self.journal.pin().journal_id.to_hex());
                        let original = Original::new(
                            Plan::new(&key, observed.clone())?,
                            request.deadline_tick,
                        )?;
                        tree_source_entered = true;
                        let mut source = self
                            .config
                            .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                        let current = source
                            .observe(self.config.input.root, &work.reserve(&c, CONNECT_BYTES)?)?;
                        self.consume(&current)?;
                        if current != observed {
                            return Err(denied(
                                "tree/actor/tool witness changed before preparation",
                            ));
                        }
                        c.anchor.tick = GameTick(current.tick);
                        self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, true)?;
                        self.journal.intent(
                            original.clone(),
                            &work.reserve(&c, 32 * 1024 * 1024)?,
                            &mut self.acks,
                        )?;
                        let record =
                            source.prepare(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?;
                        self.journal.prepared(
                            record,
                            &work.reserve(&c, 32 * 1024 * 1024)?,
                            &mut self.acks,
                        )?;
                        self.source = Some(source);
                    }
                }
                Action::Commit(seal, confirmed) => {
                    if !confirmed {
                        return Err(denied(
                            "review the original tree plan and confirm its exact seal",
                        ));
                    }
                    self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, true)?;
                    let seal = digest(&seal)?;
                    let original = self
                        .journal
                        .original
                        .clone()
                        .ok_or_else(|| denied("no original tree plan"))?;
                    let mut source = self.source.take().ok_or_else(journal::uncertain)?;
                    // CommitTreeHarvest performs fresh same-suspender revalidation on this exact prepared connection.
                    self.config.binding.capture(&original.plan.before)?;
                    let config = self.config.clone();
                    let lease = self.lease.ok_or_else(|| denied("tree lease unavailable"))?;
                    let leases = &self.leases;
                    let session = self.session;
                    let scope = original.plan.before.effect_scope()?;
                    let mut guard = |edge: &OperationContext| {
                        config.current()?;
                        edge.authorize(
                            Capability::Designate,
                            RiskTier::Irreversible,
                            &[],
                            Some(scope),
                        )?;
                        leases.verify_exclusive_spatial(lease, session, scope, edge.anchor.tick)?;
                        if std::env::var(WRITE).ok().as_deref() != Some("1") {
                            return Err(denied("tree designation gate revoked"));
                        }
                        Ok(())
                    };
                    // Reject an already expired original connection before claiming
                    // dispatch. No deadline/permit is renewed by this preflight.
                    tree_source_entered = true;
                    source.preflight_commit(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?;
                    let record = self.journal.dispatch(
                        seal,
                        &work.reserve(&c, 32 * 1024 * 1024)?,
                        &mut self.acks,
                        &mut guard,
                        |permit, edge| source.commit(permit, edge),
                    )?;
                    if record.phase == 2 {
                        self.observation = None;
                    }
                }
                Action::Wait => {
                    self.journal.abandon();
                    self.source = None;
                    if self.mode != Mode::Offline {
                        let original = self
                            .journal
                            .original
                            .clone()
                            .ok_or_else(|| denied("no original tree intent"))?;
                        if self.journal.native.as_ref().is_none_or(|n| n.phase < 2) {
                            tree_source_entered = true;
                            let mut source = self
                                .config
                                .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                            if let Some(record) =
                                source.query(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?
                            {
                                self.journal.retain(
                                    record,
                                    &work.reserve(&c, 32 * 1024 * 1024)?,
                                    &mut self.acks,
                                )?;
                            }
                        }
                        if self.journal.native.as_ref().is_some_and(|n| n.phase == 2)
                            && self.journal.achieved_tick.is_none()
                        {
                            fresh = self.sample_goal(&mut work, &c)?;
                        }
                    }
                }
                Action::Cancel => {
                    self.journal.abandon();
                    self.source = None;
                    if self.mode != Mode::Offline
                        && self.journal.native.as_ref().is_none_or(|n| n.phase < 2)
                    {
                        let original = self
                            .journal
                            .original
                            .clone()
                            .ok_or_else(|| denied("no original tree intent"))?;
                        tree_source_entered = true;
                        let mut source = self
                            .config
                            .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                        let record =
                            source.cancel(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?;
                        self.journal.retain(
                            record,
                            &work.reserve(&c, 32 * 1024 * 1024)?,
                            &mut self.acks,
                        )?;
                    }
                }
                Action::ClockPlan(raw) => {
                    self.clock_action(clock::Action::Plan(&raw), &mut work, &c)?;
                }
                Action::ClockCommit(seal, confirmed) => {
                    self.clock_action(
                        clock::Action::Commit {
                            seal: digest(&seal)?,
                            confirmed,
                        },
                        &mut work,
                        &c,
                    )?;
                }
                Action::ClockWait => {
                    if self.clock_action(clock::Action::Wait, &mut work, &c)? {
                        fresh = self.sample_goal(&mut work, &c)?;
                    }
                }
                Action::ClockCancel => {
                    self.clock_action(clock::Action::Cancel, &mut work, &c)?;
                }
            }
            let clock = self.clock_view(&mut work, &c, fresh)?;
            let mut view = self.view(fresh);
            view["clock"] = clock;
            Ok(view)
        })();
        if let Err(e) = &result {
            for (key, value) in &e.details {
                if key == "consumed_tree_tick"
                    && let Ok(tick) = value.parse::<u64>()
                {
                    self.high_tick = self.high_tick.max(tick);
                }
            }
            if tree_source_entered && let Some(clock) = &mut self.clock {
                clock.fence_advancement();
            }
            self.withdraw();
        }
        result
    }
    pub(super) fn reference(&self) -> Value {
        let pin = self.journal.pin();
        let mut reference = json!({"acknowledged":self.acks.latest().map(|a|json!({"journal_id":a.journal_id.to_hex(),"head":a.head.to_hex(),"frames":a.frames})),
            "journal_pin_unverified":{"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames},
            "original_key":self.journal.original.as_ref().map(|o|o.plan.key()),"original_review_seal":self.journal.original.as_ref().map(|o|o.seal().to_hex()),
            "dispatch_started":self.journal.dispatch_started,"uncertain_effect":self.journal.effect_uncertain,
            "consumed_tick_floor":self.high_tick.max(self.journal.high_tick)
                .max(self.clock.as_ref().map_or(0,clock::Owner::high_tick)),"automatic_redispatch":false});
        if let Some(clock) = &self.clock {
            reference["clock"] = clock.reference();
        }
        reference
    }
    fn view(&self, fresh: bool) -> Value {
        let original=self.journal.original.as_ref().map(|o|json!({"key":o.plan.key(),"plan_digest":o.plan.digest.to_hex(),"review_seal":o.seal().to_hex(),
            "goal":"root_coordinate_predicate_absent_and_new_eligible_ground_wood_observed","stable_plant_id":null,"causal_item_provenance":"unknown",
            "root":o.plan.before.root,"species":o.plan.before.species,"deadline_tick":o.deadline_tick,"poll_interval_ticks":10,"stable_samples":1,
            "original_source":capture_json(&o.plan.before),"state":format!("{:?}",self.journal.state),"native_phase":self.journal.native.as_ref().map(|n|n.phase),
            "native_receipt":self.journal.native.as_ref().map(|n|n.receipt.to_hex()),"prepared_in_this_connection":self.journal.dispatchable()&&self.source.is_some(),
            "unresolved":matches!(self.journal.state,State::Intent|State::Dispatched|State::Tracking)||self.journal.sample_pending,"sample_publication_pending":self.journal.sample_pending,"designation_acknowledged":self.journal.native.as_ref().is_some_and(|n|n.phase==2),
            "historical_supply_observed_tick":self.journal.achieved_tick,"fresh_supply_observed":fresh&&self.journal.achieved_tick.is_some(),
            "continuity_lost":self.journal.continuity_lost,"samples":self.journal.samples,"latest_progress":self.journal.latest.as_ref().map(progress_json)}));
        json!({"ok":true,"source_profile":"tree-harvest/1.21","mode":self.mode.as_str(),"retained_state_currently_verified":true,"original":original,
            "observation":if self.journal.original.is_none(){self.observation.as_ref().map(capture_json)}else{None},"recommended_deadline_tick":self.observation.as_ref().map(|c|(c.tick+journal::MAX_HORIZON_TICKS).min(self.config.input.authority_expires_tick)),"custody":self.reference(),"canonical_world_anchor":null,"canonical_world_continuity":"unestablished",
            "direct_effect_scope":"observed tree/halo plus complete designation scheduling block","deferred_clock_exposure":"separately authorized; fallout unproved","local_lease_global_fence":false,"visibility_policy":"complete_tree_body_and_horizontal_halo_visible; belowground roots unobserved",
            "tool_assignment_proven":false,"clock_advanced":if self.config.has_clock(){Value::Null}else{json!(false)},"max_game_ticks":self.budget.max_game_ticks,
            "automatic_redispatch":false,"runtime_admitted":false})
    }
}

#[cfg(test)]
#[path = "clock_runtime_tests.rs"]
mod clock_runtime_tests;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|n| format!("{n:02x}")).collect()
}
fn capture_json(c: &Capture) -> Value {
    json!({"profile":"tree-harvest/1.21","witness":c.witness().to_hex(),"native_capture_hex":hex(c.bytes()),"generation":c.generation,"sequence":c.sequence,"tick":c.tick,
        "world_folder":c.source.folder(),"site_id":c.source.site(),"map_dimensions":c.dimensions,"root":c.root,"stable_plant_id":null,"species":c.species,
        "computed_designation":c.designation,"tree_dimensions":c.size,"visible_domain_complete":c.complete,"eligible_at_anchor":c.eligible(),"next_item_id":c.next_item,
        "observed_eligible_workers":c.workers.iter().filter(|w|w.eligible()).map(|w|w.id).collect::<Vec<_>>(),
        "tool_offers":c.tools.iter().map(|t|json!({"item_id":t.id,"holder_id":t.holder,"role":t.role,"flags":t.flags,"offered_at_anchor":t.offered(),"equipment_status":"unestablished","assignment":"unknown"})).collect::<Vec<_>>()})
}
fn progress_json(p: &Progress) -> Value {
    json!({"generation":p.generation,"tick":p.tick,"root_predicate":p.root_predicate,"stable_plant_id":null,"nearby_felling_jobs":p.nearby_felling_jobs,
    "new_eligible_ground_wood":p.wood.iter().map(|w|json!({"item_id":w.id,"position":w.position,"material":w.material,"material_index":w.material_index,"flags":w.flags})).collect::<Vec<_>>(),
    "causal_item_provenance":"unknown","workshop_1_20_revalidation_required":true})
}
