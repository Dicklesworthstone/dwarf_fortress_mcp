//! Cache-bearing native vectors and custody. These are component doubles, not DF.
use super::*;
fn vector23(name: &str) -> Result<Vec<u8>> {
    let prefix = format!("\"{name}\": \"");
    let text = include_str!("../../../../tests/fixtures/manager_v1_23_component.json")
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.split('"').next())
        .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing Manager/1.23 fixture"))?;
    require(text.len().is_multiple_of(2), "Manager/1.23 fixture hex")?;
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| error(ErrorCode::AdapterRejected, "fixture UTF-8"))?;
            u8::from_str_radix(pair, 16)
                .map_err(|_| error(ErrorCode::AdapterRejected, "fixture hex"))
        })
        .collect()
}
fn plan23() -> Result<Plan> {
    Plan::new("manager-001", Capture::decode(&vector23("capture")?)?)
}
#[test]
fn manager_v1_23_native_vectors_require_cache_in_full_transaction() -> Result<()> {
    let p = plan23()?;
    assert_eq!(p.before.protocol, ManagerProtocol::V1_23);
    assert!(p.before.eligible());
    assert_eq!(p.digest.as_bytes().as_slice(), vector23("plan")?);
    assert_eq!(p.token.as_slice(), vector23("token")?);
    assert_eq!(Plan::decode(&p.encode())?, p);
    let prepared = Record::decode(&vector23("prepared")?, &p)?;
    assert_eq!(prepared.phase, 0);
    let done = Record::decode(&vector23("assigned")?, &p)?;
    assert!(done.complete());
    let after = done
        .after
        .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing after"))?;
    assert_eq!(after, p.before.expected_after()?);
    assert_eq!(
        after.group.role_buckets[MANAGE_PRODUCTION],
        vec![
            p.before
                .target()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing target"))?
                .1
                .id
        ]
    );
    for i in 0..p.before.group.role_buckets.len() {
        if i != MANAGE_PRODUCTION {
            assert_eq!(after.group.role_buckets[i], p.before.group.role_buckets[i]);
        }
    }
    assert_eq!(after.civ.role_buckets, p.before.civ.role_buckets);
    Ok(())
}
#[test]
fn manager_v1_23_rejects_duplicate_foreign_and_mixed_role_domains() -> Result<()> {
    let p = plan23()?;
    let id = p.before.group.assignments[0].id;
    for kind in 0..4 {
        let mut c = p.before.clone();
        match kind {
            0 => c.group.role_buckets[0] = vec![id, id],
            1 => c.group.role_buckets[0] = vec![i32::MAX],
            2 => {
                c.civ.role_buckets.pop();
            }
            _ => c.group.role_buckets.resize(MAX_ROLE_BUCKETS + 1, vec![]),
        }
        assert!(Capture::decode(&c.encode()).is_err());
    }
    let mut occupied = p.before.clone();
    occupied.group.role_buckets[MANAGE_PRODUCTION].push(id);
    let observed = Capture::decode(&occupied.encode())?;
    assert!(!observed.eligible());
    assert!(Plan::new("cannot-borrow-cache", observed).is_err());
    Ok(())
}
#[test]
fn manager_v1_23_missing_cache_cannot_be_certified_by_rehashed_receipt() -> Result<()> {
    let p = plan23()?;
    let raw = vector23("assigned")?;
    let done = Record::decode(&raw, &p)?;
    let after = done
        .after
        .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing after"))?;
    let start = raw
        .windows(after.bytes().len())
        .position(|v| v == after.bytes())
        .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing after bytes"))?;
    require(start >= 4, "after length missing")?;
    let mut incomplete = after.clone();
    incomplete.group.role_buckets[MANAGE_PRODUCTION].clear();
    let bytes = incomplete.encode();
    assert!(Capture::decode(&bytes).is_ok());
    let mut forged = raw[..start - 4].to_vec();
    blob(&mut forged, &bytes);
    forged.extend_from_slice(&raw[start + after.bytes().len()..raw.len() - 32]);
    let receipt = hash(b"dfmcp-manager-receipt/1", &forged);
    forged.extend_from_slice(receipt.as_bytes());
    assert!(Record::decode(&forged, &p).is_err());
    Ok(())
}
#[test]
fn manager_v1_23_binding_and_original_reject_legacy_generation() -> Result<()> {
    let p = plan23()?;
    let mut b = journal::journal_tests::binding(
        "127.0.0.1:5000"
            .parse()
            .map_err(|_| error(ErrorCode::AdapterRejected, "endpoint"))?,
    )?;
    assert!(b.capture(&p.before).is_err());
    b.protocol = ManagerProtocol::V1_23;
    b.capture(&p.before)?;
    let legacy = tests::native_plan()?;
    assert!(b.capture(&legacy.before).is_err());
    assert!(Record::decode(&vector23("assigned")?, &legacy).is_err());
    let o = journal::Original::new(p.clone(), p.before.tick + 12000)?;
    assert_eq!(journal::Original::decode(&o.encode())?, o);
    let mut mixed = o.encode();
    mixed[..8].copy_from_slice(b"DFMMO022");
    assert!(journal::Original::decode(&mixed).is_err());
    Ok(())
}
#[test]
fn manager_v1_23_private_ack_and_journal_cannot_reopen_as_legacy() -> Result<()> {
    use journal::{Mode, Original, open_private};
    let d = journal::journal_tests::Directory::new()?;
    let c = journal::journal_tests::context()?;
    let mut b = journal::journal_tests::binding(
        "127.0.0.1:5000"
            .parse()
            .map_err(|_| error(ErrorCode::AdapterRejected, "endpoint"))?,
    )?;
    b.protocol = ManagerProtocol::V1_23;
    let (mut j, mut ack) = open_private(
        &d.0,
        &c,
        Mode::Control,
        &b,
        b.selection,
        true,
        None,
        [7; 32],
    )?;
    let p = plan23()?;
    let o = Original::new(p.clone(), p.before.tick + 12000)?;
    j.intent(o.clone(), &c, &mut ack)?;
    j.prepared(Record::decode(&vector23("prepared")?, &p)?, &c, &mut ack)?;
    let mut calls = 0;
    j.dispatch(o.seal(), &c, &mut ack, &mut |_| Ok(()), |_, _| {
        calls += 1;
        Record::decode(&vector23("assigned")?, &p)
    })?;
    assert_eq!(calls, 1);
    let pin = ack
        .latest()
        .ok_or_else(|| error(ErrorCode::CorruptLedger, "missing ACK"))?;
    drop(j);
    drop(ack);
    let journal_bytes = std::fs::read(d.0.join(b.protocol.journal_name()))
        .map_err(|_| error(ErrorCode::CorruptLedger, "read"))?;
    let ack_bytes = std::fs::read(d.0.join(b.protocol.ack_name()))
        .map_err(|_| error(ErrorCode::CorruptLedger, "read"))?;
    assert_eq!(&journal_bytes[..8], b"DFMMJ023");
    assert_eq!(&ack_bytes[..8], b"DFMMA023");
    let (j, ack) = open_private(
        &d.0,
        &c,
        Mode::Offline,
        &b,
        b.selection,
        false,
        Some(pin),
        [0; 32],
    )?;
    assert!(!j.dispatchable());
    assert_eq!(j.original, Some(o));
    drop(j);
    drop(ack);
    std::fs::copy(
        d.0.join(b.protocol.journal_name()),
        d.0.join("manager-appointment.journal"),
    )
    .map_err(|_| error(ErrorCode::CorruptLedger, "copy"))?;
    std::fs::copy(
        d.0.join(b.protocol.ack_name()),
        d.0.join("manager-appointment.ack"),
    )
    .map_err(|_| error(ErrorCode::CorruptLedger, "copy"))?;
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(d.0.join("manager-appointment.ack"))
            .map_err(|_| error(ErrorCode::CorruptLedger, "metadata"))?
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
    b.protocol = ManagerProtocol::V1_22;
    assert!(
        open_private(
            &d.0,
            &c,
            Mode::Offline,
            &b,
            b.selection,
            false,
            Some(pin),
            [0; 32]
        )
        .is_err()
    );
    Ok(())
}
