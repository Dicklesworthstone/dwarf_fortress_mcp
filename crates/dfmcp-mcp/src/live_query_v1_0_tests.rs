//! These tests run the real canonical live adapter and production query packet
//! path with explicitly scripted observations. They are not native-game proof.
use super::*;
use dfmcp_adapter::{
    BridgeManifest, CitizenRecord, LiveObservationSource, LiveReadAdapterConfig, ObservationPage,
};
use std::collections::VecDeque;

struct ScriptedSource {
    pages: VecDeque<ObservationPage>,
    reads: usize,
    poisoned: bool,
}

fn manifest() -> BridgeManifest {
    BridgeManifest {
        bridge_version: "0.1.0".to_owned(),
        dfhack_version: "0.51.11-r1".to_owned(),
        df_version: "0.51.11".to_owned(),
        world_loaded: true,
        fortress_mode: true,
        bridge_generation: 42,
        supported_methods: BTreeSet::from(["Handshake".to_owned(), "ReadObservation".to_owned()]),
    }
}

impl LiveObservationSource for ScriptedSource {
    fn bridge_manifest(&self) -> BridgeManifest {
        manifest()
    }

    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
    ) -> Result<ObservationPage> {
        assert_eq!(offset, 0);
        assert!(maximum >= 64);
        assert!(names);
        self.reads += 1;
        self.pages
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterFailure, "scripted source exhausted"))
    }
}

impl LiveSessionAdapter for LiveReadAdapter<ScriptedSource> {
    fn query_projection(&self) -> Option<&dfmcp_adapter::LiveWorldProjection> {
        self.current_projection()
    }
    fn query_capsule(&self) -> Option<&dfmcp_adapter::LiveObservationCapsule> {
        self.last_capsule()
    }
    fn query_source_poisoned(&self) -> bool {
        self.source().poisoned
    }
    fn query_source_poison_reason(&self) -> Option<&str> {
        self.source().poisoned.then_some("scripted source fenced")
    }
}

fn page(count: i32, tick: u32, escaped: bool) -> ObservationPage {
    ObservationPage {
        bridge_generation: 42,
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: 105,
        current_year_tick: tick,
        world_name: "Scripted query test fortress".to_owned(),
        world_folder: "query-fixture".to_owned(),
        site_id: 7,
        citizen_count_total: count as u32,
        citizen_offset: 0,
        complete: true,
        citizens: (0..count)
            .map(|id| CitizenRecord {
                unit_id: id,
                name: if escaped {
                    format!("Urist {id} \"\\\n{}", "\u{0001}".repeat(220))
                } else {
                    format!("Urist {id}")
                },
                race: "dwarf".to_owned(),
                profession: 4,
                x: id,
                y: 2,
                z: 3,
                alive: true,
                sane: true,
                active: true,
                visible: true,
                citizen: true,
                resident: false,
                baby: false,
                child: false,
                adult: true,
            })
            .collect(),
    }
}

fn session(count: i32, escaped: bool) -> Result<LiveSession<LiveReadAdapter<ScriptedSource>>> {
    let mut adapter = LiveReadAdapter::new(
        ScriptedSource {
            pages: VecDeque::from([page(count, 12_345, escaped), page(count, 12_346, escaped)]),
            reads: 0,
            poisoned: false,
        },
        LiveReadAdapterConfig {
            fortress_id: FortressId::new(9),
            page_size: 64,
            max_citizens: 64,
            include_names: true,
            initial_epoch: 3,
        },
    )?;
    let anchor = adapter.bootstrap()?.snapshot.anchor();
    Ok(LiveSession {
        session_id: SessionId::new((1u128 << 127) + 99),
        adapter,
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 128,
            max_bytes: 4 * 1024 * 1024,
            max_output_tokens: 16_384,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        next_request_id: 0,
    })
}

fn decode(raw: &str) -> Result<JsonValue> {
    serde_json::from_str(raw)
        .map_err(|failure| error(ErrorCode::InternalInvariantViolation, failure.to_string()))
}

fn query(
    s: &mut LiveSession<LiveReadAdapter<ScriptedSource>>,
    mode: &str,
    limit: u32,
    next: Option<String>,
) -> Result<JsonValue> {
    decode(&query_session(
        s,
        Some(mode.to_owned()),
        Some(limit),
        next,
        Instant::now(),
    ))
}

fn next(value: &JsonValue) -> Option<String> {
    value["continuation"].as_str().map(str::to_owned)
}

fn expected_ids(s: &LiveSession<LiveReadAdapter<ScriptedSource>>, mode: &str) -> Vec<String> {
    s.adapter
        .current_projection()
        .expect("bootstrap projection")
        .snapshot
        .graph
        .entities
        .values()
        .filter(|entity| mode == "all" || entity.kind == EntityKind::Unit)
        .map(|entity| entity.id.to_string())
        .collect()
}

