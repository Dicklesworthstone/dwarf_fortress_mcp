use super::*;
use std::io::Cursor;

struct Script {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}
impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.input.read(out)
    }
}
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn reply(known: bool, applied: bool) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 1);
    number(&mut out, 2, 0);
    bytes(&mut out, 3, &[b'n'; 16]);
    number(&mut out, 4, 1);
    number(&mut out, 5, 7);
    number(&mut out, 6, 9);
    bytes(&mut out, 7, b"df");
    bytes(&mut out, 8, b"dfhack");
    if known {
        number(&mut out, 10, 1);
        number(&mut out, 11, u64::from(applied));
        number(&mut out, 12, 0);
        number(&mut out, 13, 42);
        if applied {
            bytes(&mut out, 14, &[7; 32]);
        }
    }
    out
}
fn omit(input: &[u8], absent: u32) -> Result<Vec<u8>> {
    let message = Message::parse(input, 14)?;
    let mut out = Vec::new();
    for (field, value) in message.0 {
        if field != absent {
            match value {
                Field::Number(value) => number(&mut out, field, value),
                Field::Bytes(value) => bytes(&mut out, field, value),
            }
        }
    }
    Ok(out)
}
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = header(-1, payload.len() as i32).to_vec();
    out.extend_from_slice(payload);
    out
}
fn client(tail: Vec<u8>) -> Result<ControlRpcClient<Script>> {
    let mut input = b"DFHack!\n".to_vec();
    input.extend_from_slice(&1i32.to_le_bytes());
    for id in [2, 3, 4, 5] {
        let mut binding = Vec::new();
        number(&mut binding, 1, id);
        input.extend(frame(&binding));
    }
    input.extend(frame(&reply(false, false)));
    input.extend(tail);
    ControlRpcClient::negotiate(
        Script {
            input: Cursor::new(input),
            output: Vec::new(),
        },
        vec![b't'; 32],
        vec![b'n'; 16],
    )
}

#[test]
fn known_effect_requires_explicit_outcome_pause_and_tick() -> Result<()> {
    for applied in [false, true] {
        let complete = reply(true, applied);
        assert!(decode(&complete, &[b'n'; 16]).is_ok());
        for missing in [11, 12, 13] {
            assert!(decode(&omit(&complete, missing)?, &[b'n'; 16]).is_err());
        }
    }
    Ok(())
}

#[test]
fn missing_receipt_is_ambiguous_not_a_terminal_proof() -> Result<()> {
    let (_, pending) = decode(&reply(true, false), &[b'n'; 16])?;
    assert!(pending.known);
    assert!(!pending.applied);
    assert!(pending.receipt_digest.is_empty());
    let missing_receipt = omit(&reply(true, true), 14)?;
    assert!(decode(&missing_receipt, &[b'n'; 16]).is_err());
    Ok(())
}

#[test]
fn present_tokens_and_receipts_have_exact_lengths() {
    for (field, exact) in [(9, 16), (14, 32)] {
        for length in [0, 1, exact - 1, exact + 1] {
            let mut message = reply(true, false);
            bytes(&mut message, field, &vec![7; length]);
            assert!(decode(&message, &[b'n'; 16]).is_err());
        }
        let mut valid = reply(true, false);
        bytes(&mut valid, field, &vec![7; exact]);
        assert!(decode(&valid, &[b'n'; 16]).is_ok());
    }
}

#[test]
fn oversized_reply_fences_before_a_followup_can_consume_unread_bytes() -> Result<()> {
    let mut tail = header(-1, 4097).to_vec();
    tail.extend(vec![0; 4097]);
    tail.extend(frame(&reply(true, true)));
    let mut client = client(tail)?;
    assert!(
        matches!(client.query_pause("k", Digest32::ZERO), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    assert!(client.poisoned());
    let output = client.stream.output.len();
    let position = client.stream.input.position();
    assert!(
        matches!(client.query_pause("k", Digest32::ZERO), Err(e) if e.code == ErrorCode::AdapterUnavailable)
    );
    assert_eq!(client.stream.output.len(), output);
    assert_eq!(client.stream.input.position(), position);
    Ok(())
}

#[test]
fn incomplete_outcome_fences_the_connection_without_synthesizing_false() -> Result<()> {
    let malformed = omit(&reply(true, true), 12)?;
    let mut client = client(frame(&malformed))?;
    assert!(
        matches!(client.query_pause("k", Digest32::ZERO), Err(e) if e.code == ErrorCode::AdapterRejected)
    );
    assert!(client.poisoned());
    Ok(())
}

#[test]
fn local_key_validation_neither_writes_nor_poison_the_connection() -> Result<()> {
    let mut client = client(Vec::new())?;
    let output = client.stream.output.len();
    assert!(
        matches!(client.query_pause("", Digest32::ZERO), Err(e) if e.code == ErrorCode::InvalidRequest)
    );
    assert_eq!(client.stream.output.len(), output);
    assert!(!client.poisoned());
    Ok(())
}

#[test]
fn connection_and_handshake_share_one_timeout_budget() -> Result<()> {
    let total = Duration::from_millis(500);
    assert_eq!(
        remaining_timeout(total, Duration::from_millis(400))?,
        Duration::from_millis(100)
    );
    assert_eq!(
        remaining_timeout(total, Duration::from_millis(499))?,
        Duration::from_millis(1)
    );
    for elapsed in [
        Duration::from_micros(499_001),
        total,
        Duration::from_millis(501),
    ] {
        assert!(
            matches!(remaining_timeout(total, elapsed), Err(e) if e.code == ErrorCode::BudgetExceeded)
        );
    }
    Ok(())
}
