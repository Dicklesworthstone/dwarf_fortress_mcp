//! Reserve the complete Agent Turn before spending the result-byte budget.
//! Safety, admission, source coverage, attention, and authority are never removed
//! to make a query fit. Result producers paginate inside the remaining budget.

use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus,
    ObservationProfile, empty_active_work};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use serde_json::{Value, json};

pub(super) struct QueryResponseProjection {
    pub session_id: String,
    pub request_id: String,
    pub anchor: Value,
    pub briefing: Value,
    pub attention: Vec<Value>,
    pub affordances: Vec<Value>,
    pub uncertainty: Vec<Value>,
    pub coverage: Value,
    pub budget: Value,
    pub references: Vec<Value>,
    pub maximum_bytes: usize,
}

impl QueryResponseProjection {
    /// The longest admitted continuation is included twice (payload and coverage).
    /// A small margin covers the structured-mode tag and JSON punctuation. This is
    /// reserved from the existing ceiling, never added to the caller's authority.
    pub fn result_byte_budget(&self) -> Result<usize> {
        let sample = json!({"mode":"structured","truncated":true,
            "continuation":"x".repeat(256)});
        let overhead = self.render(sample).len().checked_add(64).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded,"query packet overhead overflow")
        })?;
        let remaining = self.maximum_bytes.checked_sub(overhead).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded,
                "the negotiated response budget cannot fit the required Agent Turn; open a session with a larger output budget")
        })?;
        if remaining == 0 {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded,
                "no result bytes remain after reserving required query context"));
        }
        Ok(remaining)
    }

    pub fn finish(&self, payload: Value) -> Result<String> {
        if !payload.is_object() {
            return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation,
                "query result is not an object"));
        }
        if payload.get("anchor").is_some_and(|anchor| anchor != &self.anchor) {
            return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation,
                "query result and Agent Turn name different anchors"));
        }
        if payload.get("continuation").and_then(Value::as_str)
            .is_some_and(|token| token.len() > 256) {
            return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation,
                "query producer exceeded the reserved continuation bound"));
        }
        let encoded = self.render(payload);
        if encoded.len() > self.maximum_bytes {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded,
                "query result and required Agent Turn exceed the response budget; narrow the query"));
        }
        Ok(encoded)
    }

    fn render(&self, mut payload: Value) -> String {
        payload["ok"] = json!(true);
        payload["session_id"] = json!(self.session_id);
        payload["request_id"] = json!(self.request_id);
        payload["source_evidence"] = json!(self.references);
        payload["query_help"] = json!({"tool":"fortress.query","arguments":{
            "session_id":self.session_id,"mode":"schema"}});
        let partial = payload.get("truncated").and_then(Value::as_bool) == Some(true);
        let continuity = if partial { ContinuityStatus::Partial } else { ContinuityStatus::Continuous };
        let mut coverage = self.coverage.clone();
        if partial {
            coverage["status"] = json!("partial");
            coverage["continuation"] = payload.get("continuation").cloned().unwrap_or(Value::Null);
            if let Some(domains) = coverage.get_mut("partial_domains").and_then(Value::as_array_mut) {
                domains.push(json!({"domain":"query.result",
                    "reason":"query output is page- or depth-bounded; continue or narrow explicitly"}));
            }
        }
        AgentTurnBuilder::new("fortress.query",AgentPhase::Inspect)
            .session_id(self.session_id.clone())
            .request_id(self.request_id.clone())
            .turn_id(format!("live-v1-1-turn-{}",self.request_id))
            .anchor(self.anchor.clone())
            .continuity(continuity,Some(self.anchor.clone()),None,None)
            .profile(ObservationProfile::Tactical)
            .briefing(self.briefing.clone())
            .changes(Vec::new())
            .attention(self.attention.clone())
            .active_work(empty_active_work())
            .affordances(self.affordances.clone())
            .recommendations(Vec::new())
            .uncertainty(self.uncertainty.clone())
            .coverage(coverage)
            .budget(self.budget.clone())
            .references(self.references.clone())
            .attach(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(maximum_bytes:usize)->QueryResponseProjection {
        QueryResponseProjection {
            session_id:"11111111111111111111111111111111".to_owned(),
            request_id:"00000000000000000000000000000001".to_owned(),
            anchor:json!({"fortress_id":"7","epoch":1,"sequence":2,"game_tick":3,"state_hash":"a".repeat(64)}),
            briefing:json!({"read_only":true,"mutation_admissible":false,"runtime_admitted":false,
                "runtime":"unadmitted_development","paused":true}),
            attention:vec![json!({"finding":"retain this hazard","severity":"high"})],
            affordances:vec![json!({"tool":"fortress.query","enabled":true,"risk":"read_only"})],
            uncertainty:vec![json!({"domain":"history","state":"unknown","reason":"gap before retained window"})],
            coverage:json!({"status":"partial","partial_domains":[{"domain":"fortress.history","reason":"retained suffix only"}],
                "omitted_domains":["fortress.jobs","fortress.items"]}),
            budget:json!({"admitted":{"max_bytes":maximum_bytes}}),
            references:vec![json!({"kind":"observation_capsule","digest":"b".repeat(64)})],
            maximum_bytes,
        }
    }
    fn decode(text:&str)->Result<Value> {
        serde_json::from_str(text).map_err(|_|DfmcpError::new(ErrorCode::InternalInvariantViolation,"test JSON"))
    }

    #[test]
    fn reserved_budget_keeps_a_large_page_and_every_required_warning()->Result<()> {
        let view=projection(8192);
        let remaining=view.result_byte_budget()?;
        assert!(remaining<8192);
        let payload=json!({"anchor":view.anchor,"truncated":true,"continuation":"x".repeat(256),
            "rows":[{"text":"z".repeat(remaining.saturating_sub(700))}]});
        assert!(serde_json::to_vec(&payload).map_err(|_|DfmcpError::new(ErrorCode::InvalidRequest,"test"))?.len()<=remaining);
        let encoded=view.finish(payload)?;
        assert!(encoded.len()<=8192);
        let result=decode(&encoded)?;
        assert_eq!(result["ok"],true);
        assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"],false);
        assert_eq!(result["agent_turn"]["briefing"]["mutation_admissible"],false);
        assert_eq!(result["agent_turn"]["attention"][0]["finding"],"retain this hazard");
        assert_eq!(result["agent_turn"]["uncertainty"][0]["domain"],"history");
        assert_eq!(result["agent_turn"]["coverage"]["partial_domains"][0]["domain"],"fortress.history");
        assert_eq!(result["agent_turn"]["coverage"]["continuation"],"x".repeat(256));
        assert_eq!(result["agent_turn"]["continuity"]["status"],"partial");
        assert!(result["rows"][0]["text"].as_str().is_some_and(|text|text.len()>1000));
        Ok(())
    }

    #[test]
    fn a_complete_page_does_not_hide_incomplete_source_history()->Result<()> {
        let view=projection(8192);
        let result=decode(&view.finish(json!({"anchor":view.anchor,"truncated":false,"continuation":null,"rows":[]}))?)?;
        assert_eq!(result["agent_turn"]["continuity"]["status"],"continuous");
        assert_eq!(result["agent_turn"]["coverage"]["status"],"partial");
        assert_eq!(result["agent_turn"]["coverage"]["partial_domains"].as_array().map(Vec::len),Some(1));
        assert_eq!(result["source_evidence"],result["agent_turn"]["references"]);
        Ok(())
    }

    #[test]
    fn anchor_drift_is_not_papered_over_by_the_envelope() {
        let view=projection(8192);
        let mut other=view.anchor.clone(); other["sequence"]=json!(9);
        assert!(matches!(view.finish(json!({"anchor":other})),Err(error) if error.code==ErrorCode::InternalInvariantViolation));
    }

    #[test]
    fn inadequate_budget_and_oversized_cursor_fail_closed() {
        let view=projection(1);
        assert!(matches!(view.result_byte_budget(),Err(error) if error.code==ErrorCode::BudgetExceeded));
        let view=projection(8192);
        assert!(matches!(view.finish(json!({"continuation":"x".repeat(257)})),Err(error) if error.code==ErrorCode::InternalInvariantViolation));
        assert!(matches!(view.finish(json!({"rows":["x".repeat(8192)]})),Err(error) if error.code==ErrorCode::BudgetExceeded));
    }

    #[test]
    fn actual_search_pages_survive_full_packet_budget_without_loss()->Result<()> {
        use std::collections::BTreeMap;
        use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId,
            FortressId, GameTick, ObservationCursor, OperationContext, RequestId,
            RiskTier, SessionId, WorkBudget};
        use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue,
            WorldGraph, WorldSnapshot};
        let mut graph=WorldGraph::default();
        for id in 1..=32 {
            graph.entities.insert(EntityId::new(id),EntityRecord {
                id:EntityId::new(id),generation:1,revision:1,kind:EntityKind::Announcement,
                label:format!("Report {id}"),fields:BTreeMap::from([("text".to_owned(),
                    Fact::known(WorldValue::Text("flood at workshop".to_owned()),GameTick(3),
                        FactSource::DfhackField("announcement.text".to_owned()),Digest32::of_bytes(b"source")))]),
            });
        }
        let snapshot=WorldSnapshot::new(FortressId::new(7),GameTick(3),
            ObservationCursor {epoch:1,sequence:2},true,graph);
        let mut view=projection(8192);
        view.anchor=json!({"fortress_id":"7","epoch":1,"sequence":2,"game_tick":3,
            "state_hash":snapshot.state_hash.to_string()});
        let mut context=OperationContext {
            session_id:SessionId::new(7),request_id:RequestId::new(1),anchor:snapshot.anchor(),
            budget:WorkBudget {max_wall_millis:5000,max_game_ticks:1000,max_entities:128,
                max_bytes:8192,max_output_tokens:2048,max_actions:1},
            grants:vec![CapabilityGrant {capability:Capability::Query,
                scope:CapabilityScope {fortress_id:Some(snapshot.fortress_id),..CapabilityScope::default()},
                max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}],
            cancellation_requested:false,
        };
        view.session_id=context.session_id.to_string();
        context.budget.max_bytes=u64::try_from(view.result_byte_budget()?).map_err(|_| {
            DfmcpError::new(ErrorCode::InternalInvariantViolation,"test budget")
        })?;
        let mut request=json!({"schema":"dfmcp.query/1","query":{
            "kind":"search","text":"flood","kinds":["announcement"],
            "text_fields":["text"],"include_labels":false,"limit":128
        }});
        let mut ids=Vec::new();
        let mut pages=0usize;
        loop {
            let mut payload=super::super::semantic_query::execute(&snapshot,&context,&request)?;
            payload["mode"]=json!("structured");
            let encoded=view.finish(payload)?;
            assert!(encoded.len()<=8192);
            let response=decode(&encoded)?;
            assert_eq!(response["ok"],true);
            assert_eq!(response["matched"],32);
            assert_eq!(response["agent_turn"]["briefing"]["runtime_admitted"],false);
            assert_eq!(response["agent_turn"]["attention"][0]["finding"],"retain this hazard");
            assert_eq!(response["agent_turn"]["uncertainty"][0]["domain"],"history");
            let rows=response["rows"].as_array().ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation,"test rows")
            })?;
            assert!(!rows.is_empty());
            for row in rows {
                ids.push(row["entity_id"].as_str().ok_or_else(|| {
                    DfmcpError::new(ErrorCode::InternalInvariantViolation,"test entity ID")
                })?.to_owned());
            }
            pages+=1;
            assert!(pages<=32);
            if response["continuation"].is_null() { break; }
            assert_eq!(response["continuation"],response["agent_turn"]["coverage"]["continuation"]);
            request["query"]["continuation"]=response["continuation"].clone();
            request["query"]["limit"]=json!(7);
        }
        assert!(pages>1);
        assert_eq!(ids,(1..=32).map(|id|id.to_string()).collect::<Vec<_>>());
        Ok(())
    }
}
