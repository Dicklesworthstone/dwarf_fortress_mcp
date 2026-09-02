#![forbid(unsafe_code)]

//! Canonical retained-suffix model for protocol-1.1 announcement reads.
//!
//! Dwarf Fortress exposes only the reports retained by the running process.
//! This model therefore distinguishes a complete suffix through the observed
//! high-water mark from complete fortress history. Transport page size and
//! protobuf field order do not enter semantic identity.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

pub const MAX_ANNOUNCEMENTS_PER_BATCH: usize = 512;
pub const MAX_ANNOUNCEMENT_TEXT_BYTES: usize = 2_048;
pub const MAX_CANONICAL_ANNOUNCEMENT_BATCH_BYTES: usize = 2 * 1024 * 1024;
const TICKS_PER_YEAR: i32 = 403_200;
const BATCH_DOMAIN: &[u8] = b"dfmcp.live-announcement-batch.v1\0";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnnouncementContinuity {
    CompleteSuffix,
    GapBeforeRetainedWindow,
}

impl AnnouncementContinuity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompleteSuffix => "complete_suffix",
            Self::GapBeforeRetainedWindow => "gap_before_retained_window",
        }
    }

    #[must_use]
    const fn tag(self) -> u8 {
        match self {
            Self::CompleteSuffix => 0,
            Self::GapBeforeRetainedWindow => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnouncementReplyContext {
    pub expected_after_report_id: i32,
    pub bridge_generation: u64,
    pub paused: bool,
    pub current_year: u32,
    pub current_year_tick: u32,
    pub site_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementBatchRecord {
    pub report_id: i32,
    pub report_type: i32,
    pub text: String,
    pub year: i32,
    pub year_tick: i32,
    pub repeat_count: i32,
    pub continuation: bool,
    pub unconscious: bool,
    pub announcement: bool,
}

impl AnnouncementBatchRecord {
    pub fn validate(&self) -> Result<()> {
        if self.report_id < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement report ID must not be negative",
            ));
        }
        if self.text.len() > MAX_ANNOUNCEMENT_TEXT_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "announcement text exceeds the {MAX_ANNOUNCEMENT_TEXT_BYTES}-byte ceiling"
                ),
            ));
        }
        if self.year < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement year must not be negative",
            ));
        }
        if !(0..TICKS_PER_YEAR).contains(&self.year_tick) {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "announcement year tick {} is outside 0..{}",
                    self.year_tick,
                    TICKS_PER_YEAR - 1
                ),
            ));
        }
        if self.repeat_count < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement repeat count must not be negative",
            ));
        }
        Ok(())
    }

    fn validate_at(&self, current_year: u32, current_year_tick: u32) -> Result<()> {
        self.validate()?;
        let observed_year = i32::try_from(current_year).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                "announcement observation year does not fit the report-year domain",
            )
        })?;
        let observed_tick = i32::try_from(current_year_tick).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "validated observation year tick does not fit i32",
            )
        })?;
        if self.year > observed_year
            || (self.year == observed_year && self.year_tick > observed_tick)
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "announcement report {} is dated after the observation clock",
                    self.report_id
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnouncementCoverage {
    pub requested_after_id: i32,
    pub oldest_available_id: i32,
    pub latest_available_id: i32,
    pub returned: u32,
    pub complete_through_latest: bool,
    pub continuity: AnnouncementContinuity,
    pub next_after_id: i32,
}

impl AnnouncementCoverage {
    #[must_use]
    pub const fn has_gap(self) -> bool {
        matches!(self.continuity, AnnouncementContinuity::GapBeforeRetainedWindow)
    }

    #[must_use]
    pub const fn needs_continuation(self) -> bool {
        !self.complete_through_latest
    }

