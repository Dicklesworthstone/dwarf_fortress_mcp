//! Real native decoding and generation publication, plus adversarial canonical
//! fact provenance. These cases do not qualify a native plugin or live fortress.
use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{
    LiveSpatialObservation, citizens::LiveSpatialCitizenObservation,
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, RequestId, SessionId, WorkBudget,
};
use dfmcp_world::WorldGraph;

fn part(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
#[derive(Clone)]
struct Input {
    tick: u32,
    categories: Vec<i32>,
    available: [u8; 2],
    bridge: u64,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            tick: 3,
            categories: vec![3],
            available: [1, 1],
            bridge: 7,
        }
    }
}
fn observation(input: &Input) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid("fixture hex")))
        .collect::<Result<Vec<_>>>()?;
    let base =
        LiveSpatialObservation::decode_payload(&bytes, input.bridge, "df".into(), "dfhack".into())?;
    let mut operations = base.operations().clone();
    let mut terrain = base.terrain().clone();
    operations.jobs.year_tick = input.tick;
    terrain.year_tick = input.tick;
    operations.jobs.jobs.clear();
    operations.items.clear();
    operations.buildings.clear();
    operations.attachments.clear();
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&(input.categories.len() as u32).to_be_bytes());
    for (offset, category) in input.categories.iter().enumerate() {
        citizens.extend_from_slice(&(10 + offset as u32).to_be_bytes());
        text(&mut citizens, &format!("Urist {offset}"));
        text(&mut citizens, "DWARF");
        for n in [0i32, 1, 0, 5] {
            citizens.extend_from_slice(&n.to_be_bytes());
        }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());
        citizens.extend_from_slice(&category.to_be_bytes());
        citizens.extend_from_slice(&input.available);
        citizens.extend_from_slice(&0u16.to_be_bytes());
    }
    let mut full = b"DFMS1800".to_vec();
    part(&mut full, &spatial);
    part(&mut full, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&full, input.bridge, "df".into(), "dfhack".into())
}
fn states(
    first: &Input,
    last: &Input,
) -> Result<(LiveSpatialCitizenState, LiveSpatialCitizenState)> {
    let mut before = LiveSpatialCitizenState::default();
    before.publish(observation(first)?)?;
    let mut after = before.clone();
    after.publish(observation(last)?)?;
    Ok((before, after))
}
fn snapshot(state: &LiveSpatialCitizenState) -> Result<&WorldSnapshot> {
    state
        .snapshot()
        .ok_or_else(|| invariant("fixture snapshot absent"))
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    Ok(OperationContext {
        session_id: SessionId::new(81),
        request_id: RequestId::new(1),
        anchor: snapshot(state)?.anchor(),
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
    })
}
fn endpoint_ref(record: u64) -> Value {
    json!({"record":record,"record_digest":Digest32::of_bytes(&record.to_be_bytes()).to_string()})
}
fn row(report: &Report) -> Result<Value> {
    report.row(0, &endpoint_ref(1), &endpoint_ref(2))
}
fn page(
    report: &Report,
    c: &OperationContext,
    limit: u32,
    continuation: Option<&str>,
    selection: Value,
    binding: Digest32,
    maximum: u64,
) -> Result<Value> {
    paginate(
        report,
        Page {
            context: c,
            from: &endpoint_ref(1),
            to: &endpoint_ref(2),
            binding,
            selection,
            maximum_work: maximum,
            limit,
            continuation,
        },
    )
}
fn edited(
    state: &LiveSpatialCitizenState,
    edit: impl FnOnce(&mut WorldGraph) -> Result<()>,
) -> Result<WorldSnapshot> {
    let original = snapshot(state)?;
    let a = original.anchor();
    let mut graph = original.graph.clone();
    edit(&mut graph)?;
    Ok(WorldSnapshot::new(
        a.fortress_id,
        a.tick,
        a.cursor,
        original.paused,
        graph,
    ))
}
fn evidence<'a>(
    state: &LiveSpatialCitizenState,
    snapshot: &'a WorldSnapshot,
    complete: bool,
) -> Result<Endpoint<'a>> {
    Ok(Endpoint {
        snapshot,
        source: state.source_digest()?,
        roster: if complete {
            Some(
                snapshot
                    .graph
                    .entities
                    .values()
                    .filter(|e| e.kind == EntityKind::Unit)
                    .map(|e| e.id)
                    .collect(),
            )
        } else {
            None
        },
    })
}
fn compare_edited(
    before: &LiveSpatialCitizenState,
    after: &LiveSpatialCitizenState,
    changed: &WorldSnapshot,
) -> Result<Report> {
    assess(
        Endpoint::state(before)?,
        evidence(after, changed, true)?,
        &context(after)?,
        None,
        true,
        MAX_WORK,
    )
}
fn unit(graph: &mut WorldGraph) -> Result<&mut EntityRecord> {
    graph
        .entities
        .get_mut(&citizen_entity_id(10))
        .ok_or_else(|| invariant("fixture citizen absent"))
}
fn field<'a>(graph: &'a mut WorldGraph, name: &str) -> Result<&'a mut Fact> {
    unit(graph)?
        .fields
        .get_mut(name)
        .ok_or_else(|| invariant("fixture citizen field absent"))
}
fn set(fact: &mut Fact, value: W) {
    fact.value = value.clone();
    fact.presence = Some(FactPresence::Known(value));
}

