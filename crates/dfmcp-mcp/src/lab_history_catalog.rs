//! Bounded, read-only discovery of retained laboratory observation identities.
//!
//! Owning bead: df-lab-structured-query-f7q.4.
//! Catalog membership uses retention recording ordinals, never inferred ticks.
//! It neither scans world content nor creates observations, pins, or watch state.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{WorldSnapshot, retention::VersionRetention};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_QUERY_BYTES: usize = 65_536;
const MAX_REQUEST_BYTES: usize = 2_048;
const MAX_CATALOG_VERSIONS: usize = 4_096;
const MAX_PAGE_VERSIONS: usize = 32;
const MAX_REPLAY_VERSIONS: usize = 32;
const CURSOR_BYTES: usize = 85;
const DOMAIN: &[u8] = b"dfmcp-laboratory-retained-anchor-catalog/1\0";
const CURSOR_DOMAIN: &[u8] = b"dfmcp-laboratory-retained-anchor-page/1\0";
const ANCHOR_BYTES: usize = 64;
const RECORD_BYTES: usize = 8 + ANCHOR_BYTES;

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    History,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Order {
    OldestFirst,
    #[default]
    NewestFirst,
}

impl Order {
    fn as_str(self) -> &'static str {
        match self {
            Self::OldestFirst => "oldest_first",
            Self::NewestFirst => "newest_first",
        }
    }

    fn index(self, position: usize, length: usize) -> usize {
        match self {
            Self::OldestFirst => position,
            Self::NewestFirst => length - position - 1,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    limit: Option<u32>,
    #[serde(default)]
    order: Order,
    continuation: Option<String>,
}

#[derive(Clone, Copy)]
struct Interval {
    first: usize,
    last: usize,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Parse the original JSON bytes so duplicate or unknown fields cannot disappear
/// before validation. Other laboratory query generations keep their own grammar.
pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_QUERY_BYTES {
        return Err(exhausted("laboratory query exceeds its request byte bound"));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("query requires one unambiguous mode: {error}")))?;
    if selector.mode != "history" {
        return Ok(None);
    }
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted("history request exceeds its metadata-only byte bound"));
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid retained history request: {error}")))?;
    let Mode::History = request.mode;
    if request
        .limit
        .is_some_and(|limit| !(1..=MAX_PAGE_VERSIONS as u32).contains(&limit))
    {
        return Err(invalid("history page limit must be between one and 32"));
    }
    if let Some(cursor) = &request.continuation {
        decode_cursor(cursor)?;
    }
    Ok(Some(request))
}

fn check(current: &OperationContext, started: Instant) -> Result<()> {
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted("retained history exhausted its shared wall-time budget"));
    }
    Ok(())
}

fn encode_anchor(bytes: &mut Vec<u8>, anchor: StateAnchor) {
    bytes.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&anchor.tick.0.to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
}

fn catalog_binding(
    records: &[(u64, StateAnchor)],
    current: &OperationContext,
    bytes: usize,
) -> Digest32 {
    let mut encoded = Vec::with_capacity(bytes);
    encoded.extend_from_slice(DOMAIN);
    encoded.extend_from_slice(&current.session_id.get().to_be_bytes());
    encode_anchor(&mut encoded, current.anchor);
    encoded.extend_from_slice(&(records.len() as u64).to_be_bytes());
    for (sequence, anchor) in records {
        encoded.extend_from_slice(&sequence.to_be_bytes());
        encode_anchor(&mut encoded, *anchor);
    }
    Digest32::of_bytes(&encoded)
}

fn cursor_checksum(binding: Digest32, sequence: u64, order: Order) -> Digest32 {
    let mut encoded = Vec::with_capacity(CURSOR_DOMAIN.len() + 32 + 8 + 1);
    encoded.extend_from_slice(CURSOR_DOMAIN);
    encoded.extend_from_slice(binding.as_bytes());
    encoded.extend_from_slice(&sequence.to_be_bytes());
    encoded.push(match order {
        Order::OldestFirst => 0,
        Order::NewestFirst => 1,
    });
    Digest32::of_bytes(&encoded)
}

fn cursor(binding: Digest32, sequence: u64, order: Order) -> String {
    format!(
        "lh1:{sequence:016x}:{}",
        cursor_checksum(binding, sequence, order)
    )
}

