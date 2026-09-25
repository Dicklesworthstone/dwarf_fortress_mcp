use super::*;

fn hex(value: &str) -> Vec<u8> {
    let value = value.trim();
    (0..value.len()).step_by(2)
        .map(|n| u8::from_str_radix(&value[n..n + 2], 16).unwrap()).collect()
}
pub(super) fn before_bytes() -> Vec<u8> {
    hex(include_str!("../../tests/fixtures/excavation_run_capture_v1_18.hex"))
}
pub(super) fn plan() -> Result<ExcavationRunPlan> {
    ExcavationRunPlan::decode(&hex(include_str!("../../tests/fixtures/excavation_run_intent_v1_18.hex")))
}
pub(super) fn prepared() -> Result<ExcavationRunRecord> {
    ExcavationRunRecord::decode(&hex(include_str!("../../tests/fixtures/excavation_run_prepared_v1_18.hex")))
}
pub(super) fn stopped() -> Result<ExcavationRunRecord> {
    ExcavationRunRecord::decode(&hex(include_str!("../../tests/fixtures/excavation_run_stopped_v1_18.hex")))
}
fn reseal(mut bytes: Vec<u8>) -> Vec<u8> {
    let n = bytes.len() - 32;
    let digest = hash(b"dfmcp-excavation-run-receipt/1", &bytes[..n]);
    bytes[n..].copy_from_slice(digest.as_bytes());
    bytes
}
fn phase_offset(p: &ExcavationRunPlan) -> usize {
    8 + 2 + p.key().len() + 24 + 2 + p.before().canonical_bytes().len() + 32 + 16
}

#[test]
fn independent_native_vectors_and_keyed_intent_round_trip() -> Result<()> {
    let p = plan()?;
    assert_eq!(p.before().canonical_bytes(), before_bytes());
    assert_eq!(p.digest().as_bytes().as_slice(),
        hex(include_str!("../../tests/fixtures/excavation_run_plan_v1_18.hex")));
    assert_eq!(p.token().as_slice(),
        hex(include_str!("../../tests/fixtures/excavation_run_token_v1_18.hex")));
    assert_eq!(ExcavationRunPlan::decode(&p.canonical_bytes())?, p);
    assert_eq!(prepared()?.plan(), &p);
    let result = stopped()?;
    assert!(result.resolved() && result.historical_pause_verified());
    assert!(result.sampled_floor_reported());
    assert_eq!(result.stable_samples(), 2);
    assert_eq!(result.observed_tick(), Some(806504));
    assert_eq!(result.first_stable_tick(), 806501);
    prepared()?.validate_successor(&result)?;
    result.validate_successor(&result)?;
    Ok(())
}

