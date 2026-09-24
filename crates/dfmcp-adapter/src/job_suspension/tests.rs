use super::*;

const OBSERVATION: &str = include_str!("../../tests/fixtures/job_suspension_observation_v1_9.hex");
const EFFECT: &str = include_str!("../../tests/fixtures/job_suspension_effect_v1_9.hex");

fn hex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    require(text.len().is_multiple_of(2), "odd fixture hex")?;
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let a = char::from(pair[0])
                .to_digit(16)
                .ok_or_else(|| invalid("fixture hex"))?;
            let b = char::from(pair[1])
                .to_digit(16)
                .ok_or_else(|| invalid("fixture hex"))?;
            Ok((a * 16 + b) as u8)
        })
        .collect()
}
fn plan() -> Result<SuspensionPlan> {
    SuspensionPlan::new(JobObservation::decode(&hex(OBSERVATION)?)?, "job-001", true)
}
fn proof(bytes: &mut [u8]) {
    let mut preimage = b"dfmcp-job-suspension-receipt/1\0".to_vec();
    preimage.extend_from_slice(&bytes[8..16]);
    preimage.extend_from_slice(&bytes[192..]);
    preimage.extend_from_slice(&bytes[69..160]);
    let digest = Digest32::of_bytes(&preimage);
    bytes[160..192].copy_from_slice(digest.as_bytes());
}
fn record(state: SuspensionState) -> Result<Vec<u8>> {
    let mut out = hex(EFFECT)?;
    out[117] = state as u8;
    if !matches!(
        state,
        SuspensionState::Applied | SuspensionState::NotApplied
    ) {
        out[118..192].fill(0);
    }
    if state == SuspensionState::NotApplied {
        out[119] = 0;
        let mut after = hex(OBSERVATION)?;
        after[16..24].copy_from_slice(&1u64.to_be_bytes());
        out[128..160].copy_from_slice(Digest32::of_bytes(&after).as_bytes());
    }
    if state.terminal() {
        proof(&mut out);
    }
    Ok(out)
}

#[test]
fn native_observation_and_applied_receipt_are_byte_exact() -> Result<()> {
    let plan = plan()?;
    let o = plan.observation();
    assert_eq!(o.generation(), 7);
    assert_eq!(o.sequence(), 0);
    assert_eq!(o.tick(), 42_336_003);
    assert_eq!((o.job_id(), o.next_job_id(), o.site_id()), (3, 10, 1));
    assert_eq!(o.holder_id(), Some(20));
    assert_eq!(o.holder_type(), Some(1));
    assert_eq!(o.worker_id(), None);
    assert_eq!(o.position(), [1, 2, 5]);
    assert_eq!(o.world_folder(), "region1");
    assert_eq!(o.type_key(), "BrewDrink");
    assert_eq!(o.reaction(), "");
    assert!(o.eligible() && o.repeating() && !o.suspended());
    assert_eq!(
        o.witness().to_hex(),
        "5c7a7bf9ecef8a6a8026082bba1c326f233d294e3930360b41d82c07b9abd613"
    );
    assert_eq!(
        plan.digest().to_hex(),
        "4173fb3e7bcaf7e907f509a36be6a8d0868dcd93b159ef32c904d8623830acd6"
    );
    assert_eq!(
        plan.prepare_token().as_slice(),
        &hex("d001cf4cf833637c7703c69c9772079d")?
    );
    let effect = SuspensionEffect::decode(&hex(EFFECT)?, &plan)?;
    assert_eq!(effect.state(), SuspensionState::Applied);
    assert_eq!(effect.observed_suspended(), Some(true));
    assert!(effect.receipt().is_some() && effect.after_witness().is_some());
    assert_eq!(effect.canonical_bytes(), hex(EFFECT)?);
    Ok(())
}

