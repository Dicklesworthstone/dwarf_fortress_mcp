#![forbid(unsafe_code)]

use dfmcp_core::{Digest32, EntityId, StateAnchor};

use crate::model::{EntityKind, Value, WorldSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttentionSignalKind {
    MilitaryThreat,
    StarvationRisk,
    StressAnomaly,
    ResourceBottleneck,
    BlockedJob,
    IdleWorkshop,
    MandateRisk,
    PlanRegression,
}

impl AttentionSignalKind {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::MilitaryThreat => "military_threat",
            Self::StarvationRisk => "starvation_risk",
            Self::StressAnomaly => "stress_anomaly",
            Self::ResourceBottleneck => "resource_bottleneck",
            Self::BlockedJob => "blocked_job",
            Self::IdleWorkshop => "idle_workshop",
            Self::MandateRisk => "mandate_risk",
            Self::PlanRegression => "plan_regression",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionSignal {
    pub kind: AttentionSignalKind,
    pub subject: Option<EntityId>,
    pub severity_score: u32,
    pub summary: String,
    pub contributing_factors: Vec<String>,
    pub evidence_digest: Digest32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletenessStatus {
    Complete,
    BudgetTruncated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionLedger {
    pub generation: u64,
    pub anchor: StateAnchor,
    pub signals: Vec<AttentionSignal>,
    pub completeness: CompletenessStatus,
    pub ledger_digest: Digest32,
}

impl AttentionLedger {
    pub fn new(
        generation: u64,
        anchor: StateAnchor,
        mut signals: Vec<AttentionSignal>,
        completeness: CompletenessStatus,
    ) -> Self {
        signals.sort_by(|a, b| {
            b.severity_score
                .cmp(&a.severity_score)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.subject.cmp(&b.subject))
        });

        let mut hasher_bytes = Vec::new();
        crate::canonical::put_str(&mut hasher_bytes, "dfmcp-attention-ledger-v1");
        crate::canonical::put_u64(&mut hasher_bytes, generation);
        crate::canonical::put_anchor(&mut hasher_bytes, anchor);
        hasher_bytes.push(match completeness {
            CompletenessStatus::Complete => 0,
            CompletenessStatus::BudgetTruncated => 1,
        });
        crate::canonical::put_u64(&mut hasher_bytes, signals.len() as u64);

        for sig in &signals {
            crate::canonical::put_str(&mut hasher_bytes, sig.kind.name());
            crate::canonical::put_u32(&mut hasher_bytes, sig.severity_score);
            match sig.subject {
                Some(subject) => {
                    hasher_bytes.push(1);
                    crate::canonical::put_u64(&mut hasher_bytes, subject.get());
                }
                None => hasher_bytes.push(0),
            }
            crate::canonical::put_str(&mut hasher_bytes, &sig.summary);
            crate::canonical::put_u64(&mut hasher_bytes, sig.contributing_factors.len() as u64);
            for factor in &sig.contributing_factors {
                crate::canonical::put_str(&mut hasher_bytes, factor);
            }
            crate::canonical::put_bytes(&mut hasher_bytes, sig.evidence_digest.as_bytes());
        }

        let ledger_digest = Digest32::of_bytes(&hasher_bytes);

        Self {
            generation,
            anchor,
            signals,
            completeness,
            ledger_digest,
        }
    }
}

pub struct AttentionEngine;

impl AttentionEngine {
    #[must_use]
    pub fn rank_attention(
        snapshot: &WorldSnapshot,
        generation: u64,
        max_signals: usize,
    ) -> AttentionLedger {
        let mut signals = Vec::new();

        for (id, entity) in &snapshot.graph.entities {
            if entity.kind == EntityKind::Unit
                && let Some(fact) = entity.fields.get("stress")
                && let Value::I64(stress_val) = fact.value
                && stress_val > 50
            {
                let severity = u32::try_from(stress_val).map_or(1_000, |value| value.min(1_000));
                signals.push(AttentionSignal {
                    kind: AttentionSignalKind::StressAnomaly,
                    subject: Some(*id),
                    severity_score: severity,
                    summary: format!(
                        "High stress detected on unit {}: {stress_val}",
                        entity.label
                    ),
                    contributing_factors: vec![format!("stress={stress_val}")],
                    evidence_digest: fact.source_digest,
                });
            }
        }

        signals.sort_by(|a, b| {
            b.severity_score
                .cmp(&a.severity_score)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.subject.cmp(&b.subject))
        });

        let mut completeness = CompletenessStatus::Complete;
        if signals.len() > max_signals {
            signals.truncate(max_signals);
            completeness = CompletenessStatus::BudgetTruncated;
        }

        AttentionLedger::new(generation, snapshot.anchor(), signals, completeness)
    }
}

/// Lexicographic attention key: higher `class`, then higher `magnitude`, then
/// higher `freshness` rank first; equal keys resolve by `identity` ascending.
/// This is a total order, so selection never depends on insertion order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionKey {
    /// Signal-class priority (e.g. severity).
    pub class: u8,
    /// Magnitude within the class (e.g. urgency).
    pub magnitude: u64,
    /// Freshness or category priority.
    pub freshness: u64,
    /// Stable identity used only to break exact ties.
    pub identity: String,
}

impl AttentionKey {
    /// `Less` means `self` ranks before `other`.
    #[must_use]
    pub fn rank_cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .class
            .cmp(&self.class)
            .then(other.magnitude.cmp(&self.magnitude))
            .then(other.freshness.cmp(&self.freshness))
            .then_with(|| self.identity.cmp(&other.identity))
    }

    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.class);
        out.extend_from_slice(&self.magnitude.to_be_bytes());
        out.extend_from_slice(&self.freshness.to_be_bytes());
        out.extend_from_slice(&(self.identity.len() as u64).to_be_bytes());
        out.extend_from_slice(self.identity.as_bytes());
    }
}

