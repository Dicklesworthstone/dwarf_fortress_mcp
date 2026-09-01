#![forbid(unsafe_code)]

//! Canonical assembly for a bounded retained Dwarf Fortress announcement window.
//!
//! Dwarf Fortress does not expose an eternal event log. The bridge can only
//! witness the reports still retained by the running process. This module
//! therefore models an explicitly bounded window with a frozen high-water mark,
//! stable retained bounds, deterministic pagination, and honest partial-history
//! coverage. Transport page size is not semantic.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, sha256};

pub const MAX_ANNOUNCEMENTS_PER_PAGE: u32 = 4_096;
pub const MAX_ANNOUNCEMENT_TEXT_BYTES: usize = 2_048;
pub const MAX_ANNOUNCEMENT_WINDOW_RECORDS: usize = 100_000;
pub const MAX_CANONICAL_ANNOUNCEMENT_BYTES: usize = 64 * 1024 * 1024;
const TICKS_PER_YEAR: u32 = 403_200;
const WINDOW_DOMAIN: &[u8] = b"dfmcp.live-announcement-window.v1\0";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn validate_text(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.len() > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!("{field} exceeds its {maximum}-byte bound"),
        ));
    }
    Ok(())
}

fn validate_nonempty_text(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.is_empty() {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!("{field} must not be empty"),
        ));
    }
    validate_text(value, field, maximum)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementSourceIdentity {
    pub bridge_version: String,
    pub dfhack_version: String,
    pub dwarf_fortress_version: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub bridge_generation: u64,
}

impl AnnouncementSourceIdentity {
    pub fn validate(&self) -> Result<()> {
        validate_nonempty_text(&self.bridge_version, "bridge version", 64)?;
        validate_nonempty_text(&self.dfhack_version, "DFHack version", 128)?;
        validate_nonempty_text(
            &self.dwarf_fortress_version,
            "Dwarf Fortress version",
            128,
        )?;
        if self.protocol_major != 1 || self.protocol_minor < 1 {
            return Err(error(
                ErrorCode::VersionMismatch,
                "announcement windows require bridge protocol 1.1 or later in major generation 1",
            ));
        }
        if self.bridge_generation == 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "bridge generation zero is reserved",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementRecord {
    pub report_id: i32,
    pub announcement_type: i32,
    pub text: String,
    pub year: u32,
    pub year_tick: u32,
    pub has_position: bool,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub repeat_count: u32,
    pub continuation: bool,
    pub unconscious: bool,
    pub announcement: bool,
}

impl AnnouncementRecord {
    pub fn validate(&self) -> Result<()> {
        if self.report_id < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement report ID must not be negative",
            ));
        }
        validate_text(
            &self.text,
            "announcement text",
            MAX_ANNOUNCEMENT_TEXT_BYTES,
        )?;
        if self.year_tick >= TICKS_PER_YEAR {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "announcement year tick {} is outside 0..{}",
                    self.year_tick,
                    TICKS_PER_YEAR - 1
                ),
            ));
        }
        if !self.has_position && (self.x != 0 || self.y != 0 || self.z != 0) {
            return Err(error(
                ErrorCode::AdapterRejected,
                "an announcement without a position must use canonical zero coordinates",
            ));
        }
        if !self.announcement {
            return Err(error(
                ErrorCode::AdapterRejected,
                "the announcement stream contains a report not marked as an announcement",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementPage {
    pub bridge_generation: u64,
    pub requested_after_report_id: i32,
    pub requested_maximum: u32,
    pub oldest_retained_report_id: i32,
    pub latest_retained_report_id: i32,
    pub window_latest_report_id: i32,
    pub next_after_report_id: i32,
    pub history_truncated: bool,
    pub complete: bool,
    pub announcements: Vec<AnnouncementRecord>,
}

impl AnnouncementPage {
    pub fn validate(&self) -> Result<()> {
        if self.bridge_generation == 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement page bridge generation zero is reserved",
            ));
        }
        if self.requested_after_report_id < -1 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "announcement cursor must be -1 or a nonnegative report ID",
            ));
        }
        if self.requested_maximum == 0
            || self.requested_maximum > MAX_ANNOUNCEMENTS_PER_PAGE
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "announcement page size must be in 1..={MAX_ANNOUNCEMENTS_PER_PAGE}"
                ),
            ));
        }
        validate_retained_bounds(
            self.oldest_retained_report_id,
            self.latest_retained_report_id,
            self.window_latest_report_id,
        )?;
        let page_len = u32::try_from(self.announcements.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "announcement page length does not fit u32",
            )
        })?;
        if page_len > self.requested_maximum {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement page exceeds the requested record count",
            ));
        }
        if !self.complete && page_len != self.requested_maximum {
            return Err(error(
                ErrorCode::AdapterRejected,
                "a nonterminal announcement page must fill the requested page size",
            ));
        }
        if !self.complete && self.announcements.is_empty() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "an empty announcement page cannot be nonterminal",
            ));
        }

        let expected_truncated = history_is_truncated(
            self.requested_after_report_id,
            self.oldest_retained_report_id,
        );
        if self.history_truncated != expected_truncated {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement history-truncation flag disagrees with retained bounds",
            ));
        }

        let mut previous = self.requested_after_report_id;
        for record in &self.announcements {
            record.validate()?;
            if record.report_id <= previous {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "announcement records are not in strict report-ID order after the cursor",
                ));
            }
            if self.window_latest_report_id >= 0
                && record.report_id > self.window_latest_report_id
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "announcement record exceeds the frozen high-water mark",
                ));
            }
            if self.oldest_retained_report_id >= 0
                && record.report_id < self.oldest_retained_report_id
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "announcement record predates the retained-window lower bound",
                ));
            }
            previous = record.report_id;
        }
        let expected_next = self
            .announcements
            .last()
            .map_or(self.requested_after_report_id, |record| record.report_id);
        if self.next_after_report_id != expected_next {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement next cursor does not equal the last returned report ID",
            ));
        }
        if self.next_after_report_id > self.window_latest_report_id
            && self.window_latest_report_id >= 0
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement next cursor exceeds the frozen high-water mark",
            ));
        }
        if !self.complete && self.next_after_report_id >= self.window_latest_report_id {
            return Err(error(
                ErrorCode::AdapterRejected,
                "a nonterminal announcement page has already reached the frozen high-water mark",
            ));
        }
        Ok(())
    }
}

