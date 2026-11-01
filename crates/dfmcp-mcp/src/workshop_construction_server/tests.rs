use super::*;
#[test]
fn workshop_namespace_is_process_scoped_and_distinct() -> Result<()> {
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    assert_eq!(id(&session.to_string())?, session);
    for family in 0..31 {
        assert!(id(&SessionId::new((1u128 << 127) | (family << 57) | 1).to_string()).is_err());
    }
    assert!(id("1").is_err());
    Ok(())
}
#[test]
fn workshop_output_and_zero_tick_bounds_are_enforced() -> Result<()> {
    let b = budgets(None, None, None)?;
    assert_eq!(b.max_game_ticks, 0);
    assert!(budgets(Some(60001), None, None).is_err());
    assert!(budgets(None, Some(64 * 1024 * 1024 - 1), None).is_err());
    assert!(budgets(None, None, Some(32767)).is_err());
    Ok(())
}
#[test]
fn missing_or_oversized_workshop_configuration_is_refused() {
    assert!(Config::parse("{}").is_err());
    assert!(Config::parse(&"x".repeat(16385)).is_err());
}
#[test]
fn failure_packet_withholds_current_completion_and_production_provenance() -> Result<()> {
    let text = failure(
        "fortress.commit",
        error(ErrorCode::EffectIndeterminate, "lost reply"),
        None,
        None,
        json!({"original":"retain","current_goal_available":false}),
    );
    let v: Value =
        serde_json::from_str(&text).map_err(|_| error(ErrorCode::InvalidRequest, "bad JSON"))?;
    assert_eq!(v["result"]["current_goal_available"], false);
    assert_eq!(
        v["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert!(v["agent_turn"]["briefing"].get("admission").is_none());
    assert_eq!(v["result"]["automatic_redispatch"], false);
    Ok(())
}

#[test]
fn overflow_preserves_full_agent_turn_spine_and_compact_original_hazard() -> Result<()> {
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    let request = RequestId::new(7);
    let pin = json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":3});
    let reference = json!({"acknowledged":pin,"original":{"plan_digest":"c".repeat(64),"review_seal":"d".repeat(64)}});
    let text = presentation::packet(
        "fortress.commit",
        Some(session),
        Some(request),
        Some(budgets(None, None, None)?),
        json!({"ok":true,"retained_state_currently_verified":true,"oversized":"x".repeat(RESPONSE_BYTES as usize+1)}),
        reference,
    );
    let v: Value =
        serde_json::from_str(&text).map_err(|_| error(ErrorCode::InvalidRequest, "bad JSON"))?;
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../architecture/agent_turn_contract.json"
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "bad contract"))?;
    for key in contract["field_order"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing contract field order"))?
    {
        assert!(
            v["agent_turn"]
                .get(
                    key.as_str()
                        .ok_or_else(|| error(ErrorCode::InvalidRequest, "bad key"))?
                )
                .is_some(),
            "{key}"
        );
    }
    assert_eq!(v["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(v["agent_turn"]["session_id"], session.to_string());
    assert_eq!(v["agent_turn"]["request_id"], request.to_string());
    assert_ne!(v["agent_turn"]["turn_id"], v["agent_turn"]["request_id"]);
    assert!(v["agent_turn"]["anchor"].is_null());
    assert_eq!(
        v["agent_turn"]["continuity"]["gap"]["canonical_world_anchor_available"],
        false
    );
    assert_eq!(v["retained_reference"]["acknowledged"]["frames"], 3);
    assert!(text.len() < RESPONSE_BYTES as usize);
    assert!(v["agent_turn"]["briefing"].get("admission").is_none());
    Ok(())
}

#[test]
fn failed_budget_withdraws_prepared_connection_and_old_seal() -> Result<()> {
    if std::env::var("WORKSHOP_BUDGET_UNIT_CHILD").ok().as_deref() == Some("1") {
        return crate::run_with_runtime_cx(|_|async {
            let config=Config::environment()?;
            let session=SessionId::new((1u128<<127)|FAMILY|1);
            let mut runtime=Runtime::open(config,WorkshopMode::Control,session,budgets(Some(60000),None,None)?)?;
            let c=runtime.context()?;
            let observed=runtime.inspect(&c)?;
            let request=json!({"kind":"carpenters","expected_observation":observed["observation"]["witness"],
                "material":observed["observation"]["material_handle"]}).to_string();
            let prepared=runtime.perform(Action::Plan(request),&c)?;
            assert_eq!(prepared["original"]["prepared_in_this_connection"],true);
            let seal=prepared["original"]["review_seal"].as_str().ok_or_else(||error(ErrorCode::InvalidRequest,"review absent"))?.to_owned();
            let mut invalid=c.clone(); invalid.budget.max_wall_millis=0;
            assert!(runtime.perform(Action::Inspect,&invalid).is_err());
            let valid=runtime.context()?;
            assert!(runtime.perform(Action::Commit {seal,confirmed:true},&valid).is_err());
            let retained=runtime.inspect(&valid)?;
            assert_eq!(retained["original"]["state"],"prepared");
            assert_eq!(retained["original"]["prepared_in_this_connection"],false);
            assert!(retained["observation"].is_null());
            assert_eq!(retained["original"]["dispatch_started"],false);
            Ok(())
        }).map_err(|_|error(ErrorCode::InternalInvariantViolation,"owned runtime failed"))?;
    }
    let executable = std::env::current_exe()
        .map_err(|_| error(ErrorCode::AdapterUnavailable, "test executable unavailable"))?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let logs = std::env::temp_dir().join(format!("dfmcp-workshop-budget-{}", std::process::id()));
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("tests/stdio/workshop.py"))
        .arg("--budget-unit")
        .arg(executable)
        .arg("--logs")
        .arg(&logs)
        .output()
        .map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "joined TCP budget regression could not start",
            )
        })?;
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}\nlogs={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        logs.display()
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("zero commit frames"));
    Ok(())
}

#[test]
fn workshop_source_presentation_preserves_hidden_and_absent_facts() -> Result<()> {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../../tests/native/workshop/vectors.json"
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid native vectors"))?;
    let raw = fixtures["capture"]
        .as_str()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing capture"))?;
    let bytes = raw
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair)
                    .map_err(|_| error(ErrorCode::InvalidRequest, "bad hex"))?,
                16,
            )
            .map_err(|_| error(ErrorCode::InvalidRequest, "bad hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut hidden = bytes.clone();
    hidden.splice(94..111, [1]);
    let capture = dfmcp_adapter::workshop_construction::WorkshopCapture::decode(&hidden)?;
    let view = runtime::source_json(&capture);
    assert_eq!(view["tiles"][0]["presence"], "hidden");
    for key in ["dry", "empty_floor", "building_occupied_at_anchor"] {
        assert!(view["tiles"][0][key].is_null());
    }
    for (tag, presence) in [(0, "absent"), (1, "hidden")] {
        let mut changed = bytes.clone();
        let end = changed.len() - 24; // one complete native worker plus census count
        changed.splice(247..end, [tag]);
        let capture = dfmcp_adapter::workshop_construction::WorkshopCapture::decode(&changed)?;
        let view = runtime::source_json(&capture);
        assert_eq!(view["selected_material"]["presence"], presence);
        assert!(view["selected_material"]["available_at_anchor"].is_null());
        assert_eq!(view["eligible_at_anchor"], false);
    }
    Ok(())
}
