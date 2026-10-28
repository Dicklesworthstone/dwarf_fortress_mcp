//! Operator-owned native composition for the original furniture goal runtime.
//! Every path, endpoint, credential, source pin and exception stays out of MCP input.

use super::policy::SourceAcceptance;
use super::*;
use dfmcp_adapter::build_placement::journal::private_file::{
    PrivateBuildFile, open_private_build_exact,
};
use dfmcp_adapter::build_placement::journal::{BuildDispatch, BuildStage};
use dfmcp_adapter::build_placement::rpc::{BuildCancellation, BuildPermission, BuildRpc};
use dfmcp_adapter::build_placement::{
    BuildKind, BuildNativeSummary, BuildPlan, BuildPreparation, BuildRecord, FortressIdentity,
};
use dfmcp_adapter::live_jobs_rpc::operations::map::spatial::{
    SpatialLimits, citizens::CitizenSpatialLimits,
};
use dfmcp_adapter::live_jobs_rpc::operations::paged::PagedOperationsLimits;
use dfmcp_adapter::plan_archive::{
    create_private_plan_archive, open_private_plan_archive_at_authority_floor,
};
use dfmcp_adapter::semantic_furniture::evidence_owner::{
    SpatialFurnitureEvidenceIo, SpatialFurnitureEvidenceOwner, SpatialFurnitureEvidenceSource,
    SpatialFurnitureRpcSource, evidence_reserve_bytes,
};
use dfmcp_adapter::semantic_furniture::goal_history::open_private_goal_history_store;
use dfmcp_adapter::semantic_furniture::goal_monitor::NativeFurnitureGoalSource;
use dfmcp_adapter::semantic_furniture::open_private_association_store;
use dfmcp_core::{LeaseId, LeaseManager, MapCoord, MapCuboid, cuboids_intersect};
use dfmcp_world::map_region::Region;
use serde::Deserialize;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[path = "native_clock.rs"]
mod clock;
pub use clock::PinnedClock;
use clock::{ClockConfig, ClockInput};

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_SEMANTIC_FURNITURE";
const CONFIG: &str = "DFMCP_SEMANTIC_FURNITURE_CONFIG";
const PLACE: &str = "DFMCP_SEMANTIC_FURNITURE_ALLOW_PLACE";
const SPATIAL_TOKEN: &str = "DFMCP_SEMANTIC_FURNITURE_SPATIAL_TOKEN";
const BUILD_TOKEN: &str = "DFMCP_SEMANTIC_FURNITURE_BUILD_TOKEN";
const OPERATIONS_TOKEN: &str = "DFMCP_SEMANTIC_FURNITURE_OPERATIONS_TOKEN";
const CLOCK_AUTHORITY: &str = "DFMCP_SEMANTIC_FURNITURE_ALLOW_CLOCK";
const CLOCK_TOKEN: &str = "DFMCP_SEMANTIC_FURNITURE_CLOCK_TOKEN";
const ALLOWED: [&str; 8] = [
    OPT_IN,
    CONFIG,
    PLACE,
    SPATIAL_TOKEN,
    BUILD_TOKEN,
    OPERATIONS_TOKEN,
    CLOCK_AUTHORITY,
    CLOCK_TOKEN,
];
const MAX_CONFIG_BYTES: usize = 16 * 1024;
const SPATIAL_READ_BYTES: u64 = 192 * MIB;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorInput {
    fortress_id: String,
    epoch: u64,
    sequence: u64,
    tick: u64,
    state_hash: String,
}
impl AnchorInput {
    fn decode(&self) -> Result<StateAnchor> {
        let fortress = self
            .fortress_id
            .parse::<u64>()
            .map_err(|_| invalid("invalid recovery fortress identity"))?;
        if fortress == 0
            || fortress.to_string() != self.fortress_id
            || self.epoch >= u64::from(u32::MAX)
        {
            return Err(invalid("invalid external furniture recovery anchor"));
        }
        Ok(StateAnchor {
            fortress_id: FortressId::new(fortress),
            cursor: ObservationCursor {
                epoch: self.epoch,
                sequence: self.sequence,
            },
            tick: GameTick(self.tick),
            state_hash: digest(&self.state_hash)?,
        })
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    journal_id: String,
    plan_digest: String,
    object_digest: String,
    anchor: AnchorInput,
    #[serde(default)]
    history_present: bool,
    #[serde(default)]
    archive_publication_unknown: bool,
    #[serde(default)]
    paired_preparation_present: bool,
    clock_journal_id: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigInput {
    schema: String,
    directory: PathBuf,
    endpoint: String,
    initialize: bool,
    journal_id: Option<String>,
    sources: SourceAcceptance,
    scope: [u32; 6],
    #[serde(default)]
    protected: Vec<[u32; 6]>,
    checkpoint: Option<String>,
    recovery: Option<RecoveryInput>,
    clock: Option<ClockInput>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Checkpoint {
    Required,
    Disposable,
}
#[derive(Clone)]
pub struct Config {
    directory: PathBuf,
    endpoint: SocketAddr,
    initialize: bool,
    journal_id: Option<Digest32>,
    sources: SourceAcceptance,
    scope: MapCuboid,
    protected: Vec<MapCuboid>,
    region: Region,
    checkpoint: Checkpoint,
    recovery: Option<Recovery>,
    clock: Option<ClockConfig>,
    environment_digest: Digest32,
    runtime_check: Option<fn() -> Result<()>>,
}
fn area(raw: [u32; 6], dimensions: [u32; 3]) -> Result<MapCuboid> {
    if (0..3).any(|i| raw[i] > raw[i + 3] || raw[i + 3] >= dimensions[i]) {
        return Err(invalid(
            "furniture scope must be ordered and inside pinned map dimensions",
        ));
    }
    MapCuboid::new(
        MapCoord::new(raw[0] as i32, raw[1] as i32, raw[2] as i32),
        MapCoord::new(raw[3] as i32, raw[4] as i32, raw[5] as i32),
    )
}
impl Config {
    pub fn from_environment() -> Result<Self> {
        Self::validate_environment()?;
        Self::parse(
            &std::env::var(CONFIG)
                .map_err(|_| denied("operator furniture configuration is missing"))?,
        )
    }
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
            return Err(invalid("operator furniture configuration exceeds 16 KiB"));
        }
        let input: ConfigInput = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed operator furniture configuration"))?;
        input.sources.validate()?;
        let path = input
            .directory
            .to_str()
            .ok_or_else(|| invalid("custody directory must be UTF-8"))?;
        if input.schema != "dfmcp.semantic-furniture-development/1"
            || !input.directory.is_absolute()
            || path.len() < 2
            || path.len() > 3800
            || path[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\0'))
            || input.initialize == input.journal_id.is_some()
            || (input.initialize && input.recovery.is_some())
            || input.protected.len() > 32
        {
            return Err(invalid(
                "invalid furniture schema, custody directory or exact initialization/recovery identity",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&input.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != input.endpoint {
            return Err(invalid(
                "furniture endpoint requires canonical numeric IPv4 loopback and nonzero port",
            ));
        }
        let scope = area(input.scope, input.sources.map_dimensions)?;
        let protected = input
            .protected
            .into_iter()
            .map(|region| area(region, input.sources.map_dimensions))
            .collect::<Result<Vec<_>>>()?;
        let region = Region {
            origin: [input.scope[0], input.scope[1], input.scope[2]],
            size: [
                input.scope[3] - input.scope[0] + 1,
                input.scope[4] - input.scope[1] + 1,
                input.scope[5] - input.scope[2] + 1,
            ],
        };
        region
            .volume()
            .map_err(|_| invalid("spatial furniture scope exceeds the bounded region profile"))?;
        let checkpoint = match input.checkpoint.as_deref() {
            None | Some("required") => Checkpoint::Required,
            Some("disposable-fortress-no-checkpoint") => Checkpoint::Disposable,
            _ => {
                return Err(invalid(
                    "checkpoint must be required or disposable-fortress-no-checkpoint",
                ));
            }
        };
        let journal_id = input.journal_id.as_deref().map(digest).transpose()?;
        let clock = input
            .clock
            .map(|clock| clock.validate(input.initialize))
            .transpose()?;
        let has_recovery = input.recovery.is_some();
        let recovery_clock = input
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.clock_journal_id.as_deref())
            .map(digest)
            .transpose()?;
        let recovery = input
            .recovery
            .map(|input| {
                // Copyable handoff diagnostics never recreate a live preparation or
                // excuse a missing/mismatched exact archive.
                let _ = (
                    input.archive_publication_unknown,
                    input.paired_preparation_present,
                );
                let recovery = Recovery {
                    journal_id: digest(&input.journal_id)?,
                    seal: PlanArchiveSeal {
                        plan_digest: digest(&input.plan_digest)?,
                        object_digest: digest(&input.object_digest)?,
                    },
                    anchor: input.anchor.decode()?,
                    history_present: input.history_present,
                };
                if Some(recovery.journal_id) != journal_id {
                    return Err(denied(
                        "furniture recovery and configured journal identities differ",
                    ));
                }
                Ok(recovery)
            })
            .transpose()?;
        if recovery.is_some_and(|r| r.anchor.fortress_id != input.sources.fortress()) {
            return Err(denied(
                "furniture recovery belongs to another accepted fortress",
            ));
        }
        if has_recovery {
            match (&clock, recovery_clock) {
                (Some(clock), Some(id)) if clock.journal_id() == Some(id) => {}
                (None, None) => {}
                _ => {
                    return Err(denied(
                        "original furniture recovery must retain the exact configured clock journal identity",
                    ));
                }
            }
        }
        Ok(Self {
            directory: input.directory,
            endpoint,
            initialize: input.initialize,
            journal_id,
            sources: input.sources,
            scope,
            protected,
            region,
            checkpoint,
            recovery,
            clock,
            environment_digest: Digest32::of_bytes(raw.as_bytes()),
            runtime_check: None,
        })
    }
    pub fn validate_environment() -> Result<()> {
        if std::env::var(OPT_IN).ok().as_deref() != Some("1") {
            return Err(denied(
                "explicit unadmitted semantic furniture opt-in is required",
            ));
        }
        if let Some(value) = std::env::var_os(PLACE)
            && value.to_str() != Some("1")
        {
            return Err(denied(
                "operator placement enablement must be absent or exactly 1",
            ));
        }
        if let Some(value) = std::env::var_os(CLOCK_AUTHORITY)
            && value.to_str() != Some("1")
        {
            return Err(denied(
                "independent operator clock enablement must be absent or exactly 1",
            ));
        }
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key.starts_with("DFMCP_") && !ALLOWED.contains(&key.as_ref()) {
                return Err(denied(
                    "semantic furniture rejects production launch and other DFMCP profile state",
                ));
            }
        }
        Ok(())
    }
    pub fn placement_enabled() -> bool {
        std::env::var(PLACE).ok().as_deref() == Some("1")
    }
    pub fn clock_enabled() -> bool {
        std::env::var(CLOCK_AUTHORITY).ok().as_deref() == Some("1")
    }
    fn current(&self) -> Result<()> {
        if let Some(check) = self.runtime_check {
            check()?;
        }
        Self::validate_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator furniture source configuration was removed"))?;
        if raw.len() > MAX_CONFIG_BYTES
            || Digest32::of_bytes(raw.as_bytes()) != self.environment_digest
        {
            return Err(denied(
                "operator furniture source acceptance changed; close this session",
            ));
        }
        Ok(())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    fn limits(&self) -> CitizenSpatialLimits {
        CitizenSpatialLimits {
            spatial: SpatialLimits {
                operations: PagedOperationsLimits::default(),
                region: self.region,
            },
            citizens: 4096,
        }
    }
    fn binding(&self, binding: &BuildBinding) -> Result<()> {
        self.sources.binding(binding)?;
        if binding.endpoint() != self.endpoint {
            return Err(denied(
                "native furniture endpoint differs from exact operator pins",
            ));
        }
        Ok(())
    }
    fn fortress_identity(&self) -> Result<FortressIdentity> {
        FortressIdentity::new(&self.sources.world_folder, self.sources.site_id)
    }
    fn original_custody(&self) -> Result<()> {
        fn present(path: PathBuf) -> Result<bool> {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() => Ok(true),
                Ok(_) => Err(denied(
                    "furniture custody must be regular files, never links or special files",
                )),
                Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(_) => Err(denied("furniture custody presence could not be determined")),
            }
        }
        let clock = present(self.path("clock.journal"))?;
        match (&self.clock, clock) {
            (None, false) => {}
            (Some(_), present) if present != self.initialize => {}
            _ => {
                return Err(denied(
                    "clock custody requires exact initialization or externally identified reopening before furniture custody is opened",
                ));
            }
        }
        let journal = present(self.path("furniture.journal"))?;
        let associations = present(self.path("semantic.associations"))?;
        if self.initialize == journal || self.initialize == associations {
            return Err(denied(
                "furniture initialization or reopening differs from existing paired custody",
            ));
        }
        let plan = present(self.path("original.plan"))?;
        let history = present(self.path("goal.history"))?;
        match self.recovery {
            None if !plan && !history => Ok(()),
            Some(recovery) if plan && history == recovery.history_present => Ok(()),
            _ => Err(denied(
                "original furniture files require the exact external recovery handoff and history-presence flag",
            )),
        }
    }
    fn selection(&self, selection: BuildSelection) -> Result<MapCuboid> {
        let [x, y, z] = selection.target().map(|n| n as i32);
        let halo = MapCuboid::new(
            MapCoord::new(x - 1, y - 1, z),
            MapCoord::new(x + 1, y + 1, z),
        )?;
        if !self.scope.contains_cuboid(halo)
            || self
                .protected
                .iter()
                .any(|protected| cuboids_intersect(protected, &halo))
        {
            return Err(denied(
                "furniture target halo leaves the operator scope or intersects a protected region",
            ));
        }
        Ok(halo)
    }
    fn cancellation(self: &Arc<Self>) -> BuildCancellation {
        let config = Arc::clone(self);
        BuildCancellation::with_check(Arc::new(move || config.current()))
    }
    fn build_permission(self: &Arc<Self>, control: bool, pinned: Vec<u8>) -> BuildPermission {
        let config = Arc::clone(self);
        Box::new(move |write| {
            config.current()?;
            if token(BUILD_TOKEN)? != pinned
                || (write
                    && (!control
                        || !Self::placement_enabled()
                        || config.checkpoint != Checkpoint::Disposable))
            {
                return Err(denied(
                    "operator construction credential, permission or checkpoint exception was revoked",
                ));
            }
            Ok(())
        })
    }
    fn spatial(
        self: &Arc<Self>,
        c: &OperationContext,
    ) -> Result<SpatialFurnitureRpcSource<NativeSpatialIo>> {
        self.current()?;
        let pinned = token(SPATIAL_TOKEN)?;
        let end = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("spatial connection deadline overflow"))?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let timeout = end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| exhausted("spatial connection deadline exhausted"))?;
        let stream = TcpStream::connect_timeout(&self.endpoint, timeout)
            .map_err(|_| denied("spatial furniture connection failed"))?;
        stream
            .set_nodelay(true)
            .map_err(|_| denied("spatial furniture socket configuration failed"))?;
        self.current()?;
        let mut current = c.clone();
        current.budget.max_wall_millis = u64::try_from(
            end.checked_duration_since(Instant::now())
                .ok_or_else(|| exhausted("spatial connection deadline exhausted"))?
                .as_millis(),
        )
        .map_err(|_| exhausted("spatial deadline overflow"))?;
        let connection_nonce = nonce(&mut current)?;
        SpatialFurnitureRpcSource::negotiate_until(
            NativeSpatialIo {
                stream,
                config: Arc::clone(self),
                pinned: pinned.clone(),
            },
            pinned,
            connection_nonce.to_vec(),
            self.limits(),
            &current,
            end,
        )
    }
}
fn token(name: &str) -> Result<Vec<u8>> {
    let value = std::env::var(name)
        .map_err(|_| denied("required operator native credential is absent or not UTF-8"))?;
    if !(32..=256).contains(&value.len()) || value.contains('\0') {
        return Err(invalid(
            "operator native credential exceeds its fixed bound",
        ));
    }
    Ok(value.into_bytes())
}
fn nonce(c: &mut OperationContext) -> Result<[u8; 32]> {
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if c.budget.max_bytes < 32 {
        return Err(exhausted("native nonce exceeds connection allowance"));
    }
    c.budget.max_bytes -= 32;
    let mut nonce = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut nonce))
        .map_err(|_| denied("native connection entropy unavailable"))?;
    if nonce == [0; 32] {
        return Err(denied("native connection nonce is reserved"));
    }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(nonce)
}

