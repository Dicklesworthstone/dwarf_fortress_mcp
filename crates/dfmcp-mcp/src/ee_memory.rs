#![forbid(unsafe_code)]

//! Eidetic Engine (EE) Campaign Memory Curation & Export Bridge.
//!
//! WP-MCP-04: Curates high-value operational insights (procedural rules, incident
//! diagnoses, anti-patterns, terminal predicate proofs) into structured EE memory batches.

use dfmcp_core::{Digest32, GameTick, StateAnchor};

/// Structured knowledge item destined for Eidetic Engine long-term memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EeMemoryItem {
    pub memory_kind: String, // e.g. "procedural_rule", "incident_diagnosis", "anti_pattern"
    pub content: String,
    pub provenance_anchor: StateAnchor,
    pub evidence_digest: Digest32,
    pub tags: Vec<String>,
}

/// Curated batch of memory items ready for `ee remember --batch` ingestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EeMemoryBatch {
    pub batch_id: String,
    pub created_at_tick: GameTick,
    pub items: Vec<EeMemoryItem>,
}

impl EeMemoryBatch {
    #[must_use]
    pub fn new(batch_id: String, created_at_tick: GameTick) -> Self {
        Self {
            batch_id,
            created_at_tick,
            items: Vec::new(),
        }
    }

    /// Add a curated memory item to the batch.
    pub fn add_item(&mut self, item: EeMemoryItem) {
        self.items.push(item);
    }

    /// Serialize items into JSON-lines representation.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        let mut lines = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let tags_str = item.tags.join(",");
            let line = format!(
                r#"{{"kind":"{}","content":"{}","provenance":"{:?}","digest":"{:?}","tags":"{}"}}"#,
                item.memory_kind,
                item.content,
                item.provenance_anchor.state_hash,
                item.evidence_digest,
                tags_str
            );
            lines.push(line);
        }
        lines.join("\n")
    }
}
