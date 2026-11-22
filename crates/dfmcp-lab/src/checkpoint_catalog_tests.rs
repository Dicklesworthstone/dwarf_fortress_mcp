//! Original checkpoint labels and read-only recovery metadata.
//!
//! These cases exercise the actual adapter; metadata inspection must never
//! restore a world, consume a request identity, or manufacture old labels.

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CheckpointId, ErrorCode, FortressId,
    GameTick, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{WorldGraph, WorldSnapshot};

use super::{MAX_LAB_CHECKPOINTS, MemoryAdapter};

fn source(fortress: u64, tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(fortress),
        GameTick(tick),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    )
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [Capability::Checkpoint, Capability::Restore]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(adapter.snapshot().fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}

#[test]
fn checkpoint_catalog_keeps_exact_labels_and_read_only_identifier_order() {
    let mut adapter = MemoryAdapter::new(source(7, 5));
    let first = adapter
        .checkpoint("before mining", &context(&adapter))
        .expect("first checkpoint");
    let second = adapter
        .checkpoint("before furniture", &context(&adapter))
        .expect("second checkpoint");
    let unchanged = adapter.snapshot().clone();
    let transcript = adapter.transcript().clone();
    let mut control = adapter.clone();

    let rows: Vec<_> = adapter
        .checkpoint_entries()
        .map(|(id, world, label)| (id, world.anchor(), label.map(str::to_owned)))
        .collect();
    let mut expected = vec![
        (first.checkpoint_id, first.anchor, Some(first.label)),
        (second.checkpoint_id, second.anchor, Some(second.label)),
    ];
    expected.sort_by_key(|row| row.0);
    assert_eq!(rows, expected);
    assert_eq!(adapter.checkpoint_count(), 2);
    assert_eq!(adapter.snapshot(), &unchanged);
    assert_eq!(adapter.transcript(), &transcript);
    assert!(adapter.checkpoint_label(CheckpointId::new(999)).is_none());

    let next = adapter
        .checkpoint("next", &context(&adapter))
        .expect("next checkpoint after inspection");
    let without_read = control
        .checkpoint("next", &context(&control))
        .expect("next checkpoint without inspection");
    assert_eq!(next, without_read, "inspection must not consume identity state");
}

#[test]
fn checkpoint_adoption_distinguishes_unknown_and_exact_empty_historical_labels() {
    let original = source(7, 3);
    let mut adapter = MemoryAdapter::new(source(7, 9));
    let id = CheckpointId::new(4);
    adapter
        .adopt_checkpoint(id, original.clone())
        .expect("legacy adoption");
    assert_eq!(adapter.checkpoint_label(id), None);
    assert_eq!(adapter.checkpoint_entries().next().expect("row").2, None);

    adapter
        .adopt_checkpoint_labeled(id, original.clone(), "")
        .expect("an exact empty label is valid in historical durable records");
    assert_eq!(adapter.checkpoint_label(id), Some(""));
    adapter
        .adopt_checkpoint(id, original.clone())
        .expect("unlabeled compatibility adoption");
    assert_eq!(adapter.checkpoint_label(id), Some(""));
    adapter
        .adopt_checkpoint_labeled(id, original.clone(), "")
        .expect("identical adoption is idempotent");
    assert_eq!(adapter.checkpoint_count(), 1);
    assert_eq!(adapter.checkpoint_snapshot(id), Some(&original));

    let before = adapter.snapshot().clone();
    assert!(
        adapter.checkpoint("", &context(&adapter)).is_err(),
        "a new public checkpoint still requires a nonempty label"
    );
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(adapter.checkpoint_count(), 1);
}

#[test]
fn checkpoint_adoption_rejects_conflicts_without_partial_metadata() {
    let original = source(7, 3);
    let other = source(7, 4);
    let mut adapter = MemoryAdapter::new(source(7, 9));
    let known = CheckpointId::new(2);
    let unknown = CheckpointId::new(3);
    adapter
        .adopt_checkpoint_labeled(known, original.clone(), "original")
        .expect("known label");
    adapter
        .adopt_checkpoint(unknown, original.clone())
        .expect("unknown label");

    let before = adapter.snapshot().clone();
    let transcript = adapter.transcript().clone();
    for (id, candidate, label) in [
        (known, original.clone(), "renamed"),
        (known, other.clone(), "original"),
        (unknown, other, "must not leak"),
    ] {
        assert_eq!(
            adapter
                .adopt_checkpoint_labeled(id, candidate, label)
                .expect_err("conflicting import")
                .code,
            ErrorCode::Conflict
        );
    }
    assert_eq!(adapter.checkpoint_label(known), Some("original"));
    assert_eq!(adapter.checkpoint_label(unknown), None);
    assert_eq!(adapter.checkpoint_snapshot(known), Some(&original));
    assert_eq!(adapter.checkpoint_snapshot(unknown), Some(&original));
    assert_eq!(adapter.checkpoint_count(), 2);
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(adapter.transcript(), &transcript);
}

