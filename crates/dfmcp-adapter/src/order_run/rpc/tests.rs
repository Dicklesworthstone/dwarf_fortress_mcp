use super::*;
use crate::order_run::tests::{capture_bytes, context, plan, record_bytes};
use std::io::Cursor;

struct Script {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
    narrows: usize,
}
impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(3);
        self.input.read(&mut out[..n])
    }
}
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = bytes.len().min(5);
        self.output.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl OrderRunStream for Script {
    fn narrow_deadline(&mut self, _: Duration) -> Result<()> {
        self.narrows += 1;
        Ok(())
    }
}
fn frame(data: &[u8]) -> Vec<u8> {
    let mut out = header(-1, data.len() as i32).to_vec();
    out.extend_from_slice(data);
    out
}
fn reply(capture: Option<&[u8]>, record: Option<&[u8]>, active: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, n) in [
        (1, 1),
        (2, 0),
        (4, 1),
        (5, 14),
        (6, 41),
        (11, u64::from(active)),
        (12, u64::from(record.is_some())),
    ] {
        number(&mut out, tag, n);
    }
    bytes(&mut out, 3, &[b'n'; 16]);
    bytes(&mut out, 7, b"df");
    bytes(&mut out, 8, b"dfhack");
    if let Some(value) = capture {
        bytes(&mut out, 9, value);
    }
    if let Some(value) = record {
        bytes(&mut out, 10, value);
    }
    frame(&out)
}
fn script(replies: Vec<Vec<u8>>) -> Script {
    let mut input = b"DFHack!\n\x01\x00\x00\x00".to_vec();
    for id in 2..8 {
        let mut body = Vec::new();
        number(&mut body, 1, id);
        input.extend(frame(&body));
    }
    input.extend(reply(None, None, false));
    for response in replies {
        input.extend(response);
    }
    Script {
        input: Cursor::new(input),
        output: Vec::new(),
        narrows: 0,
    }
}
#[test]
fn fixed_bindings_fragmentation_and_complete_lifecycle() -> Result<()> {
    let p = plan()?;
    let c = context()?;
    let prepared = record_bytes(&p, 0, 0, 0, None, 0);
    let running = record_bytes(&p, 1, 0, 0, None, 0);
    let mut client = OrderRunRpc::negotiate(
        script(vec![
            reply(Some(p.before().canonical_bytes()), None, false),
            reply(None, Some(&prepared), false),
            reply(None, Some(&running), true),
            reply(None, None, false),
        ]),
        vec![b't'; 32],
        vec![b'n'; 16],
        p.before().fortress().clone(),
        &c,
    )?;
    assert_eq!(client.observe(9, &c, Duration::from_secs(1))?, *p.before());
    assert_eq!(
        client.prepare(&p, &c, Duration::from_secs(1))?.phase(),
        RunPhase::Prepared
    );
    assert_eq!(
        client.commit(&p, &c, Duration::from_secs(1))?.phase(),
        RunPhase::Running
    );
    assert!(client.query(&p, &c, Duration::from_secs(1))?.is_none());
    for method in METHODS {
        assert!(
            client
                .stream
                .output
                .windows(method.len())
                .any(|w| w == method.as_bytes())
        );
    }
    assert_eq!(client.endpoint(), None);
    assert!(!client.poisoned());
    Ok(())
}
#[test]
fn lost_commit_reply_fences_without_redispatch() -> Result<()> {
    let p = plan()?;
    let c = context()?;
    let mut client = OrderRunRpc::negotiate(
        script(vec![header(-1, 100).to_vec()]),
        vec![b't'; 32],
        vec![b'n'; 16],
        p.before().fortress().clone(),
        &c,
    )?;
    assert!(client.commit(&p, &c, Duration::from_secs(1)).is_err());
    assert!(client.poisoned());
    let size = client.stream.output.len();
    assert!(client.commit(&p, &c, Duration::from_secs(1)).is_err());
    assert_eq!(client.stream.output.len(), size);
    Ok(())
}
#[test]
fn observation_rejects_replacement_fortress_and_expired_authority() -> Result<()> {
    let p = plan()?;
    let c = context()?;
    for (folder, tick) in [("region2", 100), ("region1", 1000)] {
        let capture = capture_bytes(folder, 3, tick, true, 0);
        let mut limited = c.clone();
        for grant in &mut limited.grants {
            grant.expires_at_tick = Some(GameTick(200));
        }
        let mut client = OrderRunRpc::negotiate(
            script(vec![reply(Some(&capture), None, false)]),
            vec![b't'; 32],
            vec![b'n'; 16],
            p.before().fortress().clone(),
            &limited,
        )?;
        assert!(client.observe(9, &limited, Duration::from_secs(1)).is_err());
        assert!(client.poisoned());
    }
    Ok(())
}
#[test]
fn malformed_protobuf_notifications_and_endpoint_are_refused() -> Result<()> {
    assert!(Message::parse(&[8, 1, 8, 1], 12).is_err());
    assert!(Message::parse(&[8, 128, 0], 12).is_err());
    assert!(Message::parse(&[104, 1], 12).is_err());
    let mut input = Vec::new();
    for _ in 0..9 {
        input.extend(header(-3, 0));
    }
    let mut s = Script {
        input: Cursor::new(input),
        output: Vec::new(),
        narrows: 0,
    };
    assert!(call(&mut s, 2, &[]).is_err());
    let c = context()?;
    assert!(
        OrderRunRpc::connect(
            SocketAddr::from(([192, 0, 2, 1], 5000)),
            vec![b't'; 32],
            vec![b'n'; 16],
            FortressIdentity::new("region1", 7)?,
            &c
        )
        .is_err()
    );
    assert!(deadline_bound(Duration::ZERO).is_err());
    Ok(())
}
#[test]
fn insufficient_budget_or_authority_is_pre_io() -> Result<()> {
    let p = plan()?;
    let c = context()?;
    let mut client = OrderRunRpc::negotiate(
        script(vec![]),
        vec![b't'; 32],
        vec![b'n'; 16],
        p.before().fortress().clone(),
        &c,
    )?;
    let n = client.stream.output.len();
    let mut narrow = c.clone();
    narrow.grants.retain(|g| g.capability == Capability::Query);
    assert!(client.commit(&p, &narrow, Duration::from_secs(1)).is_err());
    narrow = c.clone();
    narrow.budget.max_bytes = 1;
    assert!(client.observe(9, &narrow, Duration::from_secs(1)).is_err());
    assert_eq!(client.stream.output.len(), n);
    assert!(!client.poisoned());
    Ok(())
}
