//! PUB-OBS: root-last publication of observation capsules.
//!
//! `architecture/publication_primitives.json` pins PUB-OBS as
//! reserve(successor anchor) → materialize(capsule and version rows) →
//! publish(world root), with abort tombstoning the reservation and recovery
//! replaying materialized children or discarding them.
//!
//! [`CapsulePublisher`] implements that sequence in memory over a
//! [`DurableLedger`]. Children (the capsule, the successor snapshot) are fully
//! built and validated before the root is swapped; the root is one
//! `Arc<PublishedRoot>` replaced under a lock, so a [`RootReader`] observes
//! either the previous complete root or the next complete root, never a
//! partial one. The stages are linear types: [`CapsulePublisher::publish`]
//! consumes the [`Materialization`], so aborting a published capsule does not
//! type-check. Durable storage of the same discipline lives in the lab store.

use std::sync::{Arc, PoisonError, RwLock};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, StateAnchor};

use crate::delta::apply_delta;
use crate::ledger::{DurableLedger, ObservationCapsule};
use crate::model::WorldSnapshot;

/// One complete published world root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedRoot {
    /// Anchor of the published snapshot.
    pub anchor: StateAnchor,
    /// The complete published world version.
    pub snapshot: WorldSnapshot,
    /// Number of capsules published up to and including this root.
    pub capsule_high_water_mark: u64,
}

/// A reader handle that only ever sees complete published roots.
#[derive(Clone, Debug)]
pub struct RootReader(Arc<RwLock<Arc<PublishedRoot>>>);

impl RootReader {
    /// The currently published root.
    #[must_use]
    pub fn current(&self) -> Arc<PublishedRoot> {
        Arc::clone(&self.0.read().unwrap_or_else(PoisonError::into_inner))
    }
}

/// A reserved successor anchor; nothing is visible to readers yet.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a reservation must be materialized or aborted"]
pub struct Reservation {
    serial: u64,
    basis: StateAnchor,
    successor: StateAnchor,
}

impl Reservation {
    /// The anchor the successor is built on.
    #[must_use]
    pub const fn basis(&self) -> StateAnchor {
        self.basis
    }

    /// The reserved successor anchor.
    #[must_use]
    pub const fn successor(&self) -> StateAnchor {
        self.successor
    }
}

/// A validated, staged capsule whose root has not been published.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a materialization must be published or aborted"]
pub struct Materialization {
    reservation: Reservation,
    capsule_digest: Digest32,
}

/// What recovery found and did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Whether a staged, unpublished capsule was discarded.
    pub discarded_staged_capsule: bool,
    /// Whether an outstanding reservation was tombstoned.
    pub tombstoned_reservation: bool,
    /// Head anchor re-derived by replaying every published capsule from the root.
    pub rederived_head: StateAnchor,
    /// Capsules replayed.
    pub replayed_capsules: u64,
}

/// Root-last observation publisher over an in-memory ledger.
#[derive(Debug)]
pub struct CapsulePublisher {
    ledger: DurableLedger,
    root: Arc<RwLock<Arc<PublishedRoot>>>,
    next_serial: u64,
    outstanding: Option<u64>,
}

