use super::*;

fn queue(sequence: u64, next: u32, ids: &[u32]) -> Result<WorkOrderObservation> {
    let mut bytes = b"DFMWO010".to_vec();
    for number in [53_u64, sequence, 120] {
        bytes.extend_from_slice(&number.to_be_bytes());
    }
    bytes.extend_from_slice(&next.to_be_bytes());
    bytes.extend_from_slice(&4_u32.to_be_bytes());
    bytes.push(1);
    let folder = b"region1";
    bytes.extend_from_slice(&(folder.len() as u16).to_be_bytes());
    bytes.extend_from_slice(folder);
    bytes.extend_from_slice(&(ids.len() as u32).to_be_bytes());
    for id in ids {
        bytes.extend_from_slice(&id.to_be_bytes());
    }
    WorkOrderObservation::decode(&bytes)
}

#[test]
fn stock_targets_queue_prefix_requires_exact_membership_allocator_and_sequence() -> Result<()> {
    let (_, c) = super::super::stock_target_request::tests::stock_fixture()?;
    let expected = queue(12, 101, &[100])?;
    let prefix = ReceiptPrefix {
        session: c.session_id, request: c.request_id, step: StepId::new(1),
        ids: vec![100], after: Some(expected.witness()),
    };
    assert!(prefix.matches(&expected, &c));
    for drift in [
        queue(13, 101, &[100])?,
        queue(12, 102, &[100])?,
        queue(13, 102, &[100, 101])?,
        queue(13, 101, &[])?,
        queue(13, 102, &[101])?,
    ] {
        assert!(!prefix.matches(&drift, &c), "queue drift passed original receipt prefix");
    }
    for (at, value) in [(8, 54_u64), (24, 121_u64)] {
        let mut changed = expected.canonical_bytes().to_vec();
        changed[at..at + 8].copy_from_slice(&value.to_be_bytes());
        assert!(!prefix.matches(&WorkOrderObservation::decode(&changed)?, &c));
    }
    let mut another_call = c.clone();
    another_call.request_id = RequestId::new(c.request_id.get() + 1);
    assert!(!prefix.matches(&expected, &another_call));
    another_call = c.clone();
    another_call.session_id = SessionId::new(c.session_id.get() + 1);
    assert!(!prefix.matches(&expected, &another_call));
    Ok(())
}

#[test]
fn stock_targets_initial_prefix_never_accepts_unclassified_manager_orders() -> Result<()> {
    let (_, c) = super::super::stock_target_request::tests::stock_fixture()?;
    let prefix = ReceiptPrefix {
        session: c.session_id, request: c.request_id, step: StepId::ZERO,
        ids: Vec::new(), after: None,
    };
    assert!(prefix.matches(&queue(11, 100, &[])?, &c));
    assert!(!prefix.matches(&queue(11, 100, &[99])?, &c));
    let expected = queue(13, 102, &[100, 101])?;
    let prefix = ReceiptPrefix {
        session: c.session_id, request: c.request_id, step: StepId::new(2),
        ids: vec![100, 101], after: Some(expected.witness()),
    };
    assert!(prefix.matches(&expected, &c));
    assert!(!prefix.matches(&queue(13, 102, &[101])?, &c));
    assert!(!prefix.matches(&queue(13, 103, &[100, 102])?, &c));
    Ok(())
}
