//! Operator configuration and inherited, joined blocking work.
use super::{error, unbound};
use asupersync::Cx;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::build_placement::BuildPlan;
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildMode, BuildStage};
use dfmcp_adapter::build_placement::rpc::{BuildCancellation, BuildRpc};
use dfmcp_adapter::build_placement::{BuildBinding, BuildSelection, FortressIdentity};
use dfmcp_core::{ErrorCode, MapCoord, MapCuboid, OperationContext, Result};
use std::cell::RefCell;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::time::Instant;

pub(super) const NAMES: [&str; 18] = [
    "DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19",
    "DFMCP_BUILD_WORLD_FOLDER",
    "DFMCP_BUILD_SITE_ID",
    "DFMCP_BUILD_SCOPE",
    "DFMCP_BUILD_JOURNAL",
    "DFMCP_BUILD_BATCH",
    "DFMCP_BUILD_COMPLETION",
    "DFMCP_OPERATIONS_PAGED_TOKEN",
    "DFMCP_BUILD_ENDPOINT",
    "DFMCP_BUILD_TOKEN",
    "DFMCP_BUILD_ALLOW_PLACE",
    "DFMCP_BUILD_CHECKPOINT_POLICY",
    "DFMCP_BUILD_PROTECTED",
    "DFMCP_BUILD_MODE",
    "DFMCP_BUILD_CLOCK_CONFIG",
    "DFMCP_BUILD_CLOCK_TOKEN",
    "DFMCP_BUILD_ALLOW_CLOCK",
    "DFMCP_BUILD_CLOCK_MONITOR_RECOVERY",
];
fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "furniture development operator/runtime boundary refused",
    )
}
fn blocking_io_authorized(cx: &Cx) -> bool {
    // This adapter uses std TCP on a joined, runtime-owned blocking worker.
    // The pinned runtime carries I/O authority separately from its optional
    // async IoCap implementation, which request_cx_with_budget leaves absent.
    // Check the effective (including inherited restrictions) authority and
    // require an inherited pool so spawn_blocking cannot run the work inline.
    let caps = cx.capabilities();
    caps.io && caps.spawn && cx.blocking_pool_handle().is_some()
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CheckpointPolicy {
    #[default]
    Required,
    DisposableFortress,
}
impl CheckpointPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::DisposableFortress => "disposable-fortress-no-checkpoint",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub path: PathBuf,
    pub batch_path: Option<PathBuf>,
    pub completion_path: Option<PathBuf>,
    pub completion_only: bool,
    pub clock_only: bool,
    pub clock_monitor_recovery: Option<dfmcp_adapter::construction_plan::store::Recovery>,
    pub clock: Option<super::clock::Config>,
    pub scope: MapCuboid,
    pub fortress: FortressIdentity,
    pub endpoint: SocketAddr,
    pub protected: Vec<MapCuboid>,
    pub checkpoint: CheckpointPolicy,
    pub mode: BuildMode,
}
impl Config {
    pub fn matches(&self, binding: &BuildBinding) -> Result<()> {
        if binding.fortress() != &self.fortress || binding.endpoint() != self.endpoint {
            return Err(denied());
        }
        Ok(())
    }
    pub fn selection(&self, selection: BuildSelection) -> Result<MapCuboid> {
        let [x, y, z] = selection.target();
        let x = i32::try_from(x).map_err(|_| denied())?;
        let y = i32::try_from(y).map_err(|_| denied())?;
        let z = i32::try_from(z).map_err(|_| denied())?;
        let halo = MapCuboid::new(
            MapCoord::new(x - 1, y - 1, z),
            MapCoord::new(x + 1, y + 1, z),
        )?;
        if !self.scope.contains_cuboid(halo) {
            return Err(denied());
        }
        Ok(halo)
    }
}
fn required(name: &str, limit: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_| denied())?;
    if value.is_empty() || value.len() > limit || value.contains('\0') {
        return Err(denied());
    }
    Ok(value)
}
fn optional(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        _ => Err(denied()),
    }
}
pub(super) fn enabled() -> Result<bool> {
    match optional("DFMCP_BUILD_ALLOW_PLACE")?.as_deref() {
        None => Ok(false),
        Some("1") => Ok(true),
        _ => Err(denied()),
    }
}
pub(super) fn clock_enabled() -> Result<bool> {
    match optional("DFMCP_BUILD_ALLOW_CLOCK")?.as_deref() {
        None => Ok(false),
        Some("1") => Ok(true),
        _ => Err(denied()),
    }
}
pub(super) fn clock_grants(control: bool) -> Vec<dfmcp_core::CapabilityGrant> {
    [
        dfmcp_core::Capability::Query,
        dfmcp_core::Capability::Plan,
        dfmcp_core::Capability::ControlClock,
    ]
    .into_iter()
    .filter(|capability| control || *capability == dfmcp_core::Capability::Query)
    .map(|capability| dfmcp_core::CapabilityGrant {
        capability,
        scope: dfmcp_core::CapabilityScope {
            fortress_id: Some(dfmcp_core::FortressId::NIL),
            ..dfmcp_core::CapabilityScope::default()
        },
        max_risk: if capability == dfmcp_core::Capability::Query {
            dfmcp_core::RiskTier::ReadOnly
        } else {
            dfmcp_core::RiskTier::Guarded
        },
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect()
}
fn area(v: [i32; 6]) -> Result<MapCuboid> {
    if v.iter().any(|n| !(0..=32767).contains(n)) {
        return Err(denied());
    }
    MapCuboid::new(
        MapCoord::new(v[0], v[1], v[2]),
        MapCoord::new(v[3], v[4], v[5]),
    )
}
#[allow(clippy::too_many_arguments)]
fn configured(
    folder: &str,
    site: &str,
    scope: &str,
    path: &str,
    endpoint: &str,
    protected: Option<&str>,
    checkpoint: Option<&str>,
    mode: Option<&str>,
) -> Result<Config> {
    if site.len() > 10
        || scope.len() > 128
        || path.len() > 4096
        || !path.starts_with('/')
        || path.contains('\0')
        || path[1..]
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || endpoint.len() > 128
    {
        return Err(denied());
    }
    let site_number: u32 = site.parse().map_err(|_| denied())?;
    if site_number.to_string() != site {
        return Err(denied());
    }
    let fortress = FortressIdentity::new(folder, site_number)?;
    let scope = area(serde_json::from_str(scope).map_err(|_| denied())?)?;
    let endpoint: SocketAddr = endpoint.parse().map_err(|_| denied())?;
    if !matches!(endpoint,SocketAddr::V4(v) if v.ip().is_loopback() && v.port()!=0) {
        return Err(denied());
    }
    let raw = protected.unwrap_or("[]");
    if raw.len() > 4096 {
        return Err(denied());
    }
    let values: Vec<[i32; 6]> = serde_json::from_str(raw).map_err(|_| denied())?;
    if values.len() > 32 {
        return Err(denied());
    }
    let mut protected = values.into_iter().map(area).collect::<Result<Vec<_>>>()?;
    protected.sort_by_key(|v| [v.min.x, v.min.y, v.min.z, v.max.x, v.max.y, v.max.z]);
    protected.dedup();
    let checkpoint = match checkpoint {
        None | Some("required") => CheckpointPolicy::Required,
        Some("disposable-fortress-no-checkpoint") => CheckpointPolicy::DisposableFortress,
        _ => return Err(denied()),
    };
    let (mode, completion_only, clock_only) = match mode {
        None | Some("control") => (BuildMode::Control, false, false),
        Some("recover") => (BuildMode::Recover, false, false),
        Some("offline") => (BuildMode::Offline, false, false),
        Some("completion-recover") => (BuildMode::Recover, true, false),
        Some("completion-offline") => (BuildMode::Offline, true, false),
        Some("clock-recover") => (BuildMode::Recover, false, true),
        Some("clock-offline") => (BuildMode::Offline, false, true),
        _ => return Err(denied()),
    };
    Ok(Config {
        path: PathBuf::from(path),
        batch_path: None,
        completion_path: None,
        completion_only,
        clock_only,
        clock_monitor_recovery: None,
        clock: None,
        scope,
        fortress,
        endpoint,
        protected,
        checkpoint,
        mode,
    })
}
fn isolated(opt: &str, names: impl IntoIterator<Item = String>, admitted: bool) -> Result<()> {
    if opt != "1" || admitted {
        return Err(denied());
    }
    for (i, name) in names.into_iter().enumerate() {
        if i >= 1024
            || name.len() > 512
            || (name.starts_with("DFMCP_") && !NAMES.contains(&name.as_str()))
        {
            return Err(denied());
        }
    }
    Ok(())
}
pub(super) fn monitor_recovery(
    raw: &str,
) -> Result<dfmcp_adapter::construction_plan::store::Recovery> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        monitor_id: String,
        head: String,
    }
    if raw.is_empty() || raw.len() > 2048 {
        return Err(denied());
    }
    let value: Input = serde_json::from_str(raw).map_err(|_| denied())?;
    let monitor_id = super::presentation::digest(&value.monitor_id)?;
    let head = super::presentation::digest(&value.head)?;
    if monitor_id == dfmcp_core::Digest32::ZERO || head == dfmcp_core::Digest32::ZERO {
        return Err(denied());
    }
    Ok(dfmcp_adapter::construction_plan::store::Recovery { monitor_id, head })
}
pub(super) fn configuration() -> Result<Config> {
    isolated(
        &required("DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19", 1)?,
        std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()),
        crate::admission::current_admission_provenance().is_some(),
    )?;
    enabled()?;
    clock_enabled()?;
    let mut config = configured(
        &required("DFMCP_BUILD_WORLD_FOLDER", 512)?,
        &required("DFMCP_BUILD_SITE_ID", 10)?,
        &required("DFMCP_BUILD_SCOPE", 128)?,
        &required("DFMCP_BUILD_JOURNAL", 4096)?,
        optional("DFMCP_BUILD_ENDPOINT")?
            .as_deref()
            .unwrap_or("127.0.0.1:5000"),
        optional("DFMCP_BUILD_PROTECTED")?.as_deref(),
        optional("DFMCP_BUILD_CHECKPOINT_POLICY")?.as_deref(),
        optional("DFMCP_BUILD_MODE")?.as_deref(),
    )?;
    if let Some(path) = optional("DFMCP_BUILD_BATCH")? {
        if path.len() > 4096
            || !path.starts_with('/')
            || path.contains('\0')
            || path[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || path == config.path
        {
            return Err(denied());
        }
        config.batch_path = Some(PathBuf::from(path));
    }
    if let Some(path) = optional("DFMCP_BUILD_COMPLETION")? {
        if path.len() > 4096
            || !path.starts_with('/')
            || path.contains('\0')
            || path[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || path == config.path
            || config
                .batch_path
                .as_ref()
                .is_none_or(|batch| batch.as_path() == std::path::Path::new(&path))
        {
            return Err(denied());
        }
        config.completion_path = Some(PathBuf::from(path));
    }
    if config.completion_only && config.completion_path.is_none() {
        return Err(denied());
    }
    config.clock = optional("DFMCP_BUILD_CLOCK_CONFIG")?
        .as_deref()
        .map(super::clock::Config::parse)
        .transpose()?;
    config.clock_monitor_recovery = optional("DFMCP_BUILD_CLOCK_MONITOR_RECOVERY")?
        .as_deref()
        .map(monitor_recovery)
        .transpose()?;
    if config.clock_only != config.clock_monitor_recovery.is_some()
        || (config.clock_only && (config.clock.is_none() || config.completion_path.is_none()))
    {
        return Err(denied());
    }
    if let Some(clock) = &config.clock {
        if config.completion_only
            || config.completion_path.is_none()
            || config.batch_path.is_none()
            || clock.path == config.path
            || config.batch_path.as_ref() == Some(&clock.path)
            || config.completion_path.as_ref() == Some(&clock.path)
            || (clock.initialize && (config.mode != BuildMode::Control || config.clock_only))
        {
            return Err(denied());
        }
    } else if clock_enabled()? || optional("DFMCP_BUILD_CLOCK_TOKEN")?.is_some() {
        return Err(denied());
    }
    Ok(config)
}
pub(super) struct RequestControl {
    pub started: Instant,
    parent: Cx,
    worker: Cx,
    abandoned: Arc<AtomicBool>,
    acknowledged: Arc<AtomicBool>,
}
impl RequestControl {
    pub fn acknowledgement(&self) -> Arc<AtomicBool> {
        self.acknowledged.clone()
    }
    pub fn checkpoint(&self) -> Result<()> {
        if self.abandoned.load(Ordering::Acquire)
            || self.parent.checkpoint().is_err()
            || self.worker.checkpoint().is_err()
        {
            return Err(error(
                ErrorCode::CancellationRequested,
                "furniture request cancelled or abandoned",
            ));
        }
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        self.checkpoint()?;
        if !blocking_io_authorized(&self.parent) || !blocking_io_authorized(&self.worker) {
            return Err(denied());
        }
        // A restriction pushed onto this thread after the worker started is
        // visible only through the ambient context; honour it as well.
        if Cx::current().is_some_and(|ambient| !blocking_io_authorized(&ambient)) {
            return Err(denied());
        }
        Ok(())
    }
}
type RequestCheck = Arc<dyn Fn() -> Result<()> + Send + Sync>;
thread_local! {static REQUEST_CHECK:RefCell<Option<RequestCheck>>=RefCell::new(None);}
struct RequestScope(Option<RequestCheck>);
impl Drop for RequestScope {
    fn drop(&mut self) {
        let old = self.0.take();
        REQUEST_CHECK.with(|slot| {
            slot.replace(old);
        });
    }
}
fn current_request_check() -> Result<()> {
    let check = REQUEST_CHECK
        .with(|slot| slot.borrow().clone())
        .ok_or_else(denied)?;
    check()
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) async fn owned<F>(op: &'static str, f: F) -> String
where
    F: FnOnce(RequestControl) -> String + Send + 'static,
{
    let Some(cx) = Cx::current() else {
        return unbound(op, &denied());
    };
    if !blocking_io_authorized(&cx) {
        return unbound(op, &denied());
    }
    if cx.checkpoint().is_err() {
        return unbound(
            op,
            &error(
                ErrorCode::CancellationRequested,
                "furniture request cancelled",
            ),
        );
    }
    let started = Instant::now();
    let abandoned = Arc::new(AtomicBool::new(false));
    let acknowledged = Arc::new(AtomicBool::new(false));
    let worker_acknowledged = acknowledged.clone();
    let _cancel = CancelOnDrop(abandoned.clone());
    let parent = cx.clone();
    let task = cx.spawn_blocking(move |worker| {
        let _ambient = Cx::set_current(Some(worker.clone()));
        let control = RequestControl {
            started,
            parent,
            worker,
            abandoned,
            acknowledged: worker_acknowledged,
        };
        let p = control.parent.clone();
        let w = control.worker.clone();
        let abandoned = control.abandoned.clone();
        let check: RequestCheck = Arc::new(move || {
            if abandoned.load(Ordering::Acquire)
                || p.checkpoint().is_err()
                || w.checkpoint().is_err()
            {
                return Err(error(
                    ErrorCode::CancellationRequested,
                    "furniture request cancelled at native I/O",
                ));
            }
            if !blocking_io_authorized(&p) || !blocking_io_authorized(&w) {
                return Err(denied());
            }
            Ok(())
        });
        let _request = RequestScope(REQUEST_CHECK.with(|slot| slot.replace(Some(check))));
        if let Err(e) = control.check() {
            return unbound(op, &e);
        }
        f(control)
    });
    let mut task = match task {
        Ok(v) => v,
        Err(_) => return unbound(op, &denied()),
    };
    match task.join(&cx).await {
        Ok(v) => {
            if cx.checkpoint().is_err() || !blocking_io_authorized(&cx) {
                return unbound(
                    op,
                    &error(
                        ErrorCode::CancellationIncomplete,
                        "furniture request lost authority before worker acknowledgement",
                    ),
                );
            }
            acknowledged.store(true, Ordering::Release);
            v
        }
        Err(_) => unbound(
            op,
            &error(
                ErrorCode::CancellationIncomplete,
                "furniture worker has no acknowledged outcome; inspect original journal before further effects",
            ),
        ),
    }
}
pub(super) fn boundary(control: &RequestControl, config: &Config, write: bool) -> Result<()> {
    control.check()?;
    if &configuration()? != config || (write && !enabled()?) {
        return Err(denied());
    }
    Ok(())
}
pub(super) struct Guard<'a> {
    pub control: &'a RequestControl,
    pub config: &'a Config,
}
impl BuildGuard for Guard<'_> {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        _plan: Option<&BuildPlan>,
        selection: BuildSelection,
        _c: &OperationContext,
    ) -> Result<()> {
        boundary(
            self.control,
            self.config,
            matches!(stage, BuildStage::Prepare | BuildStage::Commit),
        )?;
        self.config.matches(binding)?;
        if matches!(
            stage,
            BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
        ) {
            self.config.selection(selection)?;
        }
        Ok(())
    }
}
pub(super) fn connect(
    config: &Config,
    selection: BuildSelection,
    binding: Option<&BuildBinding>,
    c: &OperationContext,
    remaining: Duration,
) -> Result<BuildRpc> {
    current_request_check()?;
    if &configuration()? != config
        || config.mode == BuildMode::Offline
        || config.completion_only
        || config.clock_only
    {
        return Err(denied());
    }
    let token = required("DFMCP_BUILD_TOKEN", 256)?.into_bytes();
    if token.len() < 32 {
        return Err(denied());
    }
    let mut nonce = [0; 32];
    nonce[..16].copy_from_slice(&c.session_id.get().to_be_bytes());
    nonce[16..].copy_from_slice(&c.request_id.get().to_be_bytes());
    let config_for_guard = config.clone();
    let pinned_token = token.clone();
    let permission = Box::new(move |write| {
        current_request_check()?;
        if configuration()? != config_for_guard
            || required("DFMCP_BUILD_TOKEN", 256)?.as_bytes() != pinned_token
            || (write && (!enabled()? || config_for_guard.mode != BuildMode::Control))
        {
            return Err(denied());
        }
        Ok(())
    });
    let cancellation = BuildCancellation::with_check(Arc::new(current_request_check));
    let mut narrowed = c.clone();
    narrowed.budget.max_wall_millis = narrowed
        .budget
        .max_wall_millis
        .min(u64::try_from(remaining.as_millis()).map_err(|_| denied())?);
    if let Some(original) = binding {
        BuildRpc::connect_trusted_recovery(
            original,
            token,
            nonce,
            selection,
            &narrowed,
            cancellation,
            permission,
        )
    } else {
        BuildRpc::connect_trusted(
            config.endpoint,
            token,
            nonce,
            config.fortress.clone(),
            selection,
            &narrowed,
            cancellation,
            permission,
        )
    }
}
/// A query-only receipt bracket on the same joined foreground request owner.
pub(super) fn completion_sample(
    config: &Config,
    binding: &BuildBinding,
    goal: &dfmcp_adapter::construction_plan::Goal,
    c: &OperationContext,
) -> Result<dfmcp_adapter::construction_plan::LinkedSample> {
    current_request_check()?;
    if &configuration()? != config || config.mode == BuildMode::Offline || config.completion_only {
        return Err(denied());
    }
    config.matches(binding)?;
    let build_token = required("DFMCP_BUILD_TOKEN", 256)?.into_bytes();
    let operations_token = required("DFMCP_OPERATIONS_PAGED_TOKEN", 256)?.into_bytes();
    if build_token.len() < 32 || operations_token.len() < 32 {
        return Err(denied());
    }
    let pinned_build = build_token.clone();
    let pinned_operations = operations_token.clone();
    let pinned_config = config.clone();
    let permission = Box::new(move || {
        current_request_check()?;
        if configuration()? != pinned_config
            || required("DFMCP_BUILD_TOKEN", 256)?.as_bytes() != pinned_build
            || required("DFMCP_OPERATIONS_PAGED_TOKEN", 256)?.as_bytes() != pinned_operations
        {
            return Err(denied());
        }
        Ok(())
    });
    let mut nonce = [0; 32];
    nonce[..16].copy_from_slice(&c.session_id.get().to_be_bytes());
    nonce[16..].copy_from_slice(&c.request_id.get().to_be_bytes());
    dfmcp_adapter::construction_plan::rpc::acquire_trusted(
        binding,
        goal,
        operations_token,
        build_token,
        nonce,
        c,
        BuildCancellation::with_check(Arc::new(current_request_check)),
        permission,
    )
}
/// Acquire supply evidence on the same joined foreground request owner. This
/// connection binds operations/1.4 only; build/1.19 is checked independently
/// after the allocator has selected a complete immutable set of items.
pub(super) fn furniture_supply(
    config: &Config,
    c: &OperationContext,
) -> Result<dfmcp_adapter::live_operations::LiveOperationsState> {
    current_request_check()?;
    if &configuration()? != config || config.mode != BuildMode::Control || config.completion_only {
        return Err(denied());
    }
    let token = required("DFMCP_OPERATIONS_PAGED_TOKEN", 256)?.into_bytes();
    if token.len() < 32 {
        return Err(denied());
    }
    let pinned_token = token.clone();
    let pinned_config = config.clone();
    let permission = Box::new(move || {
        current_request_check()?;
        if configuration()? != pinned_config
            || required("DFMCP_OPERATIONS_PAGED_TOKEN", 256)?.as_bytes() != pinned_token
        {
            return Err(denied());
        }
        Ok(())
    });
    let mut nonce = [0; 32];
    nonce[..16].copy_from_slice(&c.session_id.get().to_be_bytes());
    nonce[16..].copy_from_slice(&c.request_id.get().to_be_bytes());
    dfmcp_adapter::furniture_handoff::rpc::acquire_trusted(
        config.endpoint,
        &config.fortress,
        token,
        nonce,
        c,
        BuildCancellation::with_check(Arc::new(current_request_check)),
        permission,
    )
}
pub(super) struct ClockIo<'a> {
    pub control: &'a RequestControl,
    pub config: &'a Config,
    pub opening_recovery: Option<dfmcp_adapter::construction_plan::clock::Recovery>,
}
pub(super) struct ClockSource<'a> {
    inner: dfmcp_adapter::bounded_run::rpc::RunRpcClient<
        dfmcp_adapter::bounded_run::rpc::RunTcpStream,
    >,
    control: &'a RequestControl,
    config: &'a Config,
    token_digest: dfmcp_core::Digest32,
}
fn clock_boundary(control: &RequestControl, config: &Config, effect: bool) -> Result<()> {
    boundary(control, config, false)?;
    if effect && !clock_enabled()? {
        return Err(denied());
    }
    Ok(())
}
impl ClockSource<'_> {
    fn boundary(&self, effect: bool) -> Result<()> {
        clock_boundary(self.control, self.config, effect)?;
        let token = required("DFMCP_BUILD_CLOCK_TOKEN", 256)?;
        if token.len() < 32 || dfmcp_core::Digest32::of_bytes(token.as_bytes()) != self.token_digest
        {
            return Err(denied());
        }
        Ok(())
    }
}
impl dfmcp_adapter::bounded_run::rpc::RunSource for ClockSource<'_> {
    fn manifest(&self) -> &dfmcp_adapter::bounded_run::rpc::RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(
        &mut self,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunObservation> {
        self.boundary(false)?;
        self.inner.restrict_deadline(c)?;
        // Return decoded evidence before another runtime rejection can discard
        // its consumed clock or durable receipt; the owner checks publication.
        self.inner.observe(c)
    }
    fn prepare(
        &mut self,
        p: &dfmcp_adapter::bounded_run::RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.boundary(true)?;
        self.inner.restrict_deadline(c)?;
        // Return decoded evidence before another runtime rejection can discard
        // its consumed clock or durable receipt; the owner checks publication.
        self.inner.prepare(p, c)
    }
    fn commit(
        &mut self,
        p: &dfmcp_adapter::bounded_run::RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.boundary(true)?;
        self.inner.restrict_deadline(c)?;
        // Return decoded evidence before another runtime rejection can discard
        // its consumed clock or durable receipt; the owner checks publication.
        self.inner.commit(p, c)
    }
    fn query(
        &mut self,
        p: &dfmcp_adapter::bounded_run::RunPlan,
        c: &OperationContext,
    ) -> Result<Option<dfmcp_adapter::bounded_run::RunRecord>> {
        self.boundary(false)?;
        self.inner.restrict_deadline(c)?;
        // Return decoded evidence before another runtime rejection can discard
        // its consumed clock or durable receipt; the owner checks publication.
        self.inner.query(p, c)
    }
    fn cancel(
        &mut self,
        p: &dfmcp_adapter::bounded_run::RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.boundary(true)?;
        self.inner.restrict_deadline(c)?;
        // Return decoded evidence before another runtime rejection can discard
        // its consumed clock or durable receipt; the owner checks publication.
        self.inner.cancel(p, c)
    }
}
impl<'a> super::clock::ClockIo for ClockIo<'a> {
    type Source = ClockSource<'a>;
    fn open(
        &mut self,
        selected: &super::clock::Config,
        spec: &dfmcp_adapter::construction_plan::origin::MonitorDefinition,
        operations: &dfmcp_adapter::construction_plan::Manifest,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<
        dfmcp_adapter::construction_plan::clock::Clock<
            dfmcp_adapter::bounded_run::private_file::PrivateRunFile,
        >,
    > {
        use dfmcp_adapter::bounded_run::journal::RunMode;
        let deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(
                c.budget.max_wall_millis.min(n.budget.max_wall_millis),
            ))
            .ok_or_else(super::exhausted)?;
        if c.budget.max_bytes.min(n.budget.max_bytes) < super::clock::OPEN_BYTES {
            return Err(super::exhausted());
        }
        let remaining = || {
            deadline
                .checked_duration_since(Instant::now())
                .and_then(|left| u64::try_from(left.as_millis()).ok())
                .filter(|left| *left > 0)
                .ok_or_else(super::exhausted)
        };
        if self.config.clock.as_ref() != Some(selected) {
            return Err(denied());
        }
        clock_boundary(self.control, self.config, selected.initialize)?;
        selected.matches(spec)?;
        c.authorize(
            dfmcp_core::Capability::Query,
            dfmcp_core::RiskTier::ReadOnly,
            &[],
            None,
        )?;
        let mode = match self.config.mode {
            BuildMode::Offline => RunMode::Offline,
            BuildMode::Control if clock_enabled()? => RunMode::Control,
            BuildMode::Recover if self.config.clock_only && clock_enabled()? => RunMode::Control,
            _ => RunMode::Recover,
        };
        if selected.initialize && mode != RunMode::Control {
            return Err(denied());
        }
        // Preserve the intended header before exclusive creation, or retain the
        // externally acknowledged existing root until ancestry is verified.
        // This reference alone never certifies successful publication.
        self.opening_recovery = if selected.initialize {
            let id = selected
                .binding
                .initial_journal_id(n.session_id, n.request_id);
            Some(dfmcp_adapter::construction_plan::clock::Recovery {
                journal_id: id,
                head: id,
            })
        } else {
            selected.recovery
        };
        let mut opening = n.clone();
        opening.budget.max_bytes = super::clock::VIEW_BYTES;
        opening.budget.max_wall_millis = remaining()?;
        let journal = dfmcp_adapter::bounded_run::private_file::open_private_run_journal_exact(
            &selected.path,
            &opening,
            mode,
            Some(selected.binding.clone()),
            selected.initialize,
        )?;
        clock_boundary(self.control, self.config, selected.initialize)?;
        let mut canonical = c.clone();
        let mut native = n.clone();
        let wall = remaining()?;
        canonical.budget.max_bytes = 2 * super::clock::VIEW_BYTES;
        native.budget.max_bytes = canonical.budget.max_bytes;
        canonical.budget.max_wall_millis = wall;
        native.budget.max_wall_millis = wall;
        let owner = dfmcp_adapter::construction_plan::clock::Clock::new(
            journal,
            spec.clone(),
            operations.clone(),
            selected.recovery,
            &canonical,
            &native,
        )?;
        // A rejected file must never replace the operator's acknowledged pin.
        // Only a successfully verified owner may promote a complete descendant.
        self.opening_recovery = Some(owner.recovery());
        Ok(owner)
    }
    fn opening_recovery(&self) -> Option<dfmcp_adapter::construction_plan::clock::Recovery> {
        self.opening_recovery
    }
    fn connect(
        &mut self,
        selected: &super::clock::Config,
        n: &OperationContext,
        effect: bool,
    ) -> Result<Self::Source> {
        clock_boundary(self.control, self.config, effect)?;
        if self.config.mode == BuildMode::Offline {
            return Err(denied());
        }
        if effect {
            n.authorize(
                dfmcp_core::Capability::ControlClock,
                dfmcp_core::RiskTier::Guarded,
                &[],
                None,
            )?;
        }
        if self.config.clock.as_ref() != Some(selected) {
            return Err(denied());
        }
        let token = required("DFMCP_BUILD_CLOCK_TOKEN", 256)?.into_bytes();
        if token.len() < 32 {
            return Err(denied());
        }
        let token_digest = dfmcp_core::Digest32::of_bytes(&token);
        let mut nonce = n.session_id.get().to_be_bytes().to_vec();
        nonce.extend_from_slice(&n.request_id.get().to_be_bytes());
        let inner = dfmcp_adapter::bounded_run::rpc::RunRpcClient::connect(
            selected.binding.endpoint(),
            token,
            nonce,
            n,
        )?;
        // Exact incarnation is required by the coordinator before prepare or
        // commit. Recovery cancellation may ask a newer incarnation for the
        // original record; it cannot authorize pausing a replacement fortress.
        if !inner.manifest().same_software(selected.binding.manifest())
            || inner.manifest().generation < selected.binding.manifest().generation
        {
            return Err(denied());
        }
        clock_boundary(self.control, self.config, effect)?;
        let source = ClockSource {
            inner,
            control: self.control,
            config: self.config,
            token_digest,
        };
        source.boundary(effect)?;
        Ok(source)
    }
    fn sample(
        &mut self,
        definition: &dfmcp_adapter::construction_plan::origin::MonitorDefinition,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::construction_plan::LinkedSample> {
        if self.config.clock_only {
            return Err(denied());
        }
        completion_sample(
            self.config,
            definition.origin().definition().binding(),
            definition.goal(),
            c,
        )
    }
    fn checkpoint(&mut self, c: &OperationContext, effect: bool) -> Result<()> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "clock request cancelled",
            ));
        }
        clock_boundary(self.control, self.config, effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(
        checkpoint: Option<&str>,
        protected: Option<&str>,
        mode: Option<&str>,
    ) -> Result<Config> {
        configured(
            "region1",
            "2",
            "[0,0,0,63,63,7]",
            "/private/build/journal",
            "127.0.0.1:5000",
            protected,
            checkpoint,
            mode,
        )
    }
    #[test]
    fn checkpoint_and_modes_have_closed_operator_spellings() -> Result<()> {
        assert_eq!(
            config(None, None, None)?.checkpoint,
            CheckpointPolicy::Required
        );
        assert_eq!(
            config(Some("disposable-fortress-no-checkpoint"), None, None)?.checkpoint,
            CheckpointPolicy::DisposableFortress
        );
        assert_eq!(
            config(None, None, Some("offline"))?.mode,
            BuildMode::Offline
        );
        for (spelling, mode) in [
            ("completion-recover", BuildMode::Recover),
            ("completion-offline", BuildMode::Offline),
        ] {
            let config = config(None, None, Some(spelling))?;
            assert_eq!(config.mode, mode);
            assert!(config.completion_only);
        }
        for v in ["true", "1", "none", ""] {
            assert!(config(Some(v), None, None).is_err());
            assert!(config(None, None, Some(v)).is_err());
        }
        Ok(())
    }
    #[test]
    fn production_and_other_profiles_are_rejected_even_with_empty_values() {
        let names = NAMES.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert!(isolated("1", names.clone(), false).is_ok());
        for extra in [
            "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
            "DFMCP_ALLOW_UNADMITTED_BUILD_V1_19",
            "DFMCP_DIG_TOKEN",
        ] {
            let mut changed = names.clone();
            changed.push(extra.into());
            assert!(isolated("1", changed, false).is_err());
        }
        assert!(isolated("true", names.clone(), false).is_err());
        assert!(isolated("1", names, true).is_err());
    }
    #[test]
    fn protected_regions_and_source_paths_are_strict() {
        for v in [
            "{}",
            "[[2,0,0,1,1,1]]",
            "[[0,0,0,1.0,1,1]]",
            "[[0,0,0,32768,1,1]]",
        ] {
            assert!(config(None, Some(v), None).is_err());
        }
        for path in [
            "relative",
            "/private/../journal",
            "/private//journal",
            "/private/./journal",
        ] {
            assert!(
                configured(
                    "region1",
                    "2",
                    "[0,0,0,63,63,7]",
                    path,
                    "127.0.0.1:5000",
                    None,
                    None,
                    None
                )
                .is_err()
            );
        }
        assert!(
            configured(
                "region1",
                "02",
                "[0,0,0,63,63,7]",
                "/private/journal",
                "127.0.0.1:5000",
                None,
                None,
                None
            )
            .is_err()
        );
    }
    #[test]
    fn runtime_owns_joined_worker() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let caller = std::thread::current().id();
        let value = crate::run_with_runtime_cx(|cx| async move {
            // Match production: a real reactor/pool runtime grants I/O but
            // does not install the optional async IoCap used by cx.io().
            assert!(cx.io().is_none());
            assert!(cx.capabilities().io);
            owned("fortress.doctor", move |control| {
                assert_ne!(caller, std::thread::current().id());
                assert!(control.check().is_ok());
                assert!(current_request_check().is_ok());
                assert!(control.worker.io().is_none());
                assert!(control.worker.capabilities().io);
                "owned".into()
            })
            .await
        })?;
        assert_eq!(value, "owned");
        Ok(())
    }
    #[test]
    fn restricted_runtime_cannot_create_fallback_worker()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let called = Arc::new(AtomicBool::new(false));
        let marker = called.clone();
        let value = crate::run_with_runtime_cx(|cx| async move {
            let _restricted = cx
                .restrict::<asupersync::cx::cap::None>()
                .set_current_restricted();
            owned("fortress.commit", move |_| {
                marker.store(true, Ordering::Release);
                String::new()
            })
            .await
        })?;
        assert!(!called.load(Ordering::Acquire));
        let parsed: serde_json::Value = serde_json::from_str(&value)?;
        assert_eq!(parsed["result"]["error"]["code"], "capability_denied");
        assert_eq!(parsed["result"]["retry_commit_permitted"], false);
        Ok(())
    }
    #[test]
    fn io_and_spawn_authority_are_each_required()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use asupersync::cx::cap::{CapMask, CapSet, CapSetRuntimeMask};
        let without_io = <CapSet<true, true, true, false, true> as CapSetRuntimeMask>::MASK;
        let without_spawn = <CapSet<false, true, true, true, true> as CapSetRuntimeMask>::MASK;
        for mask in [without_io, without_spawn, CapMask::none()] {
            let called = Arc::new(AtomicBool::new(false));
            let marker = called.clone();
            let value = crate::run_with_runtime_cx(|_| async move {
                let _restriction = Cx::push_restriction(mask);
                owned("fortress.doctor", move |_| {
                    marker.store(true, Ordering::Release);
                    String::new()
                })
                .await
            })?;
            assert!(!called.load(Ordering::Acquire));
            let parsed: serde_json::Value = serde_json::from_str(&value)?;
            assert_eq!(parsed["result"]["error"]["code"], "capability_denied");
        }
        Ok(())
    }
    #[test]
    fn missing_pool_cannot_fall_back_to_inline_work()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let called = Arc::new(AtomicBool::new(false));
        let marker = called.clone();
        let value = crate::run_with_runtime_cx(|cx| async move {
            let detached = cx.with_blocking_pool_handle(None);
            assert!(detached.capabilities().io);
            assert!(detached.capabilities().spawn);
            let _ambient = Cx::set_current(Some(detached));
            owned("fortress.doctor", move |_| {
                marker.store(true, Ordering::Release);
                String::new()
            })
            .await
        })?;
        assert!(!called.load(Ordering::Acquire));
        let parsed: serde_json::Value = serde_json::from_str(&value)?;
        assert_eq!(parsed["result"]["error"]["code"], "capability_denied");
        Ok(())
    }
}