fn validate_retained_bounds(oldest: i32, latest: i32, window_latest: i32) -> Result<()> {
    let empty = oldest == -1 && latest == -1 && window_latest == -1;
    if empty {
        return Ok(());
    }
    if oldest < 0 || latest < 0 || window_latest < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "retained announcement bounds must be all -1 or all nonnegative",
        ));
    }
    if oldest > latest || window_latest > latest {
        return Err(error(
            ErrorCode::AdapterRejected,
            "retained announcement bounds are not ordered",
        ));
    }
    Ok(())
}

fn history_is_truncated(after_report_id: i32, oldest_retained_report_id: i32) -> bool {
    if after_report_id < 0 || oldest_retained_report_id < 0 {
        return false;
    }
    oldest_retained_report_id
        .checked_sub(1)
        .is_some_and(|last_safe_cursor| after_report_id < last_safe_cursor)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementWindow {
    pub source: AnnouncementSourceIdentity,
    pub after_report_id: i32,
    pub oldest_retained_report_id: i32,
    pub latest_retained_report_id: i32,
    pub window_latest_report_id: i32,
    pub next_after_report_id: i32,
    pub history_truncated: bool,
    pub complete: bool,
    pub announcements: Vec<AnnouncementRecord>,
    pub canonical_bytes: Vec<u8>,
    pub content_digest: Digest32,
}

impl LiveAnnouncementWindow {
    #[must_use]
    pub const fn can_prove_absence_in_frozen_interval(&self) -> bool {
        self.complete && !self.history_truncated
    }

    pub fn validate(&self) -> Result<()> {
        self.source.validate()?;
        validate_retained_bounds(
            self.oldest_retained_report_id,
            self.latest_retained_report_id,
            self.window_latest_report_id,
        )?;
        if !self.complete {
            return Err(error(
                ErrorCode::CursorGap,
                "a published announcement window must have complete frozen-window coverage",
            ));
        }
        if self.after_report_id < -1 || self.next_after_report_id < self.after_report_id {
            return Err(error(
                ErrorCode::CorruptLedger,
                "published announcement cursor bounds are invalid",
            ));
        }
        if self.announcements.len() > MAX_ANNOUNCEMENT_WINDOW_RECORDS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "published announcement window exceeds its record ceiling",
            ));
        }
        if self.canonical_bytes.len() > MAX_CANONICAL_ANNOUNCEMENT_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical announcement window exceeds its byte ceiling",
            ));
        }
        if self.history_truncated
            != history_is_truncated(self.after_report_id, self.oldest_retained_report_id)
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "published announcement truncation posture disagrees with retained bounds",
            ));
        }
        let mut previous = self.after_report_id;
        for record in &self.announcements {
            record.validate()?;
            if record.report_id <= previous {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "published announcements are not in strict report-ID order",
                ));
            }
            previous = record.report_id;
        }
        let expected_next = self
            .announcements
            .last()
            .map_or(self.after_report_id, |record| record.report_id);
        if self.next_after_report_id != expected_next {
            return Err(error(
                ErrorCode::CorruptLedger,
                "published announcement next cursor does not match its records",
            ));
        }
        let recomputed = canonical_bytes(
            &self.source,
            self.after_report_id,
            self.oldest_retained_report_id,
            self.latest_retained_report_id,
            self.window_latest_report_id,
            self.next_after_report_id,
            self.history_truncated,
            self.complete,
            &self.announcements,
        )?;
        if recomputed != self.canonical_bytes {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement window fields do not reproduce the stored canonical bytes",
            ));
        }
        if sha256(&self.canonical_bytes) != *self.content_digest.as_bytes() {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement window digest does not match its canonical bytes",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AnnouncementWindowAssembler {
    source: AnnouncementSourceIdentity,
    after_report_id: i32,
    next_after_report_id: i32,
    window: Option<(i32, i32, i32, bool)>,
    announcements: Vec<AnnouncementRecord>,
    complete: bool,
}

