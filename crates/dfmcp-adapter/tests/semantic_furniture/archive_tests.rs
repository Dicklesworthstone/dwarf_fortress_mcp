//! Retained archive authorization time does not create a new canonical anchor.
use super::fixtures::*;
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_core::{Capability, Digest32, ErrorCode, FortressId, GameTick, Result};
use dfmcp_intent::plan_archive::ArchiveLimits;

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_bytes: 64 * 1024,
        max_steps: 1,
        max_nodes: 1024,
        max_depth: 32,
    }
}

#[test]
fn retained_archive_clock_preserves_original_and_cannot_rewind_or_weaken_strict_reads() -> Result<()>
{
    let fixture = Fixture::new()?;
    let disk = Memory::default();
    let mut archive =
        PlanArchiveStore::create(disk.clone(), &fixture.original, limits(), &fixture.context)?;
    let seal = archive.seal();
    let bytes = disk.bytes();
    let syncs = disk.syncs();
    let mut current = query_context(&fixture.context);
    current.anchor.tick = GameTick(current.anchor.tick.get() + 20);
    assert!(archive.recover(&current).is_err());
    assert!(PlanArchiveStore::open(disk.clone(), seal, limits(), &current).is_err());
    let retained = archive.recover_at_authority_floor(&current)?;
    assert_eq!(retained, fixture.original);
    assert_eq!(retained.anchor, fixture.original.anchor);
    assert_ne!(retained.anchor.tick, current.anchor.tick);
    let mut rewind = current.clone();
    rewind.anchor.tick = GameTick(rewind.anchor.tick.get() - 10);
    assert!(archive.recover_at_authority_floor(&rewind).is_err());
    assert_eq!(archive.recover_at_authority_floor(&current)?, retained);
    drop(archive);
    let mut reopened =
        PlanArchiveStore::open_at_authority_floor(disk.clone(), seal, limits(), &current)?;
    assert_eq!(
        reopened.recover_at_authority_floor(&current)?,
        fixture.original
    );
    assert!(reopened.recover_at_authority_floor(&rewind).is_err());
    assert_eq!(disk.bytes(), bytes);
    assert_eq!(disk.syncs(), syncs);
    assert_eq!(fixture.native.borrow().prepares, 0);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn retained_archive_clock_still_rejects_changed_hash_older_cursor_and_other_fortress() -> Result<()>
{
    let fixture = Fixture::new()?;
    // Advance the actual typed publication, then derive a new original plan.
    // The first valid publication uses cursor zero and has no older cursor.
    let mut state = fixture.original_state.clone();
    let mut raw = fixture.capture.canonical_bytes().to_vec();
    raw[24..32].copy_from_slice(&(fixture.capture.tick() + 1).to_be_bytes());
    let capture = dfmcp_adapter::build_placement::BuildCapture::decode(&raw)?;
    state.publish(spatial_observation(&capture, SPATIAL_GENERATION)?)?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("later original snapshot"))?;
    let original = original_plan(snapshot, &capture)?;
    let opening = context(snapshot);
    let disk = Memory::default();
    let archive = PlanArchiveStore::create(disk.clone(), &original, limits(), &opening)?;
    let seal = archive.seal();
    let bytes = disk.bytes();
    drop(archive);
    let mut current = query_context(&opening);
    current.anchor.tick = GameTick(current.anchor.tick.get() + 20);
    assert!(current.anchor.cursor.sequence > 0);
    for case in 0..3 {
        let mut invalid_context = current.clone();
        match case {
            0 => {
                invalid_context.anchor.state_hash =
                    Digest32::of_bytes(b"another original canonical state")
            }
            1 => invalid_context.anchor.cursor.sequence -= 1,
            _ => {
                let other = FortressId::new(current.anchor.fortress_id.get() + 1);
                invalid_context.anchor.fortress_id = other;
                for grant in &mut invalid_context.grants {
                    grant.scope.fortress_id = Some(other);
                }
            }
        }
        assert!(
            PlanArchiveStore::open_at_authority_floor(
                Memory::from_bytes(bytes.clone()),
                seal,
                limits(),
                &invalid_context
            )
            .is_err()
        );
        let mut held = PlanArchiveStore::open(disk.clone(), seal, limits(), &opening)?;
        assert!(held.recover_at_authority_floor(&invalid_context).is_err());
        assert_eq!(held.recover_at_authority_floor(&current)?, original);
    }
    assert_eq!(disk.bytes(), bytes);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn retained_archive_clock_checks_expired_query_before_storage_or_original_tick_reuse() -> Result<()>
{
    let fixture = Fixture::new()?;
    let disk = Memory::default();
    let mut archive =
        PlanArchiveStore::create(disk.clone(), &fixture.original, limits(), &fixture.context)?;
    let seal = archive.seal();
    let bytes = disk.bytes();
    let mut expired = query_context(&fixture.context);
    expired.anchor.tick = GameTick(expired.anchor.tick.get() + 20);
    for grant in &mut expired.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(fixture.original.anchor.tick.get() + 10));
        }
    }
    // Lost storage custody would fail if I/O preceded current Query authority.
    disk.lose_identity();
    let cause = archive
        .recover_at_authority_floor(&expired)
        .err()
        .ok_or_else(|| invalid("expired Query read original archive"))?;
    assert_eq!(cause.code, ErrorCode::CapabilityDenied);
    let cause = PlanArchiveStore::open_at_authority_floor(disk.clone(), seal, limits(), &expired)
        .err()
        .ok_or_else(|| invalid("expired Query opened archive"))?;
    assert_eq!(cause.code, ErrorCode::CapabilityDenied);
    assert_eq!(disk.bytes(), bytes);
    assert_eq!(fixture.native.borrow().commits, 0);
    Ok(())
}
