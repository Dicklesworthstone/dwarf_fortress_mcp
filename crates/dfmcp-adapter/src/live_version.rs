#![forbid(unsafe_code)]

//! Observation-epoch and lineage tracking for the live DFHack read path.
//!
//! A semantic state change advances the sequence inside one epoch. An unchanged
//! complete capsule is a heartbeat. A bridge restart or compatible tool-version
//! transition starts a fresh epoch. Loading a different fortress is not a
//! resumable reset and is rejected so callers must open a new fortress lineage.

use dfmcp_core::{
    ContinuityStatus, DfmcpError, Digest32, ErrorCode, FortressId, ObservationCursor, Result,
};

use crate::LiveObservationCapsule;

const MAX_IDENTITY_TEXT_BYTES: usize = 512;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn validate_text(value: &str, field: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_IDENTITY_TEXT_BYTES {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!(
                "live identity field {field} must contain 1..={MAX_IDENTITY_TEXT_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveWorldIdentity {
    pub site_id: i32,
    pub world_folder: String,
    pub world_name: String,
}

impl LiveWorldIdentity {
    pub fn from_capsule(capsule: &LiveObservationCapsule) -> Result<Self> {
        if capsule.site_id < 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "live fortress site ID must be nonnegative",
            ));
        }
        validate_text(&capsule.world_folder, "world_folder")?;
        validate_text(&capsule.world_name, "world_name")?;
        Ok(Self {
            site_id: capsule.site_id,
            world_folder: capsule.world_folder.clone(),
            world_name: capsule.world_name.clone(),
        })
    }

    #[must_use]
    pub fn same_fortress(&self, other: &Self) -> bool {
        self.site_id == other.site_id && self.world_folder == other.world_folder
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCompatibilityIdentity {
    pub dwarf_fortress_version: String,
    pub dfhack_version: String,
    pub bridge_version: String,
}

impl LiveCompatibilityIdentity {
    pub fn from_capsule(capsule: &LiveObservationCapsule) -> Result<Self> {
        validate_text(&capsule.bridge.df_version, "dwarf_fortress_version")?;
        validate_text(&capsule.bridge.dfhack_version, "dfhack_version")?;
        validate_text(&capsule.bridge.bridge_version, "bridge_version")?;
        Ok(Self {
            dwarf_fortress_version: capsule.bridge.df_version.clone(),
            dfhack_version: capsule.bridge.dfhack_version.clone(),
            bridge_version: capsule.bridge.bridge_version.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEpochResetReason {
    BridgeRestart,
    CompatibilityChanged,
}

impl LiveEpochResetReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BridgeRestart => "bridge_restart",
            Self::CompatibilityChanged => "compatibility_changed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveVersionDecision {
    pub cursor: ObservationCursor,
    pub continuity: ContinuityStatus,
    pub reset_reason: Option<LiveEpochResetReason>,
    pub previous_cursor: Option<ObservationCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveVersionTracker {
    fortress_id: FortressId,
    initial_epoch: u64,
    cursor: Option<ObservationCursor>,
    world: Option<LiveWorldIdentity>,
    compatibility: Option<LiveCompatibilityIdentity>,
    bridge_generation: Option<u64>,
    capsule_digest: Option<Digest32>,
}

impl LiveVersionTracker {
    pub fn new(fortress_id: FortressId, initial_epoch: u64) -> Result<Self> {
        if fortress_id == FortressId::NIL {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live version tracker fortress lineage must not be zero",
            ));
        }
        Ok(Self {
            fortress_id,
            initial_epoch,
            cursor: None,
            world: None,
            compatibility: None,
            bridge_generation: None,
            capsule_digest: None,
        })
    }

    #[must_use]
    pub const fn fortress_id(&self) -> FortressId {
        self.fortress_id
    }

    #[must_use]
    pub const fn cursor(&self) -> Option<ObservationCursor> {
        self.cursor
    }

    #[must_use]
    pub fn world_identity(&self) -> Option<&LiveWorldIdentity> {
        self.world.as_ref()
    }

    #[must_use]
    pub fn compatibility_identity(&self) -> Option<&LiveCompatibilityIdentity> {
        self.compatibility.as_ref()
    }

    pub fn observe(&mut self, capsule: &LiveObservationCapsule) -> Result<LiveVersionDecision> {
        capsule.validate()?;
        let world = LiveWorldIdentity::from_capsule(capsule)?;
        let compatibility = LiveCompatibilityIdentity::from_capsule(capsule)?;
        let generation = capsule.bridge.bridge_generation;
        let digest = capsule.content_digest;

        let Some(previous_cursor) = self.cursor else {
            let cursor = ObservationCursor {
                epoch: self.initial_epoch,
                sequence: 0,
            };
            self.cursor = Some(cursor);
            self.world = Some(world);
            self.compatibility = Some(compatibility);
            self.bridge_generation = Some(generation);
            self.capsule_digest = Some(digest);
            return Ok(LiveVersionDecision {
                cursor,
                continuity: ContinuityStatus::Bootstrap,
                reset_reason: None,
                previous_cursor: None,
            });
        };

        let previous_world = self.world.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live version cursor exists without a world identity",
            )
        })?;
        if !previous_world.same_fortress(&world) {
            return Err(error(
                ErrorCode::RestoreRequired,
                format!(
                    "DFHack now exposes site {} in {:?}, but this session is bound to site {} in {:?}; open a new fortress session",
                    world.site_id,
                    world.world_folder,
                    previous_world.site_id,
                    previous_world.world_folder
                ),
            ));
        }

        let previous_compatibility = self.compatibility.as_ref().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live version cursor exists without compatibility identity",
            )
        })?;
        let previous_generation = self.bridge_generation.ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live version cursor exists without bridge generation",
            )
        })?;
        let previous_digest = self.capsule_digest.ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "live version cursor exists without capsule digest",
            )
        })?;

        let reset_reason = if previous_compatibility != &compatibility {
            Some(LiveEpochResetReason::CompatibilityChanged)
        } else if previous_generation != generation {
            Some(LiveEpochResetReason::BridgeRestart)
        } else {
            None
        };

        if let Some(reason) = reset_reason {
            let epoch = previous_cursor.epoch.checked_add(1).ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "live observation epoch space is exhausted",
                )
            })?;
            let cursor = ObservationCursor { epoch, sequence: 0 };
            self.cursor = Some(cursor);
            self.world = Some(world);
            self.compatibility = Some(compatibility);
            self.bridge_generation = Some(generation);
            self.capsule_digest = Some(digest);
            return Ok(LiveVersionDecision {
                cursor,
                continuity: ContinuityStatus::Reset,
                reset_reason: Some(reason),
                previous_cursor: Some(previous_cursor),
            });
        }

        self.world = Some(world);
        if previous_digest == digest {
            return Ok(LiveVersionDecision {
                cursor: previous_cursor,
                continuity: ContinuityStatus::Heartbeat,
                reset_reason: None,
                previous_cursor: Some(previous_cursor),
            });
        }

        let sequence = previous_cursor.sequence.checked_add(1).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "live observation sequence space is exhausted",
            )
        })?;
        let cursor = ObservationCursor {
            epoch: previous_cursor.epoch,
            sequence,
        };
        self.cursor = Some(cursor);
        self.capsule_digest = Some(digest);
        Ok(LiveVersionDecision {
            cursor,
            continuity: ContinuityStatus::Continuous,
            reset_reason: None,
            previous_cursor: Some(previous_cursor),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{BridgeManifest, CitizenCoverage};

    fn capsule(
        digest_discriminator: u8,
        generation: u64,
        site_id: i32,
        folder: &str,
        df_version: &str,
    ) -> Result<LiveObservationCapsule> {
        let canonical_bytes = vec![digest_discriminator];
        let content_digest = Digest32::of_bytes(&canonical_bytes);
        let value = LiveObservationCapsule {
            bridge: BridgeManifest {
                bridge_version: "0.1.0".to_owned(),
                dfhack_version: "0.51.11-r1".to_owned(),
                df_version: df_version.to_owned(),
                world_loaded: true,
                fortress_mode: true,
                bridge_generation: generation,
                supported_methods: BTreeSet::from([
                    "Handshake".to_owned(),
                    "ReadObservation".to_owned(),
                ]),
            },
            paused: true,
            current_year: 105,
            current_year_tick: 12345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: folder.to_owned(),
            site_id,
            citizen_coverage: CitizenCoverage {
                offset: 0,
                returned: 0,
                total: 0,
                complete: true,
            },
            citizens: Vec::new(),
            canonical_bytes,
            content_digest,
        };
        value.validate()?;
        Ok(value)
    }

    #[test]
    fn bootstrap_change_and_heartbeat_have_distinct_semantics() -> Result<()> {
        let mut tracker = LiveVersionTracker::new(FortressId::new(7), 4)?;
        let bootstrap = tracker.observe(&capsule(1, 10, 3, "region1", "0.51.11")?)?;
        assert_eq!(bootstrap.continuity, ContinuityStatus::Bootstrap);
        assert_eq!(bootstrap.cursor, ObservationCursor { epoch: 4, sequence: 0 });

        let heartbeat = tracker.observe(&capsule(1, 10, 3, "region1", "0.51.11")?)?;
        assert_eq!(heartbeat.continuity, ContinuityStatus::Heartbeat);
        assert_eq!(heartbeat.cursor, bootstrap.cursor);

        let changed = tracker.observe(&capsule(2, 10, 3, "region1", "0.51.11")?)?;
        assert_eq!(changed.continuity, ContinuityStatus::Continuous);
        assert_eq!(changed.cursor, ObservationCursor { epoch: 4, sequence: 1 });
        Ok(())
    }

    #[test]
    fn bridge_restart_starts_a_new_epoch() -> Result<()> {
        let mut tracker = LiveVersionTracker::new(FortressId::new(7), 4)?;
        let first = tracker.observe(&capsule(1, 10, 3, "region1", "0.51.11")?)?;
        let restarted = tracker.observe(&capsule(1, 11, 3, "region1", "0.51.11")?)?;
        assert_eq!(restarted.continuity, ContinuityStatus::Reset);
        assert_eq!(
            restarted.reset_reason,
            Some(LiveEpochResetReason::BridgeRestart)
        );
        assert_eq!(restarted.previous_cursor, Some(first.cursor));
        assert_eq!(restarted.cursor, ObservationCursor { epoch: 5, sequence: 0 });
        Ok(())
    }

    #[test]
    fn version_change_starts_a_new_epoch() -> Result<()> {
        let mut tracker = LiveVersionTracker::new(FortressId::new(7), 4)?;
        tracker.observe(&capsule(1, 10, 3, "region1", "0.51.11")?)?;
        let changed = tracker.observe(&capsule(1, 10, 3, "region1", "0.52.0")?)?;
        assert_eq!(changed.continuity, ContinuityStatus::Reset);
        assert_eq!(
            changed.reset_reason,
            Some(LiveEpochResetReason::CompatibilityChanged)
        );
        Ok(())
    }

    #[test]
    fn different_fortress_requires_a_new_session() -> Result<()> {
        let mut tracker = LiveVersionTracker::new(FortressId::new(7), 4)?;
        tracker.observe(&capsule(1, 10, 3, "region1", "0.51.11")?)?;
        let failure = tracker
            .observe(&capsule(2, 10, 4, "region2", "0.51.11")?)
            .err()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "different fortress was accepted by the lineage tracker",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::RestoreRequired);
        Ok(())
    }
}
