//! Authority-free, bounded review and recovery projections. No canonical bytes or secrets.
use super::error;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work, recommendation, uncertainty,
};
use dfmcp_adapter::workforce_control::journal::{AssignmentRecord, WorkforceMode, WorkforceView};
use dfmcp_adapter::workforce_control::{AssignmentPhase, Citizen, Detail, WorkforceCapture};
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const COMPACT_OUTPUT: u64 = 32 * 1024;
pub const DETAIL_OUTPUT: u64 = 192 * 1024;
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
    for (i, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let part = std::str::from_utf8(pair)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        out[i] = u8::from_str_radix(part, 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(out))
}
pub fn mode_name(mode: WorkforceMode) -> &'static str {
    match mode {
        WorkforceMode::Control => "control",
        WorkforceMode::Recover => "recover",
        WorkforceMode::Offline => "offline",
    }
}
fn identity(c: &WorkforceCapture) -> Value {
    json!({
        "fortress_id":c.fortress_id().to_string(),"world_folder":c.folder(),"site_id":c.site(),
        "generation":c.generation(),"sequence":c.sequence(),"tick":c.tick(),"paused":c.paused(),
        "automatic_professions":c.automatic(),"witness":c.witness().to_string(),"current_freshness_proven":false,
    })
}
fn citizen(c: &Citizen, masks: bool) -> Value {
    let mut out = json!({"native_id":c.id(),"historical_figure_id":c.historical_id(),"eligible":c.eligible()});
    if masks {
        out["labor_mask"] = json!(c.labors());
    }
    out
}
pub fn capture_summary(c: &WorkforceCapture) -> Value {
    json!({
        "source":identity(c),"witness":c.witness().to_string(),
        "citizens":c.citizens().iter().map(|v| citizen(v,false)).collect::<Vec<_>>(),
        "work_detail_count":c.details().len(),"labor_column_count":c.labor_keys().len(),
        "detail_query":{"mode":"details","witness":c.witness().to_string(),"offset":0,"limit":4},
        "configuration_coverage":"complete_bounded_native_capture","detail_indices_are_capture_local":true,
        "labor_permissions_prove_job_assignment":false,
    })
}
fn detail(index: usize, d: &Detail, members: bool) -> Value {
    let mut out = json!({"detail_index":index,"name":d.name(),"selected_only":d.selected_only(),
        "allowed_labor_mask":d.labors(),"member_count":d.members().len()});
    if members {
        out["native_member_ids"] = json!(d.members());
    }
    out
}
pub fn details_page(c: &WorkforceCapture, offset: usize, limit: usize) -> Result<Value> {
    if !(1..=MAX_PAGE).contains(&limit) || offset > c.details().len() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "detail page is outside this exact capture",
        ));
    }
    let end = (offset + limit).min(c.details().len());
    let next = (end < c.details().len()).then(
        || json!({"mode":"details","witness":c.witness().to_string(),"offset":end,"limit":limit}),
    );
    Ok(json!({"source":identity(c),"labor_keys":c.labor_keys(),
        "citizens":c.citizens().iter().map(|v| citizen(v,true)).collect::<Vec<_>>(),
        "details":c.details()[offset..end].iter().enumerate().map(|(i,d)| detail(offset+i,d,true)).collect::<Vec<_>>(),
        "offset":offset,"total_details":c.details().len(),"next_query":next,
        "complete_detail_set_in_this_response":offset==0 && end==c.details().len(),
        "mask_columns":"exact labor_keys order; no raw labor enum input","historical_capture_only":true}))
}
pub fn record_summary(r: &AssignmentRecord) -> Value {
    json!({
        "idempotency_key":r.plan().key(),"plan_digest":r.plan().digest().to_string(),"state":r.state().as_str(),
        "settled_in_this_coordinator":r.state().settled(),"reconciliation_required":r.state().unresolved(),
        "native_query_can_help":r.needs_query(),"native_phase":r.effect().map(|e| e.phase().as_str()),
        "receipt_digest":r.effect().map(|e| e.receipt().to_string()),"detail_index":r.plan().spec().detail(),
        "assigned":r.plan().spec().assigned(),"selected_citizens":r.plan().before().citizens().len(),
        "evidence_scope":"historical_coordination_not_current_workforce",
    })
}
pub fn record_detail(r: &AssignmentRecord) -> Value {
    let p = r.plan();
    let c = p.before();
    let index = p.spec().detail() as usize;
    let changed: Vec<u32> = c.details().get(index).map_or_else(Vec::new, |d| {
        c.citizens()
            .iter()
            .map(Citizen::id)
            .filter(|id| d.members().binary_search(id).is_ok() != p.spec().assigned())
            .collect()
    });
    json!({"record":record_summary(r),"review":{"source":identity(c),"labor_keys":c.labor_keys(),
        "detail":c.details().get(index).map(|d| detail(index,d,true)),"assigned":p.spec().assigned(),
        "citizens":c.citizens().iter().map(|v| citizen(v,true)).collect::<Vec<_>>(),"changed_citizen_ids":changed,
        "removal_disables_all_overlapping_permissions":false},
        "native":r.effect().map(|e| json!({"phase":e.phase().as_str(),"receipt_digest":e.receipt().to_string(),
            "after_witness":e.after_witness().map(|d| d.to_string()),
            "post_citizens":e.post_citizens().iter().map(|v| citizen(v,true)).collect::<Vec<_>>(),
            "immediate_membership_and_recompute_verified":e.phase()==AssignmentPhase::Applied})),
        "historical_evidence_only":true,"current_workforce_proven":false,"jobs_completed_proven":false})
}
pub fn summary(v: &WorkforceView) -> Value {
    json!({"journal_id":v.id.to_string(),"head":v.head.to_string(),"events":v.events,
    "bytes":v.bytes,"records":v.records.len(),"pending":v.records.iter().filter(|r| !r.state().settled()).count(),
    "unresolved":v.records.iter().filter(|r| r.state().unresolved()).count(),
    "permanent_unknown":v.records.iter().filter(|r| r.effect().is_some_and(|e| e.phase()==AssignmentPhase::Unknown)).count(),
    "terminal":v.records.iter().filter(|r| r.state().settled()).count()})
}
pub fn failure(e: &dfmcp_core::DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message},
    "effect_outcome_inferred":false,"retry_commit":false,"recovery":"discover the retained key and digest before further control"})
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    All,
    Pending,
    Unresolved,
    Terminal,
}
impl Filter {
    pub fn matches(self, r: &AssignmentRecord) -> bool {
        match self {
            Self::All => true,
            Self::Pending => !r.state().settled(),
            Self::Unresolved => r.state().unresolved(),
            Self::Terminal => r.state().settled(),
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
    values: VecDeque<(String, Cursor)>,
}
impl Cursors {
    pub fn issue(
        &mut self,
        session: SessionId,
        view: &WorkforceView,
        filter: Filter,
        limit: usize,
        offset: usize,
    ) -> Result<String> {
        self.serial = self.serial.checked_add(1).ok_or_else(super::exhausted)?;
        let mut bytes = b"dfmcp-workforce-mcp-cursor/1\0".to_vec();
        bytes.extend_from_slice(&session.get().to_be_bytes());
        bytes.extend_from_slice(view.id.as_bytes());
        bytes.extend_from_slice(view.head.as_bytes());
        bytes.extend_from_slice(&self.serial.to_be_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.values.len() == 64 {
            self.values.pop_front();
        }
        self.values.push_back((
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
    pub fn resolve(
        &self,
        token: &str,
        session: SessionId,
        view: &WorkforceView,
        filter: Filter,
        limit: usize,
    ) -> Result<usize> {
        digest(token)?;
        let (_, c) = self
            .values
            .iter()
            .find(|(t, _)| t == token)
            .ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "workforce continuation expired; start a new first page",
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
                "workforce continuation belongs to another session, journal head or selection",
            ));
        }
        Ok(c.offset)
    }
}
pub fn packet(
    op: &str,
    result: Value,
    context: Option<&OperationContext>,
    mode: Option<WorkforceMode>,
    view: Option<&WorkforceView>,
) -> String {
    let phase = match op {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" | "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut active = empty_active_work();
    active["scope"] = json!("this_workforce_journal_only");
    active["inventory_verified"] = json!(view.is_some());
    active["pending_absence_proven"] =
        json!(view.is_some_and(|v| v.records.iter().all(|r| r.state().settled())));
    if let Some(v) = view {
        let refs = v
            .records
            .iter()
            .filter(|r| !r.state().settled())
            .take(4)
            .map(record_summary)
            .collect::<Vec<_>>();
        active["pending_plans"] = json!(
            refs.iter()
                .filter(|r| matches!(r["state"].as_str(), Some("intent" | "prepared")))
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
        active["counts"] = summary(v);
        active["omitted_pending_references"] = json!(
            v.records
                .iter()
                .filter(|r| !r.state().settled())
                .count()
                .saturating_sub(4)
        );
    }
    let mut builder = AgentTurnBuilder::new(op, phase)
        .continuity(ContinuityStatus::Indeterminate, None, Some(json!({"world_history":"unestablished"})), None)
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.17","runtime_admitted":false,
            "mode":mode.map(mode_name),"global_labor_lease_established":false,"current_workforce_proven":false,"jobs_completed_proven":false}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":if view.is_some(){json!(["this_journal_coordination"])}else{json!([])},
            "partial_domains":["selected_historical_workforce_evidence"],"omitted_domains":["current_world","other_controllers","completed_jobs"]}))
        .uncertainty(vec![uncertainty("historical-not-current","unknown","Membership and recomputation receipts are historical, not jobs completed or current workforce.",
            "Never redispatch an uncertain assignment or infer permission from a receipt.",None,Value::Null)]);
    if let Some(c) = context {
        let query = json!({"mode":"records","state":"pending","limit":4}).to_string();
        builder = builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .budget(json!({"admitted":{"max_wall_millis":c.budget.max_wall_millis,"max_bytes":c.budget.max_bytes,
                "max_entities":c.budget.max_entities,"max_actions":c.budget.max_actions,"max_game_ticks":0,"max_output_tokens":c.budget.max_output_tokens},
                "output_proxy_bytes_per_token":4,"accounting":"conservative_reservations_not_measured_tokens","consumed":{}}))
            .recommendations(vec![recommendation("discover-workforce","fortress.query","Inspect durable assignment state before requesting new control.",
                "high","high","read_only","not_applicable",false,json!({"session_id":c.session_id.to_string(),"query":query}))]);
    }
    if let Some(v) = view {
        builder = builder.anchor(json!({"fortress_id":v.binding.fortress().to_string(),
            "cursor":{"epoch":v.binding.manifest().generation,"sequence":v.events},"tick":0,"tick_is_sentinel":true,
            "state_hash":v.head.to_string(),"scope":"coordination_root_not_world_state"}));
    }
    let mut turn = builder.build();
    if let Some(briefing) = turn["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}
