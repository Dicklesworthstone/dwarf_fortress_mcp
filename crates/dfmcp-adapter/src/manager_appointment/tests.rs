//! Synthetic codec/state tests; these establish no SDK or live-game proof.
use super::*;
fn n(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn f(out: &mut Vec<u8>, v: &[u8]) {
    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
    out.extend_from_slice(v);
}
fn assignment(out: &mut Vec<u8>, id: i32, position: i32, hf: i32, cache: i32, flags: &[u8]) {
    for v in [id, position, hf, hf, cache, -1, -1, -1, -1, -1, 0] {
        n(out, v);
    }
    f(out, flags);
    out.extend([0, 0]);
}
pub(crate) fn capture() -> Result<Capture> {
    let mut b = b"DFMMC022".to_vec();
    for v in [7u64, 3, 30 * 403200 + 16800] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    for v in [38, 30, 16800, 96, 96, 32] {
        n(&mut b, v);
    }
    f(&mut b, b"codec-unit-fort");
    n(&mut b, 256);
    n(&mut b, 361);
    // Deliberately distinguish position ID/index, assignment ID/index and entity ID/index.
    for v in [35, 1, 1234, 777] {
        n(&mut b, v);
    }
    b.extend([0x23; 32]);
    b.extend([0, 2]);
    assignment(&mut b, 91, 8, 361, 1, &[0x81]);
    assignment(&mut b, 600, 1000, -1, -1, &[0x81, 0x40]);
    for v in [12, 0, 24, 7] {
        n(&mut b, v);
    }
    b.extend([0x24; 32]);
    b.extend([0, 0]);
    for v in [1000, 2, -1, 0, 1992, 1746] {
        n(&mut b, v);
    }
    for v in [0x25, 0x26, 0x27] {
        b.extend([v; 32]);
    }
    b.push(0);
    b.extend([0, 3]);
    for row in [
        [0, 12, 0, 100, -1, -1, -1],
        [0, 35, 1, 100, -1, -1, -1],
        [10, 35, 1, 100, 91, 0, 30],
    ] {
        for v in row {
            n(&mut b, v);
        }
    }
    b.extend([1; 11]);
    Capture::decode(&b)
}
pub(crate) fn plan() -> Result<Plan> {
    Plan::new("manager.original-1", capture()?)
}
pub(crate) fn record(
    p: &Plan,
    phase: u8,
    reason: u8,
    after: Option<&Capture>,
    proof: bool,
) -> Vec<u8> {
    let mut b = b"DFMMR022".to_vec();
    f(&mut b, p.key().as_bytes());
    b.extend_from_slice(p.digest.as_bytes());
    b.extend_from_slice(&p.token);
    b.extend_from_slice(&(p.before.bytes().len() as u32).to_be_bytes());
    b.extend_from_slice(p.before.bytes());
    b.extend([phase, reason, u8::from(after.is_some())]);
    if let Some(a) = after {
        b.extend_from_slice(&(a.bytes().len() as u32).to_be_bytes());
        b.extend_from_slice(a.bytes());
    }
    b.push(u8::from(proof));
    if proof {
        b.extend_from_slice(b"DFMMI022");
        for v in [256, 361, 35, 600, 1992] {
            n(&mut b, v);
        }
        b.extend([1, 1]);
    }
    let d = hash(b"dfmcp-manager-receipt/1", &b);
    b.extend_from_slice(d.as_bytes());
    b
}
fn reseal(c: &mut Capture) {
    c.raw = c.encode();
}
#[test]
fn lossless_capture_plan_and_full_transaction() -> Result<()> {
    let c = capture()?;
    assert!(c.eligible());
    assert_eq!(c.bytes(), c.encode());
    let p = plan()?;
    assert_eq!(Plan::decode(&p.encode())?, p);
    let a = c.expected_after()?;
    let (_, old) = c.target().ok_or_else(|| {
        error(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test target",
        )
    })?;
    let (_, new) = a.target().ok_or_else(|| {
        error(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test target",
        )
    })?;
    assert_eq!((new.figure, new.last_figure, new.cache), (361, 361, 2));
    assert_eq!(new.flags, old.flags);
    assert_eq!(new.id, 600);
    assert_eq!(a.group.assignments[0], c.group.assignments[0]);
    assert_eq!(a.civ, c.civ);
    assert_eq!(
        (a.tick, a.year_tick, a.history_count, a.next_event),
        (c.tick, 16800, 1747, 1993)
    );
    assert_eq!(a.sequence, c.sequence + 1);
    let link = a.links.last().ok_or_else(|| {
        error(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test link",
        )
    })?;
    assert_eq!(
        (
            link.entity,
            link.entity_cache,
            link.assignment,
            link.assignment_cache,
            link.strength,
            link.start_year
        ),
        (35, 1, 600, 1, 100, 30)
    );
    let e = a.last_event.as_ref().ok_or_else(|| {
        error(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test event",
        )
    })?;
    assert_eq!(
        (
            e.id,
            e.year,
            e.seconds,
            e.entity,
            e.figure,
            e.position_id,
            e.appointer,
            e.promise
        ),
        (1992, 30, 16800, 35, 361, 1000, -1, -1)
    );
    assert_eq!(e.flags, [0]);
    assert!(e.position && e.added);
    assert_eq!(a.roster_digest, c.roster_digest);
    assert_eq!(a.entity_domain_digest, c.entity_domain_digest);
    assert_ne!(a.history_digest, c.history_digest);
    let r = Record::decode(&record(&p, 2, 0, Some(&a), true), &p)?;
    assert!(r.complete());
    assert!(r.insertion.is_some());
    assert_eq!(r.after, Some(a));
    Ok(())
}
#[test]
fn eligibility_mirrors_native_ordinary_target_and_every_fact() -> Result<()> {
    for i in 0..11 {
        let mut c = capture()?;
        let fact = match i {
            0 => &mut c.paused,
            1 => &mut c.active,
            2 => &mut c.citizen,
            3 => &mut c.adult,
            4 => &mut c.sane,
            5 => &mut c.identity,
            6 => &mut c.member,
            7 => &mut c.role_eligible,
            8 => &mut c.dual_getters,
            9 => &mut c.expedition_leader,
            _ => &mut c.ui_quiescent,
        };
        *fact = false;
        reseal(&mut c);
        assert!(Plan::new("key", c).is_err());
    }
    for v in [vec![], vec![0], vec![3], vec![5], vec![9]] {
        let mut c = capture()?;
        c.group.assignments[1].flags = v;
        reseal(&mut c);
        assert!(!c.eligible());
    }
    for i in 0..10 {
        let mut c = capture()?;
        let a = &mut c.group.assignments[1];
        match i {
            0 => a.figure = 10,
            1 => a.last_figure = 10,
            2 => a.cache = 0,
            3 => a.squad = 1,
            4 => a.site = 1,
            5 => a.abstract_building = 1,
            6 => a.vassal_entity = 1,
            7 => a.vassal_assignment = 1,
            8 => a.temp = 1,
            _ => a.claims.push(100),
        };
        reseal(&mut c);
        assert!(!c.eligible());
    }
    let mut c = capture()?;
    c.group.assignments.push(c.group.assignments[1].clone());
    reseal(&mut c);
    assert!(!c.eligible());
    Ok(())
}
#[test]
fn witness_covers_vacancy_identity_source_epoch_and_bookkeeping() -> Result<()> {
    let c = capture()?;
    let p = plan()?;
    for i in 0..10 {
        let mut a = c.clone();
        match i {
            0 => a.generation += 1,
            1 => a.sequence += 1,
            2 => a.selection.figure += 1,
            3 => a.selection.unit += 1,
            4 => a.group.assignments[0].temp += 1,
            5 => a.roster_digest = Digest32::ZERO,
            6 => a.history_digest = Digest32::ZERO,
            7 => a.group.positions_digest = Digest32::ZERO,
            8 => a.links[0].strength -= 1,
            _ => a.group.next_assignment += 1,
        };
        reseal(&mut a);
        assert_ne!(a.witness(), c.witness());
        let other = Plan::new(p.key(), a)?;
        assert_ne!(p.digest, other.digest);
        assert!(Record::decode(&record(&p, 0, 0, None, false), &other).is_err());
    }
    let mut forged = c;
    forged.selection.figure += 1;
    assert!(Plan::new("key", forged).is_err());
    Ok(())
}
#[test]
fn exact_receipt_rejects_each_partial_or_extra_write() -> Result<()> {
    let p = plan()?;
    let a = p.before.expected_after()?;
    for i in 0..17 {
        let mut bad = a.clone();
        match i {
            0 => bad.group.assignments[1].figure = -1,
            1 => bad.group.assignments[1].last_figure = -1,
            2 => bad.group.assignments[1].cache = 1000,
            3 => bad.group.assignments[1].flags[0] = 1,
            4 => bad.group.next_assignment += 1,
            5 => bad.civ.next_position += 1,
            6 => bad.links.pop().map(|_| ()).unwrap_or(()),
            7 => bad.links[3].assignment_cache = 600,
            8 => bad.links[3].entity_cache = 35,
            9 => bad
                .last_event
                .as_mut()
                .map(|e| e.flags.clear())
                .unwrap_or(()),
            10 => bad
                .last_event
                .as_mut()
                .map(|e| e.seconds = 30)
                .unwrap_or(()),
            11 => bad
                .last_event
                .as_mut()
                .map(|e| e.position_id = 2)
                .unwrap_or(()),
            12 => bad.history_count += 1,
            13 => bad.next_event += 1,
            14 => bad.tick += 1,
            15 => bad.lookup_unit = -1,
            _ => bad.dual_getters = false,
        };
        reseal(&mut bad);
        assert!(
            Record::decode(&record(&p, 2, 0, Some(&bad), true), &p).is_err(),
            "mutation {i}"
        );
    }
    assert!(Record::decode(&record(&p, 2, 0, Some(&a), false), &p).is_err());
    assert!(Record::decode(&record(&p, 2, 0, None, true), &p).is_err());
    Ok(())
}
#[test]
fn refusal_uncertain_and_cancelled_are_not_success() -> Result<()> {
    let p = plan()?;
    for (phase, reason) in [(0, 0), (1, 5), (3, 1), (3, 2), (3, 3), (4, 4)] {
        let b = record(&p, phase, reason, None, false);
        let r = Record::decode(&b, &p)?;
        assert!(!r.complete());
        assert_eq!(r.phase, phase);
        assert!(r.after.is_none());
        assert_eq!(r.bytes(), b);
    }
    for (phase, reason) in [(0, 5), (1, 0), (2, 5), (3, 0), (4, 1), (5, 0)] {
        assert!(Record::decode(&record(&p, phase, reason, None, false), &p).is_err());
    }
    Ok(())
}
#[test]
fn truncation_bounds_booleans_unknown_links_and_duplicate_assignments_fail() -> Result<()> {
    let c = capture()?;
    for n in 0..c.bytes().len() {
        assert!(Capture::decode(&c.bytes()[..n]).is_err());
    }
    let mut b = c.bytes().to_vec();
    b.push(0);
    assert!(Capture::decode(&b).is_err());
    b.pop();
    let end = b.len() - 1;
    b[end] = 2;
    assert!(Capture::decode(&b).is_err());
    let mut c = c;
    c.links[0].kind = 99;
    reseal(&mut c);
    assert!(Capture::decode(c.bytes()).is_err());
    let p = plan()?;
    let b = record(&p, 0, 0, None, false);
    for n in 0..b.len() {
        assert!(Record::decode(&b[..n], &p).is_err());
    }
    assert!(Capture::decode(&vec![0; MAX_CAPTURE_BYTES + 1]).is_err());
    assert!(Record::decode(&vec![0; MAX_RECORD_BYTES + 1], &p).is_err());
    Ok(())
}

pub(crate) fn vector(name: &str) -> Result<Vec<u8>> {
    let prefix = format!("\"{name}\": \"");
    let hex = include_str!("../../../../tests/fixtures/manager_v1_22_component.json")
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix))
        .and_then(|s| s.split('"').next())
        .ok_or_else(|| {
            error(
                dfmcp_core::ErrorCode::AdapterRejected,
                "missing manager component vector",
            )
        })?;
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| {
            let s = std::str::from_utf8(v)
                .map_err(|_| error(dfmcp_core::ErrorCode::AdapterRejected, "vector UTF-8"))?;
            u8::from_str_radix(s, 16)
                .map_err(|_| error(dfmcp_core::ErrorCode::AdapterRejected, "vector hex"))
        })
        .collect()
}
pub(crate) fn native_capture() -> Result<Capture> {
    Capture::decode(&vector("capture")?)
}
pub(crate) fn native_plan() -> Result<Plan> {
    Plan::new("manager-001", native_capture()?)
}
#[test]
fn corrected_native_component_vectors_match_independent_rust_codec() -> Result<()> {
    let c = native_capture()?;
    assert!(c.eligible());
    let p = native_plan()?;
    assert_eq!(p.digest.as_bytes().as_slice(), vector("plan")?);
    assert_eq!(p.token.as_slice(), vector("token")?);
    assert_eq!(Record::decode(&vector("prepared")?, &p)?.phase, 0);
    let r = Record::decode(&vector("assigned")?, &p)?;
    assert!(r.complete());
    assert_eq!(r.after, Some(c.expected_after()?));
    assert_eq!(
        r.insertion.as_ref().map(|i| (i.assignment, i.event)),
        Some((600, 1992))
    );
    Ok(())
}
