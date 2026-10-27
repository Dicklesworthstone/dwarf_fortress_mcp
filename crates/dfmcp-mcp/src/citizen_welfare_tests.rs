//! Query and provenance tests use the real native decoder and canonical model.
use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{
    LiveSpatialObservation,
    citizens::{LiveSpatialCitizenObservation, citizen_entity_id},
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::WorldGraph;

fn put(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) {
    put(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}
fn observation(tick: u32) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid("fixture hex")))
        .collect::<Result<Vec<_>>>()?;
    let original = LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
    let mut operations = original.operations().clone();
    let mut terrain = original.terrain().clone();
    operations.jobs.jobs.clear();
    operations.attachments.clear();
    operations.items.clear();
    operations.buildings.clear();
    operations.jobs.year_tick = tick;
    terrain.year_tick = tick;
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, 7);
    for category in 0..7 {
        put(&mut citizens, 10 + category);
        text(&mut citizens, &format!("Urist {category}"));
        text(&mut citizens, "DWARF");
        for n in [0, 1, 0, 5] {
            put(&mut citizens, n);
        }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());
        put(&mut citizens, category);
        citizens.extend_from_slice(match category {
            0 => &[0, 0],
            1 => &[0, 1],
            _ => &[1, 1],
        });
        citizens.extend_from_slice(&0u16.to_be_bytes());
    }
    let mut full = b"DFMS1800".to_vec();
    part(&mut full, &spatial);
    part(&mut full, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&full, 7, "df".into(), "dfhack".into())
}
fn state() -> Result<LiveSpatialCitizenState> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observation(3)?)?;
    Ok(state)
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(81),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 100_000,
            max_wall_millis: 60_000,
            max_bytes: 1_048_576,
            max_output_tokens: 65_536,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
fn request(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":query})
}
fn query() -> Value {
    json!({"kind":"citizen_welfare"})
}
fn source() -> Digest32 {
    Digest32::of_bytes(b"welfare-fixture")
}
fn fact(name: &str, value: W) -> Fact {
    let path = match name {
        "stress_category" => "citizen.stress_category",
        "job_available_preserve_social" | "job_available_interrupt_social" => {
            "citizen.job_available"
        }
        "worker_entity" | "worker_is_strict_citizen" => "Job.getWorker",
        other => other,
    };
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(format!("spatial/1.8.{path}")),
        source(),
    )
}
fn entity(id: u64, kind: EntityKind, fields: &[(&str, W)]) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 2,
        revision: 3,
        kind,
        label: "untrusted label".into(),
        fields: fields
            .iter()
            .map(|(name, value)| (name.to_string(), fact(name, value.clone())))
            .collect(),
    }
}
fn citizen(id: u64, category: i64) -> EntityRecord {
    entity(
        id,
        EntityKind::Unit,
        &[
            ("stress_category", W::I64(category)),
            ("alive", W::Bool(true)),
            ("job_available_preserve_social", W::Bool(true)),
            ("job_available_interrupt_social", W::Bool(true)),
        ],
    )
}
fn world(entities: Vec<EntityRecord>) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
        true,
        WorldGraph {
            entities: entities.into_iter().map(|e| (e.id, e)).collect(),
            ..WorldGraph::default()
        },
    )
}