fn decode_cursor(value: &str) -> Result<(u64, Digest32)> {
    if value.len() != CURSOR_BYTES {
        return Err(invalid("history continuation has an invalid length"));
    }
    let mut pieces = value.split(':');
    if pieces.next() != Some("lh1") {
        return Err(invalid("history continuation has an unsupported generation"));
    }
    let sequence = pieces
        .next()
        .filter(|value| value.len() == 16 && lowercase_hex(value))
        .and_then(|value| u64::from_str_radix(value, 16).ok())
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| invalid("history continuation has an invalid recording ordinal"))?;
    let checksum = pieces
        .next()
        .filter(|value| value.len() == 64 && lowercase_hex(value))
        .and_then(Digest32::from_hex)
        .ok_or_else(|| invalid("history continuation has an invalid checksum"))?;
    if pieces.next().is_some() {
        return Err(invalid("history continuation has additional fields"));
    }
    Ok((sequence, checksum))
}

fn eligibility(anchor: StateAnchor, current: StateAnchor) -> &'static str {
    if anchor.fortress_id != current.fortress_id {
        "different_fortress"
    } else if anchor.cursor.epoch != current.cursor.epoch {
        "different_epoch"
    } else if anchor.cursor.sequence > current.cursor.sequence || anchor.tick > current.tick {
        "future_observation"
    } else if anchor.cursor == current.cursor && anchor != current {
        "conflicting_current_cursor"
    } else {
        "candidate"
    }
}

fn consecutive(before: (u64, StateAnchor), after: (u64, StateAnchor)) -> bool {
    before.0.checked_add(1) == Some(after.0)
        && before.1.fortress_id == after.1.fortress_id
        && before.1.cursor.epoch == after.1.cursor.epoch
        && after.1.cursor.sequence > before.1.cursor.sequence
        && after.1.tick >= before.1.tick
}

fn intervals(
    records: &[(u64, StateAnchor)],
    current: &OperationContext,
    started: Instant,
) -> Result<(Vec<Interval>, Vec<Option<usize>>)> {
    let mut intervals: Vec<Interval> = Vec::new();
    let mut membership: Vec<Option<usize>> = Vec::with_capacity(records.len());
    for (index, (_, anchor)) in records.iter().enumerate() {
        if index % 32 == 0 {
            check(current, started)?;
        }
        if eligibility(*anchor, current.anchor) != "candidate" {
            membership.push(None);
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|previous| membership[previous])
            .filter(|_| consecutive(records[index - 1], records[index]));
        let interval = if let Some(previous) = previous {
            intervals[previous].last = index;
            previous
        } else {
            intervals.push(Interval { first: index, last: index });
            intervals.len() - 1
        };
        membership.push(Some(interval));
    }
    Ok((intervals, membership))
}

fn gap(first: u64, last: u64) -> Value {
    json!({"first":first,"last":last,"count":last - first + 1})
}

fn record_json(records: &[(u64, StateAnchor)], index: usize, current: StateAnchor) -> Value {
    let (sequence, anchor) = records[index];
    let previous = index.checked_sub(1).map(|index| records[index]);
    let first_missing = previous.map_or(1, |(sequence, _)| sequence + 1);
    let recording_gap = if first_missing < sequence {
        gap(first_missing, sequence - 1)
    } else {
        Value::Null
    };
    let (observation_continuity, observation_gap) = match previous {
        None => ("no_previous_retained", Value::Null),
        Some((_, before)) if before.fortress_id != anchor.fortress_id => {
            ("fortress_changed", Value::Null)
        }
        Some((_, before)) if before.cursor.epoch != anchor.cursor.epoch => {
            ("epoch_changed", Value::Null)
        }
        Some((_, before))
            if anchor.cursor.sequence <= before.cursor.sequence || anchor.tick < before.tick =>
        {
            ("non_monotonic", Value::Null)
        }
        Some((_, before)) if anchor.cursor.sequence - before.cursor.sequence > 1 => (
            "cursor_gap",
            json!({"epoch":anchor.cursor.epoch,
                "first":before.cursor.sequence + 1, "last":anchor.cursor.sequence - 1,
                "count":anchor.cursor.sequence - before.cursor.sequence - 1}),
        ),
        Some(_) => ("adjacent_cursors", Value::Null),
    };
    json!({
        "recording_sequence":sequence,
        "anchor":super::anchor_json(&anchor),
        "is_current":anchor == current,
        "condition_replay_eligibility":eligibility(anchor, current),
        "previous_retained_recording_sequence":previous.map(|(sequence, _)| sequence),
        "recording_gap_before":recording_gap,
        "observation_continuity_from_previous":observation_continuity,
        "observation_gap_before":observation_gap,
    })
}

