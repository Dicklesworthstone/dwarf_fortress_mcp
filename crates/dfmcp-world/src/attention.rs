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