#[test]
fn every_native_effect_byte_and_bit_is_bound_to_the_prepared_intent() -> Result<()> {
    let plan = plan()?;
    let native = hex(EFFECT)?;
    for offset in 0..native.len() {
        for bit in 0..8 {
            let mut changed = native.clone();
            changed[offset] ^= 1 << bit;
            assert!(
                SuspensionEffect::decode(&changed, &plan).is_err(),
                "offset {offset}, bit {bit}"
            );
        }
    }
    Ok(())
}

#[test]
fn truncations_trailing_data_and_oversized_frames_are_refused() -> Result<()> {
    let plan = plan()?;
    let observation = hex(OBSERVATION)?;
    for end in 0..observation.len() {
        assert!(JobObservation::decode(&observation[..end]).is_err());
    }
    let effect = hex(EFFECT)?;
    for end in 0..effect.len() {
        assert!(SuspensionEffect::decode(&effect[..end], &plan).is_err());
    }
    let mut longer = observation.clone();
    longer.push(0);
    assert!(JobObservation::decode(&longer).is_err());
    let mut longer = effect.clone();
    longer.push(0);
    assert!(SuspensionEffect::decode(&longer, &plan).is_err());
    assert!(JobObservation::decode(&vec![0; MAX_OBSERVATION_BYTES + 1]).is_err());
    assert!(SuspensionEffect::decode(&vec![0; MAX_EFFECT_BYTES + 1], &plan).is_err());
    Ok(())
}

#[test]
fn pending_unknown_refusal_and_negative_readback_remain_distinct() -> Result<()> {
    let plan = plan()?;
    for state in [
        SuspensionState::Prepared,
        SuspensionState::Unknown,
        SuspensionState::Applied,
        SuspensionState::NotApplied,
        SuspensionState::Refused,
    ] {
        let effect = SuspensionEffect::decode(&record(state)?, &plan)?;
        assert_eq!(effect.state(), state);
        assert_eq!(effect.receipt().is_some(), state.terminal());
        assert_eq!(
            effect.observed_suspended(),
            match state {
                SuspensionState::Applied => Some(true),
                SuspensionState::NotApplied => Some(false),
                _ => None,
            }
        );
    }
    for state in [
        SuspensionState::Prepared,
        SuspensionState::Unknown,
        SuspensionState::Refused,
    ] {
        let valid = record(state)?;
        for index in 118..160 {
            let mut corrupt = valid.clone();
            corrupt[index] = 1;
            if state.terminal() {
                proof(&mut corrupt);
            }
            assert!(SuspensionEffect::decode(&corrupt, &plan).is_err());
        }
    }
    Ok(())
}

#[test]
fn a_recomputed_receipt_cannot_certify_unrelated_or_later_readback() -> Result<()> {
    let plan = plan()?;
    for offset in [118, 119, 120, 127, 128, 159] {
        let mut corrupt = hex(EFFECT)?;
        corrupt[offset] ^= 1;
        proof(&mut corrupt);
        assert!(
            SuspensionEffect::decode(&corrupt, &plan).is_err(),
            "offset {offset}"
        );
    }
    let mut forged = hex(EFFECT)?;
    let mut unrelated = hex(OBSERVATION)?;
    unrelated[16..24].copy_from_slice(&1u64.to_be_bytes());
    unrelated[84] |= 1;
    unrelated[67] ^= 1; // Change the job's y position as well as suspension.
    forged[128..160].copy_from_slice(Digest32::of_bytes(&unrelated).as_bytes());
    proof(&mut forged);
    assert!(SuspensionEffect::decode(&forged, &plan).is_err());
    Ok(())
}

