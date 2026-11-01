//! The actual C++ handler's emitted vectors are consumed unchanged by the
//! independent Rust decoder. These are component checks, not live evidence.
use super::*;

pub(crate) fn vector(name: &str) -> Result<Vec<u8>> {
    let source = include_str!("../../../../tests/native/workshop/vectors.json");
    let prefix = format!("\"{name}\": \"");
    let text = source
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|rest| rest.split('"').next())
        .ok_or_else(invalid)?;
    require(text.len().is_multiple_of(2), "vector hex width")?;
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let raw = std::str::from_utf8(pair).map_err(|_| invalid())?;
            u8::from_str_radix(raw, 16).map_err(|_| invalid())
        })
        .collect()
}
pub(crate) fn capture() -> Result<WorkshopCapture> {
    WorkshopCapture::decode(&vector("capture")?)
}
pub(crate) fn plan() -> Result<WorkshopPlan> {
    WorkshopPlan::new("workshop-001", capture()?)
}
pub(crate) fn record(name: &str) -> Result<WorkshopRecord> {
    WorkshopRecord::decode(&vector(name)?)
}
pub(crate) fn progress(name: &str) -> Result<WorkshopProgress> {
    WorkshopProgress::decode(&vector(name)?)
}
fn rewrite_receipt(mut bytes: Vec<u8>) -> Vec<u8> {
    let n = bytes.len() - 32;
    let digest = hash(b"dfmcp-workshop-receipt/1", &bytes[..n]);
    bytes[n..].copy_from_slice(digest.as_bytes());
    bytes
}

#[test]
fn actual_native_vectors_decode_into_one_original_plan() -> Result<()> {
    let before = capture()?;
    assert!(before.eligible(), "{:?}", before.blockers());
    assert_eq!(before.canonical_bytes(), vector("capture")?);
    assert_eq!(before.selection(), WorkshopSelection::new(42, [15, 15, 2])?);
    assert_eq!(before.workers().len(), 1);
    let worker = before.workers()[0];
    assert!(worker.eligible() && worker.carpenter_labor_enabled() && worker.job_available());
    assert!(before.item().available() && !before.item_inside_footprint());
    let plan = plan()?;
    assert_eq!(plan.digest().as_bytes().as_slice(), vector("plan")?);
    assert_eq!(plan.token().as_slice(), vector("token")?);
    assert_eq!(WorkshopPlan::decode(&plan.canonical_bytes())?, plan);
    let cases = [
        ("prepared", WorkshopPhase::Prepared, WorkshopReason::None),
        ("indeterminate", WorkshopPhase::Indeterminate, WorkshopReason::NativeFailure),
        ("placed", WorkshopPhase::Placed, WorkshopReason::None),
        ("refused", WorkshopPhase::Refused, WorkshopReason::Stale),
        ("cancelled", WorkshopPhase::Cancelled, WorkshopReason::Cancelled),
    ];
    for (name, phase, reason) in cases {
        let r = record(name)?;
        r.verify_plan(&plan)?;
        assert_eq!((r.phase(), r.reason()), (phase, reason), "{name}");
        assert_eq!(r.canonical_bytes(), vector(name)?);
    }
    let placed = record("placed")?;
    let after = placed.after().ok_or_else(invalid)?;
    assert_eq!(*after, before.expected_after()?);
    assert!(after.tiles().iter().all(|t| matches!(t, WorkshopTile::Visible(v)
        if v.occupied() && v.building_id() == Some(70))));
    let insertion = placed.insertion().ok_or_else(invalid)?;
    assert_eq!(
        (insertion.building_id(), insertion.job_id(), insertion.item_id()),
        (70, 90, 42)
    );
    assert_eq!(insertion.geometry(), [14, 14, 16, 16, 2, 15, 15]);
    assert_eq!((insertion.material(), insertion.material_index()), (420, 163));
    assert_eq!((insertion.stage(), insertion.max_stage()), (0, 3));
    assert!(insertion.setting_occupancy() && insertion.planned_tiles() == ALL_TILES);
    Ok(())
}

