use super::*;
use dfmcp_core::{FortressId, MapCoord, MapCuboid};

fn config() -> Result<Config> {
    let source = json!({"endpoint":"127.0.0.1:5000","generation":9,
        "df_version":"test-df","dfhack_version":"test-dfhack"});
    let monitor_id = Digest32::of_bytes(b"original monitor").to_string();
    let head = Digest32::of_bytes(b"acknowledged monitor head").to_string();
    let mut clock_source = source.clone();
    clock_source["generation"] = json!(41);
    Ok(Config {
        path: "/private/missing/native-journal".into(),
        scope: MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(127, 127, 7))?,
        folder: "region1".into(),
        site: 1,
        endpoint: "127.0.0.1:5000".parse().map_err(|_| exhausted())?,
        protected: vec![],
        checkpoint: dfmcp_adapter::dig_control_policy::DigCheckpointPolicy::Required,
        blueprint: None,
        monitor: Some(monitor::Config::parse(
            &json!({
                "path":"/private/retained/monitor","initialize":false,"source":source,
                "recovery":{"monitor_id":monitor_id,"head":head}
            })
            .to_string(),
        )?),
        clock: Some(clock::Config::parse(
            &json!({
                "path":"/private/retained/clock","initialize":false,"source":clock_source,
                "recovery":{"journal_id":Digest32::of_bytes(b"original run journal").to_string(),
                    "head":Digest32::of_bytes(b"acknowledged run head").to_string()}
            })
            .to_string(),
        )?),
    })
}

fn context(config: &Config, mode: clock_recovery::Mode, enabled: bool) -> OperationContext {
    let (grants, _) = clock_recovery_authority(config, mode, enabled);
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: config.fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(12345),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_bytes: MAX_WORK_BYTES,
            max_entities: monitor::MAX_ENTITIES,
            max_output_tokens: 8192,
            max_actions: 1,
        },
        grants,
        cancellation_requested: false,
    }
}

#[test]
fn independent_clock_modes_require_explicit_existing_custody_and_zero_future_ticks() -> Result<()> {
    let original = config()?;
    for (raw, expected) in [
        ("clock-offline", clock_recovery::Mode::Offline),
        ("clock-recovery", clock_recovery::Mode::Recovery),
    ] {
        assert_eq!(
            clock_recovery_selection(Some(raw), None, &original, None)?,
            Some(expected)
        );
        assert_eq!(
            clock_recovery_selection(Some(raw), None, &original, Some(0))?,
            Some(expected)
        );
        assert!(clock_recovery_selection(Some(raw), Some("[2,2,2,1,1]"), &original, None).is_err());
        for ticks in [1, 1200, 1201, u64::MAX] {
            assert!(clock_recovery_selection(Some(raw), None, &original, Some(ticks)).is_err());
        }
        let mut changed = original.clone();
        changed.clock = None;
        assert!(clock_recovery_selection(Some(raw), None, &changed, None).is_err());
        let mut changed = original.clone();
        changed.monitor = None;
        assert!(clock_recovery_selection(Some(raw), None, &changed, None).is_err());
        let mut changed = original.clone();
        if let Some(clock) = &mut changed.clock {
            clock.initialize = true;
            clock.recovery = None;
        }
        assert!(clock_recovery_selection(Some(raw), None, &changed, None).is_err());
        let mut changed = original.clone();
        if let Some(monitor) = &mut changed.monitor {
            monitor.initialize = true;
            monitor.recovery = None;
        }
        assert!(clock_recovery_selection(Some(raw), None, &changed, None).is_err());
    }
    Ok(())
}

#[test]
fn parentless_clock_configuration_does_not_create_a_fallback_native_session() -> Result<()> {
    let original = config()?;
    for raw in [None, Some("control"), Some("offline"), Some("reconcile")] {
        assert_eq!(
            clock_recovery_selection(raw, Some("[2,2,2,1,1]"), &original, None)?,
            None
        );
        assert!(opening_selection(raw, Some("[2,2,2,1,1]"), &original).is_err());
    }
    for raw in [
        "monitor-offline",
        "monitor-recovery",
        "clock",
        "clock-control",
    ] {
        assert!(opening_selection(Some(raw), None, &original).is_err());
    }
    Ok(())
}

