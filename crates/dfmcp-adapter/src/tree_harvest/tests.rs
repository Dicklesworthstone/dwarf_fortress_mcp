//! Independent Rust decoding of actual C++ vectors, not live game evidence.
use super::*;
pub(crate) fn vector(name: &str) -> Result<Vec<u8>> {
    decode_vector(
        include_str!("../../../../tests/native/tree_harvest/vectors.json"),
        name,
    )
}
fn decode_vector(source: &str, name: &str) -> Result<Vec<u8>> {
    let prefix = format!("\"{name}\": \"");
    let hex = source
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix))
        .and_then(|v| v.split('"').next())
        .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree vector"))?;
    require(hex.len().is_multiple_of(2), "tree vector width")?;
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            u8::from_str_radix(
                std::str::from_utf8(p)
                    .map_err(|_| error(ErrorCode::AdapterRejected, "tree vector encoding"))?,
                16,
            )
            .map_err(|_| error(ErrorCode::AdapterRejected, "tree vector hex"))
        })
        .collect()
}
pub(crate) fn capture() -> Result<Capture> {
    Capture::decode(&vector("capture")?)
}
pub(crate) fn plan() -> Result<Plan> {
    Plan::new("tree-001", capture()?)
}
pub(crate) fn record(name: &str) -> Result<Record> {
    Record::decode(&vector(name)?, &plan()?)
}
pub(crate) fn progress(tick: u64, generation: u64) -> Result<Progress> {
    let mut raw = vector("progress_supply")?;
    raw[8..16].copy_from_slice(&generation.to_be_bytes());
    raw[24..32].copy_from_slice(&tick.to_be_bytes());
    Progress::decode(&raw, &plan()?)
}
#[test]
fn tree_actual_native_vectors_decode_with_distinct_commitments() -> Result<()> {
    let cap = capture()?;
    assert!(cap.eligible());
    assert_eq!(cap.root, [15, 15, 2]);
    assert_eq!(cap.designation, [16, 15, 2]);
    let p = plan()?;
    assert_eq!(p.digest.as_bytes().as_slice(), vector("plan")?);
    assert_eq!(p.token.as_slice(), vector("token")?);
    assert_eq!(Plan::decode(&p.encode())?, p);
    for (name, phase) in [
        ("prepared", 0),
        ("indeterminate", 1),
        ("designated", 2),
        ("refused", 3),
        ("cancelled", 4),
    ] {
        assert_eq!(record(name)?.phase, phase);
    }
    let now = progress(cap.tick + 10, cap.generation)?;
    assert!(now.supply_observed());
    assert_eq!(
        now.wood.iter().map(|w| w.id).collect::<Vec<_>>(),
        [1000, 1001]
    );
    assert!(
        Capture::decode(&decode_vector(
            include_str!("../../../../tests/native/workshop/vectors.json"),
            "capture"
        )?)
        .is_err()
    );
    let scope = cap.effect_scope()?;
    assert_eq!(
        [
            scope.min.x,
            scope.min.y,
            scope.min.z,
            scope.max.x,
            scope.max.y,
            scope.max.z
        ],
        [13, 0, 2, 31, 17, 2]
    );
    Ok(())
}
#[test]
fn tree_complete_capture_refuses_incoherent_body_cells_and_designation() -> Result<()> {
    let original = capture()?;
    let mut bad = original.clone();
    bad.body.pop();
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut bad = original.clone();
    bad.cells[1].position = bad.cells[0].position;
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut bad = original.clone();
    bad.cells.swap(0, 1);
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut bad = original.clone();
    bad.designation = bad.root;
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut bad = original.clone();
    bad.workers[0].position = [64, 1, 2];
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut bad = original.clone();
    bad.size = [64, 64, 64];
    assert!(Capture::decode(&bad.encode()).is_err());
    let mut trailing = original.bytes().to_vec();
    trailing.push(0);
    assert!(Capture::decode(&trailing).is_err());
    for n in 0..original.bytes().len() {
        assert!(Capture::decode(&original.bytes()[..n]).is_err());
    }
    Ok(())
}
#[test]
fn tree_worker_tool_claims_and_hidden_domains_do_not_grant_eligibility() -> Result<()> {
    let original = capture()?;
    for i in 0..7 {
        let mut cap = original.clone();
        cap.workers[0].facts[i] = false;
        let cap = Capture::decode(&cap.encode())?;
        assert!(!cap.eligible());
        assert!(Plan::new("no-authority", cap).is_err());
    }
    for flag in [1u32 << 1, 1 << 2, 1 << 5, 1 << 15, 1 << 16, 1 << 19] {
        let mut cap = original.clone();
        cap.tools[0].flags |= flag;
        assert!(!Capture::decode(&cap.encode())?.eligible());
    }
    let mut cap = original.clone();
    cap.tools[0].role = 2;
    assert!(!Capture::decode(&cap.encode())?.eligible());
    let mut cap = original.clone();
    cap.tools[0].facts[2] = false;
    assert!(!Capture::decode(&cap.encode())?.eligible());
    let mut cap = original.clone();
    cap.cells[0].presence = 1;
    assert!(Capture::decode(&cap.encode()).is_err());
    let mut cap = original;
    cap.species += 1;
    assert!(Plan::new("changed-cached-fields", cap).is_err());
    Ok(())
}
