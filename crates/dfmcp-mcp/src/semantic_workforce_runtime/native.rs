//! Operator-owned custody and existing citizen/workforce sources. Every path,
//! endpoint, source pin and credential stays outside the MCP request surface.
use super::identity::{CapsuleStore, create_private_capsule, open_private_capsule};
use super::policy::SourceAcceptance;
use super::*;
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::live_connect::LiveConnectionConfig;
use dfmcp_adapter::plan_archive::{create_private_plan_archive, open_private_plan_archive};
use dfmcp_adapter::semantic_workforce::evidence_owner::{
    CitizenEvidenceLimits, CitizenEvidenceSource, CitizenRpcSource, CitizenWorkforceEvidenceOwner,
};
use dfmcp_adapter::semantic_workforce::{
    open_private_association_store, open_private_goal_history_store,
};
use dfmcp_adapter::workforce_control::journal::{WorkforceBinding, WorkforceMode};
use dfmcp_adapter::workforce_control::private_file::{
    PrivateWorkforceFile, open_private_workforce_exact,
};
use dfmcp_adapter::workforce_control::rpc::{
    WorkforceManifest, WorkforceRpcClient, WorkforceSource, WorkforceTcpStream,
};
use dfmcp_adapter::workforce_control::{AssignmentEffect, AssignmentPlan, WorkforceCapture};
use dfmcp_adapter::{
    BridgeCredentials, BridgeManifest, LiveObservationSource, ObservationPage,
    parse_loopback_endpoint, read_complete_observation_bounded,
};
use dfmcp_world::EvidencePolicy;
use serde::Deserialize;
use std::net::{SocketAddr, TcpStream};
use std::path::{Component, PathBuf};
use std::sync::Arc;

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE";
const CONFIG: &str = "DFMCP_SEMANTIC_WORKFORCE_CONFIG";
const LABOR: &str = "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR";
const CITIZEN_TOKEN: &str = "DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN";
const WORKFORCE_TOKEN: &str = "DFMCP_SEMANTIC_WORKFORCE_TOKEN";
const ALLOWED: [&str; 5] = [OPT_IN, CONFIG, LABOR, CITIZEN_TOKEN, WORKFORCE_TOKEN];
const MAX_CONFIG_BYTES: usize = 16 * 1024;

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
            .map_err(|_| invalid("invalid external fortress identity"))?;
        if fortress == 0
            || fortress.to_string() != self.fortress_id
            || self.epoch >= u64::from(u32::MAX)
        {
            return Err(invalid("invalid external workforce recovery anchor"));
        }
        Ok(StateAnchor {
            fortress_id: dfmcp_core::FortressId::new(fortress),
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
    identity_digest: String,
    anchor: AnchorInput,
    #[serde(default)]
    history_present: bool,
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
    recovery: Option<RecoveryInput>,
}
#[derive(Clone)]
pub struct Config {
    directory: PathBuf,
    endpoint: SocketAddr,
    initialize: bool,
    journal_id: Option<Digest32>,
    sources: SourceAcceptance,
    recovery: Option<Recovery>,
    environment_digest: Digest32,
    runtime_check: Option<fn() -> Result<()>>,
}
impl Config {
    pub fn from_environment() -> Result<Self> {
        Self::validate_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator semantic workforce configuration is missing"))?;
        Self::parse(&raw)
    }
    /// Closed bounded operator JSON; never a client-controlled path or policy.
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
            return Err(invalid("operator workforce configuration exceeds 16 KiB"));
        }
        let input: ConfigInput = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed operator workforce configuration"))?;
        input.sources.validate()?;
        if input.schema != "dfmcp.semantic-workforce-development/1"
            || !input.directory.is_absolute()
            || input.directory.as_os_str().len() > 3800
            || input
                .directory
                .components()
                .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
            || input.initialize == input.journal_id.is_some()
            || (input.initialize && input.recovery.is_some())
        {
            return Err(invalid(
                "invalid workforce schema, normalized custody directory or exact initialization/recovery identity",
            ));
        }
        let endpoint = parse_loopback_endpoint(&input.endpoint)?;
        if endpoint.port() == 0 {
            return Err(invalid("native workforce endpoint requires a nonzero port"));
        }
        let journal_id = input.journal_id.as_deref().map(digest).transpose()?;
        let recovery = input.recovery.map(|recovery| {
            if Some(digest(&recovery.journal_id)?) != journal_id {
                return Err(denied("original workforce recovery and configured journal identity must agree exactly"));
            }
            Ok::<_, DfmcpError>(Recovery {
                seal: PlanArchiveSeal {
                    plan_digest: digest(&recovery.plan_digest)?,
                    object_digest: digest(&recovery.object_digest)?,
                },
                identity_digest: digest(&recovery.identity_digest)?,
                anchor: recovery.anchor.decode()?,
                history_present: recovery.history_present,
            })
        }).transpose()?;
        if recovery.is_some_and(|recovery| recovery.anchor.fortress_id != input.sources.fortress())
        {
            return Err(denied(
                "original workforce recovery belongs to another operator fortress",
            ));
        }
        Ok(Self {
            directory: input.directory,
            endpoint,
            initialize: input.initialize,
            journal_id,
            sources: input.sources,
            recovery,
            environment_digest: Digest32::of_bytes(raw.as_bytes()),
            runtime_check: None,
        })
    }
    pub fn validate_environment() -> Result<()> {
        if std::env::var(OPT_IN).ok().as_deref() != Some("1") {
            return Err(denied(
                "explicit unadmitted semantic workforce opt-in is required",
            ));
        }
        if let Some(value) = std::env::var_os(LABOR)
            && value.to_str() != Some("1")
        {
            return Err(denied(
                "operator labor enablement must be absent or exactly 1",
            ));
        }
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key.starts_with("DFMCP_") && !ALLOWED.contains(&key.as_ref()) {
                return Err(denied(
                    "isolated semantic workforce rejects production launch and other DFMCP profile state",
                ));
            }
        }
        Ok(())
    }
    pub fn labor_enabled() -> bool {
        std::env::var(LABOR).ok().as_deref() == Some("1")
    }
    fn current(&self) -> Result<()> {
        if let Some(check) = self.runtime_check {
            check()?;
        }
        Self::validate_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator workforce source configuration was removed"))?;
        if raw.len() > MAX_CONFIG_BYTES
            || Digest32::of_bytes(raw.as_bytes()) != self.environment_digest
        {
            return Err(denied(
                "operator workforce source acceptance changed; close this session",
            ));
        }
        Ok(())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    /// This preflight only rejects hidden or mismatched original history. It
    /// supplies no trusted file bytes; the descriptor-pinned owners below must
    /// still open and validate every required artifact with its external seal.
    fn original_custody(&self) -> Result<()> {
        fn present(path: PathBuf) -> Result<bool> {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() => Ok(true),
                Ok(_) => Err(denied(
                    "original workforce custody must be regular files, never links or special files",
                )),
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(_) => Err(denied(
                    "original workforce custody presence could not be determined",
                )),
            }
        }
        let plan = present(self.path("original.plan"))?;
        let identity = present(self.path("original.capsule"))?;
        let history = present(self.path("goal.history"))?;
        match self.recovery {
            None if !plan && !identity && !history => Ok(()),
            Some(recovery) if plan && identity && history == recovery.history_present => Ok(()),
            _ => Err(denied(
                "original workforce files require the exact external recovery handoff and history-presence flag",
            )),
        }
    }
    fn binding(&self) -> Result<WorkforceBinding> {
        WorkforceBinding::new(
            self.endpoint,
            WorkforceManifest {
                generation: self.sources.workforce_generation,
                df_version: self.sources.df_version.clone(),
                dfhack_version: self.sources.dfhack_version.clone(),
            },
            self.sources.world_folder.clone(),
            self.sources.site_id,
        )
    }
    fn citizens(&self, c: &OperationContext) -> Result<CitizenRpcSource<TcpStream>> {
        self.current()?;
        let timeout = Duration::from_millis(c.budget.max_wall_millis);
        let source = CitizenRpcSource::connect(
            &LiveConnectionConfig {
                endpoint: self.endpoint,
                connect_timeout: timeout,
                read_timeout: timeout,
                write_timeout: timeout,
                client_name: "dfmcp-semantic-workforce-dev".into(),
                client_version: env!("CARGO_PKG_VERSION").into(),
            },
            BridgeCredentials::new(token(CITIZEN_TOKEN)?, nonce(c, b"citizens"))?,
            c,
        )?;
        self.sources.citizen_manifest(&source.bridge_manifest())?;
        self.current()?;
        Ok(source)
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
fn nonce(c: &OperationContext, family: &[u8]) -> Vec<u8> {
    let mut bytes = b"dfmcp-semantic-workforce-native/1\0".to_vec();
    bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&c.request_id.get().to_be_bytes());
    bytes.extend_from_slice(family);
    Digest32::of_bytes(&bytes).as_bytes()[..16].to_vec()
}

