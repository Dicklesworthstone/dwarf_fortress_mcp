#![forbid(unsafe_code)]

//! Deterministic semantic projection of one canonical announcement suffix.
//!
//! Announcement entities occupy a disjoint high-bit namespace. The projection
//! preserves cursor continuity and retained-window gaps; it never converts a
//! complete retained suffix into a claim of complete historical coverage.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, Digest32, EntityId, ErrorCode, GameTick, Result};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value};

use crate::{
    AnnouncementContinuity, AnnouncementRecord, LiveAnnouncementBatch,
};

const ANNOUNCEMENT_ENTITY_NAMESPACE: u64 = 1u64 << 63;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub fn report_id_to_announcement_entity_id(report_id: i32) -> Result<EntityId> {
    if report_id < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "announcement report ID must not be negative",
        ));
    }
    let ordinal = u64::from(report_id as u32)
        .checked_add(1)
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "announcement entity ordinal overflowed",
            )
        })?;
    Ok(EntityId::new(ANNOUNCEMENT_ENTITY_NAMESPACE | ordinal))
}

#[must_use]
pub fn announcement_entity_id_to_report_id(entity_id: EntityId) -> Option<i32> {
    let encoded = entity_id.get();
    if encoded & ANNOUNCEMENT_ENTITY_NAMESPACE == 0 {
        return None;
    }
    let ordinal = encoded & !ANNOUNCEMENT_ENTITY_NAMESPACE;
    let raw = ordinal.checked_sub(1)?;
    i32::try_from(raw).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncementProjection {
    pub source_digest: Digest32,
    pub requested_after_id: i32,
    pub next_after_id: i32,
    pub oldest_available_id: i32,
    pub latest_available_id: i32,
    pub complete_through_latest: bool,
    pub continuity: AnnouncementContinuity,
    pub entities: BTreeMap<EntityId, EntityRecord>,
}

impl LiveAnnouncementProjection {
    pub fn validate_against(&self, batch: &LiveAnnouncementBatch) -> Result<()> {
        batch.validate()?;
        if self.source_digest != batch.content_digest
            || self.requested_after_id != batch.coverage.requested_after_id
            || self.next_after_id != batch.coverage.next_after_id
            || self.oldest_available_id != batch.coverage.oldest_available_id
            || self.latest_available_id != batch.coverage.latest_available_id
            || self.complete_through_latest != batch.coverage.complete_through_latest
            || self.continuity != batch.coverage.continuity
            || self.entities.len() != batch.announcements.len()
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement projection does not bind its source batch and coverage",
            ));
        }
        for record in &batch.announcements {
            let entity_id = report_id_to_announcement_entity_id(record.report_id)?;
            let entity = self.entities.get(&entity_id).ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "announcement projection is missing a source report entity",
                )
            })?;
            validate_entity(entity, record, batch.content_digest)?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn historical_gap_is_known(&self) -> bool {
        matches!(
            self.continuity,
            AnnouncementContinuity::GapBeforeRetainedWindow
        )
    }
}

pub fn project_live_announcement_batch(
    batch: &LiveAnnouncementBatch,
    observed_at: GameTick,
    generation: u32,
    revision: u64,
) -> Result<LiveAnnouncementProjection> {
    batch.validate()?;
    if generation == 0 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "announcement entity generation zero is reserved",
        ));
    }
    let mut entities = BTreeMap::new();
    for record in &batch.announcements {
        let entity = announcement_entity(
            record,
            observed_at,
            batch.content_digest,
            generation,
            revision,
        )?;
        if entities.insert(entity.id, entity).is_some() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement projection produced a duplicate entity ID",
            ));
        }
    }
    let projection = LiveAnnouncementProjection {
        source_digest: batch.content_digest,
        requested_after_id: batch.coverage.requested_after_id,
        next_after_id: batch.coverage.next_after_id,
        oldest_available_id: batch.coverage.oldest_available_id,
        latest_available_id: batch.coverage.latest_available_id,
        complete_through_latest: batch.coverage.complete_through_latest,
        continuity: batch.coverage.continuity,
        entities,
    };
    projection.validate_against(batch)?;
    Ok(projection)
}

fn announcement_entity(
    record: &AnnouncementRecord,
    observed_at: GameTick,
    source_digest: Digest32,
    generation: u32,
    revision: u64,
) -> Result<EntityRecord> {
    record.validate()?;
    let mut fields = BTreeMap::new();
    for (name, value, source_field) in [
        (
            "report_id",
            Value::I64(i64::from(record.report_id)),
            "announcement.report_id",
        ),
        (
            "report_type",
            Value::I64(i64::from(record.report_type)),
            "announcement.report_type",
        ),
        (
            "text",
            Value::Text(record.text.clone()),
            "announcement.text",
        ),
        (
            "year",
            Value::I64(i64::from(record.year)),
            "announcement.year",
        ),
        (
            "year_tick",
            Value::I64(i64::from(record.year_tick)),
            "announcement.year_tick",
        ),
        (
            "repeat_count",
            Value::I64(i64::from(record.repeat_count)),
            "announcement.repeat_count",
        ),
        (
            "continuation",
            Value::Bool(record.continuation),
            "announcement.continuation",
        ),
        (
            "unconscious",
            Value::Bool(record.unconscious),
            "announcement.unconscious",
        ),
        (
            "announcement",
            Value::Bool(record.announcement),
            "announcement.announcement",
        ),
    ] {
        fields.insert(
            name.to_owned(),
            Fact::known(
                value,
                observed_at,
                FactSource::DfhackField(format!(
                    "dfmcp_bridge.ReadObservation.{source_field}"
                )),
                source_digest,
            ),
        );
    }
    Ok(EntityRecord {
        id: report_id_to_announcement_entity_id(record.report_id)?,
        generation,
        revision,
        kind: EntityKind::Announcement,
        label: format!("announcement-{}", record.report_id),
        fields,
    })
}