fn conflict(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

impl CapsulePublisher {
    /// Starts a publisher whose first published root is `ledger`'s head.
    #[must_use]
    pub fn new(ledger: DurableLedger) -> Self {
        let root = Arc::new(PublishedRoot {
            anchor: ledger.head_anchor(),
            snapshot: ledger.head_snapshot().clone(),
            capsule_high_water_mark: ledger.capsule_count() as u64,
        });
        Self {
            ledger,
            root: Arc::new(RwLock::new(root)),
            next_serial: 1,
            outstanding: None,
        }
    }

    /// A reader of complete published roots.
    #[must_use]
    pub fn reader(&self) -> RootReader {
        RootReader(Arc::clone(&self.root))
    }

    /// The underlying ledger (read-only).
    #[must_use]
    pub const fn ledger(&self) -> &DurableLedger {
        &self.ledger
    }

    /// Reserves `successor` on top of the current head. One reservation may be
    /// outstanding at a time; the successor must advance the head's cursor.
    pub fn reserve(&mut self, successor: StateAnchor) -> Result<Reservation> {
        if self.outstanding.is_some() || self.ledger.has_staged_capsule() {
            return Err(conflict("another observation publication is in flight"));
        }
        let basis = self.ledger.head_anchor();
        if successor.fortress_id != basis.fortress_id || successor.cursor <= basis.cursor {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "a reserved successor must belong to the head's fortress and advance its cursor",
            ));
        }
        let serial = self.next_serial;
        self.next_serial = self.next_serial.saturating_add(1);
        self.outstanding = Some(serial);
        Ok(Reservation {
            serial,
            basis,
            successor,
        })
    }

    /// Validates and stages `capsule` for `reservation`. On failure the
    /// reservation is tombstoned and the caller must reserve again.
    pub fn materialize(
        &mut self,
        reservation: Reservation,
        capsule: ObservationCapsule,
    ) -> Result<Materialization> {
        self.check_live(&reservation)?;
        if capsule.basis_anchor != reservation.basis
            || capsule.successor_anchor != reservation.successor
        {
            self.outstanding = None;
            return Err(conflict(
                "capsule anchors do not match its reservation; reservation tombstoned",
            ));
        }
        let capsule_digest = capsule.capsule_digest;
        if let Err(error) = self.ledger.stage_capsule(capsule) {
            self.outstanding = None;
            return Err(error);
        }
        Ok(Materialization {
            reservation,
            capsule_digest,
        })
    }

    /// Publishes the staged capsule: the ledger head advances, then the root
    /// is swapped last.
    pub fn publish(&mut self, materialization: Materialization) -> Result<Arc<PublishedRoot>> {
        self.check_live(&materialization.reservation)?;
        let anchor = self.ledger.publish_staged()?;
        self.outstanding = None;
        let root = Arc::new(PublishedRoot {
            anchor,
            snapshot: self.ledger.head_snapshot().clone(),
            capsule_high_water_mark: self.ledger.capsule_count() as u64,
        });
        *self.root.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&root);
        Ok(root)
    }

    /// Tombstones a reservation that was never materialized.
    pub fn abort(&mut self, reservation: Reservation) {
        if self.outstanding == Some(reservation.serial) {
            self.outstanding = None;
        }
    }

    /// Discards a materialized but unpublished capsule.
    pub fn abort_materialized(&mut self, materialization: Materialization) {
        if self.outstanding == Some(materialization.reservation.serial) {
            self.ledger.abort_staged();
            self.outstanding = None;
        }
    }

    /// Crash recovery: discard staged children, tombstone any reservation,
    /// re-derive the head from the root by replaying every published capsule,
    /// and republish that root. A replay that disagrees with the ledger head
    /// is a corrupt ledger and is refused rather than guessed at.
    pub fn recover(&mut self) -> Result<RecoveryReport> {
        let discarded_staged_capsule = self.ledger.has_staged_capsule();
        let tombstoned_reservation = self.outstanding.take().is_some();
        self.ledger.recover_from_crash();
        let mut head = self.ledger.root_snapshot().clone();
        for capsule in self.ledger.capsules() {
            if capsule.basis_anchor != head.anchor() {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "published capsule chain is broken",
                ));
            }
            head = apply_delta(&head, &capsule.delta)?;
            if head.anchor() != capsule.successor_anchor {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "published capsule does not reconstruct its successor",
                ));
            }
        }
        if &head != self.ledger.head_snapshot() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "re-derived head disagrees with the ledger head",
            ));
        }
        let rederived_head = head.anchor();
        let replayed_capsules = self.ledger.capsule_count() as u64;
        *self.root.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(PublishedRoot {
            anchor: rederived_head,
            snapshot: head,
            capsule_high_water_mark: replayed_capsules,
        });
        Ok(RecoveryReport {
            discarded_staged_capsule,
            tombstoned_reservation,
            rederived_head,
            replayed_capsules,
        })
    }

    fn check_live(&self, reservation: &Reservation) -> Result<()> {
        if self.outstanding == Some(reservation.serial)
            && self.ledger.head_anchor() == reservation.basis
        {
            Ok(())
        } else {
            Err(conflict(
                "reservation was tombstoned or its basis is no longer the head",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use dfmcp_core::{EntityId, FortressId, GameTick, ObservationCursor};

    use super::*;
    use crate::delta::diff_snapshots;
    use crate::model::{EntityKind, EntityRecord, WorldGraph};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn base() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(7),
            GameTick(100),
            ObservationCursor {
                epoch: 1,
                sequence: 1,
            },
            false,
            WorldGraph::default(),
        )
    }

    fn successor(
        previous: &WorldSnapshot,
        step: u64,
    ) -> Result<(WorldSnapshot, ObservationCapsule)> {
        let mut next = previous.clone();
        next.tick = GameTick(previous.tick.0 + 10);
        next.cursor = previous.cursor.next();
        next.graph.entities.insert(
            EntityId::new(step),
            EntityRecord {
                id: EntityId::new(step),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {step}"),
                fields: BTreeMap::new(),
            },
        );
        next.refresh_hash();
        let delta = diff_snapshots(previous, &next)?;
        let capsule = ObservationCapsule::new(previous.anchor(), next.anchor(), delta, next.tick)?;
        Ok((next, capsule))
    }

    fn publish_one(
        publisher: &mut CapsulePublisher,
        previous: &WorldSnapshot,
        step: u64,
    ) -> Result<WorldSnapshot> {
        let (next, capsule) = successor(previous, step)?;
        let reservation = publisher.reserve(next.anchor())?;
        let materialized = publisher.materialize(reservation, capsule)?;
        publisher.publish(materialized)?;
        Ok(next)
    }

    #[test]
    fn readers_see_only_complete_roots_while_a_writer_publishes() -> TestResult {
        let root = base();
        let mut publisher = CapsulePublisher::new(DurableLedger::new(root.clone()));
        let mut chain = vec![root.clone()];
        let mut previous = root;
        for step in 1..=64 {
            let (next, _) = successor(&previous, step)?;
            chain.push(next.clone());
            previous = next;
        }
        let legal: BTreeSet<_> = chain.iter().map(|s| s.state_hash).collect();
        let reader = publisher.reader();
        let observed = std::thread::scope(|scope| -> Result<Vec<u64>> {
            let readers: Vec<_> = (0..4)
                .map(|_| {
                    let reader = reader.clone();
                    let legal = &legal;
                    scope.spawn(move || {
                        let mut marks = Vec::new();
                        for _ in 0..2_000 {
                            let root = reader.current();
                            let whole = root.snapshot.hash_is_valid()
                                && root.snapshot.anchor() == root.anchor
                                && legal.contains(&root.anchor.state_hash)
                                && root.snapshot.graph.entities.len() as u64
                                    == root.capsule_high_water_mark;
                            marks.push(if whole {
                                root.capsule_high_water_mark
                            } else {
                                u64::MAX
                            });
                        }
                        marks
                    })
                })
                .collect();
            let mut previous = chain[0].clone();
            for step in 1..=64 {
                previous = publish_one(&mut publisher, &previous, step)?;
            }
            let mut all = Vec::new();
            for handle in readers {
                let marks = handle
                    .join()
                    .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "reader failed"))?;
                assert!(
                    marks.windows(2).all(|w| w[0] <= w[1]),
                    "roots never go back"
                );
                all.extend(marks);
            }
            Ok(all)
        })?;
        assert!(
            observed.iter().all(|mark| *mark != u64::MAX),
            "a partial root was visible"
        );
        assert_eq!(reader.current().capsule_high_water_mark, 64);
        assert_eq!(reader.current().anchor, chain[64].anchor());
        Ok(())
    }

    #[test]
    fn abort_stale_reservations_and_mismatched_capsules_never_publish() -> TestResult {
        let root = base();
        let mut publisher = CapsulePublisher::new(DurableLedger::new(root.clone()));
        let reader = publisher.reader();
        let (next, capsule) = successor(&root, 1)?;

        let reservation = publisher.reserve(next.anchor())?;
        assert!(
            publisher.reserve(next.anchor()).is_err(),
            "one publication in flight"
        );
        let materialized = publisher.materialize(reservation, capsule.clone())?;
        publisher.abort_materialized(materialized);
        assert_eq!(reader.current().anchor, root.anchor());
        assert!(!publisher.ledger().has_staged_capsule());

        let reservation = publisher.reserve(next.anchor())?;
        publisher.abort(reservation);
        let (other, other_capsule) = successor(&root, 2)?;
        let reservation = publisher.reserve(next.anchor())?;
        assert!(publisher.materialize(reservation, other_capsule).is_err());
        assert_eq!(reader.current().anchor, root.anchor());
        assert!(
            publisher.reserve(other.anchor()).is_ok(),
            "mismatch tombstoned"
        );
        Ok(())
    }

    #[test]
    fn a_crash_before_publish_recovers_the_identical_root() -> TestResult {
        let root = base();
        let mut publisher = CapsulePublisher::new(DurableLedger::new(root.clone()));
        let mut previous = root;
        for step in 1..=3 {
            previous = publish_one(&mut publisher, &previous, step)?;
        }
        let before = publisher.reader().current();
        let (next, capsule) = successor(&previous, 4)?;
        let reservation = publisher.reserve(next.anchor())?;
        // Crash between materialization and publication: the token is lost.
        let _lost = publisher.materialize(reservation, capsule)?;
        let report = publisher.recover()?;
        assert!(report.discarded_staged_capsule);
        assert!(report.tombstoned_reservation);
        assert_eq!(report.replayed_capsules, 3);
        assert_eq!(report.rederived_head, before.anchor);
        assert_eq!(*publisher.reader().current(), *before);
        // The publisher is usable again after recovery.
        publish_one(&mut publisher, &previous, 4)?;
        assert_eq!(publisher.reader().current().anchor, next.anchor());
        Ok(())
    }
}
