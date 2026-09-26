//! Operator configuration and inherited, joined blocking work.
use super::{error, unbound};
use asupersync::Cx;
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

pub(super) const NAMES: [&str; 11] = [
    "DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19",
    "DFMCP_BUILD_WORLD_FOLDER",
    "DFMCP_BUILD_SITE_ID",
    "DFMCP_BUILD_SCOPE",
    "DFMCP_BUILD_JOURNAL",
    "DFMCP_BUILD_ENDPOINT",
    "DFMCP_BUILD_TOKEN",
    "DFMCP_BUILD_ALLOW_PLACE",
    "DFMCP_BUILD_CHECKPOINT_POLICY",
    "DFMCP_BUILD_PROTECTED",
    "DFMCP_BUILD_MODE",
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
    let mode = match mode {
        None | Some("control") => BuildMode::Control,
        Some("recover") => BuildMode::Recover,
        Some("offline") => BuildMode::Offline,
        _ => return Err(denied()),
    };
    Ok(Config {
        path: PathBuf::from(path),
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
pub(super) fn configuration() -> Result<Config> {
    isolated(
        &required("DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19", 1)?,
        std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()),
        crate::admission::current_admission_provenance().is_some(),
    )?;
    enabled()?;
    configured(
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
    )
}
pub(super) struct RequestControl {
    pub started: Instant,
    parent: Cx,
    worker: Cx,
    abandoned: Arc<AtomicBool>,
}
impl RequestControl {
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
    let _cancel = CancelOnDrop(abandoned.clone());
    let parent = cx.clone();
    let task = cx.spawn_blocking(move |worker| {
        let _ambient = Cx::set_current(Some(worker.clone()));
        let control = RequestControl {
            started,
            parent,
            worker,
            abandoned,
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
        Ok(v) => v,
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
    if &configuration()? != config || config.mode == BuildMode::Offline {
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
