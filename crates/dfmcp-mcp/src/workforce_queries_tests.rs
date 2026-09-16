use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId, SessionId, StateAnchor, WorkBudget};

fn context() -> OperationContext {
    OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id: FortressId::new(3), cursor: ObservationCursor { epoch: 1, sequence: 4 },
            tick: GameTick(5), state_hash: Digest32::of_bytes(b"snapshot") },
        budget: WorkBudget { max_entities: 100, max_bytes: 100_000, max_output_tokens: 25_000, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false }
}
fn rows(context: &OperationContext, limit: u32, continuation: Option<&str>) -> Result<Value> {
    let identity = page_identity(context, Digest32::of_bytes(b"model"), 1000);
    paginate(json!({"summary":"complete model"}), 3, context,
        Page { limit, continuation, prefix: "wp1", identity }, |i| Ok(json!({"worker":i,"name":"Urist"})))
}

#[test]
fn page_width_can_change_without_consuming_or_reallocating_rows() -> Result<()> {
    let context = context(); let first = rows(&context, 1, None)?;
    let token = first["continuation"].as_str().ok_or_else(|| invalid("test token"))?;
    let next = rows(&context, 2, Some(token))?;
    assert_eq!(next["rows"], json!([{"worker":1,"name":"Urist"},{"worker":2,"name":"Urist"}]));
    assert_eq!(next["truncated"], false); assert!(next["continuation"].is_null());
    assert_eq!(rows(&context, 2, Some(token))?, next);
    Ok(())
}

#[test]
fn continuation_binds_full_anchor_session_model_and_query_family() -> Result<()> {
    let context = context(); let model = Digest32::of_bytes(b"model");
    let identity = page_identity(&context, model, 1000); let token = token("wp1", 1, identity);
    for case in 0..6 {
        let mut changed = context.clone();
        match case { 0 => changed.session_id = SessionId::new(99),
            1 => changed.anchor.fortress_id = FortressId::new(99), 2 => changed.anchor.cursor.epoch += 1,
            3 => changed.anchor.cursor.sequence += 1, 4 => changed.anchor.tick.0 += 1,
            _ => changed.anchor.state_hash = Digest32::of_bytes(b"fork") }
        assert!(matches!(page_offset(Some(&token), "wp1", page_identity(&changed, model, 1000), 3),
            Err(e) if e.code == ErrorCode::StaleAnchor));
    }
    assert!(page_offset(Some(&token), "wc2", identity, 3).is_err());
    assert!(page_offset(Some(&token), "wp1", page_identity(&context, Digest32::of_bytes(b"other model"), 1000), 3).is_err());
    assert!(page_offset(Some(&token), "wp1", page_identity(&context, model, 999), 3).is_err());
    assert!(matches!(page_offset(Some(&token), "wp1", identity, 1), Err(e) if e.code == ErrorCode::CursorGap));
    for raw in ["wp1:0:x", "wp1:01:x", "wp1:1:x", "wc1:1:x"] { assert!(page_offset(Some(raw), "wp1", identity, 3).is_err()); }
    Ok(())
}

#[test]
fn summary_and_whole_row_limits_never_emit_zero_progress_continuations() -> Result<()> {
    let mut context = context();
    let first = rows(&context, 1, None)?;
    context.budget.max_bytes = first.to_string().len() as u64;
    let page = rows(&context, 128, None)?;
    assert_eq!(page["returned"], 1); assert_eq!(page["truncated"], true);
    assert!(page.to_string().len() as u64 <= context.budget.max_bytes);
    context.budget.max_bytes -= 1;
    assert!(matches!(rows(&context, 1, None), Err(e) if e.code == ErrorCode::BudgetExceeded));
    context.budget.max_bytes = 1;
    assert!(paginate(json!({"summary":"still required when empty"}), 0, &context,
        Page { limit: 1, continuation: None, prefix: "wp1", identity: Digest32::ZERO }, |_| Ok(Value::Null)).is_err());
    assert!(rows(&context, 0, None).is_err()); assert!(rows(&context, 129, None).is_err());
    Ok(())
}

#[test]
fn embedded_schema_and_input_shape_keep_the_query_family_closed() -> Result<()> {
    let schema = extend_schema(json!({"$defs":{"query":{"oneOf":[]}}}))?;
    assert_eq!(schema["$defs"]["query"]["oneOf"].as_array().map(Vec::len), Some(2));
    assert!(handles(&json!({"query":{"kind":"workforce_candidates"}})));
    assert!(handles(&json!({"query":{"kind":"workforce_plan"}})));
    assert!(!handles(&json!({"query":{"kind":"assign_labor"}})));
    let too_wide = json!({"query":{"kind":"workforce_plan","demands":vec![Value::Null; 4096]}});
    assert!(matches!(validate_shape(&too_wide), Err(e) if e.code == ErrorCode::BudgetExceeded));
    assert!(serde_json::from_value::<Envelope>(json!({"schema":"dfmcp.query/1","query":{
        "kind":"workforce_plan","demands":[],"dispatch":true}})).is_err());
    Ok(())
}