#[test]
fn keys_targets_worlds_and_control_desires_cannot_be_substituted() -> Result<()> {
    let original = JobObservation::decode(&hex(OBSERVATION)?)?;
    let bytes = hex(EFFECT)?;
    for key in ["", "a b", "a/b", "a\nb", "é", &"a".repeat(129)] {
        assert!(SuspensionPlan::new(original.clone(), key, true).is_err());
    }
    for key in ["job-002", "x", &"a".repeat(128)] {
        let other = SuspensionPlan::new(original.clone(), key, true)?;
        assert!(SuspensionEffect::decode(&bytes, &other).is_err());
    }
    let other = SuspensionPlan::new(original.clone(), "job-001", false)?;
    assert!(SuspensionEffect::decode(&bytes, &other).is_err());
    for offset in [15, 23, 31, 35, 39, 43, 47, 87, 105] {
        let mut changed = hex(OBSERVATION)?;
        changed[offset] ^= 1;
        if let Ok(observation) = JobObservation::decode(&changed) {
            let other = SuspensionPlan::new(observation, "job-001", true)?;
            assert!(SuspensionEffect::decode(&bytes, &other).is_err());
        }
    }
    Ok(())
}

#[test]
fn native_bounds_unknown_positions_and_utf8_are_preserved() -> Result<()> {
    for (offset, value) in [
        (8, 0),
        (8, u64::MAX),
        (16, u64::MAX),
        (24, MAX_NATIVE_TICK + 1),
    ] {
        let mut bytes = hex(OBSERVATION)?;
        bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        assert!(JobObservation::decode(&bytes).is_err());
    }
    for (offset, value) in [
        (36, 3u32),
        (36, u32::MAX),
        (40, u32::MAX),
        (44, u32::MAX),
        (48, u32::MAX - 1),
        (72, u32::MAX - 1),
        (76, 65_537),
        (80, 4097),
    ] {
        let mut bytes = hex(OBSERVATION)?;
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(JobObservation::decode(&bytes).is_err());
    }
    let mut bytes = hex(OBSERVATION)?;
    bytes[84] |= 64;
    assert!(JobObservation::decode(&bytes).is_err());
    for text in [
        vec![0],
        vec![0xff],
        vec![0xc0, 0x80],
        vec![0xed, 0xa0, 0x80],
    ] {
        let mut bytes = hex(OBSERVATION)?;
        bytes[85..87].copy_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.splice(87..94, text);
        assert!(JobObservation::decode(&bytes).is_err());
    }
    let mut bytes = hex(OBSERVATION)?;
    bytes[60..64].copy_from_slice(&i32::MIN.to_be_bytes());
    bytes[44..48].copy_from_slice(&i32::MAX.to_be_bytes());
    let value = JobObservation::decode(&bytes)?;
    assert_eq!(value.position()[0], i32::MIN);
    assert_eq!(value.job_type(), i32::MAX); // Do not assume a native enum registry.
    Ok(())
}

#[test]
fn ineligible_observations_are_readable_but_do_not_make_plans() -> Result<()> {
    for flag in [4, 8, 16, 32] {
        let mut bytes = hex(OBSERVATION)?;
        bytes[84] &= !flag;
        let value = JobObservation::decode(&bytes)?;
        assert!(!value.eligible());
        assert!(SuspensionPlan::new(value, "job-001", true).is_err());
    }
    for offset in [56, 72] {
        let mut bytes = hex(OBSERVATION)?;
        bytes[offset..offset + 4].copy_from_slice(&0i32.to_be_bytes());
        let value = JobObservation::decode(&bytes)?;
        assert!(!value.eligible());
        assert!(SuspensionPlan::new(value, "job-001", true).is_err());
    }
    let mut bytes = hex(OBSERVATION)?;
    bytes[48..56].fill(0xff);
    bytes[84] &= !(8 | 32);
    let value = JobObservation::decode(&bytes)?;
    assert!(value.holder_id().is_none() && value.holder_type().is_none());
    assert!(!value.eligible());
    Ok(())
}

#[test]
fn sequence_exhaustion_cannot_become_terminal_success() -> Result<()> {
    let mut observation = hex(OBSERVATION)?;
    observation[16..24].copy_from_slice(&(u64::MAX - 1).to_be_bytes());
    let plan = SuspensionPlan::new(JobObservation::decode(&observation)?, "job-001", true)?;
    assert!(plan.observation().expected_after_witness(true).is_err());
    Ok(())
}
