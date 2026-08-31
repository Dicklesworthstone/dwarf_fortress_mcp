#![forbid(unsafe_code)]

//! Integration tests for WP-MCP-04 EE Campaign Memory Curation.

use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor, StateAnchor};
use dfmcp_mcp::ee_memory::{EeMemoryBatch, EeMemoryItem};

#[test]
fn test_ee_memory_curation_roundtrip() {
    let mut batch = EeMemoryBatch::new("batch_test".to_owned(), GameTick(100));
    let anchor = StateAnchor {
        fortress_id: FortressId::new(1),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(100),
        state_hash: Digest32::ZERO,
    };

    let item = EeMemoryItem {
        memory_kind: "procedural_rule".to_owned(),
        content: "Always excavate moat before unpausing near goblin territory".to_owned(),
        provenance_anchor: anchor,
        evidence_digest: Digest32::of_bytes(b"rule_evidence"),
        tags: vec!["defense".to_owned(), "tactics".to_owned()],
    };

    batch.add_item(item);
    assert_eq!(batch.items.len(), 1);

    let jsonl = batch.to_jsonl();
    assert!(jsonl.contains("procedural_rule"));
    assert!(jsonl.contains("Always excavate moat"));
}
