use super::*;

pub(super) fn fixture(name: &str) -> Result<Vec<u8>> {
    let raw = match name {
        "observation" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/native/dig_designation/vectors/observation.hex"
        )),
        "prepared" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/native/dig_designation/vectors/prepared.hex"
        )),
        "designated" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/native/dig_designation/vectors/designated.hex"
        )),
        "cancelled" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/native/dig_designation/vectors/cancelled.hex"
        )),
        _ => return Err(error(ErrorCode::InvalidRequest, "unknown test fixture")),
    };
    let text = raw.trim();
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| error(ErrorCode::InvalidRequest, "invalid native fixture hex"))
        })
        .collect()
}
pub(super) fn plan() -> Result<DigPlan> {
    DigPlan::new(
        "dig-001",
        false,
        DigObservation::decode(&fixture("observation")?)?,
    )
}
fn rehash(bytes: &mut [u8]) {
    let mut input = bytes[8..16].to_vec();
    input.extend_from_slice(&bytes[204..]);
    input.extend_from_slice(&bytes[85..133]);
    input.extend_from_slice(&bytes[133..172]);
    let proof = hash(b"dfmcp-dig-designation-receipt/1", &input);
    bytes[172..204].copy_from_slice(proof.as_bytes());
}

#[test]
fn native_vectors_bind_exact_plan_token_and_expected_post_state() -> Result<()> {
    let plan = plan()?;
    let native = fixture("designated")?;
    assert_eq!(plan.before().tiles().count(), 48);
    assert_eq!(plan.before().region().target_count(), 4);
    assert_eq!(plan.digest().as_bytes(), &native[85..117]);
    assert_eq!(plan.token().as_slice(), &native[117..133]);
    for (name, phase) in [
        ("prepared", DigPhase::Prepared),
        ("designated", DigPhase::Designated),
        ("cancelled", DigPhase::Refused),
    ] {
        let bytes = fixture(name)?;
        let effect = DigEffect::decode(&bytes, &plan)?;
        assert_eq!(effect.canonical_bytes(), bytes);
        assert_eq!(effect.phase(), phase);
        assert_eq!(effect.receipt().is_some(), phase.terminal());
        assert_eq!(
            effect.designated_count(),
            (phase == DigPhase::Designated).then_some(4)
        );
    }
    assert_eq!(
        DigEffect::decode(&fixture("cancelled")?, &plan)?.reason(),
        DigReason::CancelledBeforeDispatch
    );
    Ok(())
}

#[test]
fn every_effect_bit_is_covered_by_plan_or_receipt_evidence() -> Result<()> {
    let original = fixture("designated")?;
    let plan = plan()?;
    for offset in 0..original.len() {
        for bit in 0..8 {
            let mut mutated = original.clone();
            mutated[offset] ^= 1 << bit;
            assert!(
                DigEffect::decode(&mutated, &plan).is_err(),
                "uncovered effect byte {offset}, bit {bit}"
            );
        }
    }
    Ok(())
}

#[test]
fn rehashing_cannot_forge_requested_terrain_priority_or_scheduling() -> Result<()> {
    let plan = plan()?;
    for offset in [8, 16, 24, 32, 52, 53, 85, 117, 135, 139, 140, 171, 206] {
        let mut raw = fixture("designated")?;
        raw[offset] ^= 1;
        rehash(&mut raw);
        assert!(
            DigEffect::decode(&raw, &plan).is_err(),
            "rehash bypass at {offset}"
        );
    }
    for offset in [133, 134, 135] {
        let mut raw = fixture("prepared")?;
        raw[offset] = 3;
        assert!(DigEffect::decode(&raw, &plan).is_err());
    }
    Ok(())
}

