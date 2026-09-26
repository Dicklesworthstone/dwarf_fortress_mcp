use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::{JobItemAttachment, LiveBuilding, LiveOperationsObservation, LiveOperationsState};
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};

fn observation() -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7, df_version: "test-df".into(), dfhack_version: "test-dfhack".into(),
            year: 250, year_tick: 100, paused: true, site_id: 2, world_folder: "test-world".into(),
            next_job_id: 100, jobs: vec![job(5, "ConstructBuilding")],
        },
        next_building_id: 100, next_item_id: 100,
        buildings: vec![LiveBuilding {
            native_id: 10, building_type: 1, type_key: "Bed".into(), x1: 12, y1: 14,
            x2: 12, y2: 14, z: 3, build_stage: 0, max_build_stage: 3,
        }],
        items: vec![LiveItem {
            native_id: 20, item_type: 1, type_key: "BED".into(), subtype: -1,
            material_type: 0, material_index: 2, stack_size: 1, raw_position: MapCoord::new(5, 6, 3),
            flags: 66, container_native_id: None, holder_building_native_id: None,
        }],
        attachments: vec![JobItemAttachment { job_native_id: 5, item_native_id: 20, role: 0, filter_index: -1 }],
    }
}
fn job(id: u32, kind: &str) -> LiveJob {
    LiveJob {
        native_id: id, job_type: 1, type_key: kind.into(), reaction: String::new(),
        suspended: false, repeating: false, position: MapCoord::new(12, 14, 3),
        worker_native_id: None, holder_native_id: Some(10), completion_timer: -1,
        attached_item_count: u32::from(id == 5), required_item_filter_count: 0,
    }
}
fn target() -> Target {
    Target { building_native_id: 10, expected_generation: Some(1), expected_type: Some("Bed".into()), item_native_id: Some(20) }
}
fn context(state: &LiveOperationsState) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(19), request_id: RequestId::new(1),
        anchor: state.snapshot().unwrap().anchor(), budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Query, scope: CapabilityScope::default(), max_risk: RiskTier::ReadOnly,
            expires_at_tick: None, remaining_uses: None,
        }], cancellation_requested: false,
    }
}
fn report(observation: LiveOperationsObservation, targets: &[Target]) -> Report {
    let mut state = LiveOperationsState::default();
    state.publish(observation).unwrap();
    analyze(&state, &context(&state), targets, MAX_WORK).unwrap()
}
fn status(observation: LiveOperationsObservation) -> Status {
    report(observation, &[target()]).rows[0].status
}
fn completed() -> LiveOperationsObservation {
    let mut o = observation();
    o.jobs.jobs.clear(); o.attachments.clear();
    o.buildings[0].build_stage = 3;
    o.items[0].flags = 256;
    o.items[0].holder_building_native_id = Some(10);
    o
}

#[test]
fn observes_registration_construction_and_completion_as_different_conditions() {
    let o = observation();
    assert_eq!(status(o.clone()), Status::Pending);
    let mut suspended = o.clone(); suspended.jobs.jobs[0].suspended = true;
    assert_eq!(status(suspended), Status::Suspended);
    let mut missing_job = o.clone(); missing_job.jobs.jobs.clear(); missing_job.attachments.clear();
    assert_eq!(status(missing_job), Status::NoConstructionJob);
    let mut stage_only = o; stage_only.buildings[0].build_stage = 3;
    assert_eq!(status(stage_only), Status::Pending);
    assert_eq!(status(completed()), Status::SatisfiedAtObservation);
}

#[test]
fn exact_item_link_and_state_are_required_when_requested() {
    let o = completed();
    let mut v = o.clone(); v.items.clear();
    assert_eq!(status(v), Status::ItemUnverified);
    for flags in [0, 2, 8, 64, 128, 258, 264, 320, 384] {
        let mut v = o.clone(); v.items[0].flags = flags;
        assert_eq!(status(v), Status::ItemUnverified);
    }
    let mut v = o.clone(); v.items[0].holder_building_native_id = None;
    assert_eq!(status(v), Status::ItemUnverified);
    let mut v = o.clone(); v.items[0].type_key = "CHAIR".into();
    assert_eq!(status(v), Status::ItemUnverified);
    let mut t = target(); t.item_native_id = None;
    assert_eq!(report(o, &[t]).rows[0].status, Status::SatisfiedAtObservation);
}

#[test]
fn checks_all_512_observed_flag_words_against_independent_bit_reference() {
    for building_kind in ["Bed", "Chair", "Table"] {
        let mut item = completed().items.remove(0);
        item.type_key = building_kind.to_ascii_uppercase();
        for flags in 0..512u32 {
            item.flags = flags;
            for attached in 0..3 {
                let expected = flags / 256 % 2 == 1
                    && [1, 3, 6, 7].iter().all(|bit| flags / (1u32 << *bit) % 2 == 0)
                    && attached == 0;
                assert_eq!(installed_item(&item, 10, building_kind, attached), expected);
                assert!(!installed_item(&item, 11, building_kind, attached));
            }
        }
    }
}

#[test]
fn unrelated_job_attachment_prevents_installed_item_condition() {
    let mut o = completed();
    let mut j = job(5, "StoreItemInStockpile"); j.holder_native_id = None;
    o.jobs.jobs = vec![j];
    o.attachments = observation().attachments;
    let result = report(o, &[target()]);
    assert_eq!(result.rows[0].status, Status::ItemUnverified);
    assert_eq!(result.rows[0].item.as_ref().unwrap().attached_jobs, 1);
    assert_eq!(result.rows[0].item.as_ref().unwrap().attached_construction_jobs, 0);
}