#[test]
fn checkpoint_adoption_checks_labels_and_source_before_publication() {
    let original = source(7, 3);
    let mut adapter = MemoryAdapter::new(source(7, 9));
    let id = CheckpointId::new(4);
    for bad in ["x".repeat(257), "é".repeat(129), "line\nbreak".to_owned(), "\u{7f}".to_owned()] {
        assert_eq!(
            adapter
                .adopt_checkpoint_labeled(id, original.clone(), &bad)
                .expect_err("invalid label")
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(adapter.checkpoint_count(), 0);
        assert_eq!(adapter.checkpoint_label(id), None);
    }
    let mut unsealed = original.clone();
    unsealed.paused = false;
    for (candidate_id, candidate) in [
        (CheckpointId::NIL, original.clone()),
        (id, source(8, 3)),
        (id, unsealed),
    ] {
        assert_eq!(
            adapter
                .adopt_checkpoint_labeled(candidate_id, candidate, "unpublished")
                .expect_err("invalid source")
                .code,
            ErrorCode::CorruptLedger
        );
        assert_eq!(adapter.checkpoint_count(), 0);
        assert_eq!(adapter.checkpoint_label(candidate_id), None);
    }
    let boundary = "é".repeat(128);
    adapter
        .adopt_checkpoint_labeled(id, original.clone(), &boundary)
        .expect("256-byte retained label");
    assert_eq!(adapter.checkpoint_label(id), Some(boundary.as_str()));
    assert_eq!(adapter.checkpoint_snapshot(id), Some(&original));
}

#[test]
fn checkpoint_labels_and_original_worlds_survive_clone_recovery_and_restore() {
    let mut adapter = MemoryAdapter::new(source(7, 3));
    let original = adapter.snapshot().clone();
    let receipt = adapter
        .checkpoint("original retained label", &context(&adapter))
        .expect("checkpoint");
    let cloned = adapter.clone();
    assert_eq!(
        cloned.checkpoint_label(receipt.checkpoint_id),
        Some("original retained label")
    );
    assert_eq!(
        cloned.checkpoint_snapshot(receipt.checkpoint_id),
        Some(&original)
    );

    let mut recovered = MemoryAdapter::recovered(source(7, 20)).expect("recovery");
    recovered
        .adopt_checkpoint_labeled(receipt.checkpoint_id, original.clone(), &receipt.label)
        .expect("restore durable checkpoint custody");
    let current_before = recovered.snapshot().anchor();
    let restored = recovered
        .restore(receipt.checkpoint_id, &context(&recovered))
        .expect("authorized restore");
    assert_eq!(restored.content_digest, original.state_hash);
    assert_eq!(restored.prior_anchor, current_before);
    assert!(restored.restored_anchor.cursor.epoch > current_before.cursor.epoch);
    assert_eq!(
        recovered.checkpoint_snapshot(receipt.checkpoint_id),
        Some(&original),
        "restore changes current lineage, never the retained source"
    );
    assert_eq!(
        recovered.checkpoint_label(receipt.checkpoint_id),
        Some("original retained label")
    );
    assert_eq!(recovered.checkpoint_count(), 1);
}

#[test]
fn full_checkpoint_store_refuses_new_labels_but_allows_exact_metadata_recovery() {
    let original = source(7, 3);
    let mut adapter = MemoryAdapter::new(source(7, 9));
    for value in 1..=MAX_LAB_CHECKPOINTS {
        adapter
            .adopt_checkpoint(CheckpointId::new(value as u128), original.clone())
            .expect("fill checkpoint store");
    }
    let absent = CheckpointId::new(MAX_LAB_CHECKPOINTS as u128 + 1);
    assert_eq!(
        adapter
            .adopt_checkpoint_labeled(absent, original.clone(), "overflow")
            .expect_err("bounded catalog")
            .code,
        ErrorCode::BudgetExceeded
    );
    assert_eq!(adapter.checkpoint_count(), MAX_LAB_CHECKPOINTS);
    assert_eq!(adapter.checkpoint_label(absent), None);
    assert!(adapter.checkpoint_snapshot(absent).is_none());

    let existing = CheckpointId::new(1);
    adapter
        .adopt_checkpoint_labeled(existing, original, "retained at capacity")
        .expect("existing exact custody may recover its original label");
    assert_eq!(adapter.checkpoint_count(), MAX_LAB_CHECKPOINTS);
    assert_eq!(adapter.checkpoint_label(existing), Some("retained at capacity"));
}
