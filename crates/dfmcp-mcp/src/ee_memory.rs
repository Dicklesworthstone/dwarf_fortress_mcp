#![forbid(unsafe_code)]

//! Legacy campaign memory batch helper.
//!
//! This helper emits its existing JSONL shape. It does not perform evidence
//! curation or produce the separate E09 EE batch schema.

use dfmcp_core::{Digest32, GameTick, StateAnchor};

/// A caller-supplied item for the legacy batch representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EeMemoryItem {
    pub memory_kind: String, // e.g. "procedural_rule", "incident_diagnosis", "anti_pattern"
    pub content: String,
    pub provenance_anchor: StateAnchor,
    pub evidence_digest: Digest32,
    pub tags: Vec<String>,
}

/// A batch of caller-supplied memory items in the legacy JSONL representation.
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
                r#"{{"kind":{},"content":{},"provenance":{},"digest":{},"tags":{}}}"#,
                serde_json::Value::String(item.memory_kind.clone()),
                serde_json::Value::String(item.content.clone()),
                serde_json::Value::String(format!("{:?}", item.provenance_anchor.state_hash)),
                serde_json::Value::String(format!("{:?}", item.evidence_digest)),
                serde_json::Value::String(tags_str)
            );
            lines.push(line);
        }
        lines.join("\n")
    }
}
