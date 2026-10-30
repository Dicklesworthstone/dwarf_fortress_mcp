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

#[test]
fn jsonl_roundtrips_hostile_text_without_splitting_records()
-> Result<(), Box<dyn std::error::Error>> {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(100),
        state_hash: Digest32::of_bytes(b"anchor"),
    };
    let evidence = Digest32::of_bytes(b"evidence");
    let mut batch = EeMemoryBatch::new("batch".to_owned(), GameTick(100));
    let kind = "rule\"\\\n\u{0000}";
    let content = "line one\n\"quoted\" \\ path\r\ntab\t and dwarf Thîkut";
    let tags = vec!["tag\"\nfirst".to_owned(), "back\\slash".to_owned()];
    batch.add_item(EeMemoryItem {
        memory_kind: kind.to_owned(),
        content: content.to_owned(),
        provenance_anchor: anchor,
        evidence_digest: evidence,
        tags: tags.clone(),
    });
    batch.add_item(EeMemoryItem {
        memory_kind: "fact".to_owned(),
        content: "second record".to_owned(),
        provenance_anchor: anchor,
        evidence_digest: evidence,
        tags: vec![],
    });

    let encoded = batch.to_jsonl();
    let lines: Vec<_> = encoded.split('\n').collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0])?;
    let second: serde_json::Value = serde_json::from_str(lines[1])?;
    assert_eq!(first["kind"], kind);
    assert_eq!(first["content"], content);
    assert_eq!(first["tags"], tags.join(","));
    assert_eq!(first["provenance"], format!("{:?}", anchor.state_hash));
    assert_eq!(first["digest"], format!("{evidence:?}"));
    assert_eq!(second["content"], "second record");
    assert_eq!(second["tags"], "");
    assert_eq!(batch.items.len(), 2);
    Ok(())
}