    fn validate(self, records: &[AnnouncementBatchRecord]) -> Result<()> {
        if self.requested_after_id < -1 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement cursor must be -1 or nonnegative",
            ));
        }
        let retained_empty = self.oldest_available_id == -1 && self.latest_available_id == -1;
        if !retained_empty
            && (self.oldest_available_id < 0
                || self.latest_available_id < 0
                || self.oldest_available_id > self.latest_available_id)
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement retained bounds must be both -1 or ordered nonnegative IDs",
            ));
        }
        if (self.oldest_available_id == -1) != (self.latest_available_id == -1) {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement retained bounds must use the same empty sentinel",
            ));
        }
        if usize::try_from(self.returned).ok() != Some(records.len()) {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement returned count differs from the record vector",
            ));
        }
        let expected_next = records
            .last()
            .map_or(self.requested_after_id, |record| record.report_id);
        if self.next_after_id != expected_next {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement next cursor differs from the last returned report ID",
            ));
        }

        let gap_expected = self.requested_after_id >= 0
            && self.oldest_available_id >= 0
            && self
                .requested_after_id
                .checked_add(1)
                .is_some_and(|next| next < self.oldest_available_id);
        if self.has_gap() != gap_expected {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement continuity disagrees with the retained-window bounds",
            ));
        }
        if retained_empty {
            if !records.is_empty() || !self.complete_through_latest || self.has_gap() {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "empty retained announcement state has noncanonical coverage",
                ));
            }
            return Ok(());
        }

        let mut previous = self.requested_after_id;
        for record in records {
            record.validate()?;
            if record.report_id <= previous {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "announcement records are not strictly increasing after the cursor",
                ));
            }
            if record.report_id < self.oldest_available_id
                || record.report_id > self.latest_available_id
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "announcement record lies outside the retained bounds",
                ));
            }
            previous = record.report_id;
        }
        if self.complete_through_latest {
            if self.next_after_id < self.latest_available_id {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "complete announcement suffix stops before the retained high-water mark",
                ));
            }
        } else if records.is_empty() || self.next_after_id >= self.latest_available_id {
            return Err(error(
                ErrorCode::AdapterRejected,
                "partial announcement suffix has no strict continuation progress",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementBatch {
    pub bridge_generation: u64,
    pub paused: bool,
    pub current_year: u32,
    pub current_year_tick: u32,
    pub site_id: i32,
    pub coverage: AnnouncementCoverage,
    pub announcements: Vec<AnnouncementBatchRecord>,
    pub canonical_bytes: Vec<u8>,
    pub content_digest: Digest32,
}

impl LiveAnnouncementBatch {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bridge_generation: u64,
        paused: bool,
        current_year: u32,
        current_year_tick: u32,
        site_id: i32,
        coverage: AnnouncementCoverage,
        announcements: Vec<AnnouncementBatchRecord>,
    ) -> Result<Self> {
        if bridge_generation == 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement bridge generation zero is reserved",
            ));
        }
        let ticks_per_year = u32::try_from(TICKS_PER_YEAR).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "ticks-per-year constant does not fit u32",
            )
        })?;
        if current_year_tick >= ticks_per_year {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement observation year tick is outside the Dwarf Fortress year",
            ));
        }
        if i32::try_from(current_year).is_err() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement observation year does not fit the report-year domain",
            ));
        }
        if site_id < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement fortress site ID must not be negative",
            ));
        }
        if announcements.len() > MAX_ANNOUNCEMENTS_PER_BATCH {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "announcement batch exceeds the {MAX_ANNOUNCEMENTS_PER_BATCH}-record ceiling"
                ),
            ));
        }
        coverage.validate(&announcements)?;
        for record in &announcements {
            record.validate_at(current_year, current_year_tick)?;
        }
        let canonical_bytes = canonical_bytes(
            bridge_generation,
            paused,
            current_year,
            current_year_tick,
            site_id,
            coverage,
            &announcements,
        )?;
        if canonical_bytes.len() > MAX_CANONICAL_ANNOUNCEMENT_BATCH_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "canonical announcement batch exceeds its {}-byte ceiling",
                    MAX_CANONICAL_ANNOUNCEMENT_BATCH_BYTES
                ),
            ));
        }
        let batch = Self {
            bridge_generation,
            paused,
            current_year,
            current_year_tick,
            site_id,
            coverage,
            announcements,
            content_digest: Digest32::of_bytes(&canonical_bytes),
            canonical_bytes,
        };
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<()> {
        if self.bridge_generation == 0 {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement bridge generation zero is reserved",
            ));
        }
        let ticks_per_year = u32::try_from(TICKS_PER_YEAR).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "ticks-per-year constant does not fit u32",
            )
        })?;
        if self.current_year_tick >= ticks_per_year
            || self.site_id < 0
            || i32::try_from(self.current_year).is_err()
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement observation summary is outside its canonical domain",
            ));
        }
        if self.announcements.len() > MAX_ANNOUNCEMENTS_PER_BATCH
            || self.canonical_bytes.len() > MAX_CANONICAL_ANNOUNCEMENT_BATCH_BYTES
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "announcement batch exceeds its canonical bounds",
            ));
        }
        self.coverage.validate(&self.announcements)?;
        for record in &self.announcements {
            record.validate_at(self.current_year, self.current_year_tick)?;
        }
        let reproduced = canonical_bytes(
            self.bridge_generation,
            self.paused,
            self.current_year,
            self.current_year_tick,
            self.site_id,
            self.coverage,
            &self.announcements,
        )?;
        if reproduced != self.canonical_bytes
            || Digest32::of_bytes(&self.canonical_bytes) != self.content_digest
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement batch fields do not reproduce their canonical identity",
            ));
        }
        Ok(())
    }
}

