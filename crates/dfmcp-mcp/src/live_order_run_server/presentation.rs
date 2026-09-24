//! Authority-free projection of verified conditional-run coordination.
use super::error;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work, recommendation, uncertainty,
};
use dfmcp_adapter::order_run::journal::{OrderRunEntry, OrderRunMode, OrderRunView};
use dfmcp_adapter::order_run::{OrderCapture, OrderRunPlan};
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, SessionId};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const BASE_OUTPUT: u64 = 16 * 1024;
pub const ROW_OUTPUT: u64 = 16 * 1024;
pub const MAX_PAGE: usize = 8;
pub fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "expected canonical lowercase SHA-256 hex",
        ));
    }
    let mut out = [0; 32];
    for (index, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let part = std::str::from_utf8(pair)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        out[index] = u8::from_str_radix(part, 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(out))
}
pub fn mode_name(mode: OrderRunMode) -> &'static str {
    match mode {
        OrderRunMode::Control => "control",
        OrderRunMode::Recover => "recover",
        OrderRunMode::Offline => "offline",
    }
}
pub fn capture(value: &OrderCapture) -> Value {
    json!({
    "fortress_id":value.fortress().fortress_id().to_string(),"world_folder":value.fortress().folder(),"site_id":value.fortress().site(),
    "generation":value.generation(),"sequence":value.sequence(),"tick":value.tick(),"paused":value.paused(),
    "order_id":value.order_id(),"allocation_horizon":value.horizon(),"present":value.present(),"recipe_code":value.recipe(),
    "amount_total":if value.present(){Some(value.total())}else{None},"amount_remaining":if value.present(){Some(value.remaining())}else{None},
    "status_bits":if value.present(){Some(value.status())}else{None},"witness":value.witness().to_string(),
    "current_freshness_proven":false,"evidence_scope":"one_native_capture","goods_produced_proven":false})
}
fn plan(value: &OrderRunPlan) -> Value {
    let s = value.spec();
    json!({"idempotency_key":value.key(),"plan_digest":value.digest().to_string(),
    "condition":{"order_id":value.before().order_id(),"predicate":s.predicate().as_str(),"threshold":s.predicate().threshold(),
        "stable_samples":s.samples(),"interval_ticks":s.interval(),"game_ticks":s.game_ticks(),"wall_millis":s.wall_ms()},
    "before":capture(value.before())})
}
pub fn entry(value: &OrderRunEntry) -> Value {
    json!({"plan":plan(value.plan()),"state":value.state().as_str(),
    "settled_in_this_coordinator":value.settled(),"reconciliation_required":value.unresolved(),
    "native":value.native().map(|n|json!({"phase":n.phase().as_str(),"clock_reason":n.reason().as_str(),"trigger":n.trigger().as_str(),
        "unpause_attempted":n.unpause_attempted(),"pause_verified":n.pause_verified(),"observed_tick":n.observed_tick(),
        "predicate_observed":n.predicate_observed(),"reported_stable_samples":n.reported_stable_samples(),"counted_tick":n.counted_tick(),
        "sample":n.sample().map(capture),"receipt_digest":n.receipt().to_string(),"observed_tick_overshoot":n.observed_tick_overshoot(),
        "full_sample_trace_available":false})),"historical_evidence_only":true,"current_pause_unproved":true,"goods_produced_proven":false})
}
pub fn summary(view: &OrderRunView) -> Value {
    json!({"journal_id":view.id.to_string(),"head":view.head.to_string(),"transitions":view.transitions,
    "records":view.entries.len(),"pending":view.entries.iter().filter(|e|!e.settled()).count(),
    "unresolved":view.entries.iter().filter(|e|e.unresolved()).count(),"terminal":view.entries.iter().filter(|e|e.settled()).count()})
}
pub fn failure(cause: &dfmcp_core::DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message},
    "effect_outcome_inferred":false,"retry_commit":false,"recovery":"inspect retained journal; query or cancel uncertain work"})
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Pending,
    Unresolved,
    Terminal,
}
impl Filter {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "all" => Ok(Self::All),
            "pending" => Ok(Self::Pending),
            "unresolved" => Ok(Self::Unresolved),
            "terminal" => Ok(Self::Terminal),
            _ => Err(error(
                ErrorCode::InvalidRequest,
                "state must be all, pending, unresolved or terminal",
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Pending => "pending",
            Self::Unresolved => "unresolved",
            Self::Terminal => "terminal",
        }
    }
    pub fn matches(self, e: &OrderRunEntry) -> bool {
        match self {
            Self::All => true,
            Self::Pending => !e.settled(),
            Self::Unresolved => e.unresolved(),
            Self::Terminal => e.settled(),
        }
    }
}
struct Cursor {
    session: SessionId,
    journal: Digest32,
    head: Digest32,
    filter: Filter,
    limit: usize,
    offset: usize,
}
#[derive(Default)]
pub struct Cursors {
    serial: u64,
    issued: VecDeque<(String, Cursor)>,
}
impl Cursors {
    pub fn resolve(
        &self,
        raw: &str,
        session: SessionId,
        view: &OrderRunView,
        filter: Filter,
        limit: usize,
    ) -> Result<usize> {
        if raw.len() != 64 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid conditional-run continuation",
            ));
        }
        let (_, c) = self
            .issued
            .iter()
            .find(|(token, _)| token == raw)
            .ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "continuation expired; restart record discovery",
                )
            })?;
        if c.session != session
            || c.journal != view.id
            || c.head != view.head
            || c.filter != filter
            || c.limit != limit
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "continuation belongs to another session, journal head, filter or page size",
            ));
        }
        Ok(c.offset)
    }
    pub fn issue(
        &mut self,
        session: SessionId,
        view: &OrderRunView,
        filter: Filter,
        limit: usize,
        offset: usize,
    ) -> Result<String> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "continuation IDs exhausted"))?;
        let mut bytes = b"dfmcp-order-run-mcp-cursor/1\0".to_vec();
        bytes.extend_from_slice(&session.get().to_be_bytes());
        bytes.extend_from_slice(view.id.as_bytes());
        bytes.extend_from_slice(view.head.as_bytes());
        bytes.extend_from_slice(&self.serial.to_be_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.issued.len() == 64 {
            self.issued.pop_front();
        }
        self.issued.push_back((
            token.clone(),
            Cursor {
                session,
                journal: view.id,
                head: view.head,
                filter,
                limit,
                offset,
            },
        ));
        Ok(token)
    }
}
pub fn packet(
    operation: &str,
    result: Value,
    context: Option<&OperationContext>,
    mode: Option<OrderRunMode>,
    view: Option<&OrderRunView>,
) -> String {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" | "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut active = empty_active_work();
    active["scope"] = json!("this_verified_conditional_run_journal_only");
    active["inventory_verified"] = json!(view.is_some());
    active["absence_proven"] =
        json!(view.is_some_and(|v| v.entries.iter().all(OrderRunEntry::settled)));
    if let Some(view) = view {
        let refs=view.entries.iter().filter(|e|!e.settled()).take(4).map(|e|json!({"idempotency_key":e.plan().key(),
            "plan_digest":e.plan().digest().to_string(),"state":e.state().as_str(),"reconciliation_required":e.unresolved()})).collect::<Vec<_>>();
        active["pending_plans"] = json!(
            refs.iter()
                .filter(|r| matches!(r["state"].as_str(), Some("intent" | "prepared")))
                .collect::<Vec<_>>()
        );
        active["actions"] = json!(
            refs.iter()
                .filter(|r| r["reconciliation_required"] == true)
                .collect::<Vec<_>>()
        );
        active["indeterminate_effects"] = json!(
            refs.iter()
                .filter(|r| r["reconciliation_required"] == true)
                .collect::<Vec<_>>()
        );
        active["cancellation_drains"] = json!(
            refs.iter()
                .filter(|r| r["state"] == "cancel_requested")
                .collect::<Vec<_>>()
        );
        active["counts"] = summary(view);
        active["omitted_pending_references"] = json!(
            view.entries
                .iter()
                .filter(|e| !e.settled())
                .count()
                .saturating_sub(4)
        );
    }
    let mut builder=AgentTurnBuilder::new(operation,phase)
        .continuity(ContinuityStatus::Indeterminate,None,Some(json!({"game_history_continuity":"unestablished"})),None)
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.14","runtime_admitted":false,
            "mode":mode.map(mode_name),"world_state_anchor":false,"global_clock_lease_established":false,
            "limit_semantics":"native_callback_stop_triggers_not_hard_real_time","current_pause_unproved":true,"goods_produced_proven":false}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":if view.is_some(){json!(["this_journal_coordination"])}else{json!([])},
            "partial_domains":["sampled_native_order_and_pause_evidence"],"omitted_domains":["current_world_state","produced_goods","other_controllers"]}))
        .uncertainty(vec![uncertainty("sampled-not-current","unknown","Predicate samples and pause receipts are historical, not continuous truth or produced goods.",
            "Never replay a dispatched run; recover its exact key and digest.",None,Value::Null)]);
    if let Some(c) = context {
        builder=builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .budget(json!({"admitted":{"max_wall_millis":c.budget.max_wall_millis,"max_game_ticks":c.budget.max_game_ticks,
                "max_bytes":c.budget.max_bytes,"max_output_tokens":c.budget.max_output_tokens},"output_proxy_bytes_per_token":4,
                "consumed":{},"accounting":"conservative_reservation_not_measured_tokenization"}))
            .recommendations(vec![recommendation("discover-order-runs","fortress.query","Inspect durable work before requesting new control.",
                "high","high","read_only","not_applicable",false,json!({"session_id":c.session_id.to_string(),"state":"pending","limit":2}))]);
    }
    if let Some(view) = view {
        builder=builder.anchor(json!({"fortress_id":view.binding.fortress().fortress_id().to_string(),"cursor":{"epoch":view.binding.manifest().generation,
            "sequence":view.transitions},"tick":0,"state_hash":view.head.to_string(),"scope":"coordination_root_not_world_state","tick_is_sentinel":true}));
    }
    let mut turn = builder.build();
    // This isolated projection must never inherit production provenance, even
    // while rendering a refusal caused by an injected admitted process context.
    if let Some(briefing) = turn["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}
