//! Operator-owned native composition for original semantic excavation.
//! Paths, credentials, independent source pins, leases and checkpoint policy stay outside MCP inputs.
use super::policy::SourceAcceptance;
use super::*;
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::dig_control_policy::{DigCheckpointPolicy, DigControlPolicy};
use dfmcp_adapter::dig_designation::journal::private_file::{
    PrivateDigFile, open_private_dig_exact,
};
use dfmcp_adapter::dig_designation::journal::{DigGuard, DigStage};
use dfmcp_adapter::dig_designation::rpc::{DigManifest, DigPreparation, DigRpcClient, DigStream};
use dfmcp_adapter::dig_designation::{DigEffect, DigPlan};
use dfmcp_adapter::live_jobs_rpc::operations::map::spatial::{
    SpatialLimits, citizens::CitizenSpatialLimits,
};
use dfmcp_adapter::live_jobs_rpc::operations::paged::PagedOperationsLimits;
use dfmcp_adapter::plan_archive::{
    create_private_plan_archive, open_private_plan_archive_at_authority_floor,
};
use dfmcp_adapter::semantic_dig::goal_history::open_private_goal_history_store;
use dfmcp_adapter::semantic_dig::open_private_association_store;
use dfmcp_adapter::semantic_furniture::evidence_owner::{
    SpatialFurnitureEvidenceIo, SpatialFurnitureEvidenceSource, SpatialFurnitureRpcSource,
};
use dfmcp_core::{LeaseId, LeaseManager, MapCoord, MapCuboid};
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
#[path = "native_evidence.rs"]
mod evidence;
pub use evidence::NativeEvidence;
const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_SEMANTIC_DIG";
const CONFIG: &str = "DFMCP_SEMANTIC_DIG_CONFIG";
const DESIGNATE: &str = "DFMCP_SEMANTIC_DIG_ALLOW_DESIGNATE";
const SPATIAL_TOKEN: &str = "DFMCP_SEMANTIC_DIG_SPATIAL_TOKEN";
const DIG_TOKEN: &str = "DFMCP_SEMANTIC_DIG_DIG_TOKEN";
const CLOCK_AUTHORITY: &str = "DFMCP_SEMANTIC_DIG_ALLOW_CLOCK";
const CLOCK_TOKEN: &str = "DFMCP_SEMANTIC_DIG_CLOCK_TOKEN";
const ALLOWED: [&str; 7] = [
    OPT_IN,
    CONFIG,
    DESIGNATE,
    SPATIAL_TOKEN,
    DIG_TOKEN,
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
            return Err(invalid("invalid external dig recovery anchor"));
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
            "dig scope must be ordered and inside pinned map dimensions",
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
            &std::env::var(CONFIG).map_err(|_| denied("operator dig configuration is missing"))?,
        )
    }
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
            return Err(invalid("operator dig configuration exceeds 16 KiB"));
        }
        let input: ConfigInput = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed operator dig configuration"))?;
        input.sources.validate()?;
        let path = input
            .directory
            .to_str()
            .ok_or_else(|| invalid("custody directory must be UTF-8"))?;
        if input.schema != "dfmcp.semantic-dig-development/1"
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
                "invalid dig schema, custody directory or exact initialization/recovery identity",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&input.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != input.endpoint {
            return Err(invalid(
                "dig endpoint requires canonical numeric IPv4 loopback and nonzero port",
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
            .map_err(|_| invalid("spatial dig scope exceeds the bounded region profile"))?;
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
                        "dig recovery and configured journal identities differ",
                    ));
                }
                Ok(recovery)
            })
            .transpose()?;
        if recovery.is_some_and(|r| r.anchor.fortress_id != input.sources.fortress()) {
            return Err(denied("dig recovery belongs to another accepted fortress"));
        }
        if has_recovery {
            match (&clock, recovery_clock) {
                (Some(clock), Some(id)) if clock.journal_id() == Some(id) => {}
                (None, None) => {}
                _ => {
                    return Err(denied(
                        "original dig recovery must retain the exact configured clock journal identity",
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
                "explicit unadmitted semantic dig opt-in is required",
            ));
        }
        if let Some(value) = std::env::var_os(DESIGNATE)
            && value.to_str() != Some("1")
        {
            return Err(denied(
                "operator designation enablement must be absent or exactly 1",
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
                    "semantic dig rejects production launch and other DFMCP profile state",
                ));
            }
        }
        Ok(())
    }
    pub fn designation_enabled() -> bool {
        std::env::var(DESIGNATE).ok().as_deref() == Some("1")
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
            .map_err(|_| denied("operator dig source configuration was removed"))?;
        if raw.len() > MAX_CONFIG_BYTES
            || Digest32::of_bytes(raw.as_bytes()) != self.environment_digest
        {
            return Err(denied(
                "operator dig source acceptance changed; close this session",
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
    fn binding(&self, binding: &DigBinding) -> Result<()> {
        self.sources.binding(binding)?;
        if binding.endpoint() != self.endpoint || binding.scope() != self.scope {
            return Err(denied(
                "native dig endpoint differs from exact operator pins",
            ));
        }
        Ok(())
    }
    fn original_custody(&self) -> Result<()> {
        fn present(path: PathBuf) -> Result<bool> {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() => Ok(true),
                Ok(_) => Err(denied(
                    "dig custody must be regular files, never links or special files",
                )),
                Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(_) => Err(denied("dig custody presence could not be determined")),
            }
        }
        let clock = present(self.path("clock.journal"))?;
        match (&self.clock, clock) {
            (None, false) => {}
            (Some(_), present) if self.initialize != present => {}
            _ => {
                return Err(denied(
                    "clock custody requires exact initialization or externally identified reopening before dig custody is opened",
                ));
            }
        }
        let journal = present(self.path("dig.journal"))?;
        let associations = present(self.path("semantic.associations"))?;
        if self.initialize == journal || self.initialize == associations {
            return Err(denied(
                "dig initialization or reopening differs from existing paired custody",
            ));
        }
        let plan = present(self.path("original.plan"))?;
        let history = present(self.path("goal.history"))?;
        match self.recovery {
            None if !plan && !history => Ok(()),
            Some(recovery) if plan && history == recovery.history_present => Ok(()),
            _ => Err(denied(
                "original dig files require the exact external recovery handoff and history-presence flag",
            )),
        }
    }
    fn selection(&self, region: DigRegion) -> Result<()> {
        if !self.scope.contains_cuboid(region.halo())
            || !self.scope.contains_cuboid(region.write_area())
        {
            return Err(denied(
                "dig target halo or shared-block writes leave the operator scope",
            ));
        }
        Ok(())
    }
    fn bootstrap_region(&self) -> Result<DigRegion> {
        let block_x = (self.scope.min.x as u32).div_ceil(16) * 16;
        let block_y = (self.scope.min.y as u32).div_ceil(16) * 16;
        let region = DigRegion::new(block_x + 1, block_y + 1, self.scope.min.z as u32 + 1, 1, 1)?;
        self.selection(region)?;
        Ok(region)
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
            .map_err(|_| denied("spatial dig connection failed"))?;
        stream
            .set_nodelay(true)
            .map_err(|_| denied("spatial dig socket configuration failed"))?;
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

/// A retained preparation socket spans separate foreground requests. Its byte
/// reservation and one-shot permit remain owned by `DigRpcClient`; only a new
/// trusted request may start a new wall-time window. Calls within one request
/// can only narrow that window. Any observed failure permanently retires it.
struct NativeRequestWindow {
    session: SessionId,
    request: RequestId,
    end: Instant,
    fenced: bool,
}
impl NativeRequestWindow {
    fn candidate(c: &OperationContext, now: Instant) -> Result<Instant> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(DfmcpError::new(
                ErrorCode::CancellationRequested,
                "native mining request was cancelled",
            ));
        }
        if c.session_id == SessionId::NIL || c.request_id.get() == 0 {
            return Err(denied("native mining requires a named host request"));
        }
        if !(1..=MAX_WALL_MILLIS).contains(&c.budget.max_wall_millis) {
            return Err(exhausted("native mining request exceeds sixty seconds"));
        }
        now.checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("native mining request deadline overflow"))
    }
    fn new_at(c: &OperationContext, now: Instant) -> Result<Self> {
        Ok(Self {
            session: c.session_id,
            request: c.request_id,
            end: Self::candidate(c, now)?,
            fenced: false,
        })
    }
    fn enter_at(&mut self, c: &OperationContext, now: Instant) -> Result<()> {
        let result = (|| {
            if self.fenced
                || c.session_id != self.session
                || c.request_id.get() < self.request.get()
            {
                return Err(denied(
                    "native mining request changed owner, regressed or uses a fenced socket",
                ));
            }
            let candidate = Self::candidate(c, now)?;
            self.end = if c.request_id == self.request {
                self.end.min(candidate)
            } else {
                candidate
            };
            self.request = c.request_id;
            self.remaining_at(now).map(|_| ())
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    fn narrow_at(&mut self, timeout: Duration, now: Instant) -> Result<()> {
        let result = (|| {
            if timeout.is_zero() || timeout > Duration::from_millis(MAX_WALL_MILLIS) {
                return Err(exhausted("native mining wire timeout exceeds its bound"));
            }
            self.end = self.end.min(
                now.checked_add(timeout)
                    .ok_or_else(|| exhausted("native mining wire deadline overflow"))?,
            );
            self.remaining_at(now).map(|_| ())
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    fn remaining_at(&mut self, now: Instant) -> Result<Duration> {
        if self.fenced {
            return Err(denied("native mining request window is fenced"));
        }
        let result = self
            .end
            .checked_duration_since(now)
            .filter(|left| *left >= Duration::from_millis(1))
            .ok_or_else(|| exhausted("native mining request deadline expired"));
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
}
/// Every partial wire operation rechecks the original host/configuration/credential
/// and its current request deadline, including post-dispatch revocation.
struct NativeDigIo {
    stream: TcpStream,
    config: Arc<Config>,
    pinned: Vec<u8>,
    window: Arc<Mutex<NativeRequestWindow>>,
    write: Arc<AtomicBool>,
    checkpoint_required: Arc<AtomicBool>,
}
impl NativeDigIo {
    fn check(&self) -> Result<Duration> {
        self.config.current()?;
        if token(DIG_TOKEN)? != self.pinned
            || self.write.load(Ordering::Acquire)
                && (!Config::designation_enabled()
                    || (self.checkpoint_required.load(Ordering::Acquire)
                        && self.config.checkpoint != Checkpoint::Disposable))
        {
            return Err(denied(
                "native dig credential or designation authority changed",
            ));
        }
        self.window
            .try_lock()
            .map_err(|_| denied("native dig request window is unavailable"))?
            .remaining_at(Instant::now())
    }
    fn io_check(&self) -> io::Result<Duration> {
        self.check()
            .map_err(|_| io::Error::other("native dig authority or deadline changed"))
    }
}
impl Read for NativeDigIo {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.io_check()?))?;
        self.stream.read(out)
    }
}
impl Write for NativeDigIo {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.io_check()?))?;
        self.stream.write(input)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.io_check()?;
        self.stream.flush()
    }
}
impl DigStream for NativeDigIo {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()> {
        self.window
            .try_lock()
            .map_err(|_| denied("native dig request window is unavailable"))?
            .narrow_at(timeout, Instant::now())?;
        self.check()?;
        Ok(())
    }
}
pub struct PinnedNative {
    inner: DigRpcClient<NativeDigIo>,
    config: Arc<Config>,
    window: Arc<Mutex<NativeRequestWindow>>,
    control: bool,
    write: Arc<AtomicBool>,
    checkpoint_required: Arc<AtomicBool>,
    pinned: Vec<u8>,
    fenced: bool,
}
impl PinnedNative {
    fn check(&mut self, write: bool, prepare: bool) -> Result<()> {
        let result = (|| {
            self.config.current()?;
            let m = self.inner.manifest();
            if self.fenced
                || self.inner.fenced()
                || self
                    .window
                    .try_lock()
                    .map_err(|_| denied("native dig request window is unavailable"))?
                    .fenced
                || m.generation != self.config.sources.dig_generation
                || m.df_version != self.config.sources.df_version
                || m.dfhack_version != self.config.sources.dfhack_version
                || token(DIG_TOKEN)? != self.pinned
            {
                return Err(denied("dig connection source or credential changed"));
            }
            if prepare && !self.control
                || write
                    && (!Config::designation_enabled()
                        || (prepare && self.config.checkpoint != Checkpoint::Disposable))
            {
                return Err(denied("operator designation authority was revoked"));
            }
            Ok(())
        })();
        if result.is_err() {
            self.fence();
        }
        result
    }
    fn fence(&mut self) {
        self.fenced = true;
        if let Ok(mut window) = self.window.try_lock() {
            window.fenced = true;
        }
    }
    fn begin_request(&mut self, c: &OperationContext, write: bool, prepare: bool) -> Result<()> {
        self.check(write, prepare)?;
        let result = match self.window.try_lock() {
            Ok(mut window) => window.enter_at(c, Instant::now()),
            Err(_) => Err(denied("native dig request window is unavailable")),
        };
        if result.is_err() {
            self.fence();
        }
        result
    }
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.fence();
        }
        let value = result?;
        self.check(false, false)?;
        Ok(value)
    }
    fn mutation<T>(
        &mut self,
        prepare: bool,
        c: &OperationContext,
        operation: impl FnOnce(&mut DigRpcClient<NativeDigIo>) -> Result<T>,
    ) -> Result<T> {
        self.begin_request(c, true, prepare)?;
        self.checkpoint_required.store(prepare, Ordering::Release);
        self.write.store(true, Ordering::Release);
        let result = operation(&mut self.inner);
        self.write.store(false, Ordering::Release);
        self.finish(result)
    }
}
impl DigSource for PinnedNative {
    fn manifest(&self) -> &DigManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.config.endpoint)
    }
    fn observe(&mut self, region: DigRegion, c: &OperationContext) -> Result<DigObservation> {
        self.begin_request(c, false, false)?;
        let result = (|| {
            self.config.selection(region)?;
            let value = self.inner.observe(region, c)?;
            self.config.sources.capture(&value)?;
            Ok(value)
        })();
        self.finish(result)
    }
    fn prepare(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigPreparation> {
        self.mutation(true, c, |source| source.prepare(plan, c))
    }
    fn commit(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        self.mutation(true, c, |source| source.commit(plan, c))
    }
    fn query(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<Option<DigEffect>> {
        self.begin_request(c, false, false)?;
        let result = self.inner.query(plan, c);
        self.finish(result)
    }
    fn cancel(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        self.mutation(false, c, |source| source.cancel(plan, c))
    }
}
impl Config {
    fn native(
        self: &Arc<Self>,
        region: DigRegion,
        control: bool,
        c: &OperationContext,
    ) -> Result<PinnedNative> {
        self.current()?;
        self.selection(region)?;
        c.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )?;
        let mut current = c.clone();
        let secret = token(DIG_TOKEN)?;
        let window = NativeRequestWindow::new_at(c, Instant::now())?;
        let end = window.end;
        let window = Arc::new(Mutex::new(window));
        let connection_nonce = nonce(&mut current)?;
        let left = end
            .checked_duration_since(Instant::now())
            .filter(|n| !n.is_zero())
            .ok_or_else(|| exhausted("dig connection deadline expired"))?;
        let stream = TcpStream::connect_timeout(&self.endpoint, left)
            .map_err(|_| denied("native dig connection failed"))?;
        stream
            .set_nodelay(true)
            .map_err(|_| denied("native dig socket configuration failed"))?;
        self.current()?;
        let write = Arc::new(AtomicBool::new(false));
        let checkpoint_required = Arc::new(AtomicBool::new(false));
        let inner = DigRpcClient::negotiate(
            NativeDigIo {
                stream,
                config: Arc::clone(self),
                pinned: secret.clone(),
                window: Arc::clone(&window),
                write: Arc::clone(&write),
                checkpoint_required: Arc::clone(&checkpoint_required),
            },
            secret.clone(),
            connection_nonce.to_vec(),
            region,
            &current,
        )?;
        let mut source = PinnedNative {
            inner,
            config: Arc::clone(self),
            window,
            control,
            write,
            checkpoint_required,
            pinned: secret,
            fenced: false,
        };
        source.check(false, false)?;
        Ok(source)
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
impl DigGuard for NativeGuard {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        self.config.sources.capture(plan.before())?;
        self.config.selection(plan.before().region())?;
        if c.session_id != self.session || self.journal == Digest32::ZERO {
            return Err(denied("dig guard belongs to another session or journal"));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if matches!(stage, DigStage::Prepare | DigStage::Commit) {
            if !Config::designation_enabled() || self.config.checkpoint != Checkpoint::Disposable {
                return Err(denied(
                    "operator designation or checkpoint exception was revoked",
                ));
            }
            let lease = self
                .lease
                .try_lock()
                .map_err(|_| denied("dig lease is busy or poisoned"))?;
            let retained = lease
                .as_ref()
                .ok_or_else(|| denied("no current operator dig lease"))?;
            if retained.session != c.session_id || retained.journal != self.journal {
                return Err(denied("dig lease owner differs"));
            }
            let binding = DigBinding::new(
                self.config.endpoint,
                DigManifest {
                    generation: self.config.sources.dig_generation,
                    df_version: self.config.sources.df_version.clone(),
                    dfhack_version: self.config.sources.dfhack_version.clone(),
                },
                plan.before(),
                self.config.scope,
            )?;
            let policy = DigControlPolicy::new(
                binding,
                self.journal,
                self.session,
                retained.lease,
                self.config.protected.clone(),
                DigCheckpointPolicy::DisposableFortress,
            )?;
            policy.evaluate(plan, c, &retained.leases)?;
        }
        if stage == DigStage::Cancel && !Config::designation_enabled() {
            return Err(denied(
                "exact native preparation retirement requires current designation authority",
            ));
        }
        self.config.current()
    }
}
impl DigSessionGuard for NativeGuard {
    fn connect(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.config.current()?;
        self.config.binding(binding)?;
        self.config.selection(region)?;
        if c.session_id != self.session {
            return Err(denied("dig native connection belongs to another session"));
        }
        c.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )
    }
    fn observe(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.connect(binding, region, c)?;
        c.authorize(
            Capability::Observe,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )
    }
}
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
pub struct NativeGoalSource {
    config: Arc<Config>,
    binding: DigBinding,
}
impl DigGoalSource for NativeGoalSource {
    fn read_excavation(
        &mut self,
        original: &DigPlan,
        c: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation> {
        self.config.current()?;
        self.config.binding(&self.binding)?;
        self.config.sources.capture(original.before())?;
        self.config.selection(original.before().region())?;
        acquire(&self.config, c)
    }
}
fn acquire(config: &Arc<Config>, c: &OperationContext) -> Result<LiveSpatialCitizenObservation> {
    config.current()?;
    let mut work = Work::new(c)?;
    let connection = work.reserve(MIB)?;
    let read = work.reserve(SPATIAL_READ_BYTES)?;
    work.reserve(128 * MIB)?;
    let mut source = config.spatial(&work.narrow(&connection)?)?;
    let result = source.read_observation(&work.narrow(&read)?);
    drop(source);
    let observation = result?;
    config.sources.observation(&observation, config.region)?;
    let operations = observation.spatial().operations();
    let count = 2usize
        .saturating_add(operations.jobs.jobs.len())
        .saturating_add(operations.buildings.len())
        .saturating_add(operations.items.len())
        .saturating_add(observation.citizens().len())
        .saturating_add(
            config
                .region
                .volume()
                .map_err(|_| invalid("terrain region changed"))?,
        );
    if count > c.budget.max_entities as usize {
        return Err(exhausted(
            "complete terrain source exceeds negotiated entity allowance",
        ));
    }
    work.current()?;
    config.current()?;
    Ok(observation)
}
impl Shell for NativeShell {
    type Journal = PrivateDigFile;
    type Association = PrivateBuildFile;
    type Native = PinnedNative;
    type Archive = PrivateBuildFile;
    type History = PrivateBuildFile;
    type Evidence = NativeEvidence;
    type Guard = NativeGuard;
    type GoalSource = NativeGoalSource;
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
    fn authorize_clock_terrain(&self, p: &DigGoalProjection, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        self.config.sources.clock_terrain(p, self.config.region, c)
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
    fn designation_enabled(&self) -> bool {
        self.mode != Mode::Offline && Config::designation_enabled()
    }
    fn checkpoint_exception(&self) -> bool {
        self.config.checkpoint == Checkpoint::Disposable
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        DigSession<Self::Journal, Self::Native>,
        AssociationStore<Self::Association>,
        Digest32,
    )> {
        self.config.current()?;
        self.config.original_custody()?;
        if self.config.initialize && self.config.clock.is_some() {
            clock::context(&self.config, c, true)?;
        }
        let mut work = Work::new(c)?;
        let journal_mode = if mode == Mode::Control
            && (!self.designation_enabled() || self.config.recovery.is_some())
        {
            DigMode::Recover
        } else {
            mode.journal()
        };
        if self.config.initialize
            && (journal_mode != DigMode::Control || !self.checkpoint_exception())
        {
            return Err(denied(
                "initialization requires explicit operator designation control and checkpoint exception",
            ));
        }
        let expected = if self.config.initialize {
            let region = self.config.bootstrap_region()?;
            let mut source = self.config.native(region, false, &work.reserve(8 * MIB)?)?;
            let capture = source.observe(region, &work.reserve(2 * MIB)?)?;
            self.config.sources.capture(&capture)?;
            if !capture.paused() {
                return Err(denied(
                    "dig journal initialization requires a paused fortress",
                ));
            }
            let binding = DigBinding::new(
                self.config.endpoint,
                source.manifest().clone(),
                &capture,
                self.config.scope,
            )?;
            drop(source);
            self.config.binding(&binding)?;
            self.config.current()?;
            // The initial source read consumed real native time even though no
            // original canonical goal exists yet. Preserve that private floor
            // when the retained DigSession is created below.
            work.context.anchor.tick = work.context.anchor.tick.max(GameTick(capture.tick()));
            Some(binding)
        } else {
            None
        };
        let journal = open_private_dig_exact(
            &self.config.path("dig.journal"),
            &work.reserve(80 * MIB)?,
            journal_mode,
            expected,
            self.config.initialize,
        )?;
        self.config.binding(journal.binding())?;
        let id = journal.id();
        if self
            .config
            .journal_id
            .is_some_and(|expected| expected != id)
        {
            return Err(denied(
                "dig journal differs from exact external recovery identity",
            ));
        }
        let native = DigSession::new(journal, &work.reserve(20 * MIB)?)?;
        let associations = open_private_association_store(
            &self.config.path("semantic.associations"),
            id,
            self.config.initialize,
            journal_mode == DigMode::Offline,
            &work.reserve(2 * MIB)?,
        )?;
        self.config.current()?;
        Ok((native, associations, id))
    }
    fn native(
        &self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<Self::Native> {
        self.config.current()?;
        self.config.binding(binding)?;
        self.config.native(
            region,
            self.mode == Mode::Control
                && self.config.recovery.is_none()
                && self.designation_enabled(),
            c,
        )
    }
    fn guard(&self, journal: Digest32, c: &OperationContext) -> Result<Self::Guard> {
        self.config.current()?;
        if self
            .config
            .journal_id
            .is_some_and(|expected| expected != journal)
        {
            return Err(denied("dig guard names another configured journal"));
        }
        if self.mode == Mode::Control
            && self.config.recovery.is_none()
            && self.designation_enabled()
        {
            let mut guard = self
                .lease
                .try_lock()
                .map_err(|_| denied("dig lease is busy or poisoned"))?;
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
                "terrain source is fenced after an earlier acquisition failure",
            ));
        }
        let result = acquire(&self.config, c);
        if result.is_err() {
            self.spatial_fenced.store(true, Ordering::Release);
        }
        result
    }
    fn spatial_policy(
        &self,
        p: &DigTerrainProjection,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.config.current()?;
        self.config.sources.spatial(p, self.config.region, c)
    }
    fn evidence_reserve_bytes(&self, p: &DigTerrainProjection) -> Result<u64> {
        evidence::reserve_bytes(p, self.config.limits())
    }
    fn evidence(&self, p: DigTerrainProjection, c: &OperationContext) -> Result<Self::Evidence> {
        self.config.current()?;
        if self.spatial_fenced.load(Ordering::Acquire) {
            return Err(denied("original terrain source is fenced"));
        }
        NativeEvidence::new(p, Arc::clone(&self.config), c)
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
        review: &SemanticDigReview,
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
    fn goal_source(&self, binding: &DigBinding, _c: &OperationContext) -> Result<Self::GoalSource> {
        self.config.current()?;
        self.config.binding(binding)?;
        Ok(NativeGoalSource {
            config: Arc::clone(&self.config),
            binding: binding.clone(),
        })
    }
    fn goal_policy(&self, p: &DigGoalProjection, c: &OperationContext) -> Result<EvidencePolicy> {
        self.config.current()?;
        self.config.sources.goal(p, self.config.region, c)
    }
}
#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