#[test]
fn every_citizen_and_combined_row_is_reachable_without_another_capture() -> Result<()> {
    for mode in ["citizens", "all"] {
        let mut s = session(40, false)?;
        let expected = expected_ids(&s, mode);
        let anchor = s.current_anchor()?;
        let capsule = s
            .adapter
            .last_capsule()
            .expect("bootstrap capsule")
            .canonical_bytes
            .clone();
        let mut token = None;
        let mut observed = Vec::new();
        for page_number in 0..40 {
            let result = query(&mut s, mode, if page_number == 0 { 7 } else { 3 }, token)?;
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(result["anchor"], anchor_json(anchor));
            assert_eq!(result["agent_turn"]["anchor"], anchor_json(anchor));
            assert_eq!(result["matched"], expected.len());
            assert_eq!(result["offset"], observed.len());
            let rows = result["rows"].as_array().expect("whole rows");
            assert!(!rows.is_empty());
            observed.extend(
                rows.iter()
                    .map(|row| row["entity_id"].as_str().expect("ID").to_owned()),
            );
            assert!(rows.iter().all(|row| row["generation"].is_u64()));
            token = next(&result);
            if token.is_none() {
                break;
            }
            assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
            assert_eq!(
                result["agent_turn"]["coverage"]["continuation"],
                result["continuation"]
            );
            assert_eq!(
                result["next_query"]["arguments"]["continuation"],
                result["continuation"]
            );
            assert_eq!(result["complete_domain"], false);
        }
        assert_eq!(observed, expected);
        assert_eq!(s.current_anchor()?, anchor);
        assert_eq!(s.adapter.source().reads, 1);
        assert_eq!(
            s.adapter
                .last_capsule()
                .expect("retained capsule")
                .canonical_bytes,
            capsule
        );
    }
    Ok(())
}

#[test]
fn default_output_budget_keeps_whole_rows_and_required_context() -> Result<()> {
    for mode in ["summary", "citizens", "all"] {
        let mut s = session(40, false)?;
        s.budget.max_output_tokens = WorkBudget::CONSERVATIVE_DEFAULT.max_output_tokens;
        assert_eq!(s.budget.max_output_tokens, 1_500);
        let anchor = s.current_anchor()?;
        let omitted = coverage_json(&s)["omitted_domains"].clone();
        let mutation_warning = uncertainties_json(&s).pop().expect("mutation uncertainty");
        let mut continuation = None;
        let mut returned = 0;
        loop {
            let raw = query_session(
                &mut s,
                Some(mode.to_owned()),
                None,
                continuation,
                Instant::now(),
            );
            let value = decode(&raw)?;
            assert_eq!(value["ok"], true, "{mode}: {value}");
            assert!(fits(&raw, s.budget));
            assert_eq!(value["anchor"], anchor_json(anchor));
            assert_eq!(value["agent_turn"]["anchor"], value["anchor"]);
            assert_eq!(value["agent_turn"]["coverage"]["omitted_domains"], omitted);
            assert_eq!(
                value["agent_turn"]["uncertainty"][0]["epistemic_state"],
                "unknown"
            );
            assert_eq!(
                value["agent_turn"]["uncertainty"][0]["domains"],
                json!(
                    OMITTED_LIVE_DOMAINS
                        .iter()
                        .map(|(domain, _)| *domain)
                        .collect::<Vec<_>>()
                )
            );
            assert_eq!(value["agent_turn"]["uncertainty"][1], mutation_warning);
            assert_eq!(
                value["agent_turn"]["budget"]["admitted"],
                budget_json(s.budget)["admitted"]
            );
            assert_eq!(
                value["agent_turn"]["budget"]["requested_equals_admitted"],
                true
            );
            let rows = value["rows"].as_array().expect("complete query rows");
            assert!(!rows.is_empty());
            returned += rows.len();
            assert!(returned <= 41, "continuation must make progress");
            continuation = next(&value);
            if continuation.is_none() {
                break;
            }
        }
        assert_eq!(
            returned,
            match mode {
                "summary" => 1,
                "citizens" => 40,
                _ => 41,
            }
        );
        assert_eq!(s.adapter.source().reads, 1);
        assert_eq!(s.current_anchor()?, anchor);
    }
    Ok(())
}

