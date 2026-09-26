//! Native-independent engine fixtures are consumed unchanged by the Rust decoder.
use super::*;

pub(super) fn fixture(name: &str) -> Result<Vec<u8>> {
    let source =
        include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let prefix = format!("\"{name}\": \"");
    let text = source
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|rest| rest.split('"').next())
        .ok_or_else(invalid)?;
    require(text.len().is_multiple_of(2), "fixture hex width")?;
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let raw = std::str::from_utf8(pair).map_err(|_| invalid())?;
            u8::from_str_radix(raw, 16).map_err(|_| invalid())
        })
        .collect()
}
fn capture() -> Result<BuildCapture> {
    BuildCapture::decode(&fixture("capture")?)
}
fn rewrite_receipt(mut bytes: Vec<u8>) -> Vec<u8> {
    let n = bytes.len() - 32;
    let digest = hash(b"dfmcp-build-receipt/1", &bytes[..n]);
    bytes[n..].copy_from_slice(digest.as_bytes());
    bytes
}

#[test]
fn all_eight_native_vectors_decode_without_wire_changes() -> Result<()> {
    let before = capture()?;
    assert!(before.eligible());
    assert_eq!(before.encode(), fixture("capture")?);
    let plan = BuildPlan::new("golden", before.clone())?;
    assert_eq!(plan.digest().as_bytes().as_slice(), fixture("plan")?);
    assert_eq!(plan.token().as_slice(), fixture("token")?);
    assert_eq!(BuildPlan::decode(&plan.canonical_bytes())?, plan);
    let cases = [
        ("prepared", BuildPhase::Prepared, BuildReason::None),
        (
            "indeterminate",
            BuildPhase::Indeterminate,
            BuildReason::NativeFailure,
        ),
        ("placed", BuildPhase::Placed, BuildReason::None),
        ("expired", BuildPhase::Refused, BuildReason::Expired),
        ("cancelled", BuildPhase::Cancelled, BuildReason::Cancelled),
    ];
    for (name, phase, reason) in cases {
        let bytes = fixture(name)?;
        let record = BuildRecord::decode(&bytes)?;
        record.verify_plan(&plan)?;
        assert_eq!(record.canonical_bytes(), bytes);
        assert_eq!(record.phase(), phase);
        assert_eq!(record.reason(), reason);
        assert_eq!(
            record.attempted(),
            matches!(phase, BuildPhase::Indeterminate | BuildPhase::Placed)
        );
        assert_eq!(
            record.resolved(),
            matches!(
                phase,
                BuildPhase::Placed | BuildPhase::Refused | BuildPhase::Cancelled
            )
        );
        assert_eq!(record.after().is_some(), phase == BuildPhase::Placed);
    }
    let placed = BuildRecord::decode(&fixture("placed")?)?;
    assert_eq!(placed.after(), Some(&before.expected_after()?));
    let insertion = placed.insertion().ok_or_else(invalid)?;
    assert_eq!(
        (
            insertion.building_id(),
            insertion.job_id(),
            insertion.item_id()
        ),
        (70, 90, 42)
    );
    assert_eq!(insertion.stage(), 0);
    Ok(())
}

