#![forbid(unsafe_code)]

//! Deterministic MCP JSON projection for the authenticated live-read semantic
//! artifacts produced by `dfmcp-adapter`.
//!
//! This module is presentation-only. It cannot create capabilities, plan
//! effects, mutate the game, or upgrade epistemic status. Every projected fact
//! retains the capsule/receipt digests needed for `fortress.explain` and later
//! evidence retrieval.

use dfmcp_adapter::{
    LiveAttentionItem, LiveChangeSummary, LiveCoverageEntry, LiveFortressBriefing,
    LiveObservationReceipt, LiveVersionDecision,
};
use serde_json::{Value, json};

fn digest(value: dfmcp_core::Digest32) -> String {
    value.to_string()
}

fn attention_value(item: &LiveAttentionItem) -> Value {
    json!({
        "attention_id": item.attention_id,
        "category": item.category,
        "severity": item.severity.as_str(),
        "urgency": "review_now",
        "confidence": {
            "epistemic_state": "certified_derived",
            "value": 1.0,
            "evidence": [digest(item.source_digest)],
        },
        "finding": item.finding,
        "affected_unit_ids": item.affected_unit_ids,
        "score_components": item.score_components,
        "authority": "none",
    })
}

fn coverage_value(entry: &LiveCoverageEntry) -> Value {
    json!({
        "domain": entry.domain.as_str(),
        "status": entry.status.as_str(),
        "can_prove_absence": entry.status.can_prove_absence(),
        "reason": entry.reason,
    })
}

#[must_use]
pub fn live_briefing_value(briefing: &LiveFortressBriefing) -> Value {
    json!({
        "source_digest": digest(briefing.source_digest),
        "epistemic_state": "certified_derived",
        "bridge": {
            "generation": briefing.bridge_generation,
            "bridge_version": briefing.bridge_version,
            "dfhack_version": briefing.dfhack_version,
            "dwarf_fortress_version": briefing.dwarf_fortress_version,
        },
        "fortress": {
            "site_id": briefing.site_id,
            "world_name": briefing.world_name,
            "world_folder": briefing.world_folder,
            "paused": briefing.paused,
            "current_year": briefing.current_year,
            "current_year_tick": briefing.current_year_tick,
        },
        "citizens": {
            "total": briefing.citizen_status.total,
            "alive": briefing.citizen_status.alive,
            "sane": briefing.citizen_status.sane,
            "active": briefing.citizen_status.active,
            "visible": briefing.citizen_status.visible,
            "citizens": briefing.citizen_status.citizens,
            "residents": briefing.citizen_status.residents,
            "babies": briefing.citizen_status.babies,
            "children": briefing.citizen_status.children,
            "adults": briefing.citizen_status.adults,
        },
        "coverage": briefing.coverage.iter().map(coverage_value).collect::<Vec<_>>(),
        "attention": briefing.attention.iter().map(attention_value).collect::<Vec<_>>(),
        "limitations": [
            "attention items are mechanical findings, not mutation authority",
            "domains marked omitted are unknown and cannot support absence claims",
        ],
    })
}

#[must_use]
pub fn live_change_value(change: &LiveChangeSummary) -> Value {
    json!({
        "basis_digest": digest(change.basis_digest),
        "target_digest": digest(change.target_digest),
        "heartbeat": change.heartbeat,
        "pause_changed": change.pause_changed.map(|(before, after)| json!({
            "before": before,
            "after": after,
        })),
        "calendar_changed": change.calendar_changed,
        "citizens_added": change.citizens_added,
        "citizens_removed": change.citizens_removed,
        "citizens_changed": change.citizens_changed,
        "ids_truncated": change.ids_truncated,
        "epistemic_state": "certified_derived",
    })
}

#[must_use]
pub fn live_receipt_value(receipt: &LiveObservationReceipt) -> Value {
    json!({
        "schema": "dfmcp.live_observation_receipt/1",
        "fortress_id": receipt.fortress_id.to_string(),
        "anchor": {
            "epoch": receipt.cursor.epoch,
            "sequence": receipt.cursor.sequence,
            "game_tick": receipt.game_tick.0,
            "state_hash": digest(receipt.snapshot_root),
        },
        "capsule": {
            "digest": digest(receipt.capsule_digest),
            "bytes": receipt.capsule_bytes,
            "bridge_generation": receipt.bridge_generation,
            "site_id": receipt.site_id,
            "citizen_count": receipt.citizen_count,
            "citizen_coverage_complete": receipt.citizen_coverage_complete,
        },
        "projection": {
            "entity_count": receipt.projected_entity_count,
            "fact_count": receipt.fact_count,
            "fact_provenance_digest": digest(receipt.fact_provenance_digest),
        },
        "receipt_digest": digest(receipt.receipt_digest),
    })
}