impl AnnouncementWindowAssembler {
    pub fn new(source: AnnouncementSourceIdentity, after_report_id: i32) -> Result<Self> {
        source.validate()?;
        if after_report_id < -1 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "announcement cursor must be -1 or a nonnegative report ID",
            ));
        }
        Ok(Self {
            source,
            after_report_id,
            next_after_report_id: after_report_id,
            window: None,
            announcements: Vec::new(),
            complete: false,
        })
    }

    #[must_use]
    pub const fn next_after_report_id(&self) -> i32 {
        self.next_after_report_id
    }

    #[must_use]
    pub const fn frozen_high_water_mark(&self) -> Option<i32> {
        match self.window {
            Some((_, _, high_water, _)) => Some(high_water),
            None => None,
        }
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn push_page(&mut self, page: AnnouncementPage) -> Result<()> {
        self.source.validate()?;
        if self.complete {
            return Err(error(
                ErrorCode::InvalidRequest,
                "cannot append an announcement page after complete coverage",
            ));
        }
        page.validate()?;
        if page.bridge_generation != self.source.bridge_generation {
            return Err(error(
                ErrorCode::StaleAnchor,
                "bridge generation changed while assembling announcements",
            ));
        }
        if page.requested_after_report_id != self.next_after_report_id {
            return Err(error(
                ErrorCode::CursorGap,
                format!(
                    "announcement page cursor {} does not match expected {}",
                    page.requested_after_report_id, self.next_after_report_id
                ),
            ));
        }
        let candidate_window = (
            page.oldest_retained_report_id,
            page.latest_retained_report_id,
            page.window_latest_report_id,
            page.history_truncated,
        );
        if let Some(window) = self.window
            && window != candidate_window
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "retained announcement bounds or frozen high-water mark changed between pages",
            ));
        }
        if let (Some(previous), Some(first)) =
            (self.announcements.last(), page.announcements.first())
            && previous.report_id >= first.report_id
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement ordering is not strict across page boundaries",
            ));
        }
        let candidate_len = self
            .announcements
            .len()
            .checked_add(page.announcements.len())
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "announcement window record count overflowed",
                )
            })?;
        if candidate_len > MAX_ANNOUNCEMENT_WINDOW_RECORDS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "announcement window exceeds its record ceiling",
            ));
        }

        if self.window.is_none() {
            self.window = Some(candidate_window);
        }
        self.next_after_report_id = page.next_after_report_id;
        self.announcements.extend(page.announcements);
        self.complete = page.complete;
        Ok(())
    }

    pub fn finalize(self) -> Result<LiveAnnouncementWindow> {
        self.source.validate()?;
        if !self.complete {
            return Err(error(
                ErrorCode::CursorGap,
                "cannot publish an incomplete announcement window",
            ));
        }
        let (oldest, latest, window_latest, history_truncated) = self.window.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "cannot publish an announcement window without any page",
            )
        })?;
        let bytes = canonical_bytes(
            &self.source,
            self.after_report_id,
            oldest,
            latest,
            window_latest,
            self.next_after_report_id,
            history_truncated,
            true,
            &self.announcements,
        )?;
        if bytes.len() > MAX_CANONICAL_ANNOUNCEMENT_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical announcement window exceeds its byte ceiling",
            ));
        }
        let window = LiveAnnouncementWindow {
            source: self.source,
            after_report_id: self.after_report_id,
            oldest_retained_report_id: oldest,
            latest_retained_report_id: latest,
            window_latest_report_id: window_latest,
            next_after_report_id: self.next_after_report_id,
            history_truncated,
            complete: true,
            content_digest: Digest32::from_bytes(sha256(&bytes)),
            canonical_bytes: bytes,
            announcements: self.announcements,
        };
        window.validate()?;
        Ok(window)
    }
}