fn replay_windows(
    history: &VersionRetention,
    records: &[(u64, StateAnchor)],
    intervals: &[Interval],
    membership: &[Option<usize>],
    selected: &[usize],
) -> Result<Vec<Value>> {
    let selected_intervals: BTreeSet<_> = selected.iter().filter_map(|index| membership[*index]).collect();
    let mut windows = Vec::with_capacity(selected_intervals.len());
    for interval_id in selected_intervals {
        let interval = intervals[interval_id];
        let last = selected
            .iter()
            .copied()
            .filter(|index| membership[*index] == Some(interval_id))
            .max()
            .ok_or_else(|| invariant("history page interval has no returned endpoint"))?;
        let first = interval.first.max(last.saturating_sub(MAX_REPLAY_VERSIONS - 1));
        let from = records[first];
        let to = records[last];
        // Use the actual retention reader as the final membership check.
        // No canonical world contents are traversed or cloned.
        let sources = history.consecutive_range(
            &from.1.state_hash,
            &to.1.state_hash,
            MAX_REPLAY_VERSIONS,
        )?;
        if sources.len() != last - first + 1
            || sources.iter().enumerate().any(|(offset, snapshot)| {
                snapshot.anchor() != records[first + offset].1
            })
        {
            return Err(invariant("history replay window differs from retained recording metadata"));
        }
        let cursor_gaps = sources.windows(2).filter(|pair| {
            pair[1].cursor.sequence - pair[0].cursor.sequence > 1
        }).count();
        windows.push(json!({
            "from":from.1.state_hash.to_string(), "to":to.1.state_hash.to_string(),
            "first_recording_sequence":from.0, "last_recording_sequence":to.0,
            "versions":sources.len(),
            "retained_interval_versions":interval.last - interval.first + 1,
            "consecutive_retained_versions":true,
            "current_fortress_and_epoch":true,
            "observation_cursor_gaps":cursor_gaps,
            "stability_resets_at_cursor_gaps":true,
            "continuous_between_observations":false,
            "requires_condition_definition":true,
            "requires_complete_source_admission":true,
        }));
    }
    Ok(windows)
}

