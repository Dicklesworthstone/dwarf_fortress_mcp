//! Bounded, foreground, Query-only watches over a selected native order.
//! Watches are process-local; neither persistence nor downtime continuity is claimed.
use super::rpc::{ProgressClient, ProgressManifest, ProgressStream, RPC_RESERVE_BYTES};
use super::{MAX_PROGRESS_BYTES, MAX_WATCHES, OrderProgress, ProgressWatch, error};
use dfmcp_core::{
    Capability, Digest32, ErrorCode, FortressId, GameTick, OperationContext, Result, RiskTier,
    SessionId,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const WATCH_RESERVE_BYTES: u64 = 8192;

pub trait OrderProgressSource {
    fn manifest(&self) -> &ProgressManifest;
    fn read_order(&mut self, order: u32, timeout: Duration) -> Result<OrderProgress>;
    fn fence(&mut self);
}
impl<S: ProgressStream> OrderProgressSource for ProgressClient<S> {
    fn manifest(&self) -> &ProgressManifest {
        ProgressClient::manifest(self)
    }
    fn read_order(&mut self, order: u32, timeout: Duration) -> Result<OrderProgress> {
        ProgressClient::read_order(self, order, timeout)
    }
    fn fence(&mut self) {
        ProgressClient::fence(self);
    }
}
pub struct ProgressSession<N> {
    source: N,
    id: SessionId,
    fortress: FortressId,
    manifest: ProgressManifest,
    selected: Option<OrderProgress>,
    watches: BTreeMap<String, ProgressWatch>,
    tick: u64,
    sequence: u64,
    horizon: u32,
    fenced: bool,
}
impl<N: OrderProgressSource> ProgressSession<N> {
    pub fn new(source: N, context: &OperationContext) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if context.anchor.fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "progress requires an explicit fortress lineage",
            ));
        }
        let manifest = source.manifest().clone();
        if manifest.generation == 0
            || manifest.generation == u64::MAX
            || [&manifest.df_version, &manifest.dfhack_version]
                .iter()
                .any(|v| v.is_empty() || v.len() > 128 || v.contains('\0'))
        {
            return Err(error(
                ErrorCode::VersionMismatch,
                "invalid progress source identity",
            ));
        }
        Ok(Self {
            source,
            id: context.session_id,
            fortress: context.anchor.fortress_id,
            manifest,
            selected: None,
            watches: BTreeMap::new(),
            tick: context.anchor.tick.get(),
            sequence: 0,
            horizon: 0,
            fenced: false,
        })
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.id || context.anchor.fortress_id != self.fortress {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "progress belongs to another session or fortress",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = GameTick(self.tick.max(context.anchor.tick.get()));
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn fence(&mut self) {
        self.selected = None;
        self.fenced = true;
        self.source.fence();
    }
    pub fn poisoned(&self) -> bool {
        self.fenced
    }
    pub fn selected(&self, context: &OperationContext) -> Result<Option<&OrderProgress>> {
        self.access(context)?;
        Ok(self.selected.as_ref())
    }
    pub fn observe(&mut self, order: u32, context: &OperationContext) -> Result<OrderProgress> {
        let started = Instant::now();
        self.access(context)?;
        if order > i32::MAX as u32 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "native order ID out of range",
            ));
        }
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "progress source fenced; explicitly reopen and re-register watches",
            ));
        }
        if context.budget.max_entities < 4096
            || context.budget.max_bytes < RPC_RESERVE_BYTES + 2 * MAX_PROGRESS_BYTES as u64
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "progress read requires complete bounded queue scan and RPC allowance",
            ));
        }
        self.selected = None;
        let timeout = Duration::from_millis(context.budget.max_wall_millis.min(60_000));
        let result = (|| {
            let remaining = timeout
                .checked_sub(started.elapsed())
                .filter(|v| *v >= Duration::from_millis(1))
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "progress deadline expired before read",
                    )
                })?;
            let raw = self.source.read_order(order, remaining)?;
            let next = OrderProgress::decode(raw.canonical_bytes())?;
            if self.source.manifest() != &self.manifest
                || next.generation() != self.manifest.generation
                || next.order_id() != order
                || next.fortress_id() != self.fortress
                || next.tick() < self.tick
                || next.sequence() <= self.sequence
                || next.next_order_id() < self.horizon
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "progress source, fortress, sequence or clock changed",
                ));
            }
            // Do not resurrect an expired grant using an older caller anchor.
            self.tick = next.tick();
            self.sequence = next.sequence();
            self.horizon = next.next_order_id();
            self.access(context)?;
            if started.elapsed() >= timeout {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "progress read exhausted deadline",
                ));
            }
            Ok(next)
        })();
        match result {
            Ok(next) => {
                self.selected = Some(next.clone());
                Ok(next)
            }
            Err(cause) => {
                self.fence();
                Err(cause)
            }
        }
    }
    pub fn register(
        &mut self,
        key: &str,
        witness: Digest32,
        deadline: u64,
        interval: u64,
        required: u32,
        context: &OperationContext,
    ) -> Result<ProgressWatch> {
        self.access(context)?;
        crate::work_orders::validate_key(key)?;
        if context.budget.max_bytes < WATCH_RESERVE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete retained watch exceeds byte allowance",
            ));
        }
        if let Some(old) = self.watches.get(key) {
            if old.baseline().witness() != witness
                || old.deadline() != deadline
                || old.interval() != interval
                || old.required_samples() != required
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "progress watch key already names different immutable intent",
                ));
            }
            return Ok(old.clone());
        }
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "progress source fenced; reopen explicitly",
            ));
        }
        if self.watches.len() >= MAX_WATCHES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "progress watch unavailable, at capacity or over budget",
            ));
        }
        let selected = self
            .selected
            .as_ref()
            .filter(|o| o.witness() == witness)
            .ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "observe the exact order before registering a watch",
                )
            })?;
        if deadline.saturating_sub(selected.tick()) > context.budget.max_game_ticks {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "progress watch exceeds admitted game-time horizon",
            ));
        }
        let watch = ProgressWatch::new(key, selected.clone(), deadline, interval, required)?;
        self.watches.insert(key.to_owned(), watch.clone());
        Ok(watch)
    }
    pub fn watch(&self, key: &str, context: &OperationContext) -> Result<ProgressWatch> {
        self.access(context)?;
        crate::work_orders::validate_key(key)?;
        if context.budget.max_bytes < WATCH_RESERVE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete watch exceeds byte allowance",
            ));
        }
        self.watches
            .get(key)
            .cloned()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "unknown progress watch key"))
    }
    pub fn watches(&self, context: &OperationContext) -> Result<Vec<ProgressWatch>> {
        self.access(context)?;
        if self.watches.len() > context.budget.max_entities as usize
            || self.watches.len() as u64 * WATCH_RESERVE_BYTES > context.budget.max_bytes
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete progress watches exceed caller budget",
            ));
        }
        Ok(self.watches.values().cloned().collect())
    }
    /// One foreground sample. Terminal polling does not read the native source.
    pub fn poll(&mut self, key: &str, context: &OperationContext) -> Result<ProgressWatch> {
        let old = self.watch(key, context)?;
        if old.state().terminal() {
            return Ok(old);
        }
        let mut work = context.clone();
        work.budget.max_bytes = work
            .budget
            .max_bytes
            .checked_sub(4 * WATCH_RESERVE_BYTES)
            .filter(|left| *left > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "poll evidence copies exceed byte allowance",
                )
            })?;
        let observation = self.observe(old.baseline().order_id(), &work)?;
        let next = match old.sampled(observation) {
            Ok(next) => next,
            Err(cause) => {
                self.fence();
                return Err(cause);
            }
        };
        self.watches.insert(key.to_owned(), next.clone());
        Ok(next)
    }
    /// Cancels only local observation work. Never deletes or changes a game order.
    pub fn cancel(&mut self, key: &str, context: &OperationContext) -> Result<ProgressWatch> {
        let next = self.watch(key, context)?.cancelled();
        self.watches.insert(key.to_owned(), next.clone());
        Ok(next)
    }
}
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