fn push_u8(output: &mut Vec<u8>, value: u8) {
    output.push(value);
}

fn push_bool(output: &mut Vec<u8>, value: bool) {
    push_u8(output, u8::from(value));
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
    let length = u32::try_from(value.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "canonical announcement field length does not fit u32",
        )
    })?;
    push_u32(output, length);
    output.extend_from_slice(value);
    Ok(())
}

fn push_string(output: &mut Vec<u8>, value: &str) -> Result<()> {
    push_bytes(output, value.as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn canonical_bytes(
    source: &AnnouncementSourceIdentity,
    after_report_id: i32,
    oldest_retained_report_id: i32,
    latest_retained_report_id: i32,
    window_latest_report_id: i32,
    next_after_report_id: i32,
    history_truncated: bool,
    complete: bool,
    announcements: &[AnnouncementRecord],
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(WINDOW_DOMAIN);
    push_string(&mut output, &source.bridge_version)?;
    push_string(&mut output, &source.dfhack_version)?;
    push_string(&mut output, &source.dwarf_fortress_version)?;
    push_u32(&mut output, source.protocol_major);
    push_u32(&mut output, source.protocol_minor);
    push_u64(&mut output, source.bridge_generation);
    push_i32(&mut output, after_report_id);
    push_i32(&mut output, oldest_retained_report_id);
    push_i32(&mut output, latest_retained_report_id);
    push_i32(&mut output, window_latest_report_id);
    push_i32(&mut output, next_after_report_id);
    push_bool(&mut output, history_truncated);
    push_bool(&mut output, complete);
    push_u32(
        &mut output,
        u32::try_from(announcements.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "canonical announcement count does not fit u32",
            )
        })?,
    );
    for record in announcements {
        push_i32(&mut output, record.report_id);
        push_i32(&mut output, record.announcement_type);
        push_string(&mut output, &record.text)?;
        push_u32(&mut output, record.year);
        push_u32(&mut output, record.year_tick);
        push_bool(&mut output, record.has_position);
        push_i32(&mut output, record.x);
        push_i32(&mut output, record.y);
        push_i32(&mut output, record.z);
        push_u32(&mut output, record.repeat_count);
        push_bool(&mut output, record.continuation);
        push_bool(&mut output, record.unconscious);
        push_bool(&mut output, record.announcement);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> AnnouncementSourceIdentity {
        AnnouncementSourceIdentity {
            bridge_version: "0.2.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            dwarf_fortress_version: "0.51.11".to_owned(),
            protocol_major: 1,
            protocol_minor: 1,
            bridge_generation: 42,
        }
    }

    fn announcement(report_id: i32) -> AnnouncementRecord {
        AnnouncementRecord {
            report_id,
            announcement_type: 7,
            text: format!("announcement {report_id}"),
            year: 105,
            year_tick: 12_345,
            has_position: true,
            x: report_id,
            y: 20,
            z: 30,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn page(
        after: i32,
        maximum: u32,
        ids: &[i32],
        complete: bool,
    ) -> AnnouncementPage {
        AnnouncementPage {
            bridge_generation: 42,
            requested_after_report_id: after,
            requested_maximum: maximum,
            oldest_retained_report_id: 1,
            latest_retained_report_id: 4,
            window_latest_report_id: 4,
            next_after_report_id: ids.last().copied().unwrap_or(after),
            history_truncated: false,
            complete,
            announcements: ids.iter().copied().map(announcement).collect(),
        }
    }

    #[test]
    fn pagination_does_not_change_window_identity() -> Result<()> {
        let mut one = AnnouncementWindowAssembler::new(source(), -1)?;
        one.push_page(page(-1, 4, &[1, 2, 3, 4], true))?;
        let one = one.finalize()?;

        let mut many = AnnouncementWindowAssembler::new(source(), -1)?;
        many.push_page(page(-1, 2, &[1, 2], false))?;
        many.push_page(page(2, 2, &[3, 4], true))?;
        let many = many.finalize()?;

        assert_eq!(one.content_digest, many.content_digest);
        assert_eq!(one.canonical_bytes, many.canonical_bytes);
        assert!(one.can_prove_absence_in_frozen_interval());
        Ok(())
    }

    #[test]
    fn retained_history_loss_is_explicit_partial_coverage() -> Result<()> {
        let mut assembler = AnnouncementWindowAssembler::new(source(), 1)?;
        let mut truncated = page(1, 4, &[5, 6], true);
        truncated.oldest_retained_report_id = 5;
        truncated.latest_retained_report_id = 6;
        truncated.window_latest_report_id = 6;
        truncated.history_truncated = true;
        assembler.push_page(truncated)?;
        let window = assembler.finalize()?;
        assert!(window.history_truncated);
        assert!(!window.can_prove_absence_in_frozen_interval());
        Ok(())
    }

    #[test]
    fn appended_or_pruned_window_drift_is_rejected_without_partial_mutation() -> Result<()> {
        let mut assembler = AnnouncementWindowAssembler::new(source(), -1)?;
        assembler.push_page(page(-1, 2, &[1, 2], false))?;
        let cursor = assembler.next_after_report_id();
        let mut drifted = page(2, 2, &[3, 4], true);
        drifted.latest_retained_report_id = 5;
        assert!(assembler.push_page(drifted).is_err());
        assert_eq!(assembler.next_after_report_id(), cursor);
        assert!(!assembler.is_complete());
        assembler.push_page(page(2, 2, &[3, 4], true))?;
        assert!(assembler.is_complete());
        Ok(())
    }

    #[test]
    fn cross_page_reordering_and_cursor_gaps_fail_closed() -> Result<()> {
        let mut reordered = AnnouncementWindowAssembler::new(source(), -1)?;
        reordered.push_page(page(-1, 2, &[1, 3], false))?;
        assert!(reordered.push_page(page(3, 2, &[2, 4], true)).is_err());

        let mut gap = AnnouncementWindowAssembler::new(source(), -1)?;
        gap.push_page(page(-1, 2, &[1, 2], false))?;
        assert!(gap.push_page(page(3, 1, &[4], true)).is_err());
        Ok(())
    }

    #[test]
    fn nonterminal_short_or_empty_page_is_rejected() {
        assert!(page(-1, 2, &[1], false).validate().is_err());
        assert!(page(-1, 1, &[], false).validate().is_err());
    }

    #[test]
    fn malformed_record_semantics_fail_closed() {
        let mut invalid_position = announcement(1);
        invalid_position.has_position = false;
        assert!(invalid_position.validate().is_err());

        let mut not_announcement = announcement(1);
        not_announcement.announcement = false;
        assert!(not_announcement.validate().is_err());

        let mut invalid_tick = announcement(1);
        invalid_tick.year_tick = TICKS_PER_YEAR;
        assert!(invalid_tick.validate().is_err());
    }

    #[test]
    fn empty_retained_window_is_canonical() -> Result<()> {
        let mut assembler = AnnouncementWindowAssembler::new(source(), -1)?;
        assembler.push_page(AnnouncementPage {
            bridge_generation: 42,
            requested_after_report_id: -1,
            requested_maximum: 64,
            oldest_retained_report_id: -1,
            latest_retained_report_id: -1,
            window_latest_report_id: -1,
            next_after_report_id: -1,
            history_truncated: false,
            complete: true,
            announcements: Vec::new(),
        })?;
        let window = assembler.finalize()?;
        assert!(window.announcements.is_empty());
        assert!(window.can_prove_absence_in_frozen_interval());
        Ok(())
    }

    #[test]
    fn structured_or_canonical_byte_tampering_invalidates_window() -> Result<()> {
        let mut assembler = AnnouncementWindowAssembler::new(source(), -1)?;
        assembler.push_page(page(-1, 4, &[1, 2, 3, 4], true))?;
        let window = assembler.finalize()?;

        let mut structured = window.clone();
        structured.announcements[0].text = "tampered".to_owned();
        assert!(structured.validate().is_err());

        let mut bytes = window;
        bytes.canonical_bytes.push(0);
        assert!(bytes.validate().is_err());
        Ok(())
    }

    #[test]
    fn source_protocol_and_generation_are_part_of_identity() -> Result<()> {
        let mut first = AnnouncementWindowAssembler::new(source(), -1)?;
        first.push_page(page(-1, 4, &[1, 2, 3, 4], true))?;
        let first = first.finalize()?;

        let mut changed_source = source();
        changed_source.bridge_generation = 43;
        let mut second = AnnouncementWindowAssembler::new(changed_source, -1)?;
        let mut second_page = page(-1, 4, &[1, 2, 3, 4], true);
        second_page.bridge_generation = 43;
        second.push_page(second_page)?;
        let second = second.finalize()?;
        assert_ne!(first.content_digest, second.content_digest);
        Ok(())
    }
}