#[test]
fn every_truncation_and_single_byte_corruption_is_rejected() -> Result<()> {
    let r = stopped()?;
    let raw = r.canonical_bytes();
    for n in 0..raw.len() {
        assert!(ExcavationRunRecord::decode(&raw[..n]).is_err(), "prefix {n}");
        let mut corrupt = raw.to_vec();
        corrupt[n] ^= 128;
        assert!(ExcavationRunRecord::decode(&corrupt).is_err(), "byte {n}");
    }
    let mut trailing = raw.to_vec(); trailing.push(0);
    assert!(ExcavationRunRecord::decode(&trailing).is_err());
    assert!(ExcavationRunRecord::decode(&vec![0; MAX_RECORD_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn redaction_has_no_hidden_payload_and_cannot_be_planned() -> Result<()> {
    let original = before_bytes();
    let cells = original.len() - 16;
    let mut redacted = original[..cells].to_vec();
    redacted.extend_from_slice(&[0, 1, 0, 1]);
    let capture = ExcavationCapture::decode(&redacted)?;
    assert_eq!(capture.cells(), &[ExcavationCell::Missing, ExcavationCell::Hidden,
        ExcavationCell::Missing, ExcavationCell::Hidden]);
    assert!(!capture.floor_observed());
    assert!(ExcavationRunPlan::new("hidden", plan()?.spec(), capture).is_err());
    redacted.push(0);
    assert!(ExcavationCapture::decode(&redacted).is_err());
    Ok(())
}

#[test]
fn invalid_regions_presence_counts_clock_and_map_are_rejected() {
    for r in [[0, 0, 0, 0, 1], [0, 0, 0, 9, 1], [32767, 0, 0, 2, 1],
        [0, 32767, 0, 1, 2], [0, 0, 32768, 1, 1], [u32::MAX, 0, 0, 1, 1]]
    { assert!(ExcavationRegion::new(r).is_err()); }
    let raw = before_bytes();
    for (offset, byte) in [(40, 0), (41, 0), (42, 2), (raw.len() - 18, 1),
        (raw.len() - 16, 3), (raw.len() - 15, 9), (raw.len() - 14, 8)]
    {
        let mut bad = raw.clone(); bad[offset] = byte;
        assert!(ExcavationCapture::decode(&bad).is_err(), "offset {offset}");
    }
    for n in 0..raw.len() { assert!(ExcavationCapture::decode(&raw[..n]).is_err()); }
}

#[test]
fn sampling_limits_fit_strictly_before_horizon() -> Result<()> {
    let clock = RunSpec::new(100, 1000)?;
    assert!(ExcavationRunSpec::new(clock, 2, 99, 1, 10).is_err());
    assert!(ExcavationRunSpec::new(clock, 100, 0, 1, 10).is_err());
    assert!(ExcavationRunSpec::new(clock, 0, 0, 1, 10).is_err());
    assert!(ExcavationRunSpec::new(clock, 1, 0, 1, 0).is_err());
    assert!(ExcavationRunSpec::new(clock, u32::MAX, u32::MAX, u32::MAX, u32::MAX).is_err());
    ExcavationRunSpec::new(clock, 99, 0, 1, 10)?;
    ExcavationRunSpec::new(clock, 1, 98, 1, 10)?;
    Ok(())
}

#[test]
fn canonical_plan_identity_covers_all_six_limits_and_before_bytes() -> Result<()> {
    let p = plan()?;
    let [ticks, wall, samples, stable, interval, gap] = p.spec().values();
    for values in [[ticks + 1, wall, samples, stable, interval, gap],
        [ticks, wall + 1, samples, stable, interval, gap],
        [ticks, wall, samples + 1, stable, interval, gap],
        [ticks, wall, samples, stable + 1, interval, gap],
        [ticks, wall, samples, stable, interval + 1, gap],
        [ticks, wall, samples, stable, interval, gap + 1]]
    {
        let s = ExcavationRunSpec::new(RunSpec::new(values[0], values[1])?,
            values[2], values[3], values[4], values[5])?;
        assert_ne!(ExcavationRunPlan::new(p.key(), s, p.before().clone())?.digest(), p.digest());
    }
    let other = ExcavationRunPlan::new("other", p.spec(), p.before().clone())?;
    assert_eq!(p.digest(), other.digest()); // Native plan hash intentionally excludes the key.
    assert_ne!(p.token(), other.token());
    assert_ne!(p.canonical_bytes(), other.canonical_bytes());
    Ok(())
}

#[test]
fn rehashed_false_floor_windows_and_source_substitutions_are_rejected() -> Result<()> {
    let p = plan()?;
    let original = stopped()?.canonical_bytes().to_vec();
    let phase = phase_offset(&p);
    // Valid checksums cannot make these inconsistent state claims admissible.
    for (offset, value) in [(phase + 3, 0), (phase + 13, 2), (phase + 17, 1),
        (phase + 25, 255), (phase + 33, 255), (phase + 41, 255),
        (phase + 43 + 2 + 16 + 7, 42)]
    {
        let mut bad = original.clone(); bad[offset] = value;
        assert!(ExcavationRunRecord::decode(&reseal(bad)).is_err(), "offset {offset}");
    }
    Ok(())
}

#[test]
fn all_predispatch_phase_reason_flag_combinations() -> Result<()> {
    let original = prepared()?.canonical_bytes().to_vec();
    let offset = phase_offset(&plan()?);
    for phase in 0u8..6 { for reason in 0u8..10 { for flags in 0u8..8 {
        let mut bytes = original.clone();
        let attempted = flags & 1;
        let verified = (flags >> 1) & 1;
        let known = (flags >> 2) & 1;
        bytes[offset..offset + 5].copy_from_slice(&[phase, reason, attempted, verified, known]);
        let valid = attempted == u8::from(matches!(phase, 1 | 2 | 3 | 5))
            && verified == u8::from(phase == 3) && match phase {
                0 => reason == 0 && known == 0,
                1 => false, // This fixture has tick zero, older than the source capture.
                2 => [1, 2, 3, 5, 6, 8].contains(&reason),
                3 => [1, 2, 3, 4, 5, 6, 8].contains(&reason),
                4 => [3, 7, 9].contains(&reason) && known == 0,
                5 => reason == 7 && known == 0,
                _ => false,
            };
        assert_eq!(ExcavationRunRecord::decode(&reseal(bytes)).is_ok(), valid,
            "phase {phase}, reason {reason}, flags {flags}");
    } } }
    Ok(())
}

#[test]
fn source_loss_and_unverified_stop_do_not_become_resolved() -> Result<()> {
    let p = plan()?;
    let raw = stopped()?.canonical_bytes().to_vec();
    let offset = phase_offset(&p);
    let mut stopping = raw.clone();
    stopping[offset] = 2; stopping[offset + 3] = 0;
    let stopping = ExcavationRunRecord::decode(&reseal(stopping))?;
    assert!(stopping.sampled_floor_reported());
    assert!(!stopping.resolved() && !stopping.historical_pause_verified());
    let mut lost = raw;
    lost[offset..offset + 5].copy_from_slice(&[5, 7, 1, 0, 0]);
    lost[offset + 5..offset + 13].fill(0);
    let lost = ExcavationRunRecord::decode(&reseal(lost))?;
    stopping.validate_successor(&lost)?;
    assert!(lost.terminal() && !lost.resolved());
    assert!(lost.validate_successor(&stopped()?).is_err());
    Ok(())
}