#[test]
fn independent_recovery_grants_keep_named_and_nil_authority_separate_and_minimal() -> Result<()> {
    let selected = config()?;
    for mode in [
        clock_recovery::Mode::Offline,
        clock_recovery::Mode::Recovery,
    ] {
        for enabled in [false, true] {
            let canonical = context(&selected, mode, enabled);
            let (_, grants) = clock_recovery_authority(&selected, mode, enabled);
            let native = clock::native_context(&canonical, &grants)?;
            canonical.authorize(
                Capability::Query,
                RiskTier::ReadOnly,
                &[],
                Some(selected.scope),
            )?;
            native.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            assert_eq!(native.anchor.fortress_id, FortressId::NIL);
            assert_eq!(native.session_id, canonical.session_id);
            assert_eq!(native.request_id, canonical.request_id);
            let control = mode == clock_recovery::Mode::Recovery && enabled;
            assert_eq!(
                canonical
                    .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
                    .is_ok(),
                control
            );
            assert_eq!(
                native
                    .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
                    .is_ok(),
                control
            );
            for capability in [Capability::Plan, Capability::Observe, Capability::Designate] {
                assert!(
                    !canonical
                        .grants
                        .iter()
                        .any(|grant| grant.capability == capability)
                );
                assert!(
                    !native
                        .grants
                        .iter()
                        .any(|grant| grant.capability == capability)
                );
            }
            assert_eq!(canonical.budget.max_game_ticks, 0);
            let mut rewritten = canonical.clone();
            rewritten.anchor.fortress_id = FortressId::NIL;
            assert!(
                rewritten
                    .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
                    .is_err()
            );
        }
    }
    Ok(())
}