#[test]
fn all_incomplete_captures_and_effects_fail_without_partial_publication() -> Result<()> {
    let capture = fixture("observation")?;
    for n in 0..capture.len() {
        assert!(DigObservation::decode(&capture[..n]).is_err());
    }
    let mut trailing = capture;
    trailing.push(0);
    assert!(DigObservation::decode(&trailing).is_err());
    for name in ["prepared", "designated", "cancelled"] {
        let bytes = fixture(name)?;
        for n in 0..bytes.len() {
            assert!(DigEffect::decode(&bytes[..n], &plan()?).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(DigEffect::decode(&trailing, &plan()?).is_err());
    }
    Ok(())
}

#[test]
fn hidden_neighbor_is_redacted_and_requires_explicit_policy() -> Result<()> {
    let before = plan()?.before().clone();
    let offset = before.offsets[0];
    let mut bytes = fixture("observation")?;
    bytes.splice(offset..offset + 32, [1]);
    let hidden = DigObservation::decode(&bytes)?;
    assert!(matches!(hidden.tiles[0], DigTile::Hidden));
    assert!(hidden.blockers(false).contains(&DigBlocker::HiddenContext));
    assert!(hidden.blockers(true).is_empty());
    assert!(DigPlan::new("hidden", false, hidden.clone()).is_err());
    assert!(DigPlan::new("hidden", true, hidden).is_ok());
    // A presence tag cannot carry a retained hidden attribute payload.
    let mut leaking = fixture("observation")?;
    leaking[offset] = 1;
    assert!(DigObservation::decode(&leaking).is_err());
    Ok(())
}

#[test]
fn hidden_or_missing_targets_never_become_eligible() -> Result<()> {
    let before = plan()?.before().clone();
    let index = before
        .tiles()
        .position(|(p, _)| before.region.target(p))
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture target"))?;
    let offset = before.offsets[index];
    for presence in [0, 1] {
        let mut bytes = fixture("observation")?;
        bytes.splice(offset..offset + 32, [presence]);
        let changed = DigObservation::decode(&bytes)?;
        assert!(
            changed
                .blockers(true)
                .contains(&DigBlocker::UnobservedTarget)
        );
        assert!(DigPlan::new("target", true, changed).is_err());
    }
    Ok(())
}

#[test]
fn full_halo_hazards_and_target_configuration_are_load_bearing() -> Result<()> {
    let before = plan()?.before().clone();
    for mask in [1, 2, 4, 8] {
        let mut bytes = fixture("observation")?;
        bytes[before.offsets[0] + 30] = mask;
        let changed = DigObservation::decode(&bytes)?;
        assert!(changed.blockers(true).contains(&DigBlocker::KnownHazard));
    }
    let index = before
        .tiles()
        .position(|(p, _)| before.region.target(p))
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture target"))?;
    for (flag, blocker) in [
        (0, DigBlocker::NotNaturalWall),
        (3, DigBlocker::ExistingDesignation),
        (5, DigBlocker::OccupiedOrJob),
        (9, DigBlocker::OccupiedOrJob),
    ] {
        let mut bytes = fixture("observation")?;
        bytes[before.offsets[index] + 31] = flag;
        assert!(
            DigObservation::decode(&bytes)?
                .blockers(false)
                .contains(&blocker)
        );
    }
    Ok(())
}

#[test]
fn geometry_and_scope_cover_complete_shared_blocks() -> Result<()> {
    for width in 1..=8 {
        for height in 1..=8 {
            let r = DigRegion::new(15, 15, 2, width, height)?;
            assert_eq!(r.halo_count(), ((width + 2) * (height + 2) * 3) as usize);
            assert!(r.halo_count() <= MAX_CELLS);
            assert_eq!(
                (0..r.halo_count())
                    .filter(|i| r.target(r.position(*i)))
                    .count(),
                (width * height) as usize
            );
        }
    }
    let r = DigRegion::new(15, 15, 2, 2, 2)?;
    assert_eq!(r.write_area().min, MapCoord::new(0, 0, 2));
    assert_eq!(r.write_area().max, MapCoord::new(31, 31, 2));
    assert_eq!(r.halo().min, MapCoord::new(14, 14, 1));
    assert_eq!(r.halo().max, MapCoord::new(17, 17, 3));
    for [x, y, z, w, h] in [
        [0, 1, 1, 1, 1],
        [1, 0, 1, 1, 1],
        [1, 1, 0, 1, 1],
        [1, 1, 32767, 1, 1],
        [32767, 1, 1, 1, 1],
        [1, u32::MAX, 1, 1, 1],
        [1, 1, 1, 0, 1],
        [1, 1, 1, 9, 1],
        [1, 1, 1, 1, u32::MAX],
    ] {
        assert!(DigRegion::new(x, y, z, w, h).is_err());
    }
    Ok(())
}

#[test]
fn retained_plan_is_canonical_and_never_imports_a_receipt_as_authority() -> Result<()> {
    let plan = plan()?;
    let bytes = plan.canonical_bytes();
    assert!(bytes.len() <= MAX_PLAN_BYTES);
    assert_eq!(DigPlan::decode(&bytes)?, plan);
    for n in 0..bytes.len() {
        assert!(DigPlan::decode(&bytes[..n]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(DigPlan::decode(&extra).is_err());
    let mut invalid_bool = bytes;
    invalid_bool[8 + 2 + plan.key().len()] = 2;
    assert!(DigPlan::decode(&invalid_bool).is_err());
    let other = DigPlan::new("other", false, plan.before().clone())?;
    assert_ne!(other.token(), plan.token());
    assert!(DigEffect::decode(&fixture("designated")?, &other).is_err());
    let changed_policy = DigPlan::new("dig-001", true, plan.before().clone())?;
    assert_ne!(changed_policy.digest(), plan.digest());
    Ok(())
}

#[test]
fn invalid_clocks_reserved_bytes_and_sequence_exhaustion_fail_closed() -> Result<()> {
    for (offset, value) in [
        (8, 0),
        (8, u64::MAX),
        (16, u64::MAX),
        (24, MAX_NATIVE_TICK + 1),
    ] {
        let mut raw = fixture("observation")?;
        raw[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        assert!(DigObservation::decode(&raw).is_err());
    }
    let mut raw = fixture("observation")?;
    raw[16..24].copy_from_slice(&(u64::MAX - 1).to_be_bytes());
    let last = DigObservation::decode(&raw)?;
    assert!(DigPlan::new("exhausted", false, last).is_err());
    for offset in [
        68,
        plan()?.before().offsets[0],
        plan()?.before().offsets[0] + 31,
    ] {
        let mut raw = fixture("observation")?;
        raw[offset] = 255;
        assert!(DigObservation::decode(&raw).is_err());
    }
    Ok(())
}
