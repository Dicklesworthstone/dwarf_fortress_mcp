use super::*;

pub(super) fn fixture(name: &str) -> Vec<u8> {
    let raw = match name {
        "pending" => include_str!("../../tests/fixtures/order_progress_pending_v1_11.hex"),
        "active" => include_str!("../../tests/fixtures/order_progress_active_v1_11.hex"),
        "zero" => include_str!("../../tests/fixtures/order_progress_zero_v1_11.hex"),
        _ => include_str!("../../tests/fixtures/order_progress_missing_v1_11.hex"),
    };
    raw.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}
pub(super) fn sample(sequence: u64, tick: u64, remaining: u32) -> OrderProgress {
    let mut raw = fixture("active");
    raw[16..24].copy_from_slice(&sequence.to_be_bytes());
    raw[24..32].copy_from_slice(&tick.to_be_bytes());
    raw[59..63].copy_from_slice(&remaining.to_be_bytes());
    OrderProgress::decode(&raw).unwrap()
}
fn watch() -> ProgressWatch {
    ProgressWatch::new("beds", sample(1, 10, 5), 100, 2, 2).unwrap()
}

#[test]
fn native_vectors_keep_counters_and_absence_distinct() {
    for (name, class) in [
        ("pending", "awaiting_validation"),
        ("active", "active"),
        ("zero", "counter_zero"),
        ("missing", "missing"),
    ] {
        let raw = fixture(name);
        let o = OrderProgress::decode(&raw).unwrap();
        assert_eq!(o.classification(), class);
        assert_eq!(o.canonical_bytes(), raw);
        assert_eq!(o.order_id(), 10);
        assert_eq!(o.next_order_id(), 11);
        assert_eq!(o.generation(), 7);
    }
    assert!(
        OrderProgress::decode(&fixture("missing"))
            .unwrap()
            .counters()
            .is_none()
    );
}
#[test]
fn all_incomplete_and_extended_native_records_are_rejected() {
    for name in ["pending", "active", "zero", "missing"] {
        let raw = fixture(name);
        for end in 0..raw.len() {
            assert!(
                OrderProgress::decode(&raw[..end]).is_err(),
                "{name} prefix {end}"
            );
        }
        let mut extra = raw;
        extra.push(0);
        assert!(OrderProgress::decode(&extra).is_err());
    }
}
#[test]
fn malformed_scalars_booleans_utf8_and_recipe_evidence_are_refused() {
    for (offset, value) in [
        (44, 2),
        (45, 2),
        (46, 255),
        (48, 0),
        (48, 255),
        (75, 5),
        (67, 128),
        (63, 128),
        (59, 128),
    ] {
        let mut raw = fixture("active");
        raw[offset] = value;
        assert!(OrderProgress::decode(&raw).is_err(), "offset {offset}");
    }
    for offset in [8, 16] {
        for value in [0u64, u64::MAX] {
            let mut raw = fixture("active");
            raw[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
            assert!(OrderProgress::decode(&raw).is_err());
        }
    }
    let mut raw = fixture("active");
    raw[63..67].copy_from_slice(&101u32.to_be_bytes());
    assert!(OrderProgress::decode(&raw).is_err());
    let mut raw = fixture("active");
    raw[59..63].copy_from_slice(&6u32.to_be_bytes());
    assert!(OrderProgress::decode(&raw).is_err());
}
#[test]
fn unknown_configuration_keeps_raw_values_without_template_authority() {
    let mut raw = fixture("active");
    raw[75] = 0;
    raw[55..59].copy_from_slice(&900i32.to_be_bytes());
    raw[71..75].copy_from_slice(&91i32.to_be_bytes());
    raw[67..71].copy_from_slice(&255u32.to_be_bytes());
    let o = OrderProgress::decode(&raw).unwrap();
    let c = o.counters().unwrap();
    assert_eq!(c.native_job_type(), 900);
    assert_eq!(c.native_frequency(), 91);
    assert_eq!(c.raw_status(), 255);
    assert!(c.template().is_none());
    assert_eq!(o.classification(), "unsupported_configuration");
    assert!(ProgressWatch::new("unknown", o, 50_000, 1, 2).is_err());
}
#[test]
fn lineage_matches_the_existing_folder_site_domain_not_reader_incarnation() {
    let a = sample(1, 10, 5);
    let mut raw = a.canonical_bytes().to_vec();
    raw[15] = 8;
    let b = OrderProgress::decode(&raw).unwrap();
    assert_eq!(a.fortress_id(), b.fortress_id());
    assert_ne!(a.witness(), b.witness());
    let mut data = b"dfmcp-live-fortress-id-v1\0region1\0".to_vec();
    data.extend_from_slice(&1u32.to_be_bytes());
    let hash = Digest32::of_bytes(&data);
    let expected = u64::from_be_bytes(hash.as_bytes()[..8].try_into().unwrap()) | 1;
    assert_eq!(a.fortress_id().get(), expected);
}
#[test]
fn zero_stability_requires_distinct_ticks_and_the_declared_interval() {
    let w = watch().sampled(sample(2, 11, 0)).unwrap();
    assert_eq!(w.zero_samples(), 0);
    let w = w.sampled(sample(3, 12, 0)).unwrap();
    assert_eq!(w.zero_samples(), 1);
    let w = w.sampled(sample(4, 12, 0)).unwrap();
    assert_eq!(w.zero_samples(), 1);
    let w = w.sampled(sample(5, 13, 0)).unwrap();
    assert_eq!(w.zero_samples(), 1);
    let w = w.sampled(sample(6, 14, 0)).unwrap();
    assert_eq!(w.state(), ProgressState::CounterZeroStable);
    assert!(w.state().terminal());
}
#[test]
fn missing_and_changed_orders_do_not_discharge_counter_watches() {
    let mut missing = fixture("missing");
    missing[16..24].copy_from_slice(&2u64.to_be_bytes());
    missing[24..32].copy_from_slice(&12u64.to_be_bytes());
    assert_eq!(
        watch()
            .sampled(OrderProgress::decode(&missing).unwrap())
            .unwrap()
            .state(),
        ProgressState::Missing
    );
    let w = watch().sampled(sample(2, 12, 2)).unwrap();
    let changed = w.sampled(sample(3, 14, 3)).unwrap();
    assert_eq!(changed.state(), ProgressState::Changed);
    assert_eq!(
        changed.previous().unwrap().counters().unwrap().remaining(),
        2
    );
    assert_eq!(changed.last().counters().unwrap().remaining(), 3);
    let mut changed = sample(2, 12, 2).canonical_bytes().to_vec();
    changed[75] = 0;
    assert_eq!(
        watch()
            .sampled(OrderProgress::decode(&changed).unwrap())
            .unwrap()
            .state(),
        ProgressState::Changed
    );
}
#[test]
fn identity_and_backward_time_never_bridge_monitoring_intervals() {
    for offset in [15, 35, 43, 48] {
        let mut raw = sample(2, 12, 2).canonical_bytes().to_vec();
        raw[offset] ^= 1;
        if offset == 35 {
            raw[offset] = 9;
        }
        assert!(
            watch()
                .sampled(OrderProgress::decode(&raw).unwrap())
                .is_err()
        );
    }
    assert!(watch().sampled(sample(1, 12, 0)).is_err());
    assert!(watch().sampled(sample(2, 9, 0)).is_err());
    let mut raw = sample(2, 12, 2).canonical_bytes().to_vec();
    raw[39] = 10;
    // Present ID >= horizon is rejected before the watch can use it.
    assert!(OrderProgress::decode(&raw).is_err());
}
#[test]
fn deadline_precedes_success_and_terminal_history_is_immutable() {
    let w = watch().sampled(sample(2, 99, 0)).unwrap();
    let w = w.sampled(sample(3, 100, 0)).unwrap();
    assert_eq!(w.state(), ProgressState::Expired);
    assert_eq!(w.sampled(sample(4, 101, 0)).unwrap(), w);
    let c = watch().cancelled();
    assert_eq!(c.state(), ProgressState::Cancelled);
    assert_eq!(c.sampled(sample(2, 12, 0)).unwrap(), c);
}
#[test]
fn monitor_registration_bounds_and_no_baseline_temporal_credit() {
    for (deadline, interval, required) in [
        (10, 1, 2),
        (100, 0, 2),
        (100, 1, 1),
        (100, 1, 33),
        (100, 403201, 2),
    ] {
        assert!(ProgressWatch::new("x", sample(1, 10, 5), deadline, interval, required).is_err());
    }
    let w = ProgressWatch::new("zero", sample(1, 10, 0), 100, 1, 2).unwrap();
    assert_eq!(w.zero_samples(), 0);
    assert_eq!(w.state(), ProgressState::Watching);
}