#[test]
fn every_incomplete_native_prefix_is_rejected() -> Result<()> {
    for name in [
        "prepared",
        "indeterminate",
        "placed",
        "expired",
        "cancelled",
    ] {
        let bytes = fixture(name)?;
        for n in 0..bytes.len() {
            assert!(
                BuildRecord::decode(&bytes[..n]).is_err(),
                "{name} prefix {n}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(BuildRecord::decode(&trailing).is_err());
    }
    let bytes = fixture("capture")?;
    for n in 0..bytes.len() {
        assert!(
            BuildCapture::decode(&bytes[..n]).is_err(),
            "capture prefix {n}"
        );
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(BuildCapture::decode(&trailing).is_err());
    Ok(())
}

#[test]
fn every_record_byte_corruption_is_rejected() -> Result<()> {
    for name in [
        "prepared",
        "indeterminate",
        "placed",
        "expired",
        "cancelled",
    ] {
        let bytes = fixture(name)?;
        for n in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[n] ^= 1;
            assert!(
                BuildRecord::decode(&changed).is_err(),
                "{name} mutation {n}"
            );
        }
    }
    Ok(())
}

#[test]
fn rehashed_fabricated_effect_and_insertion_are_still_rejected() -> Result<()> {
    let bytes = fixture("placed")?;
    let start = bytes
        .windows(8)
        .position(|window| window == b"DFMBI019")
        .ok_or_else(invalid)?;
    for offset in [8, 12, 16, 20, 21, 25, 29, 33, 37, 41, 49, 50, 51, 52] {
        let mut changed = bytes.clone();
        changed[start + offset] ^= 1;
        assert!(
            BuildRecord::decode(&rewrite_receipt(changed)).is_err(),
            "insertion {offset}"
        );
    }
    let start = bytes
        .windows(8)
        .enumerate()
        .filter(|(_, window)| *window == b"DFMBC019")
        .map(|(i, _)| i)
        .nth(1)
        .ok_or_else(invalid)?;
    for offset in [8, 16, 24, 32, 48, 52, 56, 71] {
        let mut changed = bytes.clone();
        changed[start + offset] ^= 1;
        assert!(
            BuildRecord::decode(&rewrite_receipt(changed)).is_err(),
            "after {offset}"
        );
    }
    Ok(())
}

#[test]
fn phase_reason_attempt_and_after_shape_cannot_be_rehashed_into_success() -> Result<()> {
    let bytes = fixture("prepared")?;
    let offset = bytes.len() - 36;
    for phase in 0..=5 {
        for reason in 0..=6 {
            for attempt in 0..=2 {
                let mut changed = bytes.clone();
                changed[offset] = phase;
                changed[offset + 1] = reason;
                changed[offset + 2] = attempt;
                let valid = (phase == 0 && reason == 0 && attempt == 0)
                    || (phase == 1 && reason == 5 && attempt == 1)
                    || (phase == 3 && (1..=3).contains(&reason) && attempt == 0)
                    || (phase == 4 && reason == 4 && attempt == 0);
                assert_eq!(
                    BuildRecord::decode(&rewrite_receipt(changed)).is_ok(),
                    valid,
                    "{phase}/{reason}/{attempt}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn indeterminate_and_all_other_outcomes_remain_immutable() -> Result<()> {
    let prepared = BuildRecord::decode(&fixture("prepared")?)?;
    for name in [
        "prepared",
        "indeterminate",
        "placed",
        "expired",
        "cancelled",
    ] {
        let next = BuildRecord::decode(&fixture(name)?)?;
        prepared.validate_successor(&next)?;
        next.validate_successor(&next)?;
        if next.phase() != BuildPhase::Prepared {
            assert!(next.validate_successor(&prepared).is_err());
            for other in ["indeterminate", "placed", "expired", "cancelled"] {
                let other = BuildRecord::decode(&fixture(other)?)?;
                assert_eq!(next.validate_successor(&other).is_ok(), next == other);
            }
        }
    }
    let uncertain = BuildRecord::decode(&fixture("indeterminate")?)?;
    assert!(!uncertain.terminal());
    assert!(!uncertain.resolved());
    assert!(uncertain.attempted());
    Ok(())
}

#[test]
fn every_noncomputed_item_flag_blocks_but_all_flags_change_the_plan() -> Result<()> {
    let before = capture()?;
    let original = BuildPlan::new("golden", before.clone())?;
    for bit in 0..32 {
        let mut changed = before.clone();
        if let BuildItem::Visible(item) = &mut changed.item {
            item.other_flags = 1 << bit;
        }
        let changed = BuildCapture::decode(&changed.encode())?;
        assert_eq!(changed.eligible(), matches!(bit, 28 | 29), "flag bit {bit}");
        assert_ne!(changed.witness(), before.witness());
        if changed.eligible() {
            assert_ne!(
                BuildPlan::new("golden", changed)?.digest(),
                original.digest()
            );
        }
    }
    Ok(())
}

#[test]
fn hidden_and_missing_attributes_are_unrepresentable_and_block_placement() -> Result<()> {
    for redacted in [BuildTile::Missing, BuildTile::Hidden] {
        for index in 0..9 {
            let mut before = capture()?;
            before.tiles[index] = redacted.clone();
            let before = BuildCapture::decode(&before.encode())?;
            assert!(!before.eligible());
            assert!(
                before
                    .blockers()
                    .contains(&BuildBlocker::ContextNotFullyVisible)
            );
            assert!(BuildPlan::new("golden", before).is_err());
        }
    }
    for item in [BuildItem::Missing, BuildItem::Hidden] {
        let mut before = capture()?;
        before.item = item;
        let before = BuildCapture::decode(&before.encode())?;
        assert_eq!(before.item_position(), None);
        assert!(!before.eligible());
    }
    Ok(())
}

#[test]
fn source_horizons_floor_and_item_preconditions_fail_closed() -> Result<()> {
    let before = capture()?;
    let mut cases = Vec::new();
    let mut value = before.clone();
    value.paused = false;
    cases.push(value);
    let mut value = before.clone();
    value.free_tile = false;
    cases.push(value);
    let mut value = before.clone();
    value.supported = false;
    cases.push(value);
    let mut value = before.clone();
    value.sequence = u64::MAX - 1;
    cases.push(value);
    let mut value = before.clone();
    value.next_building = MAX_ID;
    cases.push(value);
    let mut value = before.clone();
    value.next_job = MAX_ID;
    cases.push(value);
    let mut value = before.clone();
    value.building_count = 65536;
    cases.push(value);
    let mut value = before.clone();
    if let BuildTile::Visible(tile) = &mut value.tiles[4] {
        tile.liquid = 1;
    }
    cases.push(value);
    let mut value = before.clone();
    for index in [1, 3, 5, 7] {
        if let BuildTile::Visible(tile) = &mut value.tiles[index] {
            tile.shape = 1;
        }
    }
    cases.push(value);
    let mut value = before.clone();
    if let BuildItem::Visible(item) = &mut value.item {
        item.wear = 1;
    }
    cases.push(value);
    let mut value = before.clone();
    if let BuildItem::Visible(item) = &mut value.item {
        item.other_refs = 1;
    }
    cases.push(value);
    let mut value = before;
    if let BuildItem::Visible(item) = &mut value.item {
        item.in_job = true;
    }
    cases.push(value);
    for value in cases {
        let value = BuildCapture::decode(&value.encode())?;
        assert!(!value.eligible());
        assert!(value.expected_after().is_err());
        assert!(BuildPlan::new("golden", value).is_err());
    }
    Ok(())
}

#[test]
fn bounded_unicode_identity_selection_and_binding_roundtrip() -> Result<()> {
    let mut before = capture()?;
    before.fortress = FortressIdentity::new(&"é".repeat(256), MAX_ID)?;
    let before = BuildCapture::decode(&before.encode())?;
    let plan = BuildPlan::new(&"a".repeat(128), before.clone())?;
    assert_eq!(BuildPlan::decode(&plan.canonical_bytes())?, plan);
    for invalid_key in ["", "../key", "spaces here", "é", &"a".repeat(129)] {
        assert!(BuildPlan::new(invalid_key, before.clone()).is_err());
    }
    for target in [[0, 1, 0], [1, 0, 0], [32767, 1, 0], [1, 1, 32768]] {
        assert!(BuildSelection::new(BuildKind::Bed, 42, target).is_err());
    }
    assert!(BuildSelection::new(BuildKind::Bed, MAX_ID, [1, 1, 0]).is_err());
    let endpoint = "127.0.0.1:5000".parse().map_err(|_| invalid())?;
    let binding = BuildBinding::new(endpoint, "53.01", "53.01-r1", &before)?;
    assert_eq!(BuildBinding::decode(&binding.encode())?, binding);
    assert!(binding.capture_matches(&before));
    let recovered = binding.recovery_generation(binding.generation() + 1)?;
    binding.source_matches(&recovered, false)?;
    assert!(binding.source_matches(&recovered, true).is_err());
    assert!(
        binding
            .recovery_generation(binding.generation() - 1)
            .is_err()
    );
    Ok(())
}
