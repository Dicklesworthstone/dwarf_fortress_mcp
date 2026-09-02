#![forbid(unsafe_code)]

//! Agent-facing, authority-free orientation over canonical announcement batches.
//!
//! Protocol 1.1 does not claim a stable semantic taxonomy for every raw report
//! type. This layer therefore surfaces bounded records and mechanical
//! continuity findings without interpreting text as authority or claiming that
//! the retained suffix is complete fortress history.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::live_announcement_batch::{
    AnnouncementBatchRecord, LiveAnnouncementBatch,
};

pub const MAX_ANNOUNCEMENT_BRIEFING_RECORDS: usize = 64;
pub const MAX_ANNOUNCEMENT_ATTENTION_ITEMS: usize = 66;
pub const MAX_ANNOUNCEMENT_CHANGE_IDS: usize = 1_024;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnnouncementAttentionSeverity {
    High,
    Medium,
    Low,
}

impl AnnouncementAttentionSeverity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    #[must_use]
    const fn rank(self) -> u8 {
        match self {
            Self::High => 0,
            Self::Medium => 1,
            Self::Low => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementAttentionItem {
    pub attention_id: String,
    pub severity: AnnouncementAttentionSeverity,
    pub category: String,
    pub finding: String,
    pub report_ids: Vec<i32>,
    pub score_components: BTreeMap<String, i64>,
    pub source_digest: Digest32,
}

impl AnnouncementAttentionItem {
    fn sort_key(&self) -> (u8, &str, &str) {
        (
            self.severity.rank(),
            self.category.as_str(),
            self.attention_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementBriefing {
    pub source_digest: Digest32,
    pub bridge_generation: u64,
    pub requested_after_id: i32,
    pub next_after_id: i32,
    pub oldest_available_id: i32,
    pub latest_available_id: i32,
    pub returned: u32,
    pub complete_through_latest: bool,
    pub gap_before_retained_window: bool,
    pub complete_history: bool,
    pub records_truncated_for_briefing: bool,
    pub latest_records: Vec<AnnouncementBatchRecord>,
    pub attention: Vec<AnnouncementAttentionItem>,
}

impl LiveAnnouncementBriefing {
    #[must_use]
    pub const fn needs_continuation(&self) -> bool {
        !self.complete_through_latest
    }

    #[must_use]
    pub const fn can_prove_complete_history(&self) -> bool {
        self.complete_history
    }
}

pub fn build_live_announcement_briefing(
    batch: &LiveAnnouncementBatch,
) -> Result<LiveAnnouncementBriefing> {
    batch.validate()?;
    let mut latest_records = batch.announcements.clone();
    latest_records.reverse();
    let records_truncated_for_briefing =
        latest_records.len() > MAX_ANNOUNCEMENT_BRIEFING_RECORDS;
    latest_records.truncate(MAX_ANNOUNCEMENT_BRIEFING_RECORDS);

    let mut attention = Vec::new();
    if batch.coverage.has_gap() {
        attention.push(AnnouncementAttentionItem {
            attention_id: "live.announcements.retained_window_gap".to_owned(),
            severity: AnnouncementAttentionSeverity::High,
            category: "continuity".to_owned(),
            finding: format!(
                "the requested announcement cursor predates the oldest retained report ID {}; older history is unavailable",
                batch.coverage.oldest_available_id
            ),
            report_ids: Vec::new(),
            score_components: BTreeMap::from([
                ("gap_present".to_owned(), 1),
                (
                    "oldest_available_id".to_owned(),
                    i64::from(batch.coverage.oldest_available_id),
                ),
            ]),
            source_digest: batch.content_digest,
        });
    }
    if batch.coverage.needs_continuation() {
        attention.push(AnnouncementAttentionItem {
            attention_id: "live.announcements.partial_suffix".to_owned(),
            severity: AnnouncementAttentionSeverity::Medium,
            category: "coverage".to_owned(),
            finding: format!(
                "more retained announcements remain after report ID {}",
                batch.coverage.next_after_id
            ),
            report_ids: Vec::new(),
            score_components: BTreeMap::from([
                ("continuation_required".to_owned(), 1),
                (
                    "next_after_id".to_owned(),
                    i64::from(batch.coverage.next_after_id),
                ),
            ]),
            source_digest: batch.content_digest,
        });
    }
    for record in latest_records.iter().filter(|record| record.announcement) {
        attention.push(AnnouncementAttentionItem {
            attention_id: format!("live.announcement.{}", record.report_id),
            severity: AnnouncementAttentionSeverity::Low,
            category: "new_announcement".to_owned(),
            finding: record.text.clone(),
            report_ids: vec![record.report_id],
            score_components: BTreeMap::from([
                ("report_id".to_owned(), i64::from(record.report_id)),
                ("repeat_count".to_owned(), i64::from(record.repeat_count)),
            ]),
            source_digest: batch.content_digest,
        });
    }
    attention.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
    attention.truncate(MAX_ANNOUNCEMENT_ATTENTION_ITEMS);

    Ok(LiveAnnouncementBriefing {
        source_digest: batch.content_digest,
        bridge_generation: batch.bridge_generation,
        requested_after_id: batch.coverage.requested_after_id,
        next_after_id: batch.coverage.next_after_id,
        oldest_available_id: batch.coverage.oldest_available_id,
        latest_available_id: batch.coverage.latest_available_id,
        returned: batch.coverage.returned,
        complete_through_latest: batch.coverage.complete_through_latest,
        gap_before_retained_window: batch.coverage.has_gap(),
        complete_history: false,
        records_truncated_for_briefing,
        latest_records,
        attention,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementChangeSummary {
    pub basis_digest: Digest32,
    pub target_digest: Digest32,
    pub heartbeat: bool,
    pub added_report_ids: Vec<i32>,
    pub ids_truncated: bool,
    pub cursor_advanced: bool,
    pub retained_window_gap_introduced: bool,
    pub continuation_required: bool,
}

pub fn summarize_live_announcement_change(
    basis: &LiveAnnouncementBatch,
    target: &LiveAnnouncementBatch,
) -> Result<LiveAnnouncementChangeSummary> {
    basis.validate()?;
    target.validate()?;
    if basis.bridge_generation != target.bridge_generation || basis.site_id != target.site_id {
        return Err(error(
            ErrorCode::StaleAnchor,
            "cannot summarize announcements across bridge generations or fortress sites",
        ));
    }
    if basis.content_digest == target.content_digest {
        return Ok(LiveAnnouncementChangeSummary {
            basis_digest: basis.content_digest,
            target_digest: target.content_digest,
            heartbeat: true,
            added_report_ids: Vec::new(),
            ids_truncated: false,
            cursor_advanced: false,
            retained_window_gap_introduced: false,
            continuation_required: target.coverage.needs_continuation(),
        });
    }

    let basis_by_id = basis
        .announcements
        .iter()
        .map(|record| (record.report_id, record))
        .collect::<BTreeMap<_, _>>();
    let target_by_id = target
        .announcements
        .iter()
        .map(|record| (record.report_id, record))
        .collect::<BTreeMap<_, _>>();
    for (report_id, basis_record) in &basis_by_id {
        if let Some(target_record) = target_by_id.get(report_id)
            && *target_record != *basis_record
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                format!(
                    "announcement report ID {report_id} changed semantic content across observations"
                ),
            ));
        }
    }

    let basis_ids = basis_by_id.keys().copied().collect::<BTreeSet<_>>();
    let mut added_report_ids = Vec::new();
    let mut ids_truncated = false;
    for report_id in target_by_id.keys().copied() {
        if basis_ids.contains(&report_id) {
            continue;
        }
        if added_report_ids.len() >= MAX_ANNOUNCEMENT_CHANGE_IDS {
            ids_truncated = true;
            break;
        }
        added_report_ids.push(report_id);
    }

    Ok(LiveAnnouncementChangeSummary {
        basis_digest: basis.content_digest,
        target_digest: target.content_digest,
        heartbeat: false,
        added_report_ids,
        ids_truncated,
        cursor_advanced: target.coverage.next_after_id > basis.coverage.next_after_id,
        retained_window_gap_introduced: !basis.coverage.has_gap() && target.coverage.has_gap(),
        continuation_required: target.coverage.needs_continuation(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_announcement_batch::{
        AnnouncementContinuity, AnnouncementCoverage,
    };

    fn record(report_id: i32, text: &str, announcement: bool) -> AnnouncementBatchRecord {
        AnnouncementBatchRecord {
            report_id,
            report_type: 7,
            text: text.to_owned(),
            year: 105,
            year_tick: 12_000 + report_id,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement,
        }
    }

    fn batch(
        requested_after_id: i32,
        records: Vec<AnnouncementBatchRecord>,
        latest: i32,
        complete: bool,
        continuity: AnnouncementContinuity,
    ) -> Result<LiveAnnouncementBatch> {
        let oldest = records.first().map_or(-1, |record| record.report_id);
        let next = records
            .last()
            .map_or(requested_after_id, |record| record.report_id);
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id,
                oldest_available_id: oldest,
                latest_available_id: latest,
                returned: u32::try_from(records.len()).map_err(|_| {
                    error(ErrorCode::BudgetExceeded, "test record count does not fit u32")
                })?,
                complete_through_latest: complete,
                continuity,
                next_after_id: next,
            },
            records,
        )
    }

    #[test]
    fn briefing_never_claims_complete_history() -> Result<()> {
        let batch = batch(
            9,
            vec![record(10, "A caravan has arrived", true)],
            10,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let briefing = build_live_announcement_briefing(&batch)?;
        assert!(briefing.complete_through_latest);
        assert!(!briefing.can_prove_complete_history());
        assert_eq!(briefing.latest_records[0].report_id, 10);
        Ok(())
    }

    #[test]
    fn gap_and_partial_suffix_are_highest_attention() -> Result<()> {
        let batch = batch(
            1,
            vec![record(10, "retained", true)],
            11,
            false,
            AnnouncementContinuity::GapBeforeRetainedWindow,
        )?;
        let briefing = build_live_announcement_briefing(&batch)?;
        assert_eq!(
            briefing.attention[0].severity,
            AnnouncementAttentionSeverity::High
        );
        assert_eq!(
            briefing.attention[1].severity,
            AnnouncementAttentionSeverity::Medium
        );
        assert!(briefing.needs_continuation());
        Ok(())
    }

    #[test]
    fn latest_records_are_reverse_chronological_and_bounded() -> Result<()> {
        let records = (1..=100)
            .map(|id| record(id, &format!("report-{id}"), false))
            .collect::<Vec<_>>();
        let batch = batch(
            0,
            records,
            100,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let briefing = build_live_announcement_briefing(&batch)?;
        assert!(briefing.records_truncated_for_briefing);
        assert_eq!(
            briefing.latest_records.len(),
            MAX_ANNOUNCEMENT_BRIEFING_RECORDS
        );
        assert_eq!(briefing.latest_records[0].report_id, 100);
        assert_eq!(
            briefing
                .latest_records
                .last()
                .map(|record| record.report_id),
            Some(37)
        );
        Ok(())
    }

    #[test]
    fn change_summary_detects_added_reports_and_cursor_progress() -> Result<()> {
        let basis = batch(
            9,
            vec![record(10, "first", true)],
            10,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let target = batch(
            10,
            vec![record(11, "second", true)],
            11,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let summary = summarize_live_announcement_change(&basis, &target)?;
        assert_eq!(summary.added_report_ids, vec![11]);
        assert!(summary.cursor_advanced);
        assert!(!summary.heartbeat);
        Ok(())
    }

    #[test]
    fn same_report_id_with_changed_content_fails_closed() -> Result<()> {
        let basis = batch(
            9,
            vec![record(10, "first", true)],
            10,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let target = batch(
            9,
            vec![record(10, "tampered", true)],
            10,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        assert!(summarize_live_announcement_change(&basis, &target).is_err());
        Ok(())
    }

    #[test]
    fn generation_change_requires_reset_instead_of_diff() -> Result<()> {
        let basis = batch(
            9,
            vec![record(10, "first", true)],
            10,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        let mut target = batch(
            10,
            vec![record(11, "second", true)],
            11,
            true,
            AnnouncementContinuity::CompleteSuffix,
        )?;
        target.bridge_generation = 43;
        target.canonical_bytes[0] ^= 1;
        assert!(summarize_live_announcement_change(&basis, &target).is_err());
        Ok(())
    }
}