#[test]
fn native_categories_compare_directions_and_thresholds_with_whole_roster_histograms() -> Result<()>
{
    let first = Input {
        categories: (0..7).collect(),
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 9,
            categories: (0..7).rev().collect(),
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    assert_eq!(
        report.summary["before"]["stress_categories"],
        json!([1, 1, 1, 1, 1, 1, 1])
    );
    assert_eq!(
        report.summary["after"]["stress_categories"],
        json!([1, 1, 1, 1, 1, 1, 1])
    );
    assert_eq!(report.summary["before"]["strict_citizens"], 7);
    assert_eq!(
        report.summary["after"]["elevated_stress_category_0_to_2"],
        3
    );
    for key in [
        "toward_higher_stress_category",
        "toward_lower_stress_category",
    ] {
        assert_eq!(report.summary["same_generation_category_changes"][key], 3);
    }
    for key in ["entered_elevated_category", "left_elevated_category"] {
        assert_eq!(
            report.summary["same_generation_elevated_category_transitions"][key],
            3
        );
    }
    assert_eq!(
        row(&report)?["indicators"]["stress_category"],
        "toward_lower_stress_category"
    );
    assert_eq!(
        row(&report)?["elevated_category_transition"],
        "left_elevated_category"
    );
    let filtered = compare(&before, &after, &c, None, false, MAX_WORK)?;
    assert_eq!(filtered.len(), 6);
    assert_eq!(filtered.summary["before"]["strict_citizens"], 7);
    assert_eq!(
        filtered.summary["same_generation_with_all_indicators_unchanged"],
        1
    );
    let focus = Focus {
        entity_id: citizen_entity_id(10).to_string(),
        generation: report.before[&citizen_entity_id(10)].generation,
    };
    let focused = compare(&before, &after, &c, Some(&focus), true, MAX_WORK)?;
    assert_eq!(focused.len(), 1);
    assert_eq!(focused.summary["after"]["strict_citizens"], 7);
    assert_eq!(row(&focused)?["inspect_before"]["query"]["record"], 1);
    assert_eq!(row(&focused)?["inspect_after"]["query"]["record"], 2);
    assert_eq!(
        row(&focused)?["inspect_after"]["query"]["query"]["generation"],
        row(&focused)?["after"]["citizen"]["generation"]
    );
    Ok(())
}

#[test]
fn original_citizen_focus_survives_disappearance_and_reuse_without_comparing_replacement()
-> Result<()> {
    let first = Input::default();
    let (before, mut after) = states(
        &first,
        &Input {
            tick: 4,
            categories: vec![],
            ..first.clone()
        },
    )?;
    let generation = snapshot(&before)?.graph.entities[&citizen_entity_id(10)].generation;
    let focus = Focus {
        entity_id: citizen_entity_id(10).to_string(),
        generation,
    };
    let absent = compare(
        &before,
        &after,
        &context(&after)?,
        Some(&focus),
        true,
        MAX_WORK,
    )?;
    assert_eq!(row(&absent)?["identity_status"], "no_longer_observed");
    assert!(row(&absent)?["after"].is_null());
    assert!(row(&absent)?["inspect_after"].is_null());
    after.publish(observation(&Input {
        tick: 5,
        categories: vec![0],
        ..first.clone()
    })?)?;
    let reused = compare(
        &before,
        &after,
        &context(&after)?,
        Some(&focus),
        true,
        MAX_WORK,
    )?;
    let value = row(&reused)?;
    assert_eq!(value["identity_status"], "generation_replaced");
    assert_ne!(
        value["before"]["citizen"]["generation"],
        value["after"]["citizen"]["generation"]
    );
    assert!(value["indicators"].is_null());
    assert_eq!(value["observed_indicator_change"], false);
    let bad = Focus {
        generation: generation.saturating_add(1),
        ..focus
    };
    assert_eq!(
        compare(
            &before,
            &after,
            &context(&after)?,
            Some(&bad),
            true,
            MAX_WORK
        )
        .err()
        .map(|e| e.code),
        Some(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn missing_completeness_cannot_prove_roster_absence_and_native_roster_must_match() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            categories: vec![],
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    let report = assess(
        Endpoint::state(&before)?,
        evidence(&after, snapshot(&after)?, false)?,
        &c,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(
        row(&report)?["identity_status"],
        "roster_absence_unestablished"
    );
    assert_eq!(report.summary["identity_counts"]["no_longer_observed"], 0);
    let (empty, populated) = states(
        &Input {
            categories: vec![],
            ..Input::default()
        },
        &Input {
            tick: 4,
            ..Input::default()
        },
    )?;
    let qualified = compare(
        &empty,
        &populated,
        &context(&populated)?,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(row(&qualified)?["identity_status"], "newly_observed");
    let unknown = assess(
        evidence(&empty, snapshot(&empty)?, false)?,
        Endpoint::state(&populated)?,
        &context(&populated)?,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(
        row(&unknown)?["identity_status"],
        "roster_absence_unestablished"
    );
    let mut wrong = Endpoint::state(&before)?;
    wrong.roster = Some(BTreeSet::new());
    assert_eq!(
        assess(wrong, Endpoint::state(&after)?, &c, None, true, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}

#[test]
fn every_unqualified_stress_representation_remains_unestablished() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    for variant in 0..15 {
        let changed = edited(&after, |graph| {
            if variant == 12 {
                unit(graph)?.fields.remove("stress_category");
                return Ok(());
            }
            let f = field(graph, "stress_category")?;
            match variant {
                0 => f.presence = Some(FactPresence::Unknown("not acquired".into())),
                1 => f.presence = Some(FactPresence::Absent),
                2 => f.presence = Some(FactPresence::Redacted("hidden".into())),
                3 => f.presence = Some(FactPresence::Unsupported("no field".into())),
                4 => f.presence = Some(FactPresence::Omitted("out of scope".into())),
                5 => f.presence = Some(FactPresence::Stale(snapshot(&before)?.anchor())),
                6 => f.source_digest = Digest32::ZERO,
                7 => {
                    f.source = FactSource::DfhackField("spatial/1.6.citizen.stress_category".into())
                }
                8 => set(f, W::I64(-1)),
                9 => set(f, W::I64(7)),
                10 => set(f, W::Text("3".into())),
                11 => f.presence = Some(FactPresence::Known(W::I64(6))),
                13 => f.source = FactSource::Derived("spatial/1.8.citizen.stress_category".into()),
                _ => f.observed_at = GameTick(0),
            }
            Ok(())
        })?;
        let report = compare_edited(&before, &after, &changed)?;
        let value = row(&report)?;
        assert!(
            value["after"]["stress_category"].is_null(),
            "variant {variant}"
        );
        assert_eq!(value["after"]["stress_evidence"], "unestablished");
        assert_eq!(value["indicators"]["stress_category"], "unestablished");
        assert_eq!(value["all_compared_indicators_unchanged"], false);
        assert_eq!(report.summary["after"]["stress_unestablished"], 1);
        assert_eq!(
            report.summary["after"]["elevated_stress_category_0_to_2"],
            0
        );
    }
    Ok(())
}

#[test]
fn availability_status_and_identity_require_independent_consistent_source_fields() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            available: [0, 1],
            ..first.clone()
        },
    )?;
    let report = compare(&before, &after, &context(&after)?, None, true, MAX_WORK)?;
    assert_eq!(row(&report)?["indicators"]["job_availability"], "changed");
    assert_eq!(
        row(&report)?["after"]["job_availability"],
        "available_only_interrupting_social"
    );
    assert_eq!(
        report.summary["after"]["job_availability"]["available_only_interrupting_social"],
        1
    );
    for variant in 0..5 {
        let changed = edited(&after, |graph| {
            match variant {
                0 => field(graph, "job_available_preserve_social")?.source_digest = Digest32::ZERO,
                1 => {
                    set(
                        field(graph, "job_available_preserve_social")?,
                        W::Bool(true),
                    );
                    set(
                        field(graph, "job_available_interrupt_social")?,
                        W::Bool(false),
                    );
                }
                2 => field(graph, "alive")?.source = FactSource::Replay,
                3 => set(field(graph, "native_unit_id")?, W::U64(11)),
                _ => set(field(graph, "active")?, W::Bool(false)),
            }
            Ok(())
        })?;
        let value = row(&compare_edited(&before, &after, &changed)?)?;
        match variant {
            0 | 1 => {
                assert!(value["after"]["job_availability"].is_null());
                assert_eq!(value["indicators"]["job_availability"], "unestablished");
            }
            2 => {
                assert!(value["after"]["observed_status"]["alive"].is_null());
                assert_eq!(value["indicators"]["alive"], "unestablished");
            }
            3 => {
                assert!(value["after"]["native_unit_id"].is_null());
                assert_eq!(value["indicators"]["stress_category"], "unestablished");
                assert_eq!(value["observed_indicator_change"], false);
            }
            _ => assert_eq!(value["indicators"]["active"], "changed"),
        }
        assert_eq!(value["all_compared_indicators_unchanged"], false);
    }
    Ok(())
}

#[test]
fn unestablished_to_observed_category_is_not_a_measured_change() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    let changed = edited(&before, |graph| {
        field(graph, "stress_category")?.presence =
            Some(FactPresence::Unknown("not acquired".into()));
        Ok(())
    })?;
    let report = assess(
        evidence(&before, &changed, true)?,
        Endpoint::state(&after)?,
        &context(&after)?,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(
        row(&report)?["indicators"]["stress_category"],
        "unestablished"
    );
    assert_eq!(row(&report)?["observed_indicator_change"], false);
    assert_eq!(report.summary["before"]["stress_unestablished"], 1);
    assert_eq!(report.summary["after"]["stress_categories"][3], 1);
    Ok(())
}

#[test]
fn category_three_ambiguity_and_same_tick_endpoints_never_prove_measured_stress_or_trends()
-> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            categories: vec![2],
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let value = row(&report)?;
    assert_eq!(value["before"]["category_may_be_no_soul_fallback"], true);
    assert_eq!(value["after"]["category_may_be_no_soul_fallback"], false);
    assert_eq!(
        value["indicators"]["stress_category"],
        "toward_higher_stress_category"
    );
    let result = page(&report, &c, 4, None, json!({}), Digest32::ZERO, MAX_WORK)?;
    assert_eq!(result["elapsed_game_ticks"], 0);
    for key in [
        "all_clear_proven",
        "health_change_proven",
        "raw_stress_change_proven",
        "continuous_trend_proven",
        "cause_proven",
        "current_freshness_proven",
        "mutation_dispatched",
    ] {
        assert_eq!(result[key], false);
    }
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["coverage"]["soul_presence"], "not_observed");
    assert!(
        result["interpretation"]
            .as_str()
            .is_some_and(|s| s.contains("no-soul fallback"))
    );
    Ok(())
}

#[test]
fn authority_epoch_source_and_budget_refusals_precede_results() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    for maximum in [0, 1, MAX_WORK + 1] {
        assert_eq!(
            compare(&before, &after, &c, None, true, maximum)
                .err()
                .map(|e| e.code),
            Some(ErrorCode::BudgetExceeded)
        );
    }
    for variant in 0..7 {
        let mut denied = c.clone();
        match variant {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].expires_at_tick = Some(snapshot(&before)?.anchor().tick),
            3 => denied.grants[0].remaining_uses = Some(0),
            4 => denied.anchor.fortress_id = FortressId::NIL,
            5 => denied.budget.max_wall_millis = 0,
            _ => denied.budget.max_entities = 1,
        }
        assert!(
            compare(&before, &after, &denied, None, true, MAX_WORK).is_err(),
            "variant {variant}"
        );
    }
    let (_, reset) = states(
        &first,
        &Input {
            tick: 4,
            bridge: 8,
            ..first.clone()
        },
    )?;
    assert_eq!(
        compare(&before, &reset, &context(&reset)?, None, true, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::StaleAnchor)
    );
    assert_eq!(
        compare(&after, &before, &c, None, true, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::StaleAnchor)
    );
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let mut tiny = c.clone();
    tiny.budget.max_bytes = 1;
    assert_eq!(
        page(&report, &tiny, 1, None, json!({}), Digest32::ZERO, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    for value in ["0", "01", "-1", "18446744073709551616", "abc"] {
        assert!(
            Focus {
                entity_id: value.into(),
                generation: 0
            }
            .validate()
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn complete_rows_and_bound_continuations_preserve_all_selected_citizens() -> Result<()> {
    let first = Input {
        categories: (0..12).map(|n| n % 7).collect(),
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let selection = json!({"citizen":null,"include_unchanged":true});
    let binding = Digest32::of_bytes(b"journal and exact record witnesses");
    let first_page = page(&report, &c, 1, None, selection.clone(), binding, MAX_WORK)?;
    let cursor = first_page["continuation"]
        .as_str()
        .ok_or_else(|| invariant("fixture continuation absent"))?;
    let next = page(
        &report,
        &c,
        2,
        Some(cursor),
        selection.clone(),
        binding,
        MAX_WORK,
    )?;
    assert_eq!(next["offset"], 1);
    assert_eq!(next["returned"], 2);
    assert_eq!(
        next,
        page(
            &report,
            &c,
            2,
            Some(cursor),
            selection.clone(),
            binding,
            MAX_WORK
        )?
    );
    for variant in 0..5 {
        let mut changed = c.clone();
        let mut choice = selection.clone();
        let mut archive = binding;
        let mut work = MAX_WORK;
        match variant {
            0 => changed.session_id = SessionId::new(99),
            1 => changed.anchor.cursor.sequence += 1,
            2 => choice["include_unchanged"] = json!(false),
            3 => archive = Digest32::of_bytes(b"another head"),
            _ => work -= 1,
        }
        assert_eq!(
            page(&report, &changed, 2, Some(cursor), choice, archive, work)
                .err()
                .map(|e| e.code),
            Some(ErrorCode::StaleAnchor)
        );
    }
    for malformed in ["hcw1:0:00", "hcw1:01:00", "wrong:1:00", "hcw1:2:00"] {
        assert!(
            page(
                &report,
                &c,
                2,
                Some(malformed),
                selection.clone(),
                binding,
                MAX_WORK
            )
            .is_err()
        );
    }
    let mut bounded = c.clone();
    bounded.budget.max_bytes = first_page.to_string().len() as u64 + 64;
    let limited = page(
        &report,
        &bounded,
        128,
        None,
        selection.clone(),
        binding,
        MAX_WORK,
    )?;
    assert_eq!(limited["returned"], 1);
    assert_eq!(limited["summary"]["before"]["strict_citizens"], 12);
    assert!(limited.to_string().len() as u64 <= bounded.budget.max_bytes);
    let mut all = BTreeSet::new();
    let mut continuation = None::<String>;
    loop {
        let result = page(
            &report,
            &c,
            2,
            continuation.as_deref(),
            selection.clone(),
            binding,
            MAX_WORK,
        )?;
        for item in result["rows"]
            .as_array()
            .ok_or_else(|| invariant("fixture rows absent"))?
        {
            assert!(all.insert(item["entity_id"].to_string()));
            assert_eq!(item["inspect_before"]["query"]["record"], 1);
            assert_eq!(item["inspect_after"]["query"]["record"], 2);
        }
        assert!(all.len() <= 12);
        continuation = result["continuation"].as_str().map(str::to_owned);
        if continuation.is_none() {
            break;
        }
    }
    assert_eq!(all.len(), 12);
    Ok(())
}