#[test]
fn output_clipping_and_escaped_labels_preserve_exact_cursor_progress() -> Result<()> {
    let mut s = session(40, true)?;
    s.budget.max_output_tokens = 4_096;
    let expected = expected_ids(&s, "citizens");
    let mut token = None;
    let mut observed = Vec::new();
    let mut pages = 0;
    loop {
        let raw = query_session(
            &mut s,
            Some("citizens".to_owned()),
            Some(128),
            token,
            Instant::now(),
        );
        let result = decode(&raw)?;
        assert_eq!(result["ok"], true, "{result}");
        assert!(fits(&raw, s.budget));
        assert!(raw.len() <= s.budget.max_output_tokens as usize * 4);
        let rows = result["rows"].as_array().expect("whole rows");
        assert!(!rows.is_empty());
        for row in rows {
            let id = row["entity_id"].as_str().expect("ID").to_owned();
            let stored = s
                .adapter
                .current_projection()
                .expect("projection")
                .snapshot
                .graph
                .entities
                .values()
                .find(|entity| entity.id.to_string() == id)
                .expect("selected canonical entity");
            assert_eq!(row["fields"][0][1], stored.label);
            observed.push(id);
        }
        pages += 1;
        assert!(pages <= 40);
        token = next(&result);
        if token.is_none() {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(observed, expected);
    assert_eq!(s.adapter.source().reads, 1);
    Ok(())
}

#[test]
fn escaped_wire_byte_limit_fits_complete_pages_and_maximum_numeric_id() -> Result<()> {
    let mut s = session(16, true)?;
    s.budget.max_output_tokens = 65_536;
    let first = query(&mut s, "citizens", 4, None)?;
    assert_eq!(first["returned"], 4, "{first}");
    let wire = |packet: &JsonValue| {
        CoreResult::Final(FinalCoreResult::ToolsCall {
            result: CompleteResult::new(
                FinalCallToolResult {
                    content: vec![ContentBlock::text(packet.to_string())],
                    is_error: false,
                    structured_content: None,
                },
                ResultMeta::empty(),
            ),
            diagnostic: None,
        })
        .encode()
        .expect("pinned modern codec")
    };
    let initial_bytes = wire(&first).len() + WIRE_OVERHEAD;
    assert!(initial_bytes > 8_192);
    s.budget.max_bytes = (initial_bytes - 512) as u64;
    let expected = expected_ids(&s, "citizens");
    let mut observed = Vec::new();
    let mut token = None;
    for _ in 0..16 {
        let result = query(&mut s, "citizens", 4, token)?;
        assert_eq!(result["ok"], true, "{result}");
        assert!(fits(&result.to_string(), s.budget));
        let encoded = wire(&result);
        let envelope = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{encoded}}}\n",
            "9".repeat(4096)
        );
        assert!(envelope.len() as u64 <= s.budget.max_bytes);
        let rows = result["rows"].as_array().expect("whole rows");
        assert!(!rows.is_empty());
        if observed.is_empty() {
            assert!(rows.len() < 4);
        }
        observed.extend(
            rows.iter()
                .map(|row| row["entity_id"].as_str().expect("ID").to_owned()),
        );
        token = next(&result);
        if token.is_none() {
            break;
        }
    }
    assert_eq!(observed, expected);
    assert_eq!(s.adapter.source().reads, 1);
    Ok(())
}

#[test]
fn continuation_cannot_cross_selection_session_or_new_observation() -> Result<()> {
    let mut s = session(8, false)?;
    let first = query(&mut s, "citizens", 2, None)?;
    let token = next(&first).expect("first continuation");
    assert_eq!(
        query(&mut s, "all", 2, Some(token.clone()))?["error"]["code"],
        "cursor_gap"
    );
    let mut other = session(8, false)?;
    other.session_id = SessionId::new((1u128 << 127) + 100);
    assert_eq!(
        query(&mut other, "citizens", 2, Some(token.clone()))?["error"]["code"],
        "cursor_gap"
    );
    let (_, context) = s.next_context()?;
    s.adapter.observe(
        &ObservationRequest {
            since: Some(context.anchor.cursor),
            projection: Projection::Full,
            interest: InterestSet::default(),
            max_entities: context.budget.max_entities,
            max_bytes: context.budget.max_bytes,
            max_output_tokens: context.budget.max_output_tokens,
            continuation: None,
        },
        &context,
    )?;
    assert_eq!(
        query(&mut s, "citizens", 2, Some(token))?["error"]["code"],
        "cursor_gap"
    );
    assert_eq!(s.adapter.source().reads, 2);
    Ok(())
}