#[test]
fn bounds_examples_but_never_stops_counting_or_hides_late_removal() {
    let mut o = completed();
    o.jobs.jobs = (10..30).map(|id| job(id, "Clean")).collect();
    o.jobs.jobs.push(job(30, "DestroyBuilding"));
    let result = report(o, &[target()]);
    assert_eq!(result.rows[0].status, Status::RemovalPending);
    assert_eq!(result.rows[0].jobs.other, 20);
    assert_eq!(result.rows[0].jobs.removal, 1);
    assert_eq!(result.rows[0].jobs.examples.len(), MAX_JOB_EXAMPLES);
}

#[test]
fn unknown_building_and_invalid_construction_shapes_do_not_prove_completion() {
    let mut o = completed(); o.buildings.clear(); o.items[0].holder_building_native_id = None;
    let result = report(o, &[target()]);
    assert_eq!(result.rows[0].status, Status::Missing);
    assert_eq!(result.rows[0].stage, None);
    for maximum in [0, 33, i32::MAX] {
        let mut o = completed(); o.buildings[0].max_build_stage = maximum; o.buildings[0].build_stage = maximum;
        assert_eq!(status(o), Status::Unsupported);
    }
    let mut o = completed(); o.buildings[0].x2 += 1;
    assert_eq!(status(o), Status::SatisfiedAtObservation); // Identity target, not a placement-footprint proof.
    let mut o = completed(); o.buildings[0].type_key = "Workshop".into();
    let mut t = target(); t.expected_type = None;
    assert_eq!(report(o, &[t]).rows[0].status, Status::Unsupported);
}

#[test]
fn source_reset_or_observed_id_reappearance_never_reuses_expected_generation() {
    let mut state = LiveOperationsState::default();
    let o = completed(); state.publish(o.clone()).unwrap();
    let old = context(&state);
    let mut gone = o.clone(); gone.jobs.year_tick += 1;
    gone.buildings.clear(); gone.items[0].holder_building_native_id = None;
    state.publish(gone).unwrap();
    assert!(analyze(&state, &old, &[target()], MAX_WORK).is_err());
    let mut returned = o.clone(); returned.jobs.year_tick += 2;
    state.publish(returned).unwrap();
    let row = analyze(&state, &context(&state), &[target()], MAX_WORK).unwrap().rows.remove(0);
    assert_eq!(row.status, Status::IdentityMismatch);
    assert_eq!(row.handle.unwrap().generation, 2);
    let mut reset = o; reset.jobs.bridge_generation += 1; reset.jobs.year_tick += 3;
    state.publish(reset).unwrap();
    assert_eq!(analyze(&state, &context(&state), &[target()], MAX_WORK).unwrap().rows[0].status, Status::IdentityMismatch);
}

#[test]
fn rejects_authority_cancellation_scan_limits_work_limits_and_bad_requests() {
    let mut state = LiveOperationsState::default(); state.publish(observation()).unwrap();
    let c = context(&state);
    let mut v = c.clone(); v.grants.clear();
    assert!(analyze(&state, &v, &[target()], MAX_WORK).is_err());
    let mut v = c.clone(); v.cancellation_requested = true;
    assert!(analyze(&state, &v, &[target()], MAX_WORK).is_err());
    let mut v = c.clone(); v.budget.max_entities = 1;
    assert!(analyze(&state, &v, &[target()], MAX_WORK).is_err());
    for maximum in [0, 1, MAX_WORK + 1] {
        assert!(analyze(&state, &c, &[target()], maximum).is_err());
    }
    assert!(analyze(&state, &c, &[], MAX_WORK).is_err());
    assert!(analyze(&state, &c, &[target(), target()], MAX_WORK).is_err());
    assert!(analyze(&state, &c, &vec![target(); MAX_TARGETS + 1], MAX_WORK).is_err());
    for t in [Target { expected_generation: Some(0), ..target() },
        Target { expected_type: Some("Workshop".into()), ..target() },
        Target { building_native_id: u32::MAX, ..target() },
        Target { item_native_id: Some(u32::MAX), ..target() }] {
        assert!(analyze(&state, &c, &[t], MAX_WORK).is_err());
    }
    assert!(analyze(&state, &c, &[target(), Target { building_native_id: 11, ..target() }], MAX_WORK).is_err());
}

#[test]
fn input_order_does_not_change_report_and_missing_target_does_not_hide_present_work() {
    let mut t2 = target(); t2.building_native_id = 11; t2.item_native_id = None;
    let a = report(completed(), &[target(), t2.clone()]);
    let b = report(completed(), &[t2, target()]);
    assert_eq!(a, b);
    assert_eq!(a.rows[0].status, Status::SatisfiedAtObservation);
    assert_eq!(a.rows[1].status, Status::Missing);
    assert_ne!(a.anchor.state_hash, a.source_digest);
}

#[test]
fn job_disappearance_never_sets_completion_before_maximum_stage() {
    for maximum in 1..=32 {
        for stage in 0..=maximum {
            for job_present in [false, true] {
                let mut o = completed();
                o.buildings[0].build_stage = stage; o.buildings[0].max_build_stage = maximum;
                if job_present {
                    let mut j = job(5, "ConstructBuilding"); j.attached_item_count = 0;
                    o.jobs.jobs.push(j);
                }
                let expected = if job_present { Status::Pending }
                    else if stage == maximum { Status::SatisfiedAtObservation }
                    else { Status::NoConstructionJob };
                assert_eq!(status(o), expected);
            }
        }
    }
}