#[test]
fn recovery_packets_keep_history_coverage_separate_from_live_original_evidence() -> Result<()> {
    let selected = config()?;
    let c = context(&selected, clock_recovery::Mode::Recovery, true);
    let id = Digest32::of_bytes(b"monitor packet").to_string();
    let result = json!({"ok":true,"clock_recovery_only":true,"profile":"clock-recovery",
        "blueprint_monitor":{"monitor_id":id,"history_verified":true,"terminal":false,
            "sampling_fenced":false,"attempt_limit_reached":false,"fresh_sample_acquired":true,
            "sampled_excavation_condition_proven":true,"original_designations_verified":true},
        "blueprint_clock":{"monitor_id":id,"history_verified":true,"pending":true,"unresolved":true,
            "mode":"control","needs_reconciliation":true,"active_run":{"terminal":false}}});
    let output = presentation::packet("fortress.query", result, Some(&c), None, None, None, None);
    let value: Value = serde_json::from_str(&output).map_err(|_| exhausted())?;
    assert_eq!(
        value["result"]["blueprint_monitor"]["fresh_sample_acquired"],
        false
    );
    assert_eq!(
        value["result"]["blueprint_monitor"]["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(
        value["result"]["blueprint_monitor"]["original_designations_verified"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["runtime"],
        "unadmitted_blueprint_clock_recovery"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["scope"],
        "this_original_monitor_and_clock_history_only"
    );
    let domains = value["agent_turn"]["coverage"]["complete_domains"]
        .as_array()
        .ok_or_else(exhausted)?;
    assert!(domains.contains(&json!("this_blueprint_clock_history")));
    assert!(domains.contains(&json!("this_blueprint_monitor_history")));
    assert!(!domains.contains(&json!("this_journal_coordination_inventory")));
    let next = value["agent_turn"]["recommendations"]
        .as_array()
        .ok_or_else(exhausted)?;
    assert!(
        next.iter()
            .any(|step| step["tool"] == "fortress.wait" && step["arguments"]["scope"] == "clock")
    );
    assert!(
        !next
            .iter()
            .any(|step| step["tool"] == "fortress.plan" || step["tool"] == "fortress.commit")
    );
    assert!(
        !next
            .iter()
            .any(|step| step["tool"] == "fortress.wait" && step["arguments"]["scope"] != "clock")
    );
    Ok(())
}

#[test]
fn offline_clock_packets_never_recommend_native_reconciliation_or_cancellation() -> Result<()> {
    let selected = config()?;
    let c = context(&selected, clock_recovery::Mode::Offline, true);
    let result = json!({"ok":true,"clock_recovery_only":true,"profile":"clock-offline",
        "blueprint_clock":{"monitor_id":Digest32::of_bytes(b"offline monitor").to_string(),
            "history_verified":true,"mode":"offline","pending":true,"unresolved":true,
            "needs_reconciliation":true,"active_run":{"terminal":false}}});
    let output = presentation::packet("fortress.query", result, Some(&c), None, None, None, None);
    let value: Value = serde_json::from_str(&output).map_err(|_| exhausted())?;
    let next = value["agent_turn"]["recommendations"]
        .as_array()
        .ok_or_else(exhausted)?;
    assert!(
        !next
            .iter()
            .any(|step| step["tool"] == "fortress.wait" || step["tool"] == "fortress.commit")
    );
    assert!(
        !next
            .iter()
            .any(|step| step["tool"] == "fortress.cancel" && step["arguments"]["scope"] == "clock")
    );
    assert!(next.iter().any(|step| step["tool"] == "fortress.cancel"
        && step["arguments"]["scope"] == "session"
        && step["arguments"]["release_for_recovery"] == true));
    Ok(())
}

#[test]
fn opening_failures_keep_one_recovery_shape_and_the_highest_consumed_clock() -> Result<()> {
    let monitor_id = Digest32::of_bytes(b"original monitor pin").to_string();
    let monitor_pin = json!({"monitor_id":monitor_id,
        "head":Digest32::of_bytes(b"monitor recovery head").to_string()});
    let clock_pin = json!({"journal_id":Digest32::of_bytes(b"original clock pin").to_string(),
        "head":Digest32::of_bytes(b"clock recovery head").to_string()});
    let floor = 76543;
    for full in [false, true] {
        let monitor = if full {
            json!({"monitor_id":monitor_id,"recovery":monitor_pin,"history_verified":true,
                "authority_tick_floor":floor,"fresh_sample_acquired":true})
        } else {
            monitor_pin.clone()
        };
        let clock = if full {
            json!({"monitor_id":monitor_id,"recovery":clock_pin,"history_verified":true,
                "authority_tick_floor":floor,"pending":true,"current_pause_proven":true})
        } else {
            clock_pin.clone()
        };
        let monitor = presentation::opening_monitor_reference(monitor);
        let clock = presentation::opening_clock_reference(clock);
        assert_eq!(monitor["recovery"], monitor_pin);
        assert_eq!(clock["recovery"], clock_pin);
        assert!(monitor["recovery"].get("recovery").is_none());
        assert!(clock["recovery"].get("recovery").is_none());
        assert_eq!(monitor["history_verified"], false);
        assert_eq!(monitor["fresh_sample_acquired"], false);
        assert_eq!(clock["history_verified"], false);
        assert_eq!(clock["current_pause_proven"], false);
        assert_eq!(clock["retry_commit_permitted"], false);
        let expected_floor = if full { json!(floor) } else { Value::Null };
        assert_eq!(monitor["authority_tick_floor"], expected_floor);
        assert_eq!(clock["authority_tick_floor"], expected_floor);
        let raw = presentation::packet(
            "fortress.open_session",
            json!({
                "ok":false,"clock_recovery_only":true,"blueprint_monitor":monitor,
                "blueprint_clock":clock}),
            None,
            None,
            None,
            None,
            None,
        );
        let packet: Value = serde_json::from_str(&raw).map_err(|_| exhausted())?;
        assert_eq!(packet["result"]["blueprint_clock"]["recovery"], clock_pin);
        assert_eq!(
            packet["agent_turn"]["active_work"]["blueprint_clock"]["authority_tick_floor"],
            expected_floor
        );
        assert_eq!(
            packet["agent_turn"]["active_work"]["clock_inventory_verified"],
            false
        );
        assert_eq!(
            packet["agent_turn"]["active_work"]["clock_pending_absence_proven"],
            false
        );
        let handoff = presentation::retained_handoff(&raw).ok_or_else(exhausted)?;
        assert_eq!(handoff["blueprint_monitor"]["recovery"], monitor_pin);
        assert_eq!(handoff["blueprint_clock"]["recovery"], clock_pin);
        assert_eq!(
            handoff["blueprint_monitor"]["authority_tick_floor"],
            expected_floor
        );
        assert_eq!(
            handoff["blueprint_clock"]["authority_tick_floor"],
            expected_floor
        );
    }
    Ok(())
}
