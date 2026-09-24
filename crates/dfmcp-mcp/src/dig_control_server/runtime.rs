//! Explicit operator selection and inherited, supervised blocking ownership.
use super::{error, unbound};
use dfmcp_adapter::dig_control_policy::DigCheckpointPolicy;
use dfmcp_adapter::dig_designation::journal::session::DigSessionGuard;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigGuard, DigStage};
use dfmcp_adapter::dig_designation::rpc::{DigRpcClient, DigTcpStream};
use dfmcp_adapter::dig_designation::{DigPlan, DigRegion};
use dfmcp_core::{ErrorCode, MapCoord, MapCuboid, OperationContext, Result};
use asupersync::Cx;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

const NAMES: [&str; 10] = [
    "DFMCP_ALLOW_UNADMITTED_DIG_CONTROL_V1_16",
    "DFMCP_DIG_WORLD_FOLDER",
    "DFMCP_DIG_SITE_ID",
    "DFMCP_DIG_SCOPE",
    "DFMCP_DIG_JOURNAL",
    "DFMCP_DIG_ENDPOINT",
    "DFMCP_DIG_TOKEN",
    "DFMCP_DIG_ALLOW_DESIGNATE",
    "DFMCP_DIG_CHECKPOINT_POLICY",
    "DFMCP_DIG_PROTECTED",
];
fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "mining control operator/runtime boundary refused",
    )
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub path: PathBuf,
    pub scope: MapCuboid,
    pub folder: String,
    pub site: u32,
    pub endpoint: SocketAddr,
    pub protected: Vec<MapCuboid>,
    pub checkpoint: DigCheckpointPolicy,
}
impl Config {
    pub fn fortress(&self) -> dfmcp_core::FortressId {
        dfmcp_adapter::workforce_control::fortress_id(&self.folder, self.site)
    }
    pub fn matches(&self, b: &DigBinding) -> Result<()> {
        if b.folder() != self.folder
            || b.site() != self.site
            || b.scope() != self.scope
            || b.endpoint() != self.endpoint
        {
            return Err(denied());
        }
        Ok(())
    }
    pub fn region(&self, region: DigRegion) -> Result<()> {
        if !self.scope.contains_cuboid(region.halo())
            || !self.scope.contains_cuboid(region.write_area())
        {
            return Err(denied());
        }
        Ok(())
    }
}
fn value(name: &str, limit: usize) -> Result<String> {
    let v = std::env::var(name).map_err(|_| denied())?;
    if v.is_empty() || v.len() > limit || v.contains('\0') {
        return Err(denied());
    }
    Ok(v)
}
fn optional(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        _ => Err(denied()),
    }
}
pub(super) fn enabled() -> Result<bool> {
    match optional("DFMCP_DIG_ALLOW_DESIGNATE")?.as_deref() {
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
fn configured(
    folder: String,
    site: String,
    scope: String,
    path: String,
    endpoint: String,
    protected: Option<&str>,
    checkpoint: Option<&str>,
) -> Result<Config> {
    if folder.is_empty()
        || folder.len() > 512
        || folder.contains('\0')
        || site.len() > 10
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
    let number: u32 = site.parse().map_err(|_| denied())?;
    if number > i32::MAX as u32 || number.to_string() != site {
        return Err(denied());
    }
    let scope = area(serde_json::from_str(&scope).map_err(|_| denied())?)?;
    let endpoint: SocketAddr = endpoint.parse().map_err(|_| denied())?;
    if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
        return Err(denied());
    }
    let raw = protected.map_or("[]", |v| v);
    if raw.len() > 4096 {
        return Err(denied());
    }
    let values: Vec<[i32; 6]> = serde_json::from_str(raw).map_err(|_| denied())?;
    if values.len() > 32 {
        return Err(denied());
    }
    let protected = values.into_iter().map(area).collect::<Result<Vec<_>>>()?;
    let checkpoint = match checkpoint {
        None | Some("required") => DigCheckpointPolicy::Required,
        Some("disposable-fortress-no-checkpoint") => DigCheckpointPolicy::DisposableFortress,
        _ => return Err(denied()),
    };
    Ok(Config {
        folder,
        site: number,
        scope,
        path: PathBuf::from(path),
        endpoint,
        protected,
        checkpoint,
    })
}
fn isolated(opt: &str, names: impl IntoIterator<Item = String>, admitted: bool) -> Result<()> {
    if opt != "1" || admitted {
        return Err(denied());
    }
    let mut count = 0;
    for name in names {
        count += 1;
        if count > 1024
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
        &value("DFMCP_ALLOW_UNADMITTED_DIG_CONTROL_V1_16", 1)?,
        std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()),
        crate::admission::current_admission_provenance().is_some(),
    )?;
    enabled()?;
    let endpoint =
        optional("DFMCP_DIG_ENDPOINT")?.map_or_else(|| "127.0.0.1:5000".to_owned(), |v| v);
    configured(
        value("DFMCP_DIG_WORLD_FOLDER", 512)?,
        value("DFMCP_DIG_SITE_ID", 10)?,
        value("DFMCP_DIG_SCOPE", 128)?,
        value("DFMCP_DIG_JOURNAL", 4096)?,
        endpoint,
        optional("DFMCP_DIG_PROTECTED")?.as_deref(),
        optional("DFMCP_DIG_CHECKPOINT_POLICY")?.as_deref(),
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
                "mining request cancelled or abandoned",
            ));
        }
        Ok(())
    }
    fn check(&self) -> Result<()> {
        self.checkpoint()?;
        if self.parent.io().is_none() || self.worker.io().is_none() {
            return Err(denied());
        }
        Ok(())
    }
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
    if cx.checkpoint().is_err() {
        return unbound(
            op,
            &error(ErrorCode::CancellationRequested, "mining request cancelled"),
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
        if let Err(e) = control.checkpoint() {
            return unbound(op, &e);
        }
        f(control)
    });
    let mut task = match task {
        Ok(task) => task,
        Err(_) => return unbound(op, &denied()),
    };
    match task.join(&cx).await {
        Ok(value) => value,
        Err(_) => unbound(
            op,
            &error(
                ErrorCode::CancellationIncomplete,
                "mining worker has not yielded an acknowledged outcome; recover the journal",
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
    pub binding: &'a DigBinding,
}
impl DigGuard for Guard<'_> {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, _: &OperationContext) -> Result<()> {
        boundary(
            self.control,
            self.config,
            matches!(
                stage,
                DigStage::Prepare | DigStage::Commit | DigStage::Cancel
            ),
        )?;
        self.config.matches(self.binding)?;
        self.config.region(plan.before().region())
    }
}
impl DigSessionGuard for Guard<'_> {
    fn connect(&mut self, b: &DigBinding, r: DigRegion, _: &OperationContext) -> Result<()> {
        boundary(self.control, self.config, false)?;
        if b != self.binding {
            return Err(denied());
        }
        self.config.matches(b)?;
        self.config.region(r)
    }
    fn observe(&mut self, b: &DigBinding, r: DigRegion, c: &OperationContext) -> Result<()> {
        self.connect(b, r, c)
    }
}
pub(super) fn connect(
    control: &RequestControl,
    config: &Config,
    r: DigRegion,
    c: &OperationContext,
) -> Result<DigRpcClient<DigTcpStream>> {
    boundary(control, config, false)?;
    config.region(r)?;
    let token = value("DFMCP_DIG_TOKEN", 256)?.into_bytes();
    if token.len() < 32 {
        return Err(denied());
    }
    let mut nonce = c.session_id.get().to_be_bytes().to_vec();
    nonce.extend_from_slice(&c.request_id.get().to_be_bytes());
    DigRpcClient::connect(config.endpoint, token, nonce, r, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(checkpoint: Option<&str>, protected: Option<&str>) -> Result<Config> {
        configured(
            "region1".into(),
            "1".into(),
            "[0,0,0,63,63,7]".into(),
            "/private/dig/journal".into(),
            "127.0.0.1:5000".into(),
            protected,
            checkpoint,
        )
    }
    #[test]
    fn checkpoint_exception_is_explicit_and_never_a_boolean() -> Result<()> {
        assert_eq!(
            config(None, None)?.checkpoint,
            DigCheckpointPolicy::Required
        );
        assert_eq!(
            config(Some("disposable-fortress-no-checkpoint"), None)?.checkpoint,
            DigCheckpointPolicy::DisposableFortress
        );
        for raw in ["", "true", "1", "none"] {
            assert!(config(Some(raw), None).is_err());
        }
        Ok(())
    }
    #[test]
    fn protected_regions_are_strict_bounded_cuboids() {
        assert!(config(None, Some("[[0,0,0,1,1,1]]")).is_ok());
        for raw in [
            "{}",
            "[[true,0,0,1,1,1]]",
            "[[0,0,0,1.0,1,1]]",
            "[[2,0,0,1,1,1]]",
            "[[0,0,0,32768,1,1]]",
        ] {
            assert!(config(None, Some(raw)).is_err());
        }
        let many =
            serde_json::to_string(&vec![[0, 0, 0, 1, 1, 1]; 33]).map_or(String::new(), |v| v);
        assert!(config(None, Some(&many)).is_err());
    }
    #[test]
    fn recovery_and_production_environment_cannot_select_control() {
        let names = NAMES.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert!(isolated("1", names.clone(), false).is_ok());
        for extra in [
            "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
            "DFMCP_ALLOW_UNADMITTED_DIG_RECOVERY_V1_16",
            "DFMCP_DIG_RECOVERY_ONLINE",
        ] {
            let mut changed = names.clone();
            changed.push(extra.into());
            assert!(isolated("1", changed, false).is_err());
        }
        assert!(isolated("true", names.clone(), false).is_err());
        assert!(isolated("1", names, true).is_err());
    }

    #[test]
    fn owned_control_entry_keeps_runtime_worker_ownership()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let caller = std::thread::current().id();
        let result = crate::run_with_runtime_cx(|_| async move {
            owned("fortress.doctor", move |control| {
                assert_ne!(std::thread::current().id(), caller);
                assert!(control.check().is_ok());
                assert!(Cx::current().is_some());
                "owned".into()
            })
            .await
        })?;
        assert_eq!(result, "owned");
        assert!(Cx::current().is_none());
        Ok(())
    }
    #[test]
    fn restricted_runtime_cannot_spawn_a_fallback_mutation_worker()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let called = Arc::new(AtomicBool::new(false));
        let worker = called.clone();
        let result = crate::run_with_runtime_cx(|cx| async move {
            let _scope = cx
                .restrict::<asupersync::cx::cap::None>()
                .set_current_restricted();
            owned("fortress.commit", move |_| {
                worker.store(true, Ordering::Release);
                String::new()
            })
            .await
        })?;
        assert!(!called.load(Ordering::Acquire));
        let value: serde_json::Value = serde_json::from_str(&result)?;
        assert_eq!(value["result"]["ok"], false);
        assert_eq!(value["result"]["retry_commit_permitted"], false);
        Ok(())
    }
}
