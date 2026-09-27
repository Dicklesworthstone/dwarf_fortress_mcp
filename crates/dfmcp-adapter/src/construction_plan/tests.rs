use super::fixtures::{TICK, context, goal, hex, observation, sample, with_observation};
use super::*;
use crate::build_placement::{BuildPhase, BuildPlan};
use crate::live_jobs::LiveJob;
use crate::live_operations::{JobItemAttachment, OperationsProfile};
use dfmcp_core::{CapabilityScope, EntityId, FortressId, MapCoord};

fn step(state: &Progress, goal: &Goal, sample: &LinkedSample) -> Progress {
    state
        .begin_read()
        .unwrap()
        .advance(goal, sample, &context(goal))
        .unwrap()
}
fn job(id: u32, holder: u32, kind: &str) -> LiveJob {
    LiveJob {
        native_id: id,
        job_type: match kind {
            "ConstructBuilding" => 1,
            "DestroyBuilding" => 2,
            _ => 3,
        },
        type_key: kind.to_owned(),
        reaction: String::new(),
        suspended: false,
        repeating: false,
        position: MapCoord::new(15, 15, 2),
        worker_native_id: None,
        holder_native_id: Some(holder),
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 0,
    }
}
fn field(out: &mut Vec<u8>, raw: &[u8]) {
    out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
    out.extend_from_slice(raw);
}
fn rekey(record: &BuildRecord, key: &str, phase: BuildPhase) -> BuildRecord {
    let plan = BuildPlan::new(key, record.plan().before().clone()).unwrap();
    let mut raw = b"DFMBR019".to_vec();
    field(&mut raw, key.as_bytes());
    field(&mut raw, plan.before().canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.extend_from_slice(if phase == BuildPhase::Placed {
        &[2, 0, 1, 1]
    } else {
        &[0, 0, 0, 0]
    });
    if phase == BuildPhase::Placed {
        field(&mut raw, record.after().unwrap().canonical_bytes());
        field(&mut raw, record.insertion().unwrap().canonical_bytes());
    }
    let mut hashed = b"dfmcp-build-receipt/1\0".to_vec();
    hashed.extend_from_slice(&raw);
    raw.extend_from_slice(Digest32::of_bytes(&hashed).as_bytes());
    BuildRecord::decode(&raw).unwrap()
}

#[test]
fn exact_python_goal_and_sample_bytes_share_one_complete_32_target_capture() {
    let whole = goal(32);
    assert_eq!(
        whole.canonical_bytes(),
        hex(include_str!("testdata/python_goal_32.hex"))
    );
    for (n, expected) in [
        (
            1,
            "4ebf330e303914097b504e952c09ded283766d2915856729ccef95d8c2e5f2c7",
        ),
        (
            3,
            "bd795b0d532a7b36c0005928fd44ea89019f4da3b68da4a6d37da4c3943f4906",
        ),
        (
            32,
            "5d10cbd748000e7ec217f3ab98b01211a250b512aa237a14acc7e058c633551b",
        ),
    ] {
        assert_eq!(goal(n).digest().as_bytes().as_slice(), hex(expected));
    }
    let bytes = hex(include_str!("testdata/python_sample_32.hex"));
    let original = LinkedSample::decode(&bytes).unwrap();
    assert_eq!(original.canonical_bytes().unwrap(), bytes);
    assert_eq!(sample(&whole, TICK + 1), original);
    let mut records = whole.records().to_vec();
    records.reverse();
    assert_eq!(Goal::new(records, whole.timing()).unwrap(), whole);
    let progress = step(&Progress::new(&whole), &whole, &original);
    assert_eq!(
        (
            progress.phase,
            progress.streak,
            progress.condition_met_count()
        ),
        ("candidate", 1, 32)
    );
    assert_eq!(
        progress
            .assessments
            .iter()
            .map(|r| r.building_id)
            .collect::<Vec<_>>(),
        (70..102).collect::<Vec<_>>()
    );
    assert_eq!(
        step(&progress, &whole, &sample(&whole, TICK + 2)).phase,
        "satisfied"
    );
}

#[test]
fn codecs_refuse_truncation_wrong_generations_unknown_bytes_and_noncanonical_order() {
    let g = goal(1);
    let s = sample(&g, TICK + 1);
    for n in 0..g.canonical_bytes().len() {
        assert!(Goal::decode(&g.canonical_bytes()[..n]).is_err());
    }
    let bytes = s.canonical_bytes().unwrap();
    for n in 0..bytes.len() {
        assert!(LinkedSample::decode(&bytes[..n]).is_err());
    }
    let mut raw = g.canonical_bytes().to_vec();
    raw.push(0);
    assert!(Goal::decode(&raw).is_err());
    let mut raw = bytes.clone();
    raw.push(0);
    assert!(LinkedSample::decode(&raw).is_err());
    let mut raw = bytes;
    raw[7] = b'2';
    assert!(LinkedSample::decode(&raw).is_err());
    assert!(Goal::decode(&vec![0; MAX_GOAL + 1]).is_err());
    let g = goal(3);
    let mut reversed = b"DFMCPG01".to_vec();
    reversed.push(3);
    for record in g.records().iter().rev() {
        field(&mut reversed, record.canonical_bytes());
    }
    reversed.extend_from_slice(&g.canonical_bytes()[g.canonical_bytes().len() - 32..]);
    assert!(Goal::decode(&reversed).is_err());
}

#[test]
fn goal_rejects_empty_oversize_duplicate_unplaced_and_infeasible_timing() {
    let g = goal(3);
    let records = g.records().to_vec();
    let timing = g.timing();
    assert!(Goal::new(Vec::new(), timing).is_err());
    assert!(Goal::new(vec![records[0].clone(); 33], timing).is_err());
    assert!(Goal::new(vec![records[0].clone(), records[0].clone()], timing).is_err());
    let duplicate_key = rekey(&records[1], records[0].plan().key(), BuildPhase::Placed);
    assert!(Goal::new(vec![records[0].clone(), duplicate_key], timing).is_err());
    let prepared = rekey(&records[0], "prepared-only", BuildPhase::Prepared);
    assert!(Goal::new(vec![prepared], timing).is_err());
    for timing in [
        Timing {
            deadline: TICK + 1,
            ..timing
        },
        Timing {
            interval: 0,
            ..timing
        },
        Timing {
            interval: 403_201,
            ..timing
        },
        Timing {
            stable_samples: 1,
            ..timing
        },
        Timing {
            stable_samples: 65,
            ..timing
        },
        Timing {
            stable_span: 0,
            ..timing
        },
        Timing {
            stable_span: 4_032_001,
            ..timing
        },
        Timing {
            max_gap: 0,
            ..timing
        },
        Timing {
            max_observations: 1,
            ..timing
        },
        Timing {
            max_observations: 513,
            ..timing
        },
        Timing {
            interval: 50,
            stable_samples: 3,
            ..timing
        },
    ] {
        assert!(Goal::new(records.clone(), timing).is_err(), "{timing:?}");
    }
}

#[test]
fn every_original_bracket_record_and_source_is_mandatory() {
    let g = goal(3);
    let good = sample(&g, TICK + 1);
    let mut variants = Vec::new();
    let mut s = good.clone();
    s.before_records.pop();
    variants.push(s);
    let mut s = good.clone();
    s.after_records.pop();
    variants.push(s);
    let mut s = good.clone();
    s.before_records.reverse();
    variants.push(s);
    let mut s = good.clone();
    s.after_records[2] = s.after_records[0].clone();
    variants.push(s);
    let mut s = good.clone();
    s.after.generation += 1;
    variants.push(s);
    let mut s = good.clone();
    s.operations.df_version.push('x');
    variants.push(s);
    let mut s = good.clone();
    s.operations.generation = u64::MAX;
    variants.push(s);
    for s in variants {
        assert!(s.validate(&g, &context(&g)).is_err());
    }
    assert_ne!(good.operations.generation, good.before.generation);
    assert!(good.validate(&g, &context(&g)).is_ok());
}

#[test]
fn independently_timed_member_successes_never_complete_the_original_selection() {
    let g = goal(3);
    let mut p = Progress::new(&g);
    for offset in 1..=6 {
        let mut observed = observation(&g, TICK + offset);
        observed.items[(offset as usize - 1) % 3].flags |= 2;
        p = step(&p, &g, &with_observation(&g, observed));
        assert_eq!(
            (p.phase, p.streak, p.condition_met_count()),
            ("active", 0, 2)
        );
    }
    p = step(&p, &g, &sample(&g, TICK + 7));
    assert_eq!(p.streak, 1);
    let mut observed = observation(&g, TICK + 8);
    observed.items[2].flags |= 2;
    p = step(&p, &g, &with_observation(&g, observed));
    assert_eq!(p.streak, 0);
    p = step(&p, &g, &sample(&g, TICK + 9));
    assert_eq!(step(&p, &g, &sample(&g, TICK + 10)).phase, "satisfied");
}

#[test]
fn late_removal_or_identity_failure_cannot_be_hidden_by_earlier_pending_members() {
    let g = goal(32);
    let mut observed = observation(&g, TICK + 1);
    observed.buildings[0].build_stage = 0;
    observed.jobs.jobs.push(job(121, 101, "DestroyBuilding"));
    observed.jobs.next_job_id = 122;
    let p = step(
        &Progress::new(&g),
        &g,
        &with_observation(&g, observed.clone()),
    );
    assert_eq!(
        (p.phase, p.reason, p.reason_building, p.assessments.len()),
        ("failed", "removal_pending", Some(101), 32)
    );
    observed.items[31].material_type += 1;
    let p = step(&Progress::new(&g), &g, &with_observation(&g, observed));
    assert_eq!(
        (p.phase, p.reason, p.reason_building, p.assessments.len()),
        ("invalidated", "item_identity_mismatch", Some(101), 32)
    );
}

#[test]
fn all_three_exact_furniture_kinds_require_original_singleton_material_and_flags() {
    let all = goal(3);
    for record in all.records() {
        let g = Goal::new(vec![record.clone()], all.timing()).unwrap();
        for flags in 0..512 {
            let mut observed = observation(&g, TICK + 1);
            observed.items[0].flags = flags;
            let p = step(&Progress::new(&g), &g, &with_observation(&g, observed));
            let expected =
                flags / 256 % 2 == 1 && [1, 3, 6, 7].iter().all(|bit| flags / (1 << bit) % 2 == 0);
            assert_eq!(
                p.assessments[0].condition.status == "condition_met",
                expected
            );
        }
        for stack in [0, 2, i32::MAX as u32] {
            let mut observed = observation(&g, TICK + 1);
            observed.items[0].stack_size = stack;
            assert_eq!(
                step(&Progress::new(&g), &g, &with_observation(&g, observed)).reason,
                "item_unverified"
            );
        }
        for field in 0..4 {
            let mut observed = observation(&g, TICK + 1);
            match field {
                0 => observed.items[0].item_type += 1,
                1 => observed.items[0].subtype += 1,
                2 => observed.items[0].material_type += 1,
                _ => observed.items[0].material_index += 1,
            }
            assert_eq!(
                step(&Progress::new(&g), &g, &with_observation(&g, observed)).reason,
                "item_identity_mismatch"
            );
        }
    }
}

#[test]
fn missing_jobs_stage_only_and_unrelated_item_links_never_prove_completion() {
    let g = goal(1);
    let p = &g.records()[0];
    let insertion = p.insertion().unwrap();
    let mut observed = observation(&g, TICK + 1);
    observed.buildings[0].build_stage = 0;
    assert_eq!(
        step(
            &Progress::new(&g),
            &g,
            &with_observation(&g, observed.clone())
        )
        .reason,
        "no_construction_job"
    );
    observed.jobs.jobs.push(job(
        insertion.job_id(),
        insertion.building_id(),
        "ConstructBuilding",
    ));
    assert_eq!(
        step(
            &Progress::new(&g),
            &g,
            &with_observation(&g, observed.clone())
        )
        .reason,
        "pending"
    );
    observed.jobs.jobs[0].suspended = true;
    assert_eq!(
        step(
            &Progress::new(&g),
            &g,
            &with_observation(&g, observed.clone())
        )
        .reason,
        "suspended"
    );
    observed.buildings[0].build_stage = 1;
    assert_eq!(
        step(&Progress::new(&g), &g, &with_observation(&g, observed)).reason,
        "suspended"
    );
    let mut observed = observation(&g, TICK + 1);
    let mut unrelated = job(89, insertion.building_id(), "StoreItemInStockpile");
    unrelated.holder_native_id = None;
    unrelated.attached_item_count = 2;
    observed.jobs.jobs.push(unrelated);
    observed.attachments = vec![
        JobItemAttachment {
            job_native_id: 89,
            item_native_id: insertion.item_id(),
            role: 0,
            filter_index: -1,
        },
        JobItemAttachment {
            job_native_id: 89,
            item_native_id: insertion.item_id(),
            role: 1,
            filter_index: -1,
        },
    ];
    let p = step(&Progress::new(&g), &g, &with_observation(&g, observed));
    assert_eq!(
        (p.reason, p.assessments[0].condition.item_job_links),
        ("item_unverified", 1)
    );
}

#[test]
fn all_original_identity_and_native_history_regressions_invalidate() {
    let g = goal(1);
    let mut cases = Vec::new();
    let mut o = observation(&g, TICK + 1);
    o.jobs.world_folder.push('x');
    cases.push((o, "world_identity_mismatch"));
    let mut o = observation(&g, TICK + 1);
    o.jobs.next_job_id -= 1;
    cases.push((o, "source_regressed"));
    let o = observation(&g, TICK - 1);
    cases.push((o, "source_regressed"));
    let mut o = observation(&g, TICK + 1);
    o.buildings.clear();
    o.items[0].holder_building_native_id = None;
    cases.push((o, "building_missing"));
    let mut o = observation(&g, TICK + 1);
    o.items.clear();
    cases.push((o, "item_missing"));
    let mut o = observation(&g, TICK + 1);
    o.buildings[0].x2 += 1;
    cases.push((o, "building_identity_mismatch"));
    let mut o = observation(&g, TICK + 1);
    o.buildings[0].max_build_stage += 1;
    cases.push((o, "building_identity_mismatch"));
    let mut o = observation(&g, TICK + 1);
    o.jobs.jobs.push(job(90, 70, "Clean"));
    cases.push((o, "original_job_identity_mismatch"));
    for (o, expected) in cases {
        let p = step(&Progress::new(&g), &g, &with_observation(&g, o));
        assert_eq!((p.phase, p.reason), ("invalidated", expected));
    }
    let p = step(&Progress::new(&g), &g, &sample(&g, TICK + 2));
    let mut later = Vec::new();
    let mut o = observation(&g, TICK + 3);
    o.jobs.bridge_generation += 1;
    later.push((o, "native_source_changed"));
    let mut o = observation(&g, TICK + 3);
    o.buildings[0].building_type += 1;
    later.push((o, "native_building_type_changed"));
    let mut o = observation(&g, TICK + 3);
    o.buildings[0].build_stage = 0;
    later.push((o, "construction_stage_regressed"));
    later.push((observation(&g, TICK + 1), "game_clock_regressed"));
    for (o, expected) in later {
        let next = step(&p, &g, &with_observation(&g, o));
        assert_eq!((next.phase, next.reason), ("invalidated", expected));
    }
    let mut o = observation(&g, TICK + 1);
    o.next_item_id += 1;
    let p = step(&Progress::new(&g), &g, &with_observation(&g, o));
    assert_eq!(
        step(&p, &g, &sample(&g, TICK + 2)).reason,
        "native_horizon_regressed"
    );
}

#[test]
fn cadence_pause_same_tick_changes_gaps_and_interrupted_reads_reset_global_stability() {
    let base = goal(1);
    let g = Goal::new(
        base.records().to_vec(),
        Timing {
            interval: 3,
            stable_samples: 3,
            stable_span: 6,
            max_gap: 20,
            ..base.timing()
        },
    )
    .unwrap();
    let mut p = Progress::new(&g);
    for n in 1..=6 {
        p = step(&p, &g, &sample(&g, TICK + n));
        assert!(!p.terminal());
    }
    assert_eq!(step(&p, &g, &sample(&g, TICK + 7)).phase, "satisfied");
    let first = step(&Progress::new(&base), &base, &sample(&base, TICK + 1));
    let paused = step(&first, &base, &sample(&base, TICK + 1));
    assert_eq!(paused.streak, 1);
    let mut o = observation(&base, TICK + 1);
    o.items[0].flags |= 1;
    let changed = step(&first, &base, &with_observation(&base, o));
    assert_eq!(changed.streak, 0);
    let reading = first.begin_read().unwrap();
    let resumed = step(&reading, &base, &sample(&base, TICK + 2));
    assert_eq!((resumed.streak, resumed.interruptions), (1, 1));
    let g = Goal::new(
        base.records().to_vec(),
        Timing {
            max_gap: 2,
            ..base.timing()
        },
    )
    .unwrap();
    let p = step(&Progress::new(&g), &g, &sample(&g, TICK + 1));
    assert_eq!(step(&p, &g, &sample(&g, TICK + 5)).streak, 1);
}

#[test]
fn fixed_deadline_observation_allowance_and_cancellation_are_terminal_and_immutable() {
    let base = goal(1);
    let p = step(
        &Progress::new(&base),
        &base,
        &sample(&base, base.timing().deadline - 1),
    );
    let expired = step(&p, &base, &sample(&base, base.timing().deadline));
    assert_eq!(
        (expired.phase, expired.reason),
        ("expired", "game_deadline_reached")
    );
    let g = Goal::new(
        base.records().to_vec(),
        Timing {
            max_observations: 2,
            ..base.timing()
        },
    )
    .unwrap();
    let p = step(&Progress::new(&g), &g, &sample(&g, TICK + 1));
    let exhausted = step(&p, &g, &sample(&g, TICK + 1));
    assert_eq!(
        (exhausted.phase, exhausted.reason),
        ("expired", "sample_budget_exhausted")
    );
    let cancelled = p.begin_read().unwrap().cancel();
    assert_eq!(
        (cancelled.phase, cancelled.reading, cancelled.streak),
        ("cancelled", false, 0)
    );
    let satisfied = step(&p, &g, &sample(&g, TICK + 2));
    for terminal in [expired, exhausted, cancelled, satisfied] {
        assert!(terminal.terminal());
        assert_eq!(terminal.cancel(), terminal);
        assert!(terminal.begin_read().is_err());
        assert!(
            terminal
                .advance(&g, &sample(&g, TICK + 3), &context(&g))
                .is_err()
        );
    }
}

#[test]
fn strict_complete_decode_rejects_enum_aliases_cycles_dangling_links_and_bad_counts() {
    let g = goal(3);
    let o = observation(&g, TICK + 1);
    for roster in 0..2 {
        let mut changed = o.clone();
        if roster == 0 {
            changed.buildings[1].building_type = changed.buildings[0].building_type;
        } else {
            changed.items[1].item_type = changed.items[0].item_type;
        }
        let s = with_observation(&g, changed);
        assert!(s.validate(&g, &context(&g)).is_err());
    }
    let mut changed = o.clone();
    changed.jobs.jobs = vec![job(1, 70, "Clean"), job(2, 71, "Other")];
    assert!(
        with_observation(&g, changed)
            .validate(&g, &context(&g))
            .is_err()
    );
    let mut changed = o.clone();
    changed.items[0].container_native_id = Some(changed.items[1].native_id);
    changed.items[1].container_native_id = Some(changed.items[0].native_id);
    assert!(
        changed
            .encode_profile(OperationsProfile::PagedV1_4)
            .is_err()
    );
    let mut changed = o.clone();
    changed.jobs.jobs.push(job(1, 999, "Clean"));
    assert!(
        changed
            .encode_profile(OperationsProfile::PagedV1_4)
            .is_err()
    );
    let mut changed = o;
    let mut j = job(1, 70, "Clean");
    j.attached_item_count = 1;
    changed.jobs.jobs.push(j);
    assert!(
        changed
            .encode_profile(OperationsProfile::PagedV1_4)
            .is_err()
    );
    let mut s = sample(&g, TICK + 1);
    s.capture.push(0);
    assert!(s.validate(&g, &context(&g)).is_err());
}

#[test]
fn current_query_whole_fortress_scope_high_tick_and_all_work_bounds_are_required() {
    let g = goal(3);
    let s = sample(&g, TICK + 2);
    let state = Progress::new(&g).begin_read().unwrap();
    let mut cases = Vec::new();
    let mut c = context(&g);
    c.grants.clear();
    cases.push(c);
    let mut c = context(&g);
    c.cancellation_requested = true;
    cases.push(c);
    let mut c = context(&g);
    c.grants[0].expires_at_tick = Some(GameTick(TICK + 1));
    cases.push(c);
    let mut c = context(&g);
    c.grants[0].remaining_uses = Some(1);
    cases.push(c);
    let mut c = context(&g);
    c.grants[0].scope.entity_ids.insert(EntityId::new(1));
    cases.push(c);
    let mut c = context(&g);
    c.anchor.fortress_id = FortressId::new(777);
    c.grants[0].scope = CapabilityScope::default();
    cases.push(c);
    let mut c = context(&g);
    c.budget.max_entities = 1;
    cases.push(c);
    let mut c = context(&g);
    c.budget.max_bytes = 100;
    cases.push(c);
    let mut c = context(&g);
    c.budget.max_wall_millis = 0;
    cases.push(c);
    for c in cases {
        assert!(state.advance(&g, &s, &c).is_err());
    }
    assert!(state.advance(&g, &s, &context(&g)).is_ok());
}

#[test]
fn replay_frames_and_rejected_samples_share_one_semantic_work_allowance() {
    let g = goal(1);
    let initial = Progress::new(&g).begin_read().unwrap();
    let c = context(&g);
    let mut used = 0;
    let candidate =
        advance_with_counter(&initial, &g, &sample(&g, TICK + 1), &c, &mut used).unwrap();
    let first_cost = used;
    assert!(first_cost > 0);
    let reading = candidate.begin_read().unwrap();
    let mut malformed = sample(&g, TICK + 2);
    malformed.after_records[0].push(0);
    assert!(advance_with_counter(&reading, &g, &malformed, &c, &mut used).is_err());
    assert!(used > first_cost);
    let after_rejection = used;
    let satisfied =
        advance_with_counter(&reading, &g, &sample(&g, TICK + 2), &c, &mut used).unwrap();
    assert_eq!(satisfied.phase, "satisfied");
    assert!(used > after_rejection);
    let mut exhausted = MAX_WORK;
    assert!(advance_with_counter(&initial, &g, &sample(&g, TICK + 1), &c, &mut exhausted).is_err());
    assert!(exhausted > MAX_WORK);
}
