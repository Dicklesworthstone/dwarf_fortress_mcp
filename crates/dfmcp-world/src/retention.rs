//! Reachability-based retention of immutable world versions.
//!
//! A version stays readable while it is *live*: one of the newest
//! `recent_window` versions, or named by a live root (a prepared plan's
//! anchor, a checkpoint, any anchor a caller still holds work against).
//! [`VersionRetention::collect`] frees exactly the versions no live root
//! reaches and leaves a bounded tombstone for each, so a later read can say
//! *why* a version is gone (collected because unreachable) instead of
//! guessing. Retained versions are full canonical snapshots, so reads at any
//! retained anchor are exact; nothing retained depends on a collected version.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::Digest32;

use crate::model::WorldSnapshot;

/// Tombstones kept for collected versions before the oldest are forgotten.
pub const MAX_TOMBSTONES: usize = 4_096;

/// Where a requested version stands.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionStatus<'a> {
    /// Retained and readable exactly.
    Retained(&'a WorldSnapshot),
    /// Seen, then collected because no live root reached it.
    Collected {
        /// Recording sequence at which the version was last seen.
        last_seen: u64,
        /// Recording sequence at which it was collected.
        collected_at: u64,
    },
    /// Never seen here (or its tombstone aged out).
    Unknown,
}

/// What one collection pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// Versions retained after the pass.
    pub retained: usize,
    /// Versions collected by this pass, oldest first.
    pub collected: Vec<Digest32>,
    /// Retained versions kept only because a root names them.
    pub pinned_outside_window: usize,
    /// Roots that named no retained version (already collected or unknown).
    pub dangling_roots: usize,
}

/// Bounded store of immutable world versions with reachability retention.
#[derive(Clone, Debug)]
pub struct VersionRetention {
    recent_window: usize,
    max_pinned: usize,
    next_seq: u64,
    versions: BTreeMap<Digest32, (u64, WorldSnapshot)>,
    tombstones: BTreeMap<Digest32, (u64, u64)>,
    tombstone_order: VecDeque<Digest32>,
}

impl VersionRetention {
    /// Keeps the newest `recent_window` versions (at least one) plus at most
    /// `max_pinned` older versions named by live roots.
    #[must_use]
    pub fn new(recent_window: usize, max_pinned: usize) -> Self {
        Self {
            recent_window: recent_window.max(1),
            max_pinned,
            next_seq: 0,
            versions: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            tombstone_order: VecDeque::new(),
        }
    }

    /// Records `snapshot` as the newest version. Re-recording a known version
    /// (a restore back to it) makes it newest again.
    pub fn record(&mut self, snapshot: &WorldSnapshot) {
        if self
            .newest()
            .is_some_and(|newest| newest.state_hash == snapshot.state_hash)
        {
            return;
        }
        self.next_seq += 1;
        let seq = self.next_seq;
        self.tombstones.remove(&snapshot.state_hash);
        self.versions
            .entry(snapshot.state_hash)
            .and_modify(|entry| entry.0 = seq)
            .or_insert_with(|| (seq, snapshot.clone()));
    }

    /// Frees every version that is neither in the recent window nor named by
    /// `roots`. If more than `max_pinned` older versions are rooted, the
    /// oldest of them are collected too and counted as dangling.
    pub fn collect(&mut self, roots: &BTreeSet<Digest32>) -> RetentionReport {
        let mut by_age: Vec<(u64, Digest32)> = self
            .versions
            .iter()
            .map(|(hash, (seq, _))| (*seq, *hash))
            .collect();
        by_age.sort_unstable_by(|a, b| b.cmp(a));
        let window: BTreeSet<Digest32> = by_age
            .iter()
            .take(self.recent_window)
            .map(|(_, hash)| *hash)
            .collect();
        let pinned: BTreeSet<Digest32> = by_age
            .iter()
            .filter(|(_, hash)| !window.contains(hash) && roots.contains(hash))
            .take(self.max_pinned)
            .map(|(_, hash)| *hash)
            .collect();
        let mut report = RetentionReport {
            pinned_outside_window: pinned.len(),
            dangling_roots: roots
                .iter()
                .filter(|root| !window.contains(*root) && !pinned.contains(*root))
                .count(),
            ..RetentionReport::default()
        };
        for (seq, hash) in by_age.into_iter().rev() {
            if window.contains(&hash) || pinned.contains(&hash) {
                continue;
            }
            self.versions.remove(&hash);
            self.tombstones.insert(hash, (seq, self.next_seq));
            self.tombstone_order.push_back(hash);
            report.collected.push(hash);
        }
        while self.tombstone_order.len() > MAX_TOMBSTONES {
            if let Some(old) = self.tombstone_order.pop_front()
                && !self.versions.contains_key(&old)
            {
                self.tombstones.remove(&old);
            }
        }
        report.retained = self.versions.len();
        report
    }

