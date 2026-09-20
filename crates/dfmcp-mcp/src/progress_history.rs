//! Typed, bounded history reads; no native call or mutation edge exists here.
use std::collections::VecDeque;
use serde::Deserialize;
use dfmcp_adapter::operations_journal::JournalStorage;
use dfmcp_adapter::work_order_progress::archive::{ArchivedProgress, ProgressArchive,
    ProgressArchiveEntry, ProgressArchiveSummary};
use super::{Digest32, ErrorCode, OperationContext, ProgressComparison, ProgressObservation,
    Result, SessionId, Value, comparison_json, error, json, observation_json, parse_digest};

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    List { limit: Option<u32>, continuation: Option<String> },
    Record { archive_id: String, number: u64, record_digest: String },
    Changes { archive_id: String, before_number: u64, before_digest: String, after_number: u64, after_digest: String },
}
impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 { return Err(error(ErrorCode::InvalidRequest, "progress history request exceeds 2048 bytes")); }
        let request: Self = serde_json::from_str(raw).map_err(|_| error(ErrorCode::InvalidRequest, "invalid or unknown progress history request fields"))?;
        match &request {
            Self::List { limit, continuation } => {
                if !matches!(limit, None | Some(1..=64)) { return Err(error(ErrorCode::InvalidRequest, "history page limit must be 1..64")); }
                if let Some(token) = continuation { parse_digest(token)?; }
            }
            Self::Record { archive_id, number, record_digest } => {
                parse_digest(archive_id)?; parse_digest(record_digest)?;
                if *number == 0 { return Err(error(ErrorCode::InvalidRequest, "history record numbers start at one")); }
            }
            Self::Changes { archive_id, before_number, before_digest, after_number, after_digest } => {
                parse_digest(archive_id)?; parse_digest(before_digest)?; parse_digest(after_digest)?;
                if *before_number == 0 || before_number >= after_number {
                    return Err(error(ErrorCode::InvalidRequest, "history endpoints must be positive, ordered and distinct"));
                }
            }
        }
        Ok(request)
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Cursor {
    session: SessionId, archive: Digest32, head: Digest32, after: u64, limit: usize, token: String,
}
#[derive(Default)]
pub struct Cursors { retained: VecDeque<Cursor> }
impl Cursors {
    fn issue(&mut self, session: SessionId, summary: &ProgressArchiveSummary, after: u64, limit: usize) -> String {
        let mut bytes = b"dfmcp-progress-history-cursor/1\0".to_vec();
        bytes.extend_from_slice(&session.get().to_be_bytes()); bytes.extend_from_slice(summary.archive_id.as_bytes());
        bytes.extend_from_slice(summary.head.as_bytes()); bytes.extend_from_slice(&after.to_be_bytes());
        bytes.extend_from_slice(&(limit as u64).to_be_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.retained.iter().any(|c| c.token == token) { return token; }
        if self.retained.len() == 64 { self.retained.pop_front(); }
        self.retained.push_back(Cursor { session, archive: summary.archive_id, head: summary.head, after, limit, token: token.clone() });
        token
    }
    fn resolve(&self, token: &str, session: SessionId, summary: &ProgressArchiveSummary, limit: usize) -> Result<u64> {
        let cursor = self.retained.iter().find(|c| c.token == token)
            .ok_or_else(|| error(ErrorCode::StaleAnchor, "history continuation is unknown or evicted; restart discovery"))?;
        if cursor.session != session || cursor.archive != summary.archive_id || cursor.head != summary.head || cursor.limit != limit {
            return Err(error(ErrorCode::StaleAnchor, "history cursor belongs to another session, archive head or page size"));
        }
        Ok(cursor.after)
    }
}
pub fn summary_json(s: &ProgressArchiveSummary) -> Value {
    json!({"archive_id":s.archive_id.to_string(),"head":s.head.to_string(),"fortress_id":s.fortress_id.to_string(),
        "records":s.records,"comparison_segments":s.segments,"retained_bytes":s.retained_bytes,
        "authority_tick_floor":s.authority_tick_floor,"read_only":s.read_only,
        "continuous_history_proven":false,"external_anti_rollback_floor":false})
}
pub fn entry_json(e: &ProgressArchiveEntry, archive: Digest32) -> Value {
    json!({"archive_id":archive.to_string(),"record_number":e.number,"segment":e.segment,
        "record_digest":e.record_digest.to_string(),"previous_digest":e.previous_digest.to_string(),
        "observation_witness":e.witness.to_string(),"game_tick":e.game_tick,"native_order_ids":e.native_order_ids})
}
fn record_json(r: &ArchivedProgress, archive: Digest32) -> Value {
    json!({"reference":entry_json(&r.entry, archive),"source_manifest":{"generation":r.manifest.generation,
        "df_version":r.manifest.df_version,"dfhack_version":r.manifest.dfhack_version},
        "observation":observation_json(&r.observation),"historical":true,"current_freshness_proven":false})
}
pub struct Answer {
    pub value: Value,
    pub capture: Option<ProgressObservation>,
    pub comparison: Option<ProgressComparison>,
}
pub fn query<S: JournalStorage>(archive: &mut ProgressArchive<S>, cursors: &mut Cursors,
    request: Request, c: &OperationContext) -> Result<Answer>
{
    let summary = archive.summary(c)?;
    match request {
        Request::List { limit, continuation } => {
            let limit = limit.unwrap_or(8) as usize;
            let after = continuation.as_deref().map(|v| cursors.resolve(v, c.session_id, &summary, limit)).transpose()?.unwrap_or(0);
            let page = archive.page(summary.head, after, limit, c)?;
            let next = page.next_after.map(|after| cursors.issue(c.session_id, &summary, after, limit));
            let entries = page.entries.iter().map(|e| entry_json(e, summary.archive_id)).collect::<Vec<_>>();
            Ok(Answer { value: json!({"ok":true,"historical":true,"native_calls":0,"entries":entries,
                "total_records":summary.records,"complete_set_in_this_response":after == 0 && next.is_none(),
                "continuation":next,"progress_archive":summary_json(&summary)}), capture:None, comparison:None })
        }
        Request::Record { archive_id, number, record_digest } => {
            if parse_digest(&archive_id)? != summary.archive_id { return Err(error(ErrorCode::StaleAnchor, "history reference names a different archive")); }
            let record = archive.record(number, parse_digest(&record_digest)?, c)?;
            let value = json!({"ok":true,"historical":true,"native_calls":0,"record":record_json(&record, summary.archive_id),
                "progress_archive":summary_json(&summary)});
            Ok(Answer { value, capture:Some(record.observation), comparison:None })
        }
        Request::Changes { archive_id, before_number, before_digest, after_number, after_digest } => {
            if parse_digest(&archive_id)? != summary.archive_id { return Err(error(ErrorCode::StaleAnchor, "history comparison names a different archive")); }
            let (a, b, comparison) = archive.compare_records((before_number, parse_digest(&before_digest)?),
                (after_number, parse_digest(&after_digest)?), c)?;
            let value = json!({"ok":true,"historical":true,"native_calls":0,
                "before":entry_json(&a.entry, summary.archive_id),"after":record_json(&b, summary.archive_id),
                "comparison":comparison_json(&comparison),"progress_archive":summary_json(&summary)});
            Ok(Answer { value, capture:Some(b.observation), comparison:Some(comparison) })
        }
    }
}

#[cfg(test)]
#[path = "progress_history_tests.rs"]
mod tests;
