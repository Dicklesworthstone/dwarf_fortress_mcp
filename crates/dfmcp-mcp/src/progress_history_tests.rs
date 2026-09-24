use super::*;
use dfmcp_adapter::work_order_progress::archive::ArchiveMode;
use dfmcp_adapter::work_order_progress::{ProgressManifest, ProgressObservation};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, RiskTier,
    StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn sample(sequence: u64, tick: u64, remaining: i32) -> Result<ProgressObservation> {
    let text =
        include_str!("../../dfmcp-adapter/tests/fixtures/work_order_progress_v1_12.hex").trim();
    let mut bytes = text
        .as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let s = std::str::from_utf8(p)
                .map_err(|_| error(ErrorCode::InvalidRequest, "test UTF-8"))?;
            u8::from_str_radix(s, 16).map_err(|_| error(ErrorCode::InvalidRequest, "test hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    bytes[16..24].copy_from_slice(&sequence.to_be_bytes());
    bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    bytes[68..72].copy_from_slice(&remaining.to_be_bytes());
    ProgressObservation::decode(&bytes, &[3, 8])
}
fn context() -> Result<OperationContext> {
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: sample(1, 10, 5)?.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_bytes: 2 * 1024 * 1024,
            max_entities: 4096,
            max_output_tokens: 65536,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn manifest() -> ProgressManifest {
    ProgressManifest {
        generation: 7,
        df_version: "df".into(),
        dfhack_version: "dfhack".into(),
    }
}
struct Memory {
    bytes: Rc<RefCell<Vec<u8>>>,
    position: usize,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.bytes.borrow();
        let n = out.len().min(bytes.len().saturating_sub(self.position));
        if n > 0 {
            out[..n].copy_from_slice(&bytes[self.position..self.position + n]);
        }
        self.position += n;
        Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut b = self.bytes.borrow_mut();
        let n = b.len().max(self.position + data.len());
        b.resize(n, 0);
        b[self.position..self.position + data.len()].copy_from_slice(data);
        self.position += data.len();
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
            SeekFrom::End(n) => self.bytes.borrow().len() as i128 + i128::from(n),
        };
        self.position = usize::try_from(next).map_err(|_| io::Error::other("test seek"))?;
        Ok(self.position as u64)
    }
}
impl JournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("test forbids repair"))
    }
}
fn setup() -> Result<(
    ProgressArchive<Memory>,
    Rc<RefCell<Vec<u8>>>,
    OperationContext,
)> {
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let c = context()?;
    let mut archive = ProgressArchive::open(
        Memory {
            bytes: bytes.clone(),
            position: 0,
        },
        ArchiveMode::Live,
        true,
        &c,
    )?;
    for (sequence, remaining) in [(1, 5), (2, 2), (3, 0)] {
        archive.append(
            &manifest(),
            &sample(sequence, sequence * 10, remaining)?,
            &c,
        )?;
    }
    Ok((archive, bytes, c))
}
fn text_field(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "missing test field"))
}
fn selection(
    a: &mut ProgressArchive<Memory>,
    c: &OperationContext,
) -> Result<Vec<ProgressArchiveEntry>> {
    let s = a.summary(c)?;
    Ok(a.page(s.head, 0, 64, c)?.entries)
}
#[test]
fn typed_history_requests_reject_ambiguous_unbounded_and_effect_arguments() -> Result<()> {
    let hash = "a".repeat(64);
    for value in [
        json!({"mode":"list"}),
        json!({"mode":"list","limit":64,"continuation":hash}),
        json!({"mode":"record","archive_id":hash,"number":1,"record_digest":hash}),
        json!({"mode":"changes","archive_id":hash,"before_number":1,"before_digest":hash,"after_number":2,"after_digest":hash}),
    ] {
        assert!(Request::parse(&value.to_string()).is_ok());
    }
    for raw in [
        "{}",
        "[]",
        "null",
        "{\"mode\":\"list\",\"limit\":0}",
        "{\"mode\":\"list\",\"limit\":65}",
        "{\"mode\":\"list\",\"limit\":1.5}",
        "{\"mode\":\"list\",\"limit\":2,\"limit\":3}",
        "{\"mode\":\"list\",\"mode\":\"record\"}",
        "{\"mode\":\"list\",\"path\":\"/tmp/x\"}",
        "{\"mode\":\"commit\"}",
        "{\"mode\":\"list\",\"continuation\":\"1\"}",
        "{\"mode\":\"list\"}x",
    ] {
        assert!(Request::parse(raw).is_err(), "{raw}");
    }
    assert!(Request::parse(&" ".repeat(2049)).is_err());
    for (before, after) in [(0, 1), (1, 1), (2, 1)] {
        assert!(
            Request::parse(
                &json!({"mode":"changes","archive_id":hash,"before_number":before,
            "before_digest":hash,"after_number":after,"after_digest":hash})
                .to_string()
            )
            .is_err()
        );
    }
    Ok(())
}
#[test]
fn issued_cursor_pages_retry_identically_and_bind_all_identity_dimensions() -> Result<()> {
    let (mut archive, bytes, c) = setup()?;
    let before = bytes.borrow().clone();
    let mut cursors = Cursors::default();
    let first = query(
        &mut archive,
        &mut cursors,
        Request::List {
            limit: Some(1),
            continuation: None,
        },
        &c,
    )?;
    let token = text_field(&first.value, "continuation")?;
    assert_eq!(first.value["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(first.value["complete_set_in_this_response"], false);
    let retry = query(
        &mut archive,
        &mut cursors,
        Request::List {
            limit: Some(1),
            continuation: None,
        },
        &c,
    )?;
    assert_eq!(first.value, retry.value);
    let second = query(
        &mut archive,
        &mut cursors,
        Request::List {
            limit: Some(1),
            continuation: Some(token.clone()),
        },
        &c,
    )?;
    assert_eq!(second.value["entries"][0]["record_number"], 2);
    let summary = archive.summary(&c)?;
    assert!(
        cursors
            .resolve(&token, SessionId::new(2), &summary, 1)
            .is_err()
    );
    assert!(cursors.resolve(&token, c.session_id, &summary, 2).is_err());
    let mut other = summary.clone();
    other.archive_id = Digest32::ZERO;
    assert!(cursors.resolve(&token, c.session_id, &other, 1).is_err());
    other = summary.clone();
    other.head = Digest32::ZERO;
    assert!(cursors.resolve(&token, c.session_id, &other, 1).is_err());
    assert!(
        cursors
            .resolve(&"0".repeat(64), c.session_id, &summary, 1)
            .is_err()
    );
    for after in 4..=68 {
        cursors.issue(c.session_id, &summary, after, 1);
    }
    assert!(cursors.resolve(&token, c.session_id, &summary, 1).is_err());
    assert_eq!(cursors.retained.len(), 64);
    assert_eq!(*bytes.borrow(), before);
    Ok(())
}
#[test]
fn restart_preserves_exact_record_refs_but_not_cursors_or_cross_segment_comparisons() -> Result<()>
{
    let (mut archive, bytes, c) = setup()?;
    let entries = selection(&mut archive, &c)?;
    let summary = archive.summary(&c)?;
    let mut cursors = Cursors::default();
    let token = cursors.issue(c.session_id, &summary, 1, 1);
    drop(archive);
    let mut recovered =
        ProgressArchive::open(Memory { bytes, position: 0 }, ArchiveMode::Live, false, &c)?;
    let mut cursors = Cursors::default();
    assert!(cursors.resolve(&token, c.session_id, &summary, 1).is_err());
    let exact = query(
        &mut recovered,
        &mut cursors,
        Request::Record {
            archive_id: summary.archive_id.to_string(),
            number: entries[0].number,
            record_digest: entries[0].record_digest.to_string(),
        },
        &c,
    )?;
    assert_eq!(exact.value["record"]["observation"]["game_tick"], 10);
    assert_eq!(exact.value["historical"], true);
    let new = recovered.append(&manifest(), &sample(4, 40, 0)?, &c)?;
    assert!(
        query(
            &mut recovered,
            &mut cursors,
            Request::Changes {
                archive_id: summary.archive_id.to_string(),
                before_number: 1,
                before_digest: entries[0].record_digest.to_string(),
                after_number: new.number,
                after_digest: new.record_digest.to_string()
            },
            &c
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn historical_comparisons_return_verified_endpoints_not_goods_completion() -> Result<()> {
    let (mut archive, bytes, c) = setup()?;
    let entries = selection(&mut archive, &c)?;
    let summary = archive.summary(&c)?;
    let before = bytes.borrow().clone();
    let answer = query(
        &mut archive,
        &mut Cursors::default(),
        Request::Changes {
            archive_id: summary.archive_id.to_string(),
            before_number: 1,
            before_digest: entries[0].record_digest.to_string(),
            after_number: 3,
            after_digest: entries[2].record_digest.to_string(),
        },
        &c,
    )?;
    assert_eq!(
        answer.value["comparison"]["changes"][0]["remaining_counter_decrease"],
        5
    );
    assert_eq!(
        answer.value["comparison"]["changes"][0]["goods_produced_proven"],
        false
    );
    assert_eq!(
        answer.value["after"]["observation"]["rows"][0]["production_completion_proven"],
        false
    );
    assert_eq!(
        answer.capture.as_ref().map(ProgressObservation::tick),
        Some(30)
    );
    assert_eq!(answer.value["native_calls"], 0);
    assert_eq!(*bytes.borrow(), before);
    Ok(())
}
#[test]
fn authority_budget_and_same_size_corruption_are_enforced_in_actual_history_dispatch() -> Result<()>
{
    let (mut archive, bytes, c) = setup()?;
    let summary = archive.summary(&c)?;
    let entries = selection(&mut archive, &c)?;
    let request = || Request::Record {
        archive_id: summary.archive_id.to_string(),
        number: 1,
        record_digest: entries[0].record_digest.to_string(),
    };
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(query(&mut archive, &mut Cursors::default(), request(), &denied).is_err());
    denied = c.clone();
    for g in &mut denied.grants {
        g.expires_at_tick = Some(GameTick(29));
    }
    assert!(query(&mut archive, &mut Cursors::default(), request(), &denied).is_err());
    denied = c.clone();
    denied.budget.max_bytes = 1;
    assert!(query(&mut archive, &mut Cursors::default(), request(), &denied).is_err());
    assert!(
        query(
            &mut archive,
            &mut Cursors::default(),
            Request::Record {
                archive_id: Digest32::ZERO.to_string(),
                number: 1,
                record_digest: entries[0].record_digest.to_string()
            },
            &c
        )
        .is_err()
    );
    // The first record starts immediately after the 80-byte header. Alter its body without extending the file.
    bytes.borrow_mut()[80 + 52 + 10] ^= 1;
    assert!(query(&mut archive, &mut Cursors::default(), request(), &c).is_err());
    assert!(archive.summary(&c).is_err());
    Ok(())
}