#[test]
fn completion_requires_exact_original_continuity() -> Result<()> {
    let placed = record("placed")?;
    let staged = progress("progress_stage0")?;
    assert_eq!(
        WorkshopGoal::evaluate(&placed, &staged),
        WorkshopGoal::InProgress { stage: 0, max_stage: 3, construct_jobs: 1 }
    );
    let complete = progress("progress_complete")?;
    let facts = complete.facts().ok_or_else(invalid)?;
    assert!(facts.item_contained() && facts.exists() && facts.construct_jobs() == 0);
    assert_eq!(WorkshopGoal::evaluate(&placed, &complete), WorkshopGoal::Complete);
    assert_eq!(
        WorkshopGoal::evaluate(&placed, &progress("progress_absent")?),
        WorkshopGoal::Absent
    );
    // Without a Placed receipt nothing can complete, whatever the read says.
    for name in ["prepared", "indeterminate", "refused", "cancelled"] {
        assert_eq!(
            WorkshopGoal::evaluate(&record(name)?, &complete),
            WorkshopGoal::Unknown(WorkshopUnknown::NoPlacedReceipt)
        );
    }
    // Same ID, type, geometry and material under a different native generation
    // (plugin restart, reload or restore) is NOT continuity.
    let raw = vector("progress_complete")?;
    let mut restarted = raw.clone();
    restarted[15] ^= 2;
    let restarted = WorkshopProgress::decode(&restarted)?;
    assert_eq!(restarted.facts(), complete.facts());
    assert_eq!(
        WorkshopGoal::evaluate(&placed, &restarted),
        WorkshopGoal::Unknown(WorkshopUnknown::NativeGenerationChanged)
    );
    let mut earlier = raw.clone();
    earlier[16..24].copy_from_slice(&12000u64.to_be_bytes());
    assert_eq!(
        WorkshopGoal::evaluate(&placed, &WorkshopProgress::decode(&earlier)?),
        WorkshopGoal::Unknown(WorkshopUnknown::ClockRegressed)
    );
    let mut site = raw.clone();
    site[24..28].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        WorkshopGoal::evaluate(&placed, &WorkshopProgress::decode(&site)?),
        WorkshopGoal::Unknown(WorkshopUnknown::SourceIdentityDiffers)
    );
    // Present-building identity contradictions never become completion.
    let folder = 40 + 2 + 7;
    let present = folder + 12;
    for (offset, value) in [
        (present + 1, 0u8),           // not Carpenters
        (present + 10 + 3, 13),       // x1 differs
        (present + 38 + 3, 7),        // material type differs
        (present + 42 + 3, 7),        // material index differs
        (present + 50 + 3, 4),        // maximum stage differs
    ] {
        let mut changed = raw.clone();
        changed[offset] = value;
        let changed = WorkshopProgress::decode(&changed)?;
        assert_eq!(WorkshopGoal::evaluate(&placed, &changed), WorkshopGoal::Contradicted);
    }
    // stage == max with a lingering construction job is still not complete.
    let mut lingering = raw.clone();
    lingering[present + 55] = 1;
    lingering[present + 56] = 1;
    assert!(matches!(
        WorkshopGoal::evaluate(&placed, &WorkshopProgress::decode(&lingering)?),
        WorkshopGoal::InProgress { construct_jobs: 1, .. }
    ));
    Ok(())
}

