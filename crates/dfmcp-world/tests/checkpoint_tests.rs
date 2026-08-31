#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, ErrorCode, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{CheckpointStore, WorldGraph, WorldSnapshot};

fn make_snapshot(epoch: u64, tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(tick),
        ObservationCursor {
            epoch,
            sequence: 10,
        },
        false,
        WorldGraph::default(),
    )
}

/// TEST-013: Sealed Checkpoints, Bit-Rot Detection, and Epoch Reset
#[test]
fn test_013_checkpoint_creation_and_bit_rot_detection() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = CheckpointStore::new();
    let snapshot = make_snapshot(1, 100);

    let mut files_data = BTreeMap::new();
    files_data.insert(
        "world.sav".to_string(),
        b"world_save_binary_payload_version_53_16".to_vec(),
    );
    files_data.insert(
        "world.dat".to_string(),
        b"metadata_header_block_raw_bytes".to_vec(),
    );

    let mut files_manifest = BTreeMap::new();
    for (path, bytes) in &files_data {
        files_manifest.insert(
            path.clone(),
            (bytes.len() as u64, Digest32::of_bytes(bytes)),
        );
    }

    let manifest = store.create_checkpoint(&snapshot, files_manifest)?;
    assert_ne!(manifest.manifest_digest, Digest32::ZERO);

    manifest.verify_files(|path| files_data.get(path).cloned())?;

    let mut corrupted_files = files_data.clone();
    let mut corrupted_bytes = corrupted_files["world.sav"].clone();
    corrupted_bytes[5] ^= 0xFF;
    corrupted_files.insert("world.sav".to_string(), corrupted_bytes);

    let Err(rot_err) = manifest.verify_files(|path| corrupted_files.get(path).cloned()) else {
        return Err("expected bit-rot error".into());
    };
    assert_eq!(rot_err.code, ErrorCode::CorruptLedger);
    assert!(rot_err.message.contains("bit-rot"));

    let mut missing_files = files_data.clone();
    missing_files.remove("world.dat");
    let Err(missing_err) = manifest.verify_files(|path| missing_files.get(path).cloned()) else {
        return Err("expected missing file error".into());
    };
    assert_eq!(missing_err.code, ErrorCode::CorruptLedger);

    Ok(())
}

#[test]
fn test_013_restore_epoch_bump_and_stale_anchor_rejection() -> Result<(), Box<dyn std::error::Error>>
{
    let mut store = CheckpointStore::new();
    let snapshot_epoch1 = make_snapshot(1, 100);

    let mut files_manifest = BTreeMap::new();
    files_manifest.insert("world.sav".to_string(), (100, Digest32::of_bytes(b"save")));

    let manifest = store.create_checkpoint(&snapshot_epoch1, files_manifest)?;

    let live_anchor_epoch1 = snapshot_epoch1.anchor();
    CheckpointStore::validate_anchor_epoch(&live_anchor_epoch1, 1)?;

    let mut restored_snapshot = snapshot_epoch1.clone();
    restored_snapshot.cursor.epoch = 2;
    restored_snapshot.cursor.sequence = 0;
    restored_snapshot.refresh_hash();
    let restored_anchor = restored_snapshot.anchor();

    let cert = store.restore_checkpoint(
        manifest.checkpoint_id,
        live_anchor_epoch1,
        restored_anchor,
        manifest.state_hash,
    )?;
    assert_eq!(cert.prior_anchor, live_anchor_epoch1);
    assert_eq!(cert.restored_anchor, restored_anchor);
    assert_eq!(cert.manifest_digest, manifest.manifest_digest);
    assert_eq!(cert.restored_content_digest, manifest.state_hash);
    assert!(cert.integrity_is_valid());

    let Err(stale_err) =
        CheckpointStore::validate_anchor_epoch(&live_anchor_epoch1, restored_anchor.cursor.epoch)
    else {
        return Err("expected stale anchor rejection after restore".into());
    };
    assert_eq!(stale_err.code, ErrorCode::CursorGap);
    assert!(stale_err.message.contains("restore occurred"));

    Ok(())
}

#[test]
fn checkpoint_rejects_unsafe_paths_and_wrong_restored_content()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = CheckpointStore::new();
    let snapshot = make_snapshot(1, 100);
    let mut unsafe_files = BTreeMap::new();
    unsafe_files.insert("../world.sav".to_owned(), (4, Digest32::of_bytes(b"save")));
    assert!(store.create_checkpoint(&snapshot, unsafe_files).is_err());

    let mut files = BTreeMap::new();
    files.insert("world.sav".to_owned(), (4, Digest32::of_bytes(b"save")));
    let manifest = store.create_checkpoint(&snapshot, files)?;
    let prior = snapshot.anchor();
    let mut restored = snapshot.clone();
    restored.cursor = ObservationCursor {
        epoch: 2,
        sequence: 0,
    };
    restored.refresh_hash();
    let result = store.restore_checkpoint(
        manifest.checkpoint_id,
        prior,
        restored.anchor(),
        Digest32::of_bytes(b"different content"),
    );
    assert!(matches!(result, Err(ref error) if error.code == ErrorCode::CorruptLedger));
    Ok(())
}