/// Inspect only already-retained identities while the caller owns the session.
/// The complete metadata source is bounded before allocation and hashing. This
/// does not validate snapshot contents: the eventual `at`, comparison or replay
/// read must independently admit and validate its selected complete worlds.
pub(super) fn execute(
    history: &VersionRetention,
    current_world: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
) -> Result<Value> {
    let started = Instant::now();
    check(current, started)?;
    if current_world.anchor() != current.anchor {
        return Err(stale("retained history lost its current owned world"));
    }
    let maximum_records = MAX_CATALOG_VERSIONS.min(current.budget.max_entities as usize);
    if history.len() > maximum_records {
        return Err(exhausted("retained history exceeds its complete metadata record allowance"));
    }
    let metadata_bytes = history.len().checked_mul(RECORD_BYTES)
        .and_then(|bytes| bytes.checked_add(DOMAIN.len() + 16 + ANCHOR_BYTES + 8))
        .ok_or_else(|| exhausted("retained history metadata byte count overflow"))?;
    if metadata_bytes as u64 > current.budget.max_bytes {
        return Err(exhausted("retained history exceeds its complete metadata byte allowance"));
    }
    let records = history.recorded_anchors(maximum_records)?;
    check(current, started)?;
    if records.iter().any(|(_, anchor)| anchor.fortress_id != current.anchor.fortress_id) {
        return Err(stale("retained history contains a different fortress lineage"));
    }
    let binding = catalog_binding(&records, current, metadata_bytes);
    let offset = if let Some(value) = &request.continuation {
        let (sequence, checksum) = decode_cursor(value)?;
        if checksum != cursor_checksum(binding, sequence, request.order)
            || value != &cursor(binding, sequence, request.order)
        {
            return Err(stale(
                "history catalog, current anchor, owner, ordering or cursor changed; restart discovery without its continuation",
            ));
        }
        let index = records.iter().position(|(record, _)| *record == sequence)
            .ok_or_else(|| invalid("history continuation endpoint is not retained"))?;
        let offset = match request.order {
            Order::OldestFirst => index + 1,
            Order::NewestFirst => records.len() - index,
        };
        if offset >= records.len() {
            return Err(invalid("history continuation has no remaining records"));
        }
        offset
    } else {
        0
    };
    let (intervals, membership) = intervals(&records, current, started)?;
    let eligible = membership.iter().filter(|member| member.is_some()).count();
    let current_retained = records.iter().any(|(_, anchor)| *anchor == current.anchor);
    let maximum = current.budget.max_bytes.min(u64::from(current.budget.max_output_tokens) * 4);
    let requested = request.limit.unwrap_or(4) as usize;
    let available = records.len() - offset;
    let mut count = requested.min(available);
    loop {
        check(current, started)?;
        let selected: Vec<_> = (offset..offset + count)
            .map(|position| request.order.index(position, records.len()))
            .collect();
        let rows: Vec<_> = selected.iter()
            .map(|index| record_json(&records, *index, current.anchor))
            .collect();
        let truncated = offset + count < records.len();
        let continuation = if truncated {
            let index = selected.last().ok_or_else(|| {
                exhausted("history cannot publish an empty advancing page")
            })?;
            Some(cursor(binding, records[*index].0, request.order))
        } else {
            None
        };
        let windows = replay_windows(history, &records, &intervals, &membership, &selected)?;
        let next_query = continuation.as_ref().map(|continuation| json!({
            "mode":"history", "limit":request.limit.unwrap_or(4),
            "order":request.order.as_str(), "continuation":continuation,
        }));
        let result = json!({
            "ok":true, "mode":"history", "kind":"retained_anchor_catalog",
            "schema":"dfmcp.lab-history/1",
            "session_id":current.session_id.to_string(),
            "anchor":super::anchor_json(&current.anchor),
            "current_anchor":super::anchor_json(&current.anchor),
            "game_tick":current.anchor.tick.0,
            "catalog_generation":binding.to_string(),
            "order":request.order.as_str(),
            "retained_versions":records.len(), "current_epoch_candidate_versions":eligible,
            "current_anchor_retained":current_retained,
            "oldest_retained_anchor":records.first().map(|(_, anchor)| super::anchor_json(anchor)),
            "newest_retained_anchor":records.last().map(|(_, anchor)| super::anchor_json(anchor)),
            "returned":rows.len(), "records":rows,
            "truncated":truncated, "continuation":continuation, "next_query":next_query,
            "replay_windows":windows, "maximum_condition_replay_versions":MAX_REPLAY_VERSIONS,
            "source_work":{"retained_metadata_records":records.len(),
                "canonical_metadata_bytes":metadata_bytes, "world_contents_scanned":false},
            "history_pinned":false, "advances_game":false,
            "world_contents_validated":false, "eligible_as_mutation_evidence":false,
            "evidence_scope":"laboratory_reference_world_retention_metadata",
            "observation_coverage":{
                "status":"partial", "complete_domains":["laboratory.retained_anchor_catalog_membership"],
                "partial_domains":[{"domain":"laboratory.retained_anchor_catalog_delivery",
                    "reason":if truncated {"additional retained identities require the exact continuation"} else {"no further retained identities remain in the selected ordering after this page"}},
                    {"domain":"laboratory.observation_history",
                    "reason":"retained metadata is not complete game history; missing recording positions and original observation cursor gaps remain distinct"}],
                "absence_proof_scope":[], "continuation":continuation,
                "world_contents_validated":false, "history_complete":false,
                "continuous_between_observations":false, "eligible_as_mutation_evidence":false,
            },
        });
        let bytes = serde_json::to_vec(&result)
            .map_err(|_| invariant("retained history page cannot be encoded"))?
            .len() as u64;
        if bytes <= maximum {
            check(current, started)?;
            return Ok(result);
        }
        if count <= 1 {
            return Err(exhausted(
                "one complete retained history row and its coverage exceed the response allowance",
            ));
        }
        // The cursor is rebuilt from the exact smaller prefix, never advanced
        // past a row withdrawn for size. The enclosing Agent Turn is lossless
        // or refused separately; generic row clipping is not permitted.
        count -= 1;
    }
}

#[cfg(test)]
#[path = "lab_history_catalog_tests.rs"]
mod tests;