#[test]
fn native_roster_reports_all_categories_and_prioritizes_elevated_stress() -> Result<()> {
    let state = state()?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invariant("fixture snapshot"))?;
    let result = execute(&state, &context(snapshot), &request(query()))?;
    assert_eq!(result["summary"]["strict_citizens"], 7);
    assert_eq!(
        result["summary"]["stress_categories"],
        json!([1, 1, 1, 1, 1, 1, 1])
    );
    assert_eq!(result["summary"]["stress_unestablished"], 0);
    assert_eq!(result["summary"]["elevated_stress_category_0_to_2"], 3);
    assert_eq!(result["summary"]["job_available_preserving_social"], 5);
    assert_eq!(
        result["summary"]["job_available_only_interrupting_social"],
        1
    );
    assert_eq!(result["summary"]["job_availability_unestablished"], 0);
    assert_eq!(result["returned"], 3);
    for category in 0..3 {
        let row = &result["rows"][category];
        assert_eq!(row["stress_category"], category);
        assert_eq!(
            row["citizen"]["entity_id"],
            citizen_entity_id(10 + category as u32).to_string()
        );
        assert_eq!(row["name"], format!("Urist {category}"));
        assert_eq!(row["position"], json!([1, 0, 5]));
        assert_eq!(row["unestablished_fields"], json!([]));
        assert_eq!(row["inspect_query"]["expected_anchor"], result["anchor"]);
        assert_eq!(row["current_job_assignments"]["no_assignment_proven"], true);
    }
    assert_eq!(result["rows"][0]["job_availability"], "unavailable");
    assert_eq!(
        result["rows"][1]["job_availability"],
        "available_only_interrupting_social"
    );
    for key in ["needs", "physical_health", "hunger", "thirst", "injuries"] {
        assert_eq!(result["coverage"][key], "unknown");
    }
    assert_eq!(result["all_clear_proven"], false);
    assert_eq!(result["mutation_dispatched"], false);
    assert_eq!(result["native_captures"], 0);
    assert!(
        result["interpretation"]
            .as_str()
            .is_some_and(|s| s.contains("no-soul fallback"))
    );
    assert_eq!(
        state.snapshot().map(WorldSnapshot::anchor),
        Some(snapshot.anchor())
    );
    Ok(())
}