/// Each native operation checks the live owner checkpoint and immutable operator
/// pins, including immediately after the coordinator syncs DispatchStarted.
pub struct PinnedNative {
    inner: WorkforceRpcClient<WorkforceTcpStream>,
    config: Arc<Config>,
    fenced: bool,
}
impl PinnedNative {
    fn check(&mut self, control: bool) -> Result<()> {
        if self.fenced {
            return Err(denied(
                "workforce source is fenced after an earlier boundary failure",
            ));
        }
        let result = (|| {
            self.config.current()?;
            self.config.sources.native_manifest(self.inner.manifest())?;
            if self.inner.endpoint() != Some(self.config.endpoint) {
                return Err(denied(
                    "workforce endpoint differs from exact operator pins",
                ));
            }
            if control && !Config::labor_enabled() {
                return Err(denied("operator labor authority was revoked"));
            }
            Ok(())
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
}
impl WorkforceSource for PinnedNative {
    fn manifest(&self) -> &WorkforceManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, ids: &[u32], c: &OperationContext) -> Result<WorkforceCapture> {
        self.check(false)?;
        let value = self.inner.observe(ids, c)?;
        self.check(false)?;
        Ok(value)
    }
    fn prepare(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.check(true)?;
        let value = self.inner.prepare(plan, c)?;
        self.check(false)?;
        Ok(value)
    }
    fn commit(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.check(true)?;
        let value = self.inner.commit(plan, c)?;
        self.check(false)?;
        Ok(value)
    }
    fn query(
        &mut self,
        plan: &AssignmentPlan,
        c: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        self.check(false)?;
        let value = self.inner.query(plan, c)?;
        self.check(false)?;
        Ok(value)
    }
    fn cancel(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.check(true)?;
        let value = self.inner.cancel(plan, c)?;
        self.check(false)?;
        Ok(value)
    }
}

type CitizenFactory =
    Box<dyn FnMut(&OperationContext) -> Result<CitizenRpcSource<TcpStream>> + Send>;
type CitizenAuthorizer = Box<
    dyn FnMut(
            &LiveObservationCapsule,
            &LiveWorldProjection,
            &OperationContext,
        ) -> Result<EvidencePolicy>
        + Send,
>;
pub type NativeEvidence = CitizenWorkforceEvidenceOwner<CitizenFactory, CitizenAuthorizer>;
pub struct NativeShell {
    config: Arc<Config>,
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
            },
            mode,
            session,
            budget,
        )
    }
}

