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
        hasher_bytes.extend_from_slice(&generation.to_be_bytes());
        hasher_bytes.extend_from_slice(anchor.state_hash.as_bytes());

        for sig in &signals {
            hasher_bytes.extend_from_slice(sig.kind.name().as_bytes());
            hasher_bytes.extend_from_slice(&sig.severity_score.to_be_bytes());
            if let Some(sub) = sig.subject {
                hasher_bytes.extend_from_slice(&sub.get().to_be_bytes());
            }
            hasher_bytes.extend_from_slice(sig.evidence_digest.as_bytes());
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
                let severity = (stress_val as u32).min(1000);
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

        let mut completeness = CompletenessStatus::Complete;
        if signals.len() > max_signals {
            signals.truncate(max_signals);
            completeness = CompletenessStatus::BudgetTruncated;
        }

        AttentionLedger::new(generation, snapshot.anchor(), signals, completeness)
    }
}