#[test]
fn continuation_binds_filter_work_session_and_complete_snapshot() -> Result<()> {
    let mut state = state()?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invariant("fixture snapshot"))?;
    let context = context(snapshot);
    let mut q = json!({"kind":"citizen_welfare","max_stress_category":6,"limit":2});
    let first = execute(&state, &context, &request(q.clone()))?;
    assert_eq!(first["total_rows"], 7);
    q["continuation"] = first["continuation"].clone();
    q["limit"] = json!(3);
    let second = execute(&state, &context, &request(q.clone()))?;
    assert_eq!(second["rows"][0]["stress_category"], 2);
    assert_eq!(second["rows"][2]["stress_category"], 4);
    assert_eq!(execute(&state, &context, &request(q.clone()))?, second);
    for (field, value) in [
        ("max_stress_category", json!(5)),
        ("include_unestablished", json!(false)),
        ("max_work", json!(3_999_999)),
    ] {
        let mut changed = q.clone();
        changed[field] = value;
        assert!(
            matches!(execute(&state,&context,&request(changed)),Err(e) if e.code==ErrorCode::StaleAnchor)
        );
    }
    let mut other = context.clone();
    other.session_id = SessionId::new(99);
    assert!(
        matches!(execute(&state,&other,&request(q.clone())),Err(e) if e.code==ErrorCode::StaleAnchor)
    );
    state.publish(observation(4)?)?;
    let next = self::context(
        state
            .snapshot()
            .ok_or_else(|| invariant("fixture snapshot"))?,
    );
    assert!(matches!(execute(&state,&next,&request(q)),Err(e) if e.code==ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn malformed_queries_authority_and_resource_denials_fail_closed() -> Result<()> {
    let state = state()?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invariant("fixture snapshot"))?;
    let context = context(snapshot);
    for (key, value) in [
        ("max_stress_category", json!(7)),
        ("max_stress_category", json!(-1)),
        ("max_stress_category", json!(1.5)),
        ("include_unestablished", json!("yes")),
        ("unknown", json!(true)),
        ("limit", json!(0)),
        ("limit", json!(129)),
        ("max_work", json!(0)),
    ] {
        let mut q = query();
        q[key] = value;
        assert!(
            execute(&state, &context, &request(q)).is_err(),
            "accepted {key}"
        );
    }
    let mut wrong = request(query());
    wrong["expected_anchor"] = json!({});
    assert!(matches!(execute(&state,&context,&wrong),Err(e) if e.code==ErrorCode::StaleAnchor));
    for case in 0..7 {
        let mut denied = context.clone();
        match case {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].remaining_uses = Some(0),
            3 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
            4 => denied.budget.max_wall_millis = 0,
            5 => denied.budget.max_entities = 1,
            _ => denied.budget.max_bytes = 1,
        }
        assert!(
            execute(&state, &denied, &request(query())).is_err(),
            "accepted denial {case}"
        );
    }
    let mut q = query();
    q["max_work"] = json!(1);
    assert!(
        matches!(execute(&state,&context,&request(q)),Err(e) if e.code==ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn pagination_respects_complete_summary_and_whole_row_byte_limits() -> Result<()> {
    let state = state()?;
    let mut context = context(
        state
            .snapshot()
            .ok_or_else(|| invariant("fixture snapshot"))?,
    );
    let first = execute(
        &state,
        &context,
        &request(json!({"kind":"citizen_welfare","limit":1})),
    )?;
    context.budget.max_bytes = first.to_string().len() as u64;
    let bounded = execute(
        &state,
        &context,
        &request(json!({"kind":"citizen_welfare","limit":128})),
    )?;
    assert_eq!(bounded["returned"], 1);
    assert_eq!(bounded["truncated"], true);
    assert!(bounded["continuation"].is_string());
    assert!(bounded.to_string().len() as u64 <= context.budget.max_bytes);
    context.budget.max_bytes -= 1;
    assert!(
        matches!(execute(&state,&context,&request(query())),Err(e) if e.code==ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn unknown_stress_is_not_a_negative_or_healthy_finding() -> Result<()> {
    for case in 0..12 {
        let mut citizen = citizen(10, 0);
        let value = citizen
            .fields
            .get_mut("stress_category")
            .ok_or_else(|| invariant("fixture stress"))?;
        match case {
            0 => value.presence = Some(FactPresence::Unknown("not acquired".into())),
            1 => value.presence = Some(FactPresence::Absent),
            2 => value.presence = Some(FactPresence::Redacted("hidden".into())),
            3 => value.presence = Some(FactPresence::Unsupported("no field".into())),
            4 => value.presence = Some(FactPresence::Omitted("out of scope".into())),
            5 => value.observed_at = GameTick(9),
            6 => value.source_digest = Digest32::ZERO,
            7 => value.source = FactSource::DfhackField("unrelated.stress_category".into()),
            8 => *value = fact("stress_category", W::I64(-1)),
            9 => *value = fact("stress_category", W::I64(7)),
            10 => *value = fact("stress_category", W::Text("0".into())),
            _ => value.presence = Some(FactPresence::Known(W::I64(6))),
        }
        let snapshot = world(vec![citizen]);
        let context = context(&snapshot);
        let mut work = Work::new(&context, MAX_WORK)?;
        let report = assess(&snapshot, source(), 2, true, &mut work)?;
        assert_eq!(report.summary()["stress_unestablished"], 1, "case {case}");
        assert_eq!(report.summary()["elevated_stress_category_0_to_2"], 0);
        assert_eq!(report.row(0)?["stress_evidence"], "unknown");
        assert!(report.row(0)?["stress_category"].is_null());
        assert!(
            assess(&snapshot, source(), 2, false, &mut work)?
                .selected
                .is_empty()
        );
    }
    let snapshot = world(vec![citizen(10, 3)]);
    let context = context(&snapshot);
    let report = assess(
        &snapshot,
        source(),
        6,
        true,
        &mut Work::new(&context, MAX_WORK)?,
    )?;
    assert_eq!(report.summary()["stress_categories"][3], 1);
    assert_eq!(report.summary()["elevated_stress_category_0_to_2"], 0);
    Ok(())
}

#[test]
fn assignment_examples_are_bounded_and_uncertain_links_prevent_absence_proof() -> Result<()> {
    let mut entities = vec![citizen(10, 0), citizen(11, 0)];
    for id in 100..106 {
        entities.push(entity(
            id,
            EntityKind::Job,
            &[
                ("worker_entity", W::Entity(EntityId::new(10))),
                ("worker_is_strict_citizen", W::Bool(true)),
            ],
        ));
    }
    let clean = world(entities.clone());
    let context = context(&clean);
    let clean_report = assess(
        &clean,
        source(),
        6,
        true,
        &mut Work::new(&context, MAX_WORK)?,
    )?;
    assert_eq!(
        clean_report.row(0)?["current_job_assignments"]["observed_count"],
        6
    );
    assert_eq!(
        clean_report.row(0)?["current_job_assignments"]["examples"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
    assert_eq!(
        clean_report.row(0)?["current_job_assignments"]["omitted_examples"],
        2
    );
    assert_eq!(
        clean_report.row(1)?["current_job_assignments"]["no_assignment_proven"],
        true
    );
    entities.push(entity(
        107,
        EntityKind::Job,
        &[
            ("worker_entity", W::Entity(EntityId::new(11))),
            ("worker_is_strict_citizen", W::Bool(false)),
        ],
    ));
    let inconsistent = world(entities);
    let context = self::context(&inconsistent);
    let report = assess(
        &inconsistent,
        source(),
        6,
        true,
        &mut Work::new(&context, MAX_WORK)?,
    )?;
    assert_eq!(report.summary()["unresolved_job_worker_links"], 1);
    assert_eq!(
        report.row(1)?["current_job_assignments"]["no_assignment_proven"],
        false
    );
    Ok(())
}

#[test]
fn inconsistent_availability_is_explicitly_unknown() -> Result<()> {
    let mut unit = citizen(10, 1);
    unit.fields.insert(
        "job_available_interrupt_social".into(),
        fact("job_available_interrupt_social", W::Bool(false)),
    );
    let snapshot = world(vec![unit]);
    let context = context(&snapshot);
    let report = assess(
        &snapshot,
        source(),
        2,
        true,
        &mut Work::new(&context, MAX_WORK)?,
    )?;
    assert_eq!(report.summary()["job_availability_unestablished"], 1);
    assert_eq!(report.row(0)?["job_availability"], "unknown");
    Ok(())
}

#[test]
fn historical_drilldowns_remain_pinned_and_fit_original_rows() -> Result<()> {
    let state = state()?;
    let context = context(
        state
            .snapshot()
            .ok_or_else(|| invariant("fixture snapshot"))?,
    );
    let mut result = execute(&state, &context, &request(query()))?;
    let original = result.to_string().len();
    let record_digest = Digest32::of_bytes(b"record");
    pin_historical(&mut result, u64::MAX, record_digest)?;
    assert!(result.to_string().len() <= original);
    for row in result["rows"]
        .as_array()
        .ok_or_else(|| invariant("fixture rows"))?
    {
        assert_eq!(row["inspect_query"]["query"]["kind"], "historical_query");
        assert_eq!(row["inspect_query"]["query"]["record"], u64::MAX);
        assert_eq!(
            row["inspect_query"]["query"]["record_digest"],
            record_digest.to_string()
        );
        assert_eq!(row["inspect_query"]["query"]["query"]["kind"], "inspect");
        assert_eq!(
            row["inspect_query"]["query"]["query"]["generation"],
            row["citizen"]["generation"]
        );
    }
    let shortest_anchor = StateAnchor {
        fortress_id: FortressId::new(0),
        cursor: ObservationCursor {
            epoch: 0,
            sequence: 0,
        },
        tick: GameTick(0),
        state_hash: Digest32::ZERO,
    };
    let mut minimal = json!({"kind":"citizen_welfare","rows":[{"inspect_query":{
        "schema":"dfmcp.query/1","expected_anchor":anchor_json(shortest_anchor),
        "query":{"kind":"inspect","entity_id":EntityId::new(0).to_string(),
            "generation":0,"fields":[]}}}]});
    let original = minimal.to_string().len();
    pin_historical(&mut minimal, u64::MAX, record_digest)?;
    assert!(minimal.to_string().len() <= original);
    Ok(())
}

#[test]
fn worker_absence_requires_coherent_native_negative_evidence() -> Result<()> {
    for case in 0..4 {
        let mut job = entity(
            100,
            EntityKind::Job,
            &[("worker_is_strict_citizen", W::Bool(false))],
        );
        if case != 0 {
            let mut worker = fact("worker_entity", W::Bool(false));
            worker.presence = Some(if case == 3 {
                FactPresence::Unknown("outside strict roster".into())
            } else {
                FactPresence::Absent
            });
            if case == 1 {
                worker.source_digest = Digest32::ZERO;
            }
            job.fields.insert("worker_entity".into(), worker);
        }
        let snapshot = world(vec![citizen(10, 0), job]);
        let context = context(&snapshot);
        let report = assess(
            &snapshot,
            source(),
            2,
            true,
            &mut Work::new(&context, MAX_WORK)?,
        )?;
        assert_eq!(
            report.row(0)?["current_job_assignments"]["no_assignment_proven"],
            case >= 2
        );
        assert_eq!(
            report.summary()["unresolved_job_worker_links"],
            usize::from(case < 2)
        );
    }
    Ok(())
}