/// Proof that a top-k selection is exact: no excluded key outranks the k-th
/// selected key. Advisory only: attention never authorizes an effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionCertificate {
    /// Requested bound.
    pub k: usize,
    /// Signals considered.
    pub considered: usize,
    /// Identities selected, in rank order.
    pub selected: Vec<String>,
    /// Identities excluded, in rank order.
    pub excluded: Vec<String>,
    /// Key of the last selected signal.
    pub kth: Option<AttentionKey>,
    /// Key of the best excluded signal.
    pub best_excluded: Option<AttentionKey>,
    /// Digest over the ranked keys, selected then excluded.
    pub digest: Digest32,
}

impl SelectionCertificate {
    /// Re-checks the certificate against the keys it claims to rank.
    #[must_use]
    pub fn verify(&self, keys: &[AttentionKey]) -> bool {
        let Ok(recomputed) = select_top_k(keys, self.k) else {
            return false;
        };
        let ordered = match (&self.kth, &self.best_excluded) {
            (Some(kth), Some(best)) => kth.rank_cmp(best) == std::cmp::Ordering::Less,
            _ => true,
        };
        ordered && recomputed.1 == *self
    }
}

/// Most attention keys one selection accepts.
pub const MAX_ATTENTION_KEYS: usize = 65_536;

/// Exact top-k over `keys` under [`AttentionKey::rank_cmp`]. Returns the
/// selected positions (into `keys`) in rank order and the certificate.
pub fn select_top_k(
    keys: &[AttentionKey],
    k: usize,
) -> dfmcp_core::Result<(Vec<usize>, SelectionCertificate)> {
    if keys.len() > MAX_ATTENTION_KEYS {
        return Err(dfmcp_core::DfmcpError::new(
            dfmcp_core::ErrorCode::BudgetExceeded,
            "attention selection exceeds its input bound",
        ));
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|a, b| keys[*a].rank_cmp(&keys[*b]).then(a.cmp(b)));
    let cut = k.min(order.len());
    let mut bytes = b"dfmcp-attention-top-k/1\0".to_vec();
    bytes.extend_from_slice(&(k as u64).to_be_bytes());
    for index in &order {
        keys[*index].encode(&mut bytes);
    }
    let certificate = SelectionCertificate {
        k,
        considered: keys.len(),
        selected: order[..cut]
            .iter()
            .map(|i| keys[*i].identity.clone())
            .collect(),
        excluded: order[cut..]
            .iter()
            .map(|i| keys[*i].identity.clone())
            .collect(),
        kth: cut.checked_sub(1).map(|i| keys[order[i]].clone()),
        best_excluded: order.get(cut).map(|i| keys[*i].clone()),
        digest: Digest32::of_bytes(&bytes),
    };
    Ok((order[..cut].to_vec(), certificate))
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn key(class: u8, magnitude: u64, freshness: u64, identity: &str) -> AttentionKey {
        AttentionKey {
            class,
            magnitude,
            freshness,
            identity: identity.to_owned(),
        }
    }

    #[test]
    fn random_bounded_signal_sets_are_certified_exactly() -> dfmcp_core::Result<()> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..300 {
            let n = (next() % 40) as usize;
            let keys: Vec<_> = (0..n)
                .map(|i| {
                    key(
                        (next() % 4) as u8,
                        next() % 3,
                        next() % 2,
                        &format!("s{:02}", (next() % 50) + i as u64 * 100),
                    )
                })
                .collect();
            let k = (next() % 8) as usize;
            let (picked, certificate) = select_top_k(&keys, k)?;
            assert!(certificate.verify(&keys), "round {round}");
            assert_eq!(picked.len(), k.min(n));
            // No excluded key outranks any selected key.
            for excluded in (0..n).filter(|i| !picked.contains(i)) {
                for selected in &picked {
                    assert_eq!(
                        keys[*selected].rank_cmp(&keys[excluded]),
                        std::cmp::Ordering::Less
                    );
                }
            }
            // Insertion order never changes the answer.
            let mut reversed = keys.clone();
            reversed.reverse();
            let (_, again) = select_top_k(&reversed, k)?;
            assert_eq!(again.selected, certificate.selected);
            assert_eq!(again.digest, certificate.digest);
        }
        Ok(())
    }

    #[test]
    fn equal_keys_resolve_by_identity_and_forged_certificates_fail() -> dfmcp_core::Result<()> {
        let keys = vec![
            key(2, 1, 0, "zeta"),
            key(2, 1, 0, "alpha"),
            key(3, 0, 0, "omega"),
        ];
        let (_, certificate) = select_top_k(&keys, 2)?;
        assert_eq!(certificate.selected, vec!["omega", "alpha"]);
        assert_eq!(certificate.excluded, vec!["zeta"]);
        let mut forged = certificate.clone();
        forged.selected.swap(0, 1);
        assert!(!forged.verify(&keys));
        let mut promoted = keys.clone();
        promoted[0].class = 9;
        assert!(!certificate.verify(&promoted));
        Ok(())
    }
}
