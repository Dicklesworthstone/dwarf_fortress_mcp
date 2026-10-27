//! Operator-only bootstrap and concrete existing protocol/private-file resources.
//! The isolated executable rejects production launch state and accepts no path,
//! endpoint, credential, source grant or profile selector from MCP requests.
use super::policy::SourceAcceptance;
use super::*;
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::live_connect::LiveConnectionConfig;
use dfmcp_adapter::live_jobs_rpc::{
    DeadlineStream,
    operations::{OperationsLimits, OperationsRpcClient},
};
use dfmcp_adapter::plan_archive::{create_private_plan_archive, open_private_plan_archive};
use dfmcp_adapter::semantic_work_orders::{
    open_private_association_store, open_private_goal_history_store,
};
use dfmcp_adapter::semantic_workforce::evidence_owner::{
    CitizenEvidenceLimits, CitizenEvidenceSource, CitizenRpcSource, CitizenWorkforceEvidenceOwner,
};
use dfmcp_adapter::work_order_control::{
    PrivateWorkOrderFile, WorkOrderQuerySource, open_private_work_orders_exact,
};
use dfmcp_adapter::work_order_progress::rpc::{ProgressRpcClient, ProgressTcpStream};
use dfmcp_adapter::work_orders::rpc::{WorkOrderManifest, WorkOrderRpcClient, WorkOrderTcpStream};
use dfmcp_adapter::work_orders::{WorkOrderEffect, WorkOrderObservation, WorkOrderPlan};
use dfmcp_adapter::{
    BridgeCredentials, BridgeManifest, LiveObservationSource, ObservationPage,
    parse_loopback_endpoint, read_complete_observation_bounded,
};
use dfmcp_world::EvidencePolicy;
use serde::Deserialize;
use std::net::{SocketAddr, TcpStream};
use std::path::{Component, PathBuf};
use std::sync::Arc;

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS";
const CONFIG: &str = "DFMCP_SEMANTIC_WORK_ORDER_CONFIG";
const PRODUCTION: &str = "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION";
const CITIZEN_TOKEN: &str = "DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN";
const ORDER_TOKEN: &str = "DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN";
const PROGRESS_TOKEN: &str = "DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN";
const OPERATIONS_TOKEN: &str = "DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN";
const ALLOWED: [&str; 7] = [
    OPT_IN,
    CONFIG,
    PRODUCTION,
    CITIZEN_TOKEN,
    ORDER_TOKEN,
    PROGRESS_TOKEN,
    OPERATIONS_TOKEN,
];
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
        let id = self
            .fortress_id
            .parse::<u64>()
            .map_err(|_| invalid("invalid external fortress identity"))?;
        if id == 0 || id.to_string() != self.fortress_id || self.epoch >= u64::from(u32::MAX) {
            return Err(invalid("invalid external recovery anchor"));
        }
        Ok(StateAnchor {
            fortress_id: dfmcp_core::FortressId::new(id),
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
    plan_digest: String,
    object_digest: String,
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
    sources: SourceAcceptance,
    recovery: Option<RecoveryInput>,
}
#[derive(Clone)]
pub struct Config {
    directory: PathBuf,
    endpoint: SocketAddr,
    initialize: bool,
    sources: SourceAcceptance,
    recovery: Option<Recovery>,
    environment_digest: Digest32,
}
impl Config {
    /// Bounded JSON in operator environment, not an MCP parameter or path to a
    /// caller-selected policy file. Secrets remain separate and never serialize.
    pub fn from_environment() -> Result<Self> {
        Self::validate_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator semantic runtime configuration is missing"))?;
        Self::parse(&raw)
    }
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
            return Err(invalid("operator configuration exceeds 16 KiB"));
        }
        let input: ConfigInput = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed operator semantic runtime configuration"))?;
        input.sources.validate()?;
        if input.schema != "dfmcp.semantic-work-order-development/1"
            || !input.directory.is_absolute()
            || input.directory.as_os_str().len() > 3800
            || input
                .directory
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
            || (input.initialize && input.recovery.is_some())
        {
            return Err(invalid(
                "invalid isolated development schema, normalized custody directory or recovery mode",
            ));
        }
        let endpoint = parse_loopback_endpoint(&input.endpoint)?;
        if endpoint.port() == 0 {
            return Err(invalid("native endpoint requires a nonzero port"));
        }
        let recovery = input
            .recovery
            .map(|r| {
                Ok::<_, DfmcpError>(Recovery {
                    seal: PlanArchiveSeal {
                        plan_digest: digest(&r.plan_digest)?,
                        object_digest: digest(&r.object_digest)?,
                    },
                    anchor: r.anchor.decode()?,
                    history_present: r.history_present,
                })
            })
            .transpose()?;
        if recovery.is_some_and(|r| r.anchor.fortress_id != input.sources.fortress()) {
            return Err(denied(
                "external archive handoff belongs to another operator fortress",
            ));
        }
        Ok(Self {
            directory: input.directory,
            endpoint,
            initialize: input.initialize,
            sources: input.sources,
            recovery,
            environment_digest: Digest32::of_bytes(raw.as_bytes()),
        })
    }
    pub fn validate_environment() -> Result<()> {
        if std::env::var(OPT_IN).ok().as_deref() != Some("1") {
            return Err(denied(
                "explicit unadmitted semantic work-order development opt-in is required",
            ));
        }
        if let Some(value) = std::env::var_os(PRODUCTION) {
            if value.to_str() != Some("1") {
                return Err(denied(
                    "operator production enablement must be absent or exactly 1",
                ));
            }
        }
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key.starts_with("DFMCP_") && !ALLOWED.contains(&key.as_ref()) {
                return Err(denied(
                    "isolated semantic runtime rejects production launch and other DFMCP profile state",
                ));
            }
        }
        Ok(())
    }
    pub fn production_enabled() -> bool {
        std::env::var(PRODUCTION).ok().as_deref() == Some("1")
    }
    fn current(&self) -> Result<()> {
        Self::validate_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator source configuration was removed"))?;
        if raw.len() > MAX_CONFIG_BYTES
            || Digest32::of_bytes(raw.as_bytes()) != self.environment_digest
        {
            return Err(denied(
                "operator source acceptance changed; close this session",
            ));
        }
        Ok(())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    fn timeout(c: &OperationContext) -> Duration {
        Duration::from_millis(c.budget.max_wall_millis)
    }
    fn citizen_config(&self, c: &OperationContext) -> LiveConnectionConfig {
        let timeout = Self::timeout(c);
        LiveConnectionConfig {
            endpoint: self.endpoint,
            connect_timeout: timeout,
            read_timeout: timeout,
            write_timeout: timeout,
            client_name: "dfmcp-semantic-order-dev".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
    fn citizens(&self, c: &OperationContext) -> Result<CitizenRpcSource<TcpStream>> {
        self.current()?;
        let source = CitizenRpcSource::connect(
            &self.citizen_config(c),
            BridgeCredentials::new(token(CITIZEN_TOKEN)?, nonce(c, b"citizens"))?,
            c,
        )?;
        self.sources.citizen_manifest(&source.bridge_manifest())?;
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
    let mut bytes = b"dfmcp-semantic-native-session/1\0".to_vec();
    bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&c.request_id.get().to_be_bytes());
    bytes.extend_from_slice(family);
    Digest32::of_bytes(&bytes).as_bytes()[..16].to_vec()
}

/// Actual mutation transport with the operator gate checked at every edge,
/// including the insertion edge after the coordinator's DispatchStarted sync.
pub struct PinnedNative {
    inner: WorkOrderRpcClient<WorkOrderTcpStream>,
    config: Arc<Config>,
}
impl PinnedNative {
    fn check(&mut self, control: bool) -> Result<()> {
        let result = (|| {
            self.config.current()?;
            self.config.sources.native_manifest(self.inner.manifest())?;
            if control && !Config::production_enabled() {
                return Err(denied("operator production authority was revoked"));
            }
            Ok(())
        })();
        if result.is_err() {
            self.inner.fence();
        }
        result
    }
}
impl WorkOrderQuerySource for PinnedNative {
    fn manifest(&self) -> &WorkOrderManifest {
        self.inner.manifest()
    }
    fn fence(&mut self) {
        self.inner.fence();
    }
    fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
        self.check(false)?;
        let value = self.inner.read_orders(timeout)?;
        self.check(false)?;
        Ok(value)
    }
    fn query(&mut self, p: &WorkOrderPlan, timeout: Duration) -> Result<Option<WorkOrderEffect>> {
        self.check(false)?;
        let value = self.inner.query(p, timeout)?;
        self.check(false)?;
        Ok(value)
    }
}
impl WorkOrderSource for PinnedNative {
    fn prepare(&mut self, p: &WorkOrderPlan, timeout: Duration) -> Result<WorkOrderEffect> {
        self.check(true)?;
        let value = self.inner.prepare(p, timeout)?.effect().clone();
        self.check(false)?;
        Ok(value)
    }
    fn commit(
        &mut self,
        p: &WorkOrderPlan,
        prepared: &WorkOrderEffect,
        timeout: Duration,
    ) -> Result<WorkOrderEffect> {
        self.check(true)?;
        let value = self.inner.commit_prepared(p, prepared, timeout)?;
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
    type Journal = PrivateWorkOrderFile;
    type Association = PrivateBuildFile;
    type Native = PinnedNative;
    type Archive = PrivateBuildFile;
    type History = PrivateBuildFile;
    type Evidence = NativeEvidence;
    type Progress = ProgressRpcClient<ProgressTcpStream>;
    type Operations = OperationsRpcClient<DeadlineStream>;
    fn acceptance(&self) -> &SourceAcceptance {
        &self.config.sources
    }
    fn recovery(&self) -> Option<Recovery> {
        self.config.recovery
    }
    fn check_environment(&self) -> Result<()> {
        self.config.current()
    }
    fn production_enabled(&self) -> bool {
        Config::production_enabled()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkOrderSession<Self::Journal, Self::Native>,
        AssociationStore<Self::Association>,
        Digest32,
    )> {
        self.config.current()?;
        if self.config.initialize && mode != Mode::Control {
            return Err(denied(
                "initialization requires explicit operator control mode",
            ));
        }
        let mut work = Work::new(c)?;
        let native_source = if mode == Mode::Offline {
            None
        } else {
            let native_context = work.reserve(4 * MIB)?;
            let inner = WorkOrderRpcClient::connect(
                self.config.endpoint,
                token(ORDER_TOKEN)?,
                nonce(&native_context, b"orders"),
                Config::timeout(&native_context),
            )?;
            self.config.sources.native_manifest(inner.manifest())?;
            Some(PinnedNative {
                inner,
                config: Arc::clone(&self.config),
            })
        };
        let journal = open_private_work_orders_exact(
            &self.config.path("creation.journal"),
            &work.reserve(68 * MIB)?,
            mode.journal(),
            self.config.initialize,
        )?;
        let native = WorkOrderSession::new(journal, native_source, &work.reserve(MIB)?)?;
        let identity = native.summary(&work.reserve(MIB)?)?.journal_id;
        let associations = open_private_association_store(
            &self.config.path("semantic.associations"),
            identity,
            self.config.initialize,
            mode != Mode::Control,
            &work.reserve(4 * MIB)?,
        )?;
        Ok((native, associations, identity))
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
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticWorkOrderReview,
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
    fn progress(&self, c: &OperationContext) -> Result<Self::Progress> {
        self.config.current()?;
        let source = ProgressRpcClient::connect(
            self.config.endpoint,
            token(PROGRESS_TOKEN)?,
            nonce(c, b"progress"),
            Config::timeout(c),
        )?;
        let manifest = source.manifest();
        if manifest.generation != self.config.sources.progress_generation
            || manifest.df_version != self.config.sources.df_version
            || manifest.dfhack_version != self.config.sources.dfhack_version
        {
            return Err(denied(
                "selected progress source differs from operator pins",
            ));
        }
        Ok(source)
    }
    fn operations(&self, c: &OperationContext) -> Result<Self::Operations> {
        self.config.current()?;
        OperationsRpcClient::connect(
            self.config.endpoint,
            token(OPERATIONS_TOKEN)?,
            nonce(c, b"operations"),
            Config::timeout(c),
            OperationsLimits {
                jobs: 1024,
                buildings: 1024,
                items: 2046,
                payload_bytes: 2 * 1024 * 1024,
            },
        )
    }
}