#[must_use]
pub fn live_version_value(decision: &LiveVersionDecision) -> Value {
    json!({
        "status": decision.continuity.as_str(),
        "target": {
            "epoch": decision.cursor.epoch,
            "sequence": decision.cursor.sequence,
        },
        "basis": decision.previous_cursor.map(|cursor| json!({
            "epoch": cursor.epoch,
            "sequence": cursor.sequence,
        })),
        "reset_reason": decision.reset_reason.map(|reason| reason.as_str()),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveAgentSections {
    pub briefing: Value,
    pub changes: Vec<Value>,
    pub attention: Vec<Value>,
    pub uncertainty: Vec<Value>,
    pub coverage: Value,
    pub references: Vec<Value>,
}

#[must_use]
pub fn live_agent_sections(
    briefing: &LiveFortressBriefing,
    change: Option<&LiveChangeSummary>,
    receipt: &LiveObservationReceipt,
    version: &LiveVersionDecision,
) -> LiveAgentSections {
    let briefing_value = live_briefing_value(briefing);
    let changes = change
        .filter(|value| !value.heartbeat)
        .map(live_change_value)
        .into_iter()
        .collect();
    let attention = briefing
        .attention
        .iter()
        .map(attention_value)
        .collect::<Vec<_>>();
    let omitted = briefing
        .coverage
        .iter()
        .filter(|entry| !entry.status.can_prove_absence())
        .map(|entry| {
            json!({
                "uncertainty_id": format!("live.omitted.{}", entry.domain.as_str()),
                "epistemic_state": "unknown",
                "statement": format!(
                    "live bridge V1 does not establish domain {}",
                    entry.domain.as_str()
                ),
                "consequence": "the domain cannot satisfy a precondition or support an absence claim",
                "reason": entry.reason,
                "evidence": [],
            })
        })
        .collect::<Vec<_>>();
    let coverage = json!({
        "status": "complete_for_named_projection",
        "domains": briefing.coverage.iter().map(coverage_value).collect::<Vec<_>>(),
        "continuation": null,
    });
    let references = vec![
        json!({
            "kind": "live_observation_receipt",
            "digest": digest(receipt.receipt_digest),
            "value": live_receipt_value(receipt),
        }),
        json!({
            "kind": "live_continuity",
            "value": live_version_value(version),
        }),
    ];
    LiveAgentSections {
        briefing: briefing_value,
        changes,
        attention,
        uncertainty: omitted,
        coverage,
        references,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dfmcp_adapter::{
        CitizenStatusCounts, LiveAttentionSeverity, LiveCoverageDomain, LiveCoverageStatus,
    };
    use dfmcp_core::{
        ContinuityStatus, Digest32, FortressId, GameTick, ObservationCursor,
    };

    use super::*;

    fn briefing() -> LiveFortressBriefing {
        let source = Digest32::of_bytes(b"live-source");
        LiveFortressBriefing {
            source_digest: source,
            bridge_generation: 42,
            dwarf_fortress_version: "0.51.11".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            bridge_version: "0.1.0".to_owned(),
            site_id: 7,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            paused: true,
            current_year: 105,
            current_year_tick: 12345,
            citizen_status: CitizenStatusCounts {
                total: 1,
                alive: 1,
                sane: 0,
                active: 1,
                visible: 1,
                citizens: 1,
                residents: 0,
                babies: 0,
                children: 0,
                adults: 1,
            },
            coverage: vec![
                LiveCoverageEntry {
                    domain: LiveCoverageDomain::CitizenRoster,
                    status: LiveCoverageStatus::Complete,
                    reason: None,
                },
                LiveCoverageEntry {
                    domain: LiveCoverageDomain::Threats,
                    status: LiveCoverageStatus::Omitted,
                    reason: Some("not observed".to_owned()),
                },
            ],
            attention: vec![LiveAttentionItem {
                attention_id: "live.basic_status.not_sane".to_owned(),
                severity: LiveAttentionSeverity::High,
                category: "citizen_basic_status".to_owned(),
                finding: "one citizen is not marked sane".to_owned(),
                affected_unit_ids: vec![11],
                score_components: BTreeMap::from([("affected_units".to_owned(), 1)]),
                source_digest: source,
            }],
        }
    }

    fn receipt() -> LiveObservationReceipt {
        LiveObservationReceipt {
            fortress_id: FortressId::new(77),
            cursor: ObservationCursor {
                epoch: 3,
                sequence: 9,
            },
            game_tick: GameTick::new(9120031),
            capsule_digest: Digest32::of_bytes(b"capsule"),
            capsule_bytes: 100,
            bridge_generation: 42,
            site_id: 7,
            citizen_count: 1,
            citizen_coverage_complete: true,
            projected_entity_count: 2,
            fact_count: 10,
            fact_provenance_digest: Digest32::of_bytes(b"facts"),
            snapshot_root: Digest32::of_bytes(b"snapshot"),
            receipt_digest: Digest32::of_bytes(b"receipt"),
        }
    }

    #[test]
    fn omitted_domains_become_explicit_uncertainty() {
        let briefing = briefing();
        let receipt = receipt();
        let version = LiveVersionDecision {
            cursor: receipt.cursor,
            continuity: ContinuityStatus::Bootstrap,
            reset_reason: None,
            previous_cursor: None,
        };
        let sections = live_agent_sections(&briefing, None, &receipt, &version);
        assert_eq!(sections.uncertainty.len(), 1);
        assert_eq!(
            sections.uncertainty[0]["uncertainty_id"],
            "live.omitted.threats"
        );
        assert_eq!(
            sections.coverage["domains"][0]["can_prove_absence"],
            true
        );
    }

    #[test]
    fn live_projection_never_contains_bearer_or_nonce_fields() {
        let rendered = live_briefing_value(&briefing()).to_string();
        assert!(!rendered.contains("bearer"));
        assert!(!rendered.contains("token"));
        assert!(!rendered.contains("nonce"));
    }
}
