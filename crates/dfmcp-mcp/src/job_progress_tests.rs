//! Actual native decoding and generation publication, plus adversarial fact
//! provenance. These tests qualify the comparison policy, not a live DF build.
use super::*;
use dfmcp_adapter::live_jobs::LiveJob;
use dfmcp_adapter::live_operations::{
    JobItemAttachment, LiveBuilding, LiveItem, OperationsProfile,
};
use dfmcp_adapter::live_spatial::{
    LiveSpatialObservation, citizens::LiveSpatialCitizenObservation,
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, MapCoord, RequestId, SessionId, WorkBudget,
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
    jobs: u32,
    suspended: bool,
    worker: Option<u32>,
    stage: i32,
    item: u32,
    timer: i32,
    bridge: u64,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            tick: 3,
            jobs: 1,
            suspended: true,
            worker: None,
            stage: 0,
            item: 1,
            timer: 10,
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
    let original =
        LiveSpatialObservation::decode_payload(&bytes, input.bridge, "df".into(), "dfhack".into())?;
    let mut operations = original.operations().clone();
    let mut terrain = original.terrain().clone();
    operations.jobs.year_tick = input.tick;
    terrain.year_tick = input.tick;
    operations.jobs.next_job_id = 100;
    operations.next_building_id = 10;
    operations.next_item_id = 10;
    operations.jobs.jobs = (1..=input.jobs)
        .map(|id| LiveJob {
            native_id: id,
            job_type: 0,
            type_key: "ConstructBuilding".into(),
            reaction: String::new(),
            suspended: input.suspended,
            repeating: false,
            position: MapCoord::new(1, 0, 5),
            worker_native_id: input.worker,
            holder_native_id: Some(1),
            completion_timer: input.timer,
            attached_item_count: 1,
            required_item_filter_count: 1,
        })
        .collect();
    operations.buildings = vec![LiveBuilding {
        native_id: 1,
        building_type: 0,
        type_key: "Bed".into(),
        x1: 1,
        y1: 0,
        x2: 1,
        y2: 0,
        z: 5,
        build_stage: input.stage,
        max_build_stage: 3,
    }];
    operations.items = (1..=2)
        .map(|id| LiveItem {
            native_id: id,
            item_type: 0,
            type_key: "WOOD".into(),
            subtype: -1,
            material_type: 0,
            material_index: 0,
            stack_size: 1,
            raw_position: MapCoord::new(1, 0, 5),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        })
        .collect();
    operations.attachments = (1..=input.jobs)
        .map(|id| JobItemAttachment {
            job_native_id: id,
            item_native_id: input.item,
            role: 0,
            filter_index: 0,
        })
        .collect();
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&1u32.to_be_bytes());
    citizens.extend_from_slice(&10u32.to_be_bytes());
    text(&mut citizens, "Urist");
    text(&mut citizens, "DWARF");
    for n in [0i32, 1, 0, 5] {
        citizens.extend_from_slice(&n.to_be_bytes());
    }
    citizens.extend_from_slice(&0x11fu16.to_be_bytes());
    citizens.extend_from_slice(&6i32.to_be_bytes());
    citizens.extend_from_slice(&[1, 1]);
    citizens.extend_from_slice(&0u16.to_be_bytes());
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
fn rebuild(original: &WorldSnapshot, graph: WorldGraph) -> WorldSnapshot {
    let a = original.anchor();
    WorldSnapshot::new(a.fortress_id, a.tick, a.cursor, original.paused, graph)
}
fn edited(
    state: &LiveSpatialCitizenState,
    edit: impl FnOnce(&mut WorldGraph) -> Result<()>,
) -> Result<WorldSnapshot> {
    let source = snapshot(state)?;
    let mut graph = source.graph.clone();
    edit(&mut graph)?;
    Ok(rebuild(source, graph))
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
                    .filter(|e| e.kind == EntityKind::Job)
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

#[test]
fn native_endpoints_report_qualified_job_worker_holder_and_material_transitions() -> Result<()> {
    let first = Input::default();
    let last = Input {
        tick: 9,
        suspended: false,
        worker: Some(10),
        stage: 2,
        item: 2,
        timer: 8,
        ..first.clone()
    };
    let (before, after) = states(&first, &last)?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let value = row(&report)?;
    assert_eq!(value["identity_status"], "same_generation");
    assert_eq!(value["before"]["worker_assignment"]["status"], "unassigned");
    assert_eq!(
        value["after"]["worker_assignment"]["status"],
        "assigned_strict_citizen"
    );
    assert_eq!(
        value["after"]["worker_assignment"]["worker"]["entity_id"],
        citizen_entity_id(10).to_string()
    );
    assert_eq!(value["indicators"]["suspension"], "changed");
    assert_eq!(value["indicators"]["worker_assignment"], "changed");
    assert_eq!(value["indicators"]["holder_build_stage"], "advanced");
    assert_eq!(value["indicators"]["attachment_membership"], "changed");
    assert_eq!(value["indicators"]["completion_timer_raw"], "changed");
    assert_eq!(report.summary["same_holder_generation_stage_advanced"], 1);
    let result = page(
        &report,
        &c,
        4,
        None,
        json!({"include_unchanged":true}),
        Digest32::of_bytes(b"archive"),
        MAX_WORK,
    )?;
    assert_eq!(result["elapsed_game_ticks"], 6);
    for key in [
        "completion_proven",
        "stall_proven",
        "blocker_proven",
        "current_freshness_proven",
        "mutation_dispatched",
    ] {
        assert_eq!(result[key], false);
    }
    assert_eq!(value["inspect_before"]["query"]["record"], 1);
    assert_eq!(value["inspect_after"]["query"]["record"], 2);
    assert_eq!(
        value["inspect_after"]["query"]["query"]["generation"],
        value["after"]["handle"]["generation"]
    );
    assert!(
        value["inspect_after"]["query"]["query"]
            .get("relationships")
            .is_none()
    );
    Ok(())
}

#[test]
fn unchanged_endpoints_remain_indicators_and_filter_keeps_whole_roster_summary() -> Result<()> {
    let first = Input {
        jobs: 3,
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 20,
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let value = row(&report)?;
    assert_eq!(value["all_compared_indicators_unchanged"], true);
    assert_eq!(value["observed_indicator_change"], false);
    assert_eq!(value["suspended_at_both_endpoints"], true);
    assert_eq!(value["unassigned_at_both_endpoints"], true);
    assert_ne!(
        value["before"]["handle"]["revision"],
        value["after"]["handle"]["revision"]
    );
    let filtered = compare(&before, &after, &c, None, false, MAX_WORK)?;
    assert_eq!(filtered.len(), 0);
    assert_eq!(filtered.summary["jobs_before"], 3);
    assert_eq!(
        filtered.summary["same_generation_with_all_indicators_unchanged"],
        3
    );
    assert_eq!(
        filtered.summary["same_generation_suspended_at_both_endpoints"],
        3
    );
    let focused = compare(
        &before,
        &after,
        &c,
        Some(&Focus {
            entity_id: "4".into(),
            generation: report.before[&EntityId::new(4)].identity.generation,
        }),
        true,
        MAX_WORK,
    )?;
    assert_eq!(focused.len(), 1);
    assert_eq!(focused.summary["jobs_after"], 3);
    Ok(())
}

#[test]
fn disappearance_and_reused_id_never_complete_or_substitute_the_original_job() -> Result<()> {
    let first = Input::default();
    let (before, mut absent) = states(
        &first,
        &Input {
            tick: 4,
            jobs: 0,
            ..first.clone()
        },
    )?;
    let original_generation = snapshot(&before)?.graph.entities[&EntityId::new(3)].generation;
    let focus = Focus {
        entity_id: "3".into(),
        generation: original_generation,
    };
    let report = compare(
        &before,
        &absent,
        &context(&absent)?,
        Some(&focus),
        true,
        MAX_WORK,
    )?;
    let value = row(&report)?;
    assert_eq!(value["identity_status"], "no_longer_observed");
    assert!(value["after"].is_null());
    assert!(value["indicators"].is_null());
    absent.publish(observation(&Input {
        tick: 5,
        stage: 3,
        ..first.clone()
    })?)?;
    let reused = compare(
        &before,
        &absent,
        &context(&absent)?,
        Some(&focus),
        true,
        MAX_WORK,
    )?;
    let value = row(&reused)?;
    assert_eq!(value["identity_status"], "generation_replaced");
    assert_ne!(
        value["before"]["handle"]["generation"],
        value["after"]["handle"]["generation"]
    );
    assert!(value["indicators"].is_null());
    assert_eq!(value["observed_indicator_change"], false);
    let bad = Focus {
        generation: original_generation.saturating_add(1),
        ..focus
    };
    assert_eq!(
        compare(
            &before,
            &absent,
            &context(&absent)?,
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
fn incomplete_roster_cannot_establish_new_or_disappeared_jobs() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            jobs: 0,
            ..first
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
    let reversed = assess(
        evidence(&after, snapshot(&after)?, false)?,
        Endpoint::state(&before)?,
        &c,
        None,
        true,
        MAX_WORK,
    );
    assert_eq!(reversed.err().map(|e| e.code), Some(ErrorCode::StaleAnchor));
    let empty = rebuild(snapshot(&before)?, {
        let mut graph = snapshot(&before)?.graph.clone();
        graph.entities.retain(|_, e| e.kind != EntityKind::Job);
        graph.edges.clear();
        graph
    });
    let newly = assess(
        evidence(&before, &empty, false)?,
        Endpoint::state(&after_with_jobs(&before)?)?,
        &c,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(
        row(&newly)?["identity_status"],
        "roster_absence_unestablished"
    );
    let (empty_before, populated_after) = states(
        &Input {
            jobs: 0,
            ..Input::default()
        },
        &Input {
            tick: 4,
            ..Input::default()
        },
    )?;
    let qualified_new = compare(
        &empty_before,
        &populated_after,
        &context(&populated_after)?,
        None,
        true,
        MAX_WORK,
    )?;
    assert_eq!(row(&qualified_new)?["identity_status"], "newly_observed");
    let mut wrong = evidence(&before, snapshot(&before)?, true)?;
    wrong.roster = Some(BTreeSet::new());
    assert_eq!(
        assess(wrong, Endpoint::state(&after)?, &c, None, true, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}
fn after_with_jobs(before: &LiveSpatialCitizenState) -> Result<LiveSpatialCitizenState> {
    let mut state = before.clone();
    state.publish(observation(&Input {
        tick: 4,
        ..Input::default()
    })?)?;
    Ok(state)
}

#[test]
fn assignment_join_rejects_contradictions_and_outside_roster_identity_claims() -> Result<()> {
    let first = Input {
        worker: Some(10),
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    for variation in 0..5 {
        let modified = edited(&after, |graph| {
            if variation == 0 {
                graph.edges.retain(|_, e| e.kind != EdgeKind::Performs);
                return Ok(());
            }
            let job = graph
                .entities
                .get_mut(&EntityId::new(3))
                .ok_or_else(|| invariant("fixture job absent"))?;
            let (key, value) = match variation {
                1 => ("worker_assigned", W::Bool(false)),
                2 => ("worker_native_id", W::U64(11)),
                3 => ("worker_is_strict_citizen", W::Bool(false)),
                _ => ("worker_entity", W::Entity(citizen_entity_id(11))),
            };
            let f = job
                .fields
                .get_mut(key)
                .ok_or_else(|| invariant("fixture worker field absent"))?;
            f.value = value.clone();
            f.presence = Some(FactPresence::Known(value));
            Ok(())
        })?;
        assert_eq!(
            row(&compare_edited(&before, &after, &modified)?)?["after"]["worker_assignment"]["status"],
            "unestablished"
        );
    }
    let worker_replaced = edited(&after, |graph| {
        graph
            .entities
            .get_mut(&citizen_entity_id(10))
            .ok_or_else(|| invariant("fixture citizen absent"))?
            .generation += 1;
        Ok(())
    })?;
    assert_eq!(
        row(&compare_edited(&before, &after, &worker_replaced)?)?["indicators"]["worker_assignment"],
        "changed"
    );
    let external = Input {
        worker: Some(77),
        ..Input::default()
    };
    let (before, after) = states(
        &external,
        &Input {
            tick: 4,
            ..external
        },
    )?;
    let value = row(&compare(
        &before,
        &after,
        &context(&after)?,
        None,
        true,
        MAX_WORK,
    )?)?;
    assert_eq!(
        value["after"]["worker_assignment"]["status"],
        "assigned_outside_strict_roster"
    );
    assert_eq!(value["indicators"]["worker_assignment"], "unestablished");
    assert_eq!(value["all_compared_indicators_unchanged"], false);
    Ok(())
}

#[test]
fn unknown_absent_wrong_source_digest_tick_and_presence_never_become_clear_indicators() -> Result<()>
{
    let first = Input::default();
    let (before, after) = states(&first, &Input { tick: 4, ..first })?;
    for variation in 0..8 {
        let modified = edited(&after, |graph| {
            let f = graph
                .entities
                .get_mut(&EntityId::new(3))
                .ok_or_else(|| invariant("fixture job absent"))?
                .fields
                .get_mut("suspended")
                .ok_or_else(|| invariant("fixture suspension absent"))?;
            match variation {
                0 => f.source = FactSource::Derived("spatial/1.8.job.flags.suspend".into()),
                1 => f.source = FactSource::DfhackField("spatial/1.6.job.flags.suspend".into()),
                2 => f.source_digest = Digest32::of_bytes(b"other capture"),
                3 => f.observed_at = GameTick(0),
                4 => {
                    f.value = W::Null;
                    f.presence = Some(FactPresence::Unknown("hidden".into()));
                }
                5 => {
                    f.value = W::Null;
                    f.presence = Some(FactPresence::Absent);
                }
                6 => f.presence = Some(FactPresence::Known(W::Bool(false))),
                _ => f.presence = Some(FactPresence::Stale(snapshot(&before)?.anchor())),
            }
            Ok(())
        })?;
        let value = row(&compare_edited(&before, &after, &modified)?)?;
        assert!(value["after"]["suspended"].is_null(), "variant {variation}");
        assert_eq!(value["indicators"]["suspension"], "unestablished");
        assert_eq!(value["all_compared_indicators_unchanged"], false);
    }
    Ok(())
}

#[test]
fn construction_stages_compare_only_the_same_qualified_holder_generation() -> Result<()> {
    let first = Input {
        stage: 2,
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            stage: 1,
            ..first
        },
    )?;
    assert_eq!(
        row(&compare(
            &before,
            &after,
            &context(&after)?,
            None,
            true,
            MAX_WORK
        )?)?["indicators"]["holder_build_stage"],
        "regressed"
    );
    for variation in 0..5 {
        let modified = edited(&after, |graph| {
            if variation == 0 {
                graph.edges.retain(|_, e| {
                    !(e.kind == EdgeKind::ContainedIn && e.from == EntityId::new(3))
                });
                return Ok(());
            }
            let holder = graph
                .entities
                .get_mut(&building_entity_id(1))
                .ok_or_else(|| invariant("fixture holder absent"))?;
            match variation {
                1 => holder.generation += 1,
                2 => {
                    holder
                        .fields
                        .get_mut("build_stage")
                        .ok_or_else(|| invariant("fixture stage absent"))?
                        .source_digest = Digest32::of_bytes(b"wrong capture")
                }
                3 => {
                    let f = holder
                        .fields
                        .get_mut("max_build_stage")
                        .ok_or_else(|| invariant("fixture maximum absent"))?;
                    f.value = W::I64(4);
                    f.presence = Some(FactPresence::Known(W::I64(4)));
                }
                _ => {
                    let f = holder
                        .fields
                        .get_mut("native_building_id")
                        .ok_or_else(|| invariant("fixture native ID absent"))?;
                    f.value = W::U64(2);
                    f.presence = Some(FactPresence::Known(W::U64(2)));
                }
            }
            Ok(())
        })?;
        let value = row(&compare_edited(&before, &after, &modified)?)?;
        assert_eq!(
            value["indicators"]["holder_build_stage"],
            match variation {
                1 => "holder_changed_not_comparable",
                3 => "maximum_changed_not_comparable",
                _ => "unestablished",
            }
        );
    }
    Ok(())
}

#[test]
fn item_membership_retains_generation_and_rejects_incomplete_attachment_joins() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(&first, &Input { tick: 4, ..first })?;
    for variation in 0..4 {
        let modified = edited(&after, |graph| {
            match variation {
                0 => {
                    graph
                        .entities
                        .get_mut(&item_entity_id(1))
                        .ok_or_else(|| invariant("fixture item absent"))?
                        .generation += 1
                }
                1 => {
                    let f = graph
                        .entities
                        .get_mut(&EntityId::new(3))
                        .ok_or_else(|| invariant("fixture job absent"))?
                        .fields
                        .get_mut("attached_item_count")
                        .ok_or_else(|| invariant("fixture count absent"))?;
                    f.value = W::U64(2);
                    f.presence = Some(FactPresence::Known(W::U64(2)));
                }
                2 => {
                    for e in graph
                        .edges
                        .values_mut()
                        .filter(|e| e.kind == EdgeKind::Uses)
                    {
                        e.fields
                            .get_mut("role")
                            .ok_or_else(|| invariant("fixture role absent"))?
                            .source = FactSource::Replay;
                    }
                }
                _ => {
                    for e in graph
                        .edges
                        .values_mut()
                        .filter(|e| e.kind == EdgeKind::Uses)
                    {
                        let f = e
                            .fields
                            .get_mut("filter_index")
                            .ok_or_else(|| invariant("fixture filter absent"))?;
                        f.value = W::I64(5);
                        f.presence = Some(FactPresence::Known(W::I64(5)));
                    }
                }
            }
            Ok(())
        })?;
        let value = row(&compare_edited(&before, &after, &modified)?)?;
        assert_eq!(
            value["indicators"]["attachment_membership"],
            if variation == 0 {
                "changed"
            } else {
                "unestablished"
            }
        );
        assert_eq!(value["after"]["attachments"]["readiness_proven"], false);
    }
    Ok(())
}

#[test]
fn authority_generation_source_and_work_bounds_are_checked_before_results() -> Result<()> {
    let first = Input::default();
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            ..first.clone()
        },
    )?;
    let c = context(&after)?;
    assert_eq!(
        compare(&before, &after, &c, None, true, 1)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        compare(&before, &after, &c, None, true, MAX_WORK + 1)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    for variation in 0..3 {
        let mut denied = c.clone();
        match variation {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            _ => denied.grants[0].expires_at_tick = Some(snapshot(&before)?.anchor().tick),
        };
        assert!(compare(&before, &after, &denied, None, true, MAX_WORK).is_err());
    }
    let (_, reset) = states(
        &first,
        &Input {
            tick: 4,
            bridge: 8,
            ..first
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
    assert_eq!(
        compare(
            &before,
            &after,
            &c,
            Some(&Focus {
                entity_id: "03".into(),
                generation: 0
            }),
            true,
            MAX_WORK
        )
        .err()
        .map(|e| e.code),
        Some(ErrorCode::InvalidRequest)
    );
    Ok(())
}

#[test]
fn complete_row_pagination_is_deterministic_and_bound_to_authority_archive_and_selection()
-> Result<()> {
    let first = Input {
        jobs: 8,
        ..Input::default()
    };
    let (before, after) = states(
        &first,
        &Input {
            tick: 4,
            suspended: false,
            ..first
        },
    )?;
    let c = context(&after)?;
    let report = compare(&before, &after, &c, None, true, MAX_WORK)?;
    let binding = Digest32::of_bytes(b"journal head");
    let selection = json!({"job":null,"include_unchanged":true});
    let initial = page(&report, &c, 1, None, selection.clone(), binding, MAX_WORK)?;
    let cursor = initial["continuation"]
        .as_str()
        .ok_or_else(|| invalid("fixture cursor absent"))?;
    let mut next = c.clone();
    next.budget.max_bytes = initial.to_string().len() as u64 + 50;
    let mut request_cursor = None::<String>;
    let mut seen = BTreeSet::new();
    loop {
        let value = page(
            &report,
            &next,
            128,
            request_cursor.as_deref(),
            selection.clone(),
            binding,
            MAX_WORK,
        )?;
        assert!(value.to_string().len() as u64 <= next.budget.max_bytes);
        for row in value["rows"]
            .as_array()
            .ok_or_else(|| invalid("fixture rows absent"))?
        {
            assert!(seen.insert(row["entity_id"].as_str().unwrap_or_default().to_owned()));
        }
        if value["continuation"].is_null() {
            break;
        }
        request_cursor = value["continuation"].as_str().map(str::to_owned);
        assert!(seen.len() < 8);
    }
    assert_eq!(seen.len(), 8);
    for variation in 0..6 {
        let mut altered = c.clone();
        let mut selected = selection.clone();
        let mut archive = binding;
        let mut maximum = MAX_WORK;
        match variation {
            0 => altered.session_id = SessionId::new(82),
            1 => altered.anchor.cursor.sequence += 1,
            2 => selected["include_unchanged"] = json!(false),
            3 => archive = Digest32::of_bytes(b"new head"),
            4 => maximum -= 1,
            _ => altered.anchor.state_hash = Digest32::of_bytes(b"new state"),
        };
        assert_eq!(
            page(
                &report,
                &altered,
                2,
                Some(cursor),
                selected,
                archive,
                maximum
            )
            .err()
            .map(|e| e.code),
            Some(ErrorCode::StaleAnchor)
        );
    }
    let mut too_small = c;
    too_small.budget.max_bytes = 32;
    assert_eq!(
        page(&report, &too_small, 1, None, selection, binding, MAX_WORK)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    Ok(())
}
