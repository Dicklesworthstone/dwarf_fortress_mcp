//! Bounded discovery of exact retained recording identities.
//!
//! The recording ordinal is retention metadata, not an observation cursor.
//! Copy anchors only: discovering history must not clone or serialize worlds.

use dfmcp_core::{DfmcpError, ErrorCode, Result, StateAnchor};

use super::VersionRetention;

impl VersionRetention {
    /// Return all retained recording ordinals and complete anchors, oldest first.
    ///
    /// The complete membership bound is checked before allocation. Re-recording
    /// an older world moves its ordinal just as in `consecutive_range`; gaps in
    /// those ordinals remain visible and must never be inferred from game ticks.
    /// Reading does not pin, collect, re-record, or validate world content.
    pub fn recorded_anchors(&self, max_versions: usize) -> Result<Vec<(u64, StateAnchor)>> {
        if self.versions.len() > max_versions {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "retained anchor catalog exceeds its complete metadata record bound",
            ));
        }
        let mut records = Vec::with_capacity(self.versions.len());
        for (hash, (sequence, snapshot)) in &self.versions {
            if *hash != snapshot.state_hash || *sequence == 0 {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "retained anchor catalog has an inconsistent recording identity",
                ));
            }
            records.push((*sequence, snapshot.anchor()));
        }
        records.sort_unstable_by_key(|(sequence, _)| *sequence);
        if records.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "retained anchor catalog has duplicate recording ordinals",
            ));
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};
    use crate::{WorldGraph, WorldSnapshot};

    fn world(tick: u64, sequence: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor { epoch: 1, sequence },
            false,
            WorldGraph::default(),
        )
    }

    #[test]
    fn catalog_exposes_recording_order_without_inventing_missing_positions() -> Result<()> {
        let versions = [world(10, 100), world(20, 900), world(30, 901)];
        let mut retained = VersionRetention::new(1, 1);
        for snapshot in &versions {
            retained.record(snapshot);
        }
        retained.collect(&[versions[0].state_hash].into_iter().collect());
        let before = format!("{retained:?}");
        assert_eq!(
            retained.recorded_anchors(2)?,
            vec![(1, versions[0].anchor()), (3, versions[2].anchor())]
        );
        assert_eq!(format!("{retained:?}"), before);
        assert_eq!(
            retained.recorded_anchors(1).unwrap_err().code,
            ErrorCode::BudgetExceeded
        );
        // Re-recording changes recording order, not the original cursor/tick.
        retained.record(&versions[0]);
        assert_eq!(
            retained.recorded_anchors(2)?,
            vec![(3, versions[2].anchor()), (4, versions[0].anchor())]
        );
        assert!(VersionRetention::new(1, 0).recorded_anchors(0)?.is_empty());
        Ok(())
    }
}