fn push_bool(output: &mut Vec<u8>, value: bool) {
    output.push(u8::from(value));
}

fn push_u8(output: &mut Vec<u8>, value: u8) {
    output.push(value);
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    push_u32(
        output,
        u32::try_from(value.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "canonical announcement field length does not fit u32",
            )
        })?,
    );
    output.extend_from_slice(value);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn canonical_bytes(
    bridge_generation: u64,
    paused: bool,
    current_year: u32,
    current_year_tick: u32,
    site_id: i32,
    coverage: AnnouncementCoverage,
    announcements: &[AnnouncementBatchRecord],
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(BATCH_DOMAIN);
    push_u64(&mut output, bridge_generation);
    push_bool(&mut output, paused);
    push_u32(&mut output, current_year);
    push_u32(&mut output, current_year_tick);
    push_i32(&mut output, site_id);
    push_i32(&mut output, coverage.requested_after_id);
    push_i32(&mut output, coverage.oldest_available_id);
    push_i32(&mut output, coverage.latest_available_id);
    push_u32(&mut output, coverage.returned);
    push_bool(&mut output, coverage.complete_through_latest);
    push_u8(&mut output, coverage.continuity.tag());
    push_i32(&mut output, coverage.next_after_id);
    push_u32(
        &mut output,
        u32::try_from(announcements.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "canonical announcement record count does not fit u32",
            )
        })?,
    );
    for record in announcements {
        record.validate()?;
        push_i32(&mut output, record.report_id);
        push_i32(&mut output, record.report_type);
        push_bytes(&mut output, record.text.as_bytes())?;
        push_i32(&mut output, record.year);
        push_i32(&mut output, record.year_tick);
        push_i32(&mut output, record.repeat_count);
        push_bool(&mut output, record.continuation);
        push_bool(&mut output, record.unconscious);
        push_bool(&mut output, record.announcement);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(report_id: i32) -> AnnouncementBatchRecord {
        AnnouncementBatchRecord {
            report_id,
            report_type: 7,
            text: format!("report-{report_id}"),
            year: 105,
            year_tick: 12_000 + report_id,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn batch(records: Vec<AnnouncementBatchRecord>) -> Result<LiveAnnouncementBatch> {
        let returned = u32::try_from(records.len()).map_err(|_| {
            error(ErrorCode::BudgetExceeded, "test record count does not fit u32")
        })?;
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: 9,
                oldest_available_id: 1,
                latest_available_id: 11,
                returned,
                complete_through_latest: true,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: records.last().map_or(9, |value| value.report_id),
            },
            records,
        )
    }

    #[test]
    fn canonical_batch_is_deterministic() -> Result<()> {
        let first = batch(vec![record(10), record(11)])?;
        let second = batch(vec![record(10), record(11)])?;
        assert_eq!(first, second);
        first.validate()
    }

    #[test]
    fn retained_window_gap_is_explicit() -> Result<()> {
        let value = LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: 1,
                oldest_available_id: 10,
                latest_available_id: 11,
                returned: 2,
                complete_through_latest: true,
                continuity: AnnouncementContinuity::GapBeforeRetainedWindow,
                next_after_id: 11,
            },
            vec![record(10), record(11)],
        )?;
        assert!(value.coverage.has_gap());
        Ok(())
    }

    #[test]
    fn partial_suffix_requires_strict_progress() {
        let result = LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: 9,
                oldest_available_id: 1,
                latest_available_id: 11,
                returned: 0,
                complete_through_latest: false,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: 9,
            },
            Vec::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn empty_retained_state_is_canonical_but_not_complete_history() -> Result<()> {
        let value = LiveAnnouncementBatch::new(
            42,
            false,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: -1,
                oldest_available_id: -1,
                latest_available_id: -1,
                returned: 0,
                complete_through_latest: true,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: -1,
            },
            Vec::new(),
        )?;
        assert!(!value.coverage.has_gap());
        value.validate()
    }

    #[test]
    fn duplicate_reordered_and_out_of_window_records_fail_closed() {
        assert!(batch(vec![record(10), record(10)]).is_err());
        assert!(batch(vec![record(11), record(10)]).is_err());
        assert!(batch(vec![record(10), record(12)]).is_err());
    }

    #[test]
    fn future_records_are_rejected_against_the_observation_clock() {
        let mut later_year = record(11);
        later_year.year = 106;
        assert!(batch(vec![record(10), later_year]).is_err());

        let mut later_tick = record(11);
        later_tick.year = 105;
        later_tick.year_tick = 12_346;
        assert!(batch(vec![record(10), later_tick]).is_err());

        let mut current_tick = record(11);
        current_tick.year = 105;
        current_tick.year_tick = 12_345;
        assert!(batch(vec![record(10), current_tick]).is_ok());
    }

    #[test]
    fn tampering_breaks_canonical_validation() -> Result<()> {
        let mut structured = batch(vec![record(10), record(11)])?;
        structured.announcements[0].text = "tampered".to_owned();
        assert!(structured.validate().is_err());

        let mut future = batch(vec![record(10), record(11)])?;
        future.announcements[0].year = 106;
        assert!(future.validate().is_err());

        let mut bytes = batch(vec![record(10), record(11)])?;
        bytes.canonical_bytes.push(0);
        assert!(bytes.validate().is_err());
        Ok(())
    }

    #[test]
    fn bounds_are_enforced() {
        let mut oversized = record(10);
        oversized.text = "x".repeat(MAX_ANNOUNCEMENT_TEXT_BYTES + 1);
        assert!(batch(vec![oversized]).is_err());

        let records = (0..=MAX_ANNOUNCEMENTS_PER_BATCH)
            .map(|index| record(i32::try_from(index).unwrap_or(i32::MAX) + 10))
            .collect::<Vec<_>>();
        assert!(LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: 9,
                oldest_available_id: 1,
                latest_available_id: 600,
                returned: u32::try_from(records.len()).unwrap_or(u32::MAX),
                complete_through_latest: true,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: records.last().map_or(9, |value| value.report_id),
            },
            records,
        )
        .is_err());
    }
}
