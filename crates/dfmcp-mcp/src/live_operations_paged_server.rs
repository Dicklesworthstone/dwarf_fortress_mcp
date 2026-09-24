//! Fixed, independently gated operations/1.4 entry. All query/monitoring handlers
//! are the existing operations handlers; only authenticated acquisition differs.

use super::*;
use dfmcp_adapter::live_jobs_rpc::operations::paged::{
    PagedOperationsLimits, PagedOperationsRpcClient,
};
use dfmcp_adapter::live_operations::OperationsProfile;

const PAGED_FAMILY: u128 = 3u128 << 60;
static NEXT_PAGED: LazyLock<Mutex<u128>> = LazyLock::new(|| Mutex::new(1));
static PAGED_SLOTS: AtomicUsize = AtomicUsize::new(0);

struct PagedSlot;
impl PagedSlot {
    fn reserve() -> Result<Self> {
        PAGED_SLOTS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "paged operations capacity is two sessions",
                )
            })?;
        Ok(Self)
    }
}
impl Drop for PagedSlot {
    fn drop(&mut self) {
        PAGED_SLOTS.fetch_sub(1, Ordering::AcqRel);
    }
}
struct PagedSource {
    client: PagedOperationsRpcClient<DeadlineStream>,
    _slot: PagedSlot,
}
impl OperationsSource for PagedSource {
    fn read(&mut self, timeout: Duration) -> Result<LiveOperationsObservation> {
        self.client.refresh(timeout)
    }
    fn poisoned(&self) -> bool {
        self.client.poisoned()
    }
    fn fence(&mut self) {
        self.client.fence();
    }
}
fn next_paged_id() -> Result<SessionId> {
    let mut next = lock(&NEXT_PAGED)?;
    if *next >= 1u128 << 60 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "paged session IDs exhausted",
        ));
    }
    let id = SessionId::new((1u128 << 127) | PAGED_FAMILY | *next);
    *next += 1;
    Ok(id)
}
fn allowed_paged_environment(name: &str) -> bool {
    !name.starts_with("DFMCP_")
        || matches!(
            name,
            "DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_4"
                | "DFMCP_OPERATIONS_PAGED_TOKEN"
                | "DFMCP_OPERATIONS_PAGED_ENDPOINT"
        )
}
fn validate_paged_environment() -> Result<()> {
    if std::env::var("DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_4")
        .ok()
        .as_deref()
        != Some("1")
        || std::env::vars_os().any(|(name, _)| !allowed_paged_environment(&name.to_string_lossy()))
        || crate::admission::current_admission_provenance().is_some()
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "operations/1.4 requires its exact development opt-in and refuses other profiles, journals, or production admission environment",
        ));
    }
    Ok(())
}