/// Current runtime/configuration/credential checks occur at every bounded wire edge.
pub struct NativeSpatialIo {
    stream: TcpStream,
    config: Arc<Config>,
    pinned: Vec<u8>,
}
impl Read for NativeSpatialIo {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.read(out)
    }
}
impl Write for NativeSpatialIo {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.stream.write(input)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
impl SpatialFurnitureEvidenceIo for NativeSpatialIo {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.config
            .current()
            .map_err(|_| io::Error::other("spatial operator authority changed"))?;
        let current = token(SPATIAL_TOKEN)
            .map_err(|_| io::Error::other("spatial operator credential is unavailable"))?;
        if current != self.pinned {
            return Err(io::Error::other("spatial operator credential changed"));
        }
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.set_write_timeout(Some(timeout))
    }
}

pub struct PinnedNative {
    inner: BuildRpc,
    config: Arc<Config>,
    control: bool,
}
impl PinnedNative {
    fn check(&mut self, control: bool) -> Result<()> {
        let result = (|| {
            self.config.current()?;
            self.config.binding(self.inner.binding())?;
            if control && (!self.control || !Config::placement_enabled()) {
                return Err(denied("operator placement authority was revoked"));
            }
            Ok(())
        })();
        if result.is_err() {
            self.inner.fence();
        }
        result
    }
}
impl BuildSource for PinnedNative {
    fn binding(&self) -> &BuildBinding {
        self.inner.binding()
    }
    fn native_summary(&self) -> BuildNativeSummary {
        self.inner.native_summary()
    }
    fn fence(&mut self) {
        self.inner.fence();
    }
    fn observe(
        &mut self,
        selection: BuildSelection,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildCapture> {
        self.check(false)?;
        let value = self.inner.observe(selection, c, timeout)?;
        // Keep valid negative state visible to BuildSession so its tick floor
        // advances before eligibility or policy rejects an unpaused capture.
        self.config.sources.capture(&value)?;
        self.check(false)?;
        Ok(value)
    }
    fn prepare(
        &mut self,
        plan: &BuildPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildPreparation> {
        self.check(true)?;
        let value = self.inner.prepare(plan, c, timeout)?;
        self.check(false)?;
        Ok(value)
    }
    fn commit(
        &mut self,
        dispatch: BuildDispatch<'_>,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildRecord> {
        self.check(true)?;
        let value = self.inner.commit(dispatch, c, timeout)?;
        self.check(false)?;
        Ok(value)
    }
    fn query(
        &mut self,
        plan: &BuildPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<Option<BuildRecord>> {
        self.check(false)?;
        let value = self.inner.query(plan, c, timeout)?;
        self.check(false)?;
        Ok(value)
    }
    fn cancel(
        &mut self,
        plan: &BuildPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildRecord> {
        self.check(false)?;
        let value = self.inner.cancel(plan, c, timeout)?;
        self.check(false)?;
        Ok(value)
    }
}
struct HostLease {
    leases: LeaseManager,
    lease: LeaseId,
    session: SessionId,
    journal: Digest32,
}
pub struct NativeGuard {
    config: Arc<Config>,
    lease: Arc<Mutex<Option<HostLease>>>,
    session: SessionId,
    journal: Digest32,
}
impl BuildGuard for NativeGuard {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        c: &OperationContext,
    ) -> Result<()> {
        self.config.current()?;
        self.config.binding(binding)?;
        if c.session_id != self.session || self.journal == Digest32::ZERO {
            return Err(denied(
                "furniture guard belongs to another session or journal",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if matches!(
            stage,
            BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
        ) {
            self.config.selection(selection)?;
            c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        }
        if matches!(stage, BuildStage::Prepare | BuildStage::Commit) {
            if !Config::placement_enabled() || self.config.checkpoint != Checkpoint::Disposable {
                return Err(denied(
                    "operator construction or checkpoint exception was revoked",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            c.authorize(Capability::Construct, RiskTier::Guarded, &[], None)?;
            let plan = plan
                .ok_or_else(|| invalid("construction guard requires its original sealed plan"))?;
            if !binding.capture_matches(plan.before()) {
                return Err(denied("construction plan source changed"));
            }
            let halo = self.config.selection(selection)?;
            let position = plan
                .before()
                .item_position()
                .ok_or_else(|| denied("original ground item position is unknown"))?;
            let item = MapCuboid::new(
                MapCoord::new(position[0], position[1], position[2]),
                MapCoord::new(position[0], position[1], position[2]),
            )?;
            let guard = self
                .lease
                .try_lock()
                .map_err(|_| denied("construction lease is busy or poisoned"))?;
            let retained = guard
                .as_ref()
                .ok_or_else(|| denied("no current operator construction lease"))?;
            if retained.session != c.session_id || retained.journal != self.journal {
                return Err(denied("construction lease owner differs"));
            }
            for area in [halo, item] {
                retained.leases.verify_exclusive_spatial(
                    retained.lease,
                    c.session_id,
                    area,
                    c.anchor.tick,
                )?;
                if self
                    .config
                    .protected
                    .iter()
                    .any(|protected| cuboids_intersect(protected, &area))
                {
                    return Err(denied(
                        "original item or target intersects an operator protected region",
                    ));
                }
            }
        }
        self.config.current()
    }
}

type SpatialFactory =
    Box<dyn FnMut(&OperationContext) -> Result<SpatialFurnitureRpcSource<NativeSpatialIo>> + Send>;
type SpatialAuthorizer = Box<
    dyn FnMut(
            &LiveSpatialCitizenObservation,
            &LiveSpatialCitizenState,
            &OperationContext,
        ) -> Result<EvidencePolicy>
        + Send,
>;
pub type NativeEvidence = SpatialFurnitureEvidenceOwner<SpatialFactory, SpatialAuthorizer>;
pub struct NativeShell {
    config: Arc<Config>,
    mode: Mode,
    lease: Arc<Mutex<Option<HostLease>>>,
    spatial_fenced: AtomicBool,
}
pub type NativeRuntime = Runtime<NativeShell>;
impl NativeRuntime {
    pub fn open_with_runtime_check(
        mut config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
        runtime_check: fn() -> Result<()>,
    ) -> Result<Self> {
        config.runtime_check = Some(runtime_check);
        Self::open(config, mode, session, budget)
    }
    pub fn open(
        config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        Runtime::from_shell(
            NativeShell {
                config: Arc::new(config),
                mode,
                lease: Arc::new(Mutex::new(None)),
                spatial_fenced: AtomicBool::new(false),
            },
            mode,
            session,
            budget,
        )
    }
}
impl Shell for NativeShell {
    type Journal = PrivateBuildFile;
    type Association = PrivateBuildFile;
    type Native = PinnedNative;
    type Archive = PrivateBuildFile;
    type History = PrivateBuildFile;
    type Evidence = NativeEvidence;
    type Guard = NativeGuard;
    type GoalSource = NativeFurnitureGoalSource;
    type ClockJournal = dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
    type ClockSource = PinnedClock;
    fn clock_enabled(&self) -> bool {
        self.config.clock.is_some() && Config::clock_enabled()
    }
    fn clock_context(&self, c: &OperationContext, control: bool) -> Result<OperationContext> {
        clock::context(&self.config, c, control)
    }
    fn open_clock(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<Option<dfmcp_adapter::bounded_run::journal::RunJournal<Self::ClockJournal>>> {
        clock::open(&self.config, mode, c)
    }
    fn clock_source(&self, c: &OperationContext) -> Result<Self::ClockSource> {
        clock::connect(&self.config, c)
    }
    fn fortress(&self) -> FortressId {
        self.config.sources.fortress()
    }
    fn recovery(&self) -> Option<Recovery> {
        self.config.recovery
    }
    fn check_environment(&self) -> Result<()> {
        self.config.current()
    }
    fn placement_enabled(&self) -> bool {
        self.mode == Mode::Control && self.config.recovery.is_none() && Config::placement_enabled()
    }
    fn checkpoint_exception(&self) -> bool {
        self.config.checkpoint == Checkpoint::Disposable
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        BuildSession<Self::Journal, Self::Native>,
        AssociationStore<Self::Association>,
        Digest32,
    )> {
        self.config.current()?;
        self.config.original_custody()?;
        if self.config.initialize && self.config.clock.is_some() {
            // Reject missing independent clock authority before publishing the
            // furniture journal or association file for this paired session.
            clock::context(&self.config, c, true)?;
        }
        let mut work = Work::new(c)?;
        let journal_mode = if mode == Mode::Control && !self.placement_enabled() {
            BuildMode::Recover
        } else {
            mode.journal()
        };
        if self.config.initialize && journal_mode != BuildMode::Control {
            return Err(denied(
                "initialization requires explicit operator placement control",
            ));
        }
        let expected = if self.config.initialize {
            let mut bootstrap = work.reserve(8 * MIB)?;
            let selection = BuildSelection::new(BuildKind::Bed, 0, [0, 0, 0])?;
            let secret = token(BUILD_TOKEN)?;
            let source = BuildRpc::connect_trusted(
                self.config.endpoint,
                secret.clone(),
                nonce(&mut bootstrap)?,
                self.config.fortress_identity()?,
                selection,
                &bootstrap,
                self.config.cancellation(),
                self.config.build_permission(false, secret),
            )?;
            let capture = source
                .initial_capture()
                .ok_or_else(|| invalid("native bootstrap capture is absent"))?;
            self.config.sources.capture(capture)?;
            if !capture.paused() {
                return Err(denied(
                    "fresh furniture journal initialization requires a paused fortress",
                ));
            }
            self.config.binding(source.binding())?;
            let binding = source.binding().clone();
            drop(source);
            self.config.current()?;
            Some(binding)
        } else {
            None
        };
        let mut journal = open_private_build_exact(
            &self.config.path("furniture.journal"),
            &work.reserve(80 * MIB)?,
            journal_mode,
            expected,
            self.config.initialize,
        )?;
        self.config.binding(journal.binding())?;
        let inventory = journal.inventory(&work.reserve(20 * MIB)?)?;
        let id = inventory.journal_id;
        if self
            .config
            .journal_id
            .is_some_and(|expected| expected != id)
        {
            return Err(denied(
                "furniture journal differs from exact external recovery identity",
            ));
        }
        let native = BuildSession::new(journal, &work.reserve(20 * MIB)?)?;
        let associations = open_private_association_store(
            &self.config.path("semantic.associations"),
            id,
            self.config.initialize,
            journal_mode == BuildMode::Offline,
            &work.reserve(2 * MIB)?,
        )?;
        self.config.current()?;
        Ok((native, associations, id))
    }
    fn native(
        &self,
        binding: &BuildBinding,
        selection: BuildSelection,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<Self::Native> {
        self.config.current()?;
        self.config.binding(binding)?;
        let mut current = c.clone();
        current.budget.max_wall_millis = current.budget.max_wall_millis.min(
            u64::try_from(timeout.as_millis())
                .map_err(|_| invalid("native furniture timeout overflow"))?,
        );
        let control = self.placement_enabled();
        let secret = token(BUILD_TOKEN)?;
        let permission = self.config.build_permission(control, secret.clone());
        let connection_nonce = nonce(&mut current)?;
        let inner = if control {
            BuildRpc::connect_trusted(
                self.config.endpoint,
                secret,
                connection_nonce,
                self.config.fortress_identity()?,
                selection,
                &current,
                self.config.cancellation(),
                permission,
            )?
        } else {
            BuildRpc::connect_trusted_recovery(
                binding,
                secret,
                connection_nonce,
                selection,
                &current,
                self.config.cancellation(),
                permission,
            )?
        };
        self.config.binding(inner.binding())?;
        if inner.binding() != binding {
            return Err(denied(
                "native furniture connection changed original binding",
            ));
        }
        self.config.current()?;
        Ok(PinnedNative {
            inner,
            config: Arc::clone(&self.config),
            control,
        })
    }
    fn guard(&self, journal: Digest32, c: &OperationContext) -> Result<Self::Guard> {
        self.config.current()?;
        if self
            .config
            .journal_id
            .is_some_and(|expected| expected != journal)
        {
            return Err(denied(
                "construction guard names another configured journal",
            ));
        }
        if self.placement_enabled() {
            let mut guard = self
                .lease
                .try_lock()
                .map_err(|_| denied("construction lease is busy or poisoned"))?;
            if guard.is_none() {
                let mut leases = LeaseManager::new();
                let lease = leases.acquire_spatial_lease(
                    c.session_id,
                    self.config.scope,
                    true,
                    c.anchor.tick,
                    1200,
                )?;
                *guard = Some(HostLease {
                    leases,
                    lease,
                    session: c.session_id,
                    journal,
                });
            }
        }
        Ok(NativeGuard {
            config: Arc::clone(&self.config),
            lease: Arc::clone(&self.lease),
            session: c.session_id,
            journal,
        })
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveSpatialCitizenObservation> {
        if self.spatial_fenced.load(Ordering::Acquire) {
            return Err(denied(
                "spatial acquisition is fenced after an earlier failure",
            ));
        }
        let result = (|| {
            self.config.current()?;
            let mut work = Work::new(c)?;
            let connection = work.reserve(MIB)?;
            let read = work.reserve(SPATIAL_READ_BYTES)?;
            work.reserve(128 * MIB)?;
            let mut source = self.config.spatial(&connection)?;
            let observation = source.read_observation(&work.narrow(&read)?);
            drop(source);
            let observation = observation?;
            self.config
                .sources
                .observation(&observation, self.config.region)?;
            let operations = observation.spatial().operations();
            let count = 2usize
                .saturating_add(operations.jobs.jobs.len())
                .saturating_add(operations.buildings.len())
                .saturating_add(operations.items.len())
                .saturating_add(observation.citizens().len())
                .saturating_add(
                    self.config
                        .region
                        .volume()
                        .map_err(|_| invalid("spatial region changed"))?,
                );
            if count > c.budget.max_entities as usize {
                return Err(exhausted(
                    "complete spatial source exceeds negotiated entity allowance",
                ));
            }
            work.current()?;
            self.config.current()?;
            Ok(observation)
        })();
        if result.is_err() {
            self.spatial_fenced.store(true, Ordering::Release);
        }
        result
    }
    fn spatial_policy(
        &self,
        state: &LiveSpatialCitizenState,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.config.current()?;
        self.config.sources.spatial(state, self.config.region, c)
    }
    fn evidence_reserve_bytes(&self, state: &LiveSpatialCitizenState) -> Result<u64> {
        evidence_reserve_bytes(state, self.config.limits())
    }
    fn evidence(
        &self,
        state: LiveSpatialCitizenState,
        c: &OperationContext,
    ) -> Result<Self::Evidence> {
        self.config.current()?;
        if self.spatial_fenced.load(Ordering::Acquire) {
            return Err(denied("original spatial source is fenced"));
        }
        let source = Arc::clone(&self.config);
        let authority = Arc::clone(&self.config);
        let factory: SpatialFactory = Box::new(move |c| source.spatial(c));
        let authorizer: SpatialAuthorizer = Box::new(move |observed, state, c| {
            authority.current()?;
            authority.sources.observation(observed, authority.region)?;
            authority.sources.spatial(state, authority.region, c)
        });
        SpatialFurnitureEvidenceOwner::new(state, self.config.limits(), factory, authorizer, c)
    }
    fn create_archive(
        &self,
        plan: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Self::Archive>> {
        self.config.current()?;
        create_private_plan_archive(
            &self.config.path("original.plan"),
            plan,
            archive_limits(),
            c,
        )
    }
    fn recover_archive(
        &self,
        recovery: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Self::Archive>> {
        self.config.current()?;
        open_private_plan_archive_at_authority_floor(
            &self.config.path("original.plan"),
            recovery.seal,
            archive_limits(),
            c,
        )
    }
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticFurnitureReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Self::History>> {
        self.config.current()?;
        open_private_goal_history_store(
            &self.config.path("goal.history"),
            journal,
            review,
            initialize,
            read_only,
            c,
        )
    }
    fn goal_source(
        &self,
        binding: &BuildBinding,
        c: &OperationContext,
    ) -> Result<Self::GoalSource> {
        self.config.current()?;
        self.config.binding(binding)?;
        let build = token(BUILD_TOKEN)?;
        let operations = token(OPERATIONS_TOKEN)?;
        let pinned_build = build.clone();
        let pinned_operations = operations.clone();
        let config = Arc::clone(&self.config);
        let permission = Box::new(move || {
            config.current()?;
            if token(BUILD_TOKEN)? != pinned_build || token(OPERATIONS_TOKEN)? != pinned_operations
            {
                return Err(denied("construction observation credentials changed"));
            }
            Ok(())
        });
        let mut connection = c.clone();
        let connection_nonce = nonce(&mut connection)?;
        Ok(NativeFurnitureGoalSource::new(
            binding.clone(),
            operations,
            build,
            connection_nonce,
            self.config.cancellation(),
            permission,
        ))
    }
    fn goal_policy(
        &self,
        projection: &FurnitureGoalProjection,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.config.current()?;
        self.config.sources.goal(projection, c)
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