#[test]
fn complete_anchor_and_canonical_offset_are_bound_to_each_continuation() -> Result<()> {
    let mut s = session(8, false)?;
    let (_, context) = s.next_context()?;
    let token = cursor(&context, "citizens", 2);
    assert_eq!(offset(&context, "citizens", Some(&token))?, 2);
    for changed in [
        {
            let mut c = context.clone();
            c.anchor.tick = dfmcp_core::GameTick::new(c.anchor.tick.get() + 1);
            c
        },
        {
            let mut c = context.clone();
            c.anchor.state_hash = Digest32::of_bytes(b"other projection");
            c
        },
        {
            let mut c = context.clone();
            c.anchor.cursor.epoch += 1;
            c
        },
    ] {
        assert!(
            matches!(offset(&changed, "citizens", Some(&token)), Err(e) if e.code == ErrorCode::CursorGap)
        );
    }
    for bad in [
        token.replacen("lq10:2:", "lq10:02:", 1),
        token.replacen("lq10:2:", "lq10:0:", 1),
        token.replacen("lq10:2:", "lq10:4294967296:", 1),
        format!("{token}:extra"),
        "x".repeat(MAX_CURSOR_BYTES + 1),
        ContinuationToken::new(context.anchor.fortress_id, context.anchor.cursor, 2).encode(),
    ] {
        assert!(offset(&context, "citizens", Some(&bad)).is_err(), "{bad}");
    }
    let altered = token.replacen("lq10:2:", "lq10:3:", 1);
    assert!(
        matches!(offset(&context, "citizens", Some(&altered)), Err(e) if e.code == ErrorCode::CursorGap)
    );
    Ok(())
}

#[test]
fn every_page_rechecks_current_grants_and_refuses_a_poisoned_source() -> Result<()> {
    for failure in ["revoked", "expired", "scoped", "poisoned"] {
        let mut s = session(8, false)?;
        let first = query(&mut s, "citizens", 2, None)?;
        let token = next(&first).expect("first continuation");
        match failure {
            "revoked" => s
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            "expired" => s.grants[0].expires_at_tick = Some(dfmcp_core::GameTick::new(0)),
            "scoped" => {
                s.grants[0].scope.entity_ids.insert(EntityId::new(1));
            }
            _ => s.adapter.source_mut().poisoned = true,
        }
        let refused = query(&mut s, "citizens", 2, Some(token))?;
        assert_eq!(refused["ok"], false);
        assert_eq!(
            refused["error"]["code"],
            if failure == "poisoned" {
                "adapter_failure"
            } else {
                "capability_denied"
            }
        );
        assert!(refused.get("rows").is_none());
        assert_eq!(s.adapter.source().reads, 1);
    }
    Ok(())
}

#[test]
fn empty_complete_domain_succeeds_but_impossible_row_budget_does_not_loop() -> Result<()> {
    let mut empty = session(0, false)?;
    let result = query(&mut empty, "citizens", 2, None)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["matched"], 0);
    assert_eq!(result["rows"], json!([]));
    assert_eq!(result["complete_domain"], true);
    assert!(result["continuation"].is_null());
    let mut s = session(4, false)?;
    s.budget.max_output_tokens = 1;
    let refused = query(&mut s, "citizens", 2, None)?;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    assert!(refused.get("continuation").is_none());
    assert_eq!(s.adapter.source().reads, 1);
    Ok(())
}

#[test]
fn closed_query_schema_preserves_the_eleven_tool_waist() -> Result<()> {
    let definition = FortressPagedQuery.definition();
    assert_eq!(definition.name, FortressQuery.definition().name);
    assert_eq!(definition.input_schema["additionalProperties"], false);
    assert_eq!(
        definition.input_schema["properties"]["mode"]["enum"],
        json!(["summary", "citizens", "all", null])
    );
    let nullable = serde_json::from_value::<Arguments>(json!({
        "session_id": null, "mode": null, "limit": null, "continuation": null
    }))
    .expect("optional query inputs retain their null defaults");
    assert!(nullable.session_id.is_none());
    assert!(nullable.mode.is_none());
    assert!(nullable.limit.is_none());
    assert!(nullable.continuation.is_none());
    assert_eq!(
        definition.input_schema["properties"]["limit"]["maximum"],
        256
    );
    assert_eq!(
        definition.input_schema["properties"]
            .as_object()
            .expect("properties")
            .len(),
        4
    );
    for bad in [
        json!({"offset":2}),
        json!({"mode":"citizens","at":"other"}),
        json!({"limit":-1}),
        json!({"limit":"2"}),
    ] {
        assert!(serde_json::from_value::<Arguments>(bad).is_err());
    }
    let mut s = session(4, false)?;
    let default_summary = decode(&query_session(
        &mut s,
        nullable.mode,
        nullable.limit,
        nullable.continuation,
        Instant::now(),
    ))?;
    assert_eq!(default_summary["ok"], true);
    assert_eq!(default_summary["mode"], "summary");
    assert_eq!(default_summary["returned"], 1);
    let first = query(&mut s, "citizens", 1, None)?;
    assert_eq!(first["next_query"]["tool"], definition.name);
    for limit in [0, 129, 257] {
        assert_eq!(
            query(&mut s, "citizens", limit, None)?["error"]["code"],
            "invalid_request"
        );
    }
    assert_eq!(
        query(&mut s, "unknown", 1, None)?["error"]["code"],
        "invalid_request"
    );
    assert_eq!(s.adapter.source().reads, 1);
    Ok(())
}