    /// Where version `hash` stands.
    #[must_use]
    pub fn status(&self, hash: &Digest32) -> VersionStatus<'_> {
        if let Some((_, snapshot)) = self.versions.get(hash) {
            return VersionStatus::Retained(snapshot);
        }
        match self.tombstones.get(hash) {
            Some(&(last_seen, collected_at)) => VersionStatus::Collected {
                last_seen,
                collected_at,
            },
            None => VersionStatus::Unknown,
        }
    }

    /// The retained version `hash`, if any.
    #[must_use]
    pub fn get(&self, hash: &Digest32) -> Option<&WorldSnapshot> {
        self.versions.get(hash).map(|(_, snapshot)| snapshot)
    }

    /// The most recently recorded version.
    #[must_use]
    pub fn newest(&self) -> Option<&WorldSnapshot> {
        self.versions
            .values()
            .max_by_key(|(seq, _)| *seq)
            .map(|(_, snapshot)| snapshot)
    }

    /// The least recently recorded retained version.
    #[must_use]
    pub fn oldest(&self) -> Option<&WorldSnapshot> {
        self.versions
            .values()
            .min_by_key(|(seq, _)| *seq)
            .map(|(_, snapshot)| snapshot)
    }

    /// Number of retained versions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether nothing is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// Looks up a retained version by its hex state hash.
    #[must_use]
    pub fn find_hex(&self, hex: &str) -> Option<Digest32> {
        self.versions
            .keys()
            .chain(self.tombstones.keys())
            .find(|hash| hash.to_hex() == hex)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};

    use super::*;
    use crate::model::WorldGraph;

    fn version(tick: u64, epoch: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor {
                epoch,
                sequence: tick,
            },
            false,
            WorldGraph::default(),
        )
    }

    #[test]
    fn branchy_history_keeps_exactly_what_live_roots_reach() {
        let mut store = VersionRetention::new(3, 8);
        // Main line 1..=10; a restore back to 4 opens a branch (epoch 2) 11..=14.
        let main: Vec<_> = (1..=10).map(|t| version(t, 1)).collect();
        let branch: Vec<_> = (11..=14).map(|t| version(t, 2)).collect();
        for v in &main {
            store.record(v);
        }
        store.record(&main[3]);
        for v in &branch {
            store.record(v);
        }
        // Live roots: a plan sealed on main[1], a checkpoint of main[7], and a
        // root naming a version never seen here.
        let roots: BTreeSet<_> = [
            main[1].state_hash,
            main[7].state_hash,
            Digest32::of_bytes(b"never"),
        ]
        .into_iter()
        .collect();
        let report = store.collect(&roots);
        assert_eq!(report.pinned_outside_window, 2);
        assert_eq!(report.dangling_roots, 1);
        assert_eq!(report.retained, 5);

        // Every retained anchor reads exactly; every dropped one is provably
        // unreachable: not in the window and named by no root.
        let window: BTreeSet<_> = branch[1..].iter().map(|v| v.state_hash).collect();
        for v in main.iter().chain(&branch) {
            match store.status(&v.state_hash) {
                VersionStatus::Retained(read) => {
                    assert_eq!(read, v);
                    assert_eq!(read.canonical_bytes(), v.canonical_bytes());
                    assert!(window.contains(&v.state_hash) || roots.contains(&v.state_hash));
                }
                VersionStatus::Collected { .. } => {
                    assert!(!window.contains(&v.state_hash) && !roots.contains(&v.state_hash));
                    assert!(report.collected.contains(&v.state_hash));
                }
                VersionStatus::Unknown => panic!("a recorded version has no status"),
            }
        }
        assert_eq!(store.newest(), Some(&branch[3]));
        assert_eq!(
            store.status(&Digest32::of_bytes(b"other")),
            VersionStatus::Unknown
        );

        // Releasing a root makes its version collectable on the next pass.
        let report = store.collect(&[main[7].state_hash].into_iter().collect());
        assert_eq!(report.collected, vec![main[1].state_hash]);
        // Re-recording a collected version revives it as newest.
        store.record(&main[1]);
        assert_eq!(store.newest(), Some(&main[1]));
    }

    #[test]
    fn growth_stays_bounded_under_a_long_workload() {
        let mut store = VersionRetention::new(16, 4);
        let mut peak = 0;
        let mut roots = BTreeSet::new();
        for tick in 1..=5_000 {
            let v = version(tick, 1);
            store.record(&v);
            if tick % 500 == 0 {
                roots.insert(v.state_hash);
            }
            peak = peak.max(store.collect(&roots).retained);
        }
        assert!(peak <= 16 + 4, "retained {peak}");
        assert_eq!(store.len(), 20);
        assert!(store.tombstones.len() <= MAX_TOMBSTONES);
    }
}