struct Pages<'a> {
    source: &'a mut CitizenRpcSource<TcpStream>,
    work: &'a mut Work,
    manifest: BridgeManifest,
}
impl LiveObservationSource for Pages<'_> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.manifest.clone()
    }
    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
    ) -> Result<ObservationPage> {
        let c = self.work.reserve(1024 * 1024)?;
        let page = self.source.read_page(offset, maximum, names, &c)?;
        if self.source.bridge_manifest() != self.manifest {
            return Err(denied(
                "citizen incarnation changed during complete acquisition",
            ));
        }
        Ok(page)
    }
}
impl Shell for NativeShell {
    type Journal = PrivateWorkforceFile;
    type Association = PrivateBuildFile;
    type Native = PinnedNative;
    type Archive = PrivateBuildFile;
    type Identity = PrivateBuildFile;
    type History = PrivateBuildFile;
    type Evidence = NativeEvidence;
    fn acceptance(&self) -> &SourceAcceptance {
        &self.config.sources
    }
    fn recovery(&self) -> Option<Recovery> {
        self.config.recovery
    }
    fn check_environment(&self) -> Result<()> {
        self.config.current()
    }
    fn labor_enabled(&self) -> bool {
        Config::labor_enabled()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkforceSession<Self::Journal>,
        AssociationStore<Self::Association>,
        Digest32,
    )> {
        self.config.current()?;
        let mut work = Work::new(c)?;
        self.config.original_custody()?;
        let journal_mode = match mode {
            Mode::Control if !Config::labor_enabled() => WorkforceMode::Recover,
            _ => mode.journal(),
        };
        if self.config.initialize && journal_mode != WorkforceMode::Control {
            return Err(denied(
                "initialization requires explicit operator labor control",
            ));
        }
        let journal = open_private_workforce_exact(
            &self.config.path("workforce.journal"),
            &work.reserve(208 * MIB)?,
            journal_mode,
            Some(self.config.binding()?),
            self.config.initialize,
        )?;
        let (native, view) = WorkforceSession::new(journal, &work.reserve(72 * MIB)?)?;
        if self
            .config
            .journal_id
            .is_some_and(|expected| expected != view.id)
        {
            return Err(denied(
                "workforce journal differs from the exact external handoff identity",
            ));
        }
        self.config.current()?;
        let associations = open_private_association_store(
            &self.config.path("semantic.associations"),
            view.id,
            self.config.initialize,
            journal_mode != WorkforceMode::Control,
            &work.reserve(4 * MIB)?,
        )?;
        self.config.current()?;
        Ok((native, associations, view.id))
    }
    fn native(&self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Self::Native> {
        self.config.current()?;
        if binding != &self.config.binding()? {
            return Err(denied(
                "workforce factory cannot replace the original exact source binding",
            ));
        }
        let inner = WorkforceRpcClient::connect(
            self.config.endpoint,
            token(WORKFORCE_TOKEN)?,
            nonce(c, b"workforce"),
            c,
        )?;
        self.config.sources.native_manifest(inner.manifest())?;
        self.config.current()?;
        Ok(PinnedNative {
            inner,
            config: Arc::clone(&self.config),
            fenced: false,
        })
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveObservationCapsule> {
        self.config.current()?;
        let mut work = Work::new(c)?;
        let mut source = self.config.citizens(&work.reserve(2 * MIB)?)?;
        let manifest = source.bridge_manifest();
        let mut pages = Pages {
            source: &mut source,
            work: &mut work,
            manifest,
        };
        let capsule = read_complete_observation_bounded(&mut pages, 128, false, 128)?;
        drop(source);
        work.current()?;
        self.config.current()?;
        if capsule.world_folder != self.config.sources.world_folder
            || capsule.site_id != self.config.sources.site_id as i32
        {
            return Err(denied(
                "complete citizen acquisition belongs to another fortress",
            ));
        }
        Ok(capsule)
    }
    fn evidence(
        &self,
        capsule: LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<Self::Evidence> {
        self.config.current()?;
        let source = Arc::clone(&self.config);
        let authority = Arc::clone(&self.config);
        let factory: CitizenFactory = Box::new(move |c| source.citizens(c));
        let authorizer: CitizenAuthorizer = Box::new(move |capsule, projection, c| {
            authority.current()?;
            authority.sources.citizens(capsule, projection, c)
        });
        CitizenWorkforceEvidenceOwner::new::<CitizenRpcSource<TcpStream>>(
            capsule,
            CitizenEvidenceLimits {
                page_size: 128,
                max_citizens: 128,
            },
            factory,
            authorizer,
            c,
        )
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
        open_private_plan_archive(
            &self.config.path("original.plan"),
            recovery.seal,
            archive_limits(),
            c,
        )
    }
    fn create_identity(
        &self,
        capsule: &LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Self::Identity>> {
        self.config.current()?;
        create_private_capsule(&self.config.path("original.capsule"), capsule, c)
    }
    fn recover_identity(
        &self,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Self::Identity>> {
        self.config.current()?;
        open_private_capsule(&self.config.path("original.capsule"), seal, c)
    }
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticWorkforceReview,
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
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