#[test]
fn malformed_and_noncanonical_evidence_is_refused() -> Result<()> {
    let raw = vector("capture")?;
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(WorkshopCapture::decode(&trailing).is_err());
    let mut profile = raw.clone();
    profile[7] = b'9';
    assert!(WorkshopCapture::decode(&profile).is_err());
    // Noncanonical boolean in the paused flag.
    let mut boolean = raw.clone();
    boolean[64 + 2 + 7] = 2;
    assert!(WorkshopCapture::decode(&boolean).is_err());
    // A different workshop kind byte is outside the closed profile.
    let mut kind = raw.clone();
    kind[64 + 2 + 7 + 4] = 2;
    assert!(WorkshopCapture::decode(&kind).is_err());
    // Recomputing the receipt hash cannot make a different post-state pass.
    let placed = vector("placed")?;
    let mut forged = placed.clone();
    let n = forged.len();
    forged[n - 33] ^= 1; // last insertion flag (suspended)
    assert!(WorkshopRecord::decode(&rewrite_receipt(forged)).is_err());
    let mut tampered = placed.clone();
    tampered[n - 1] ^= 1;
    assert!(WorkshopRecord::decode(&tampered).is_err());
    // Indeterminate without the attempted flag is contradictory.
    let indeterminate = vector("indeterminate")?;
    let mut contradictory = indeterminate.clone();
    let m = contradictory.len();
    contradictory[m - 34] = 0;
    assert!(WorkshopRecord::decode(&rewrite_receipt(contradictory)).is_err());
    let progress = vector("progress_absent")?;
    let mut extra = progress.clone();
    extra.push(1);
    assert!(WorkshopProgress::decode(&extra).is_err());
    Ok(())
}

#[test]
fn immutable_outcomes_permit_only_original_reconciliation() -> Result<()> {
    let prepared = record("prepared")?;
    let indeterminate = record("indeterminate")?;
    let placed = record("placed")?;
    let refused = record("refused")?;
    prepared.validate_successor(&placed)?;
    indeterminate.validate_successor(&placed)?;
    indeterminate.validate_successor(&indeterminate)?;
    assert!(placed.validate_successor(&indeterminate).is_err());
    assert!(refused.validate_successor(&placed).is_err());
    assert!(indeterminate.validate_successor(&refused).is_err());
    Ok(())
}

#[test]
fn selection_footprint_and_material_overlap_are_explicit() -> Result<()> {
    let s = WorkshopSelection::new(42, [15, 15, 2])?;
    assert_eq!(s.footprint(), [14, 14, 16, 16, 2]);
    assert!(s.contains([14, 16, 2]) && s.contains([15, 15, 2]));
    assert!(!s.contains([17, 15, 2]) && !s.contains([15, 15, 3]) && !s.contains([13, 14, 2]));
    assert!(WorkshopSelection::new(42, [0, 15, 2]).is_err());
    assert!(WorkshopSelection::new(i32::MAX as u32, [15, 15, 2]).is_err());
    // A capture whose log lies inside its own footprint can never be planned.
    let raw = vector("capture")?;
    // Header, folder, flags, selection and nine visible building-free tiles.
    let item = 64 + 2 + 7 + 4 + 17 + 9 * 17 + 1;
    let mut inside = raw.clone();
    inside[item..item + 12].copy_from_slice(&[0, 0, 0, 15, 0, 0, 0, 16, 0, 0, 0, 2]);
    let inside = WorkshopCapture::decode(&inside)?;
    assert!(inside.item().available() && inside.item_inside_footprint());
    assert_eq!(inside.blockers(), vec![WorkshopBlocker::ItemInsideFootprint]);
    assert!(WorkshopPlan::new("workshop-001", inside).is_err());
    Ok(())
}

#[test]
fn binding_is_canonical_and_exact_for_effects() -> Result<()> {
    let before = capture()?;
    let endpoint: SocketAddr = "127.0.0.1:5000".parse().map_err(|_| invalid())?;
    let binding = WorkshopBinding::new(endpoint, "test-df", "test-dfhack", &before)?;
    assert_eq!(WorkshopBinding::decode(&binding.encode())?, binding);
    let later = binding.recovery_generation(8)?;
    binding.source_matches(&later, false)?;
    assert!(binding.source_matches(&later, true).is_err());
    assert!(later.source_matches(&binding, false).is_err());
    assert!(WorkshopBinding::new("10.0.0.1:5000".parse().map_err(|_| invalid())?, "a", "b", &before).is_err());
    Ok(())
}