#[tool(
    description = "Open a read-only operations/1.4 session. Capture one coherent native world and transfer immutable pages. Supports up to 65536 items and 16 MiB, not arbitrary-size worlds. Profile, credentials and endpoint are fixed by the operator; no game effects or durable archive are enabled."
)]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(
    max_jobs: Option<u32>,
    max_buildings: Option<u32>,
    max_items: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_wall_millis: Option<u64>,
    page_bytes: Option<u32>,
    requested_capabilities: Option<Vec<String>>,
) -> String {
    let result = (|| -> Result<String> {
        validate_paged_environment()?;
        let capabilities = capabilities(requested_capabilities)?;
        let paged = PagedOperationsLimits {
            jobs: max_jobs.unwrap_or(4096),
            buildings: max_buildings.unwrap_or(4096),
            items: max_items.unwrap_or(65536),
            payload_bytes: usize::try_from(max_bytes.unwrap_or(16 * 1024 * 1024)).map_err(
                |_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "paged bytes do not fit this platform",
                    )
                },
            )?,
            page_bytes: usize::try_from(page_bytes.unwrap_or(65536)).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "page width does not fit this platform",
                )
            })?,
        };
        paged.validate()?;
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_game_ticks: 1_000_000,
            max_entities: paged.entity_limit(),
            max_bytes: paged.payload_bytes as u64,
            max_output_tokens: max_output_tokens.unwrap_or(8192),
            max_actions: 1,
        };
        budget.validate()?;
        if budget.max_bytes < 8192
            || !(2048..=65536).contains(&budget.max_output_tokens)
            || !(1..=60000).contains(&budget.max_wall_millis)
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "paged response or deadline is outside its bounds",
            ));
        }
        let slot = SessionSlot::reserve()?;
        let paged_slot = PagedSlot::reserve()?;
        let id = next_paged_id()?;
        let endpoint = std::env::var("DFMCP_OPERATIONS_PAGED_ENDPOINT")
            .unwrap_or_else(|_| "127.0.0.1:5000".to_owned());
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&endpoint)?;
        let token = std::env::var("DFMCP_OPERATIONS_PAGED_TOKEN").map_err(|_| {
            error(
                ErrorCode::CapabilityDenied,
                "DFMCP_OPERATIONS_PAGED_TOKEN is required",
            )
        })?;
        let client = PagedOperationsRpcClient::connect(
            endpoint,
            token.into_bytes(),
            id.get().to_be_bytes().to_vec(),
            Duration::from_millis(budget.max_wall_millis),
            paged,
        )?;
        let mut source = PagedSource {
            client,
            _slot: paged_slot,
        };
        let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
        state.publish(source.read(Duration::from_millis(budget.max_wall_millis))?)?;
        let fortress = state
            .snapshot()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "paged bootstrap lost snapshot",
                )
            })?
            .fortress_id;
        let grants = capabilities
            .iter()
            .map(|capability| CapabilityGrant {
                capability: *capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect();
        let limits = OperationsLimits {
            jobs: paged.jobs,
            buildings: paged.buildings,
            items: paged.items,
            payload_bytes: paged.payload_bytes,
        };
        let mut session = OperationsSession {
            id,
            source: Box::new(source),
            state,
            journal: None,
            limits,
            budget,
            grants,
            request: 0,
            _slot: slot,
        };
        let context = session.context()?;
        let response = packet(
            Some(&session),
            Some(&context),
            "fortress.open_session",
            json!({"ok":true,
            "granted_capabilities":capabilities.iter().map(|v|v.as_str()).collect::<Vec<_>>(),
            "acquisition":{"kind":"immutable_native_snapshot_pages","page_bytes":paged.page_bytes,
                "maximum_payload_bytes":paged.payload_bytes,"maximum_items":paged.items,"durable_archive":false,
                "snapshot_tick_is_capture_tick":true,"game_may_advance_during_transfer":true},
            "schema_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"mode":"schema"}}}),
        )?;
        let mut registry = lock(&SESSIONS)?;
        if registry.contains_key(&id) {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "paged session identity collision",
            ));
        }
        registry.insert(id, Arc::new(Mutex::new(session)));
        Ok(response)
    })();
    match result {
        Ok(response) => response,
        Err(failure) => AgentTurnBuilder::new("fortress.open_session", AgentPhase::Orient)
            .briefing(json!({"bridge_protocol":"1.4","runtime_admitted":false,"mutation_admissible":false}))
            .attach(json!({"ok":false,"error":{"code":failure.code.as_str(),"message":failure.message}})),
    }
}

pub fn run_stdio() {
    if let Err(failure) = validate_paged_environment() {
        eprintln!("{failure}");
        std::process::exit(1);
    }
    let server = ServerBuilder::new("dfmcp-live-operations-paged-dev", env!("CARGO_PKG_VERSION"))
        .tool(self::FortressOpenSession).tool(super::FortressObserve).tool(super::FortressQuery).tool(super::FortressPlan)
        .tool(super::FortressCommit).tool(super::FortressWait).tool(super::FortressCancel).tool(super::FortressCheckpoint)
        .tool(super::FortressRestore).tool(super::FortressExplain).tool(super::FortressDoctor).request_timeout(60)
        .instructions("Explicitly unadmitted operations/1.4. Open a paged session first. Jobs, buildings and inventory share one native capture; all pages are verified before publication. Snapshot game time is the capture time, not the transfer-completion time. Existing queries, production diagnosis, conditional allocation, baselines and foreground watches are available within their own budgets. No live effects, map coverage, citizen data, durable journal or production admission. A failed acquisition preserves the prior snapshot and fences the source.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "live_operations_paged_server_tests.rs"]
mod tests;
