//! Closed host configuration and one foreground vacant MANAGER appointment.
use dfmcp_adapter::build_placement::{
    journal::private_file::PrivateBuildFile, rpc::BuildCancellation,
};
use dfmcp_adapter::manager_appointment::{
    Capture, FortressIdentity, Plan, Selection,
    journal::{self, Journal, Mode, Original, State},
    rpc::{Binding, CONNECT_BYTES, ManagerRpc},
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
pub(super) const RESPONSE_BYTES: u64 = 512 * 1024;
const CONFIG: &str = "DFMCP_MANAGER_MCP_CONFIG";
const OPT: &str = "DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22";
// ubs:ignore[rust.security.hardcoded-secrets] — environment-variable name; credentials come from the host.
const TOKEN: &str = "DFMCP_MANAGER_TOKEN";
const WRITE: &str = "DFMCP_MANAGER_ALLOW_APPOINT";
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
        return Err(error(
            ErrorCode::InvalidRequest,
            "noncanonical manager digest",
        ));
    }
    Digest32::from_hex(s).ok_or_else(|| error(ErrorCode::InvalidRequest, "invalid manager digest"))
}
pub(super) fn environment() -> Result<()> {
    if std::env::var(OPT).ok().as_deref() != Some("1")
        || std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        return Err(denied(
            "manager/1.22 requires isolated unadmitted opt-in and refuses production provenance",
        ));
    }
    if std::env::var_os(WRITE).is_some_and(|v| v.to_str() != Some("1")) {
        return Err(denied(
            "manager appointment gate must be absent or exactly 1",
        ));
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.starts_with("DFMCP_") && ![CONFIG, OPT, TOKEN, WRITE].contains(&name.as_ref()) {
            return Err(denied(
                "manager profile refuses other DFMCP and production launch state",
            ));
        }
    }
    Ok(())
}
pub(super) fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current()
        .ok_or_else(|| denied("manager I/O requires supervised runtime context"))?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "manager request cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(denied("manager I/O capability withdrawn"));
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
    map_dimensions: [u32; 3],
    unit_id: u32,
    histfig_id: u32,
    read_scope: [u32; 6],
    scope: [u32; 6],
    authority_expires_tick: u64,
    lease_ticks: u64,
    checkpoint: String,
}
#[derive(Clone)]
pub(super) struct Config {
    input: Input,
    binding: Binding,
    external: Option<WorkshopPin>,
    hash: Digest32,
    scope: MapCuboid,
}
fn area(raw: [u32; 6], d: [u32; 3]) -> Result<MapCuboid> {
    if (0..3).any(|i| raw[i] > raw[i + 3] || raw[i + 3] >= d[i]) {
        return Err(denied(
            "manager scope must be ordered and inside the pinned map",
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
            return Err(error(
                ErrorCode::InvalidRequest,
                "manager configuration bound",
            ));
        }
        let i: Input = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed manager configuration",
            )
        })?;
        let path = i
            .directory
            .to_str()
            .ok_or_else(|| denied("manager custody path must be UTF-8"))?;
        let d = i.map_dimensions;
        if i.schema != "dfmcp.manager-appointment-mcp-development/1"
            || !i.directory.is_absolute()
            || path.len() < 2
            || path.len() > 3800
            || path[1..]
                .split('/')
                .any(|v| v.is_empty() || v == "." || v == ".." || v.chars().any(char::is_control))
            || i.initialize == i.acknowledged.is_some()
            || d.iter().any(|v| *v == 0 || *v > 32768)
            || i.unit_id >= i32::MAX as u32
            || i.histfig_id >= i32::MAX as u32
            || i.read_scope != [0, 0, 0, d[0] - 1, d[1] - 1, d[2] - 1]
            || i.scope != i.read_scope
            || i.authority_expires_tick == 0
            || !(1..=1200).contains(&i.lease_ticks)
            || i.checkpoint != "disposable-fortress-no-checkpoint"
        {
            return Err(denied(
                "manager configuration needs exact source/candidate, explicit registry read scope and disposable-fortress authority",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&i.endpoint)?;
        if endpoint.to_string() != i.endpoint {
            return Err(denied("canonical manager endpoint required"));
        }
        let scope = area(i.scope, d)?;
        let binding = Binding {
            endpoint,
            source: FortressIdentity::new(&i.world_folder, i.site_id)?,
            generation: i.generation,
            dimensions: d,
            scope,
            selection: Selection::new(i.unit_id, i.histfig_id)?,
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
        Ok(Self {
            input: i,
            binding,
            external,
            hash: Digest32::of_bytes(raw.as_bytes()),
            scope,
        })
    }
    pub(super) fn load() -> Result<Self> {
        environment()?;
        Self::parse(
            &std::env::var(CONFIG).map_err(|_| denied("manager operator configuration missing"))?,
        )
    }
    fn current(&self) -> Result<()> {
        runtime_io()?;
        environment()?;
        let raw = std::env::var(CONFIG).map_err(|_| denied("manager configuration withdrawn"))?;
        if Digest32::of_bytes(raw.as_bytes()) != self.hash {
            return Err(denied(
                "manager source or authority changed; original permission withdrawn",
            ));
        }
        Ok(())
    }
    fn connect(&self, c: &OperationContext, mode: Mode) -> Result<ManagerRpc> {
        self.current()?;
        let token = std::env::var(TOKEN)
            .map_err(|_| denied("manager credential unavailable"))?
            .into_bytes();
        let pinned = token.clone();
        let config = self.clone();
        let binding = self.binding.clone();
        ManagerRpc::connect(
            binding,
            token,
            nonce(c, self.scope)?,
            c,
            BuildCancellation::with_check(Arc::new(runtime_io)),
            Box::new(move |write| {
                config.current()?;
                if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&pinned) {
                    return Err(denied("manager credential changed"));
                }
                if write
                    && (mode != Mode::Control
                        || !config.input.initialize
                        || std::env::var(WRITE).ok().as_deref() != Some("1"))
                {
                    return Err(denied("manager appointment permission denied"));
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
            "manager entropy budget exhausted",
        ));
    }
    let mut out = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut out))
        .map_err(|_| denied("manager entropy unavailable"))?;
    if out == [0; 32] {
        return Err(denied("invalid manager entropy"));
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
        capabilities.extend([Capability::Plan, Capability::ConfigureProduction]);
    }
    let grants = capabilities
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
            expires_at_tick: if matches!(
                capability,
                Capability::Plan | Capability::ConfigureProduction
            ) {
                Some(GameTick(config.input.authority_expires_tick))
            } else {
                None
            },
            remaining_uses: None,
        })
        .collect();
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
                "manager request byte work exhausted",
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
                    "original manager request deadline exhausted",
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
    source: Option<ManagerRpc>,
    observation: Option<Capture>,
    leases: LeaseManager,
    lease: Option<LeaseId>,
    next_request: u64,
    high_tick: u64,
    high_sequence: u64,
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
                return Err(denied("new manager custody requires control"));
            }
            let mut source = config.connect(&work.reserve(&initial, CONNECT_BYTES)?, mode)?;
            let c = source.observe(
                config.binding.selection,
                &work.reserve(&initial, CONNECT_BYTES)?,
            )?;
            config.binding.capture(&c)?;
            if !c.paused {
                return Err(denied("manager initialization requires paused source"));
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
            config.binding.selection,
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
        };
        // Initial publication belongs to the same opening operation, not a renewed Query budget.
        let mut current = current;
        current.anchor.tick = GameTick(high_tick);
        runtime.guard(&work.reserve(&current, 32 * 1024 * 1024)?, false)?;
        work.reserve(&current, 0)?;
        let view = runtime.view(false);
        Ok((runtime, view))
    }
    fn withdraw(&mut self) {
        self.observation = None;
        self.journal.abandon();
        if let Some(mut source) = self.source.take() {
            source.fence();
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
            .ok_or_else(|| denied("manager request identity exhausted"))?;
        self.config.current()?;
        self.budget.validate()?;
        Ok(context(
            &self.config,
            self.mode,
            self.session,
            self.budget,
            request,
            self.high_tick.max(self.journal.high_tick),
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
                return Err(denied("manager appointment authority unavailable"));
            }
            let effect = self.config.scope;
            c.authorize(Capability::Plan, RiskTier::Irreversible, &[], Some(effect))?;
            c.authorize(
                Capability::ConfigureProduction,
                RiskTier::Irreversible,
                &[],
                Some(effect),
            )?;
            self.leases.verify_exclusive_spatial(
                self.lease
                    .ok_or_else(|| denied("original manager lease unavailable"))?,
                self.session,
                effect,
                c.anchor.tick,
            )?;
        }
        Ok(())
    }
    fn consume(&mut self, cap: &Capture) -> Result<()> {
        self.config.binding.capture(cap)?;
        if cap.selection != self.config.binding.selection
            || cap.tick < self.high_tick
            || cap.sequence < self.high_sequence
        {
            return Err(denied("manager source/candidate/frontier regressed"));
        }
        self.high_tick = cap.tick;
        self.high_sequence = cap.sequence;
        Ok(())
    }
    pub(super) fn perform(&mut self, action: Action) -> Result<Value> {
        // Fallible entry checks are inside withdrawal handling; restored bytes cannot revive a permit.
        let result = (|| {
            let start = Instant::now();
            let mut c = self.current()?;
            let mut work = TurnWork::new(&c, start)?;
            self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, false)?;

            match action {
                Action::Query => {}
                Action::Observe => {
                    if self.mode == Mode::Offline {
                        return Err(denied("offline manager custody cannot observe"));
                    }
                    self.withdraw();
                    let mut source = self
                        .config
                        .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                    let cap = source.observe(
                        self.config.binding.selection,
                        &work.reserve(&c, CONNECT_BYTES)?,
                    )?;
                    self.consume(&cap)?;
                    self.observation = Some(cap);
                }
                Action::Plan(raw) => {
                    self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, true)?;
                    if raw.len() > 4096 {
                        return Err(denied("manager request bound"));
                    }
                    let request: PlanInput = serde_json::from_str(&raw)
                        .map_err(|_| denied("invalid closed manager plan request"))?;
                    if request.kind != "appoint_vacant_manager" {
                        return Err(denied("manager profile has one vacant MANAGER intent"));
                    }
                    if let Some(original) = &self.journal.original {
                        if original.plan.before.witness() != digest(&request.expected_observation)?
                            || original.deadline_tick != request.deadline_tick
                        {
                            return Err(denied("original manager intent cannot be replaced"));
                        }
                    } else {
                        let observed = self
                            .observation
                            .clone()
                            .ok_or_else(|| denied("observe the original manager offer first"))?;
                        if observed.witness() != digest(&request.expected_observation)?
                            || request.deadline_tick > self.config.input.authority_expires_tick
                        {
                            return Err(denied("manager offer or deadline differs"));
                        }
                        let key = format!("manager-{}", self.journal.pin().journal_id.to_hex());
                        let original = Original::new(
                            Plan::new(&key, observed.clone())?,
                            request.deadline_tick,
                        )?;
                        let mut source = self
                            .config
                            .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                        let current = source.observe(
                            self.config.binding.selection,
                            &work.reserve(&c, CONNECT_BYTES)?,
                        )?;
                        self.consume(&current)?;
                        if current != observed {
                            return Err(denied(
                                "manager assignment/HF/history witness changed before preparation",
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
                            "review the original manager plan and confirm its exact seal",
                        ));
                    }
                    let seal = digest(&seal)?;
                    if self.journal.native.as_ref().is_some_and(|n| n.phase >= 2) {
                        if self
                            .journal
                            .original
                            .as_ref()
                            .is_none_or(|o| o.seal() != seal)
                        {
                            return Err(denied("original manager review seal differs"));
                        }
                        return Ok(self.view(false));
                    }
                    self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, true)?;
                    let original = self
                        .journal
                        .original
                        .clone()
                        .ok_or_else(|| denied("no original manager plan"))?;
                    let mut source = self.source.take().ok_or_else(journal::uncertain)?;
                    // CommitManager performs fresh same-suspender revalidation on this exact prepared connection.
                    self.config.binding.capture(&original.plan.before)?;
                    let config = self.config.clone();
                    let lease = self
                        .lease
                        .ok_or_else(|| denied("manager lease unavailable"))?;
                    let leases = &self.leases;
                    let session = self.session;
                    let scope = self.config.scope;
                    let mut guard = |edge: &OperationContext| {
                        config.current()?;
                        edge.authorize(
                            Capability::ConfigureProduction,
                            RiskTier::Irreversible,
                            &[],
                            Some(scope),
                        )?;
                        leases.verify_exclusive_spatial(lease, session, scope, edge.anchor.tick)?;
                        if std::env::var(WRITE).ok().as_deref() != Some("1") {
                            return Err(denied("manager appointment gate revoked"));
                        }
                        Ok(())
                    };
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
                            .ok_or_else(|| denied("no original manager intent"))?;
                        if self.journal.native.as_ref().is_none_or(|n| n.phase < 2) {
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
                            .ok_or_else(|| denied("no original manager intent"))?;
                        let mut source = self
                            .config
                            .connect(&work.reserve(&c, CONNECT_BYTES)?, self.mode)?;
                        let record = if self.journal.dispatch_started {
                            source
                                .query(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?
                                .ok_or_else(journal::uncertain)?
                        } else {
                            source.cancel(&original.plan, &work.reserve(&c, CONNECT_BYTES)?)?
                        };
                        self.journal.retain(
                            record,
                            &work.reserve(&c, 32 * 1024 * 1024)?,
                            &mut self.acks,
                        )?;
                    }
                }
            }
            self.guard(&work.reserve(&c, 32 * 1024 * 1024)?, false)?;
            Ok(self.view(false))
        })();
        if let Err(e) = &result {
            for (key, value) in &e.details {
                if key == "consumed_manager_tick"
                    && let Ok(tick) = value.parse::<u64>()
                {
                    self.high_tick = self.high_tick.max(tick);
                }
            }
            self.withdraw();
        }
        result
    }
    pub(super) fn reference(&self) -> Value {
        let pin = self.journal.pin();
        json!({"acknowledged":self.acks.latest().map(|a|json!({"journal_id":a.journal_id.to_hex(),"head":a.head.to_hex(),"frames":a.frames})),
            "journal_pin_unverified":{"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames},
            "original_key":self.journal.original.as_ref().map(|o|o.plan.key()),"original_review_seal":self.journal.original.as_ref().map(|o|o.seal().to_hex()),
            "original_plan_digest":self.journal.original.as_ref().map(|o|o.plan.digest.to_hex()),
            "native_phase":self.journal.native.as_ref().map(|n|n.phase),
            "source_identity":{"protocol":"1.22","endpoint":self.config.binding.endpoint.to_string(),"world_folder":self.config.binding.source.folder(),"site_id":self.config.binding.source.site(),"generation":self.config.binding.generation,"df_version":self.config.binding.df_version,"dfhack_version":self.config.binding.dfhack_version,"unit_id":self.config.binding.selection.unit,"histfig_id":self.config.binding.selection.figure},
            "native_anchor":self.journal.native.as_ref().and_then(|n|n.after.as_ref()).or(self.observation.as_ref()).or(self.journal.original.as_ref().map(|o|&o.plan.before)).map(|c|json!({"schema":"dfmcp.manager-native-projection-anchor/1","fortress_id":c.source.fortress_id().to_string(),"generation":c.generation,"sequence":c.sequence,"game_tick":c.tick,"state_hash":c.witness().to_hex(),"canonical_world_anchor":false})),
            "dispatch_started":self.journal.dispatch_started,"uncertain_effect":self.journal.effect_uncertain,
            "consumed_tick_floor":self.high_tick.max(self.journal.high_tick),"automatic_redispatch":false})
    }
    fn view(&self, fresh: bool) -> Value {
        let original=self.journal.original.as_ref().map(|o|json!({"key":o.plan.key(),"plan_digest":o.plan.digest.to_hex(),"review_seal":o.seal().to_hex(),
            "goal":"vacant_MANAGER_appointed_to_exact_offered_citizen_once","unit_id":o.plan.before.selection.unit,"histfig_id":o.plan.before.selection.figure,
            "deadline_tick":o.deadline_tick,"original_source":capture_json(&o.plan.before),"state":format!("{:?}",self.journal.state),
            "native_phase":self.journal.native.as_ref().map(|n|n.phase),"native_receipt":self.journal.native.as_ref().map(|n|n.receipt.to_hex()),
            "prepared_in_this_connection":self.journal.dispatchable()&&self.source.is_some(),"unresolved":matches!(self.journal.state,State::Intent|State::Dispatched|State::Tracking),
            "appointment_proved":self.journal.native.as_ref().is_some_and(|n|n.complete()),
            "after":self.journal.native.as_ref().and_then(|n|n.after.as_ref()).map(capture_json),"fresh_observation":fresh}));
        json!({"ok":true,"source_profile":"manager/1.22","mode":self.mode.as_str(),"retained_state_currently_verified":true,"original":original,
            "observation":if self.journal.original.is_none(){self.observation.as_ref().map(capture_json)}else{None},
            "recommended_deadline_tick":self.observation.as_ref().map(|c|(c.tick+journal::MAX_HORIZON_TICKS).min(self.config.input.authority_expires_tick)),
            "custody":self.reference(),"canonical_world_anchor":null,"canonical_world_continuity":"unestablished",
            "direct_effect_scope":"one existing vacant MANAGER assignment, selected HF POSITION link and one history event; conservative whole-fortress local lease",
            "local_lease_global_fence":false,"clock_advanced":false,"automatic_redispatch":false,"runtime_admitted":false})
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|n| format!("{n:02x}")).collect()
}
fn capture_json(c: &Capture) -> Value {
    json!({"profile":"manager/1.22","witness":c.witness().to_hex(),"native_capture_hex":hex(c.bytes()),"generation":c.generation,"sequence":c.sequence,"tick":c.tick,
        "world_folder":c.source.folder(),"site_id":c.source.site(),"map_dimensions":c.dimensions,"unit_id":c.selection.unit,"histfig_id":c.selection.figure,
        "manager_position_id":c.manager_position,"manager_position_index":c.manager_index,"assignment_id":c.target().map(|(_,a)|a.id),
        "assignment_index":c.target().map(|(i,_)|i),"assignment_flags_hex":c.target().map(|(_,a)|hex(&a.flags)),"eligible_at_anchor":c.eligible(),
        "paused":c.paused,"year":c.year,"year_tick":c.year_tick,"history_count":c.history_count,"next_event_id":c.next_event,"lookup_unit":c.lookup_unit,
        "dual_getters_agree":c.dual_getters,"manager_links":c.manager_links,"existing_expedition_leader":c.expedition_leader,
        "coverage":"stable projected group/civ assignments and definitions, selected HF links, history projection, roster and entity-domain digest; not all-game-field equivalence"})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config_value() -> Value {
        json!({"schema":"dfmcp.manager-appointment-mcp-development/1","directory":"/private/manager-test-custody","endpoint":"127.0.0.1:5000",
            "initialize":true,"acknowledged":null,"world_folder":"region1","site_id":38,"df_version":"test-df","dfhack_version":"test-dfhack","generation":7,
            "map_dimensions":[192,192,187],"unit_id":256,"histfig_id":361,"read_scope":[0,0,0,191,191,186],"scope":[0,0,0,191,191,186],
            "authority_expires_tick":12124800,"lease_ticks":1200,"checkpoint":"disposable-fortress-no-checkpoint"})
    }
    #[test]
    fn manager_host_config_requires_fortress_scope_and_closed_identity() -> Result<()> {
        assert!(Config::parse(&config_value().to_string()).is_ok());
        for (field, value) in [
            ("scope", json!([0, 0, 0, 1, 1, 1])),
            ("read_scope", json!([0, 0, 0, 1, 1, 1])),
            ("role", json!("MAYOR")),
            ("unit_id", json!(u32::MAX)),
            ("endpoint", json!("0.0.0.0:5000")),
            ("directory", json!("/private/../manager")),
            ("checkpoint", json!("skip")),
        ] {
            let mut v = config_value();
            v[field] = value;
            assert!(Config::parse(&v.to_string()).is_err(), "field {field}");
        }
        Ok(())
    }
    #[test]
    fn manager_current_authority_expiry_and_modes_are_explicit() -> Result<()> {
        let config = Config::parse(&config_value().to_string())?;
        let c = context(
            &config,
            Mode::Control,
            SessionId::new(1),
            WorkBudget::CONSERVATIVE_DEFAULT,
            1,
            12124801,
            Digest32::ZERO,
        );
        assert!(
            c.authorize(
                Capability::ConfigureProduction,
                RiskTier::Irreversible,
                &[],
                Some(config.scope)
            )
            .is_err()
        );
        for mode in [Mode::Recover, Mode::Offline] {
            let c = context(
                &config,
                mode,
                SessionId::new(1),
                WorkBudget::CONSERVATIVE_DEFAULT,
                1,
                0,
                Digest32::ZERO,
            );
            assert!(
                c.authorize(
                    Capability::ConfigureProduction,
                    RiskTier::Irreversible,
                    &[],
                    Some(config.scope)
                )
                .is_err()
            );
        }
        Ok(())
    }
}