fn validate_entity(
    entity: &EntityRecord,
    record: &AnnouncementRecord,
    source_digest: Digest32,
) -> Result<()> {
    if entity.kind != EntityKind::Announcement
        || entity.id != report_id_to_announcement_entity_id(record.report_id)?
        || entity.label != format!("announcement-{}", record.report_id)
    {
        return Err(error(
            ErrorCode::CorruptLedger,
            "announcement entity identity or kind is invalid",
        ));
    }
    if entity.fields.len() != 9 {
        return Err(error(
            ErrorCode::CorruptLedger,
            "announcement entity field set is incomplete",
        ));
    }
    for fact in entity.fields.values() {
        if fact.source_digest != source_digest {
            return Err(error(
                ErrorCode::CorruptLedger,
                "announcement fact does not cite its canonical source batch",
            ));
        }
    }
    let expected = [
        ("report_id", Value::I64(i64::from(record.report_id))),
        ("report_type", Value::I64(i64::from(record.report_type))),
        ("text", Value::Text(record.text.clone())),
        ("year", Value::I64(i64::from(record.year))),
        ("year_tick", Value::I64(i64::from(record.year_tick))),
        ("repeat_count", Value::I64(i64::from(record.repeat_count))),
        ("continuation", Value::Bool(record.continuation)),
        ("unconscious", Value::Bool(record.unconscious)),
        ("announcement", Value::Bool(record.announcement)),
    ];
    for (field, value) in expected {
        let fact = entity.fields.get(field).ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                format!("announcement entity is missing {field}"),
            )
        })?;
        if fact.value != value || fact.presence.is_some() {
            return Err(error(
                ErrorCode::CorruptLedger,
                format!("announcement entity field {field} does not match its source"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnnouncementCoverage, LiveAnnouncementBatch};

    fn record(report_id: i32) -> AnnouncementRecord {
        AnnouncementRecord {
            report_id,
            report_type: 7,
            text: format!("report-{report_id}"),
            year: 105,
            year_tick: 12_345 + report_id,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn batch(continuity: AnnouncementContinuity) -> Result<LiveAnnouncementBatch> {
        let records = vec![record(10), record(11)];
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_400,
            7,
            AnnouncementCoverage {
                requested_after_id: if matches!(
                    continuity,
                    AnnouncementContinuity::GapBeforeRetainedWindow
                ) {
                    1
                } else {
                    9
                },
                oldest_available_id: 10,
                latest_available_id: 11,
                returned: 2,
                complete_through_latest: true,
                continuity,
                next_after_id: 11,
            },
            records,
        )
    }

    #[test]
    fn announcement_namespace_round_trips_without_unit_collision() -> Result<()> {
        let entity = report_id_to_announcement_entity_id(0)?;
        assert_eq!(announcement_entity_id_to_report_id(entity), Some(0));
        assert_eq!(announcement_entity_id_to_report_id(EntityId::new(1)), None);
        assert_ne!(entity, crate::FORTRESS_ENTITY_ID);
        Ok(())
    }

    #[test]
    fn projection_is_deterministic_and_source_bound() -> Result<()> {
        let batch = batch(AnnouncementContinuity::CompleteSuffix)?;
        let first = project_live_announcement_batch(&batch, GameTick::new(42), 1, 7)?;
        let second = project_live_announcement_batch(&batch, GameTick::new(42), 1, 7)?;
        assert_eq!(first, second);
        assert_eq!(first.entities.len(), 2);
        first.validate_against(&batch)
    }

    #[test]
    fn retained_window_gap_survives_projection() -> Result<()> {
        let batch = batch(AnnouncementContinuity::GapBeforeRetainedWindow)?;
        let projection = project_live_announcement_batch(&batch, GameTick::new(42), 1, 7)?;
        assert!(projection.historical_gap_is_known());
        assert!(projection.complete_through_latest);
        Ok(())
    }

    #[test]
    fn projected_fact_tampering_fails_closed() -> Result<()> {
        let batch = batch(AnnouncementContinuity::CompleteSuffix)?;
        let mut projection = project_live_announcement_batch(&batch, GameTick::new(42), 1, 7)?;
        let entity_id = report_id_to_announcement_entity_id(10)?;
        let entity = projection.entities.get_mut(&entity_id).ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "test entity missing")
        })?;
        let text = entity.fields.get_mut("text").ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "test text fact missing")
        })?;
        text.value = Value::Text("tampered".to_owned());
        assert!(projection.validate_against(&batch).is_err());
        Ok(())
    }

    #[test]
    fn zero_generation_and_negative_report_ids_are_rejected() -> Result<()> {
        let batch = batch(AnnouncementContinuity::CompleteSuffix)?;
        assert!(project_live_announcement_batch(&batch, GameTick::new(42), 0, 7).is_err());
        assert!(report_id_to_announcement_entity_id(-1).is_err());
        Ok(())
    }
}
