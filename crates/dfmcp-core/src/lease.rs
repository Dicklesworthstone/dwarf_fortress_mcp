#![forbid(unsafe_code)]

//! Fine-Grained Spatial and Entity Lease Manager for Multi-Agent Concurrency.
//!
//! WP-LEA-01: Provides spatial cuboid and entity-level mutual exclusion fencing
//! (INV-008), preventing concurrent autonomous agents from interfering destructively.

use std::collections::BTreeMap;

use crate::error::{DfmcpError, ErrorCode, Result};
use crate::ids::{EntityId, LeaseId, SessionId};
use crate::model::{GameTick, MapCuboid};

/// Lease classification fencing spatial regions or entity IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseKind {
    SpatialExclusive(MapCuboid),
    SpatialShared(MapCuboid),
    EntityExclusive(EntityId),
    EntityShared(EntityId),
}

/// Active lease record held by an agent session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseRecord {
    pub lease_id: LeaseId,
    pub holder_session: SessionId,
    pub kind: LeaseKind,
    pub acquired_tick: GameTick,
    pub expires_at_tick: GameTick,
}

/// Checks whether two 3D spatial cuboids overlap.
#[must_use]
pub fn cuboids_intersect(a: &MapCuboid, b: &MapCuboid) -> bool {
    let a_min_x = a.min.x.min(a.max.x);
    let a_max_x = a.min.x.max(a.max.x);
    let a_min_y = a.min.y.min(a.max.y);
    let a_max_y = a.min.y.max(a.max.y);
    let a_min_z = a.min.z.min(a.max.z);
    let a_max_z = a.min.z.max(a.max.z);

    let b_min_x = b.min.x.min(b.max.x);
    let b_max_x = b.min.x.max(b.max.x);
    let b_min_y = b.min.y.min(b.max.y);
    let b_max_y = b.min.y.max(b.max.y);
    let b_min_z = b.min.z.min(b.max.z);
    let b_max_z = b.min.z.max(b.max.z);

    a_min_x <= b_max_x
        && a_max_x >= b_min_x
        && a_min_y <= b_max_y
        && a_max_y >= b_min_y
        && a_min_z <= b_max_z
        && a_max_z >= b_min_z
}

/// Multi-Agent Fine-Grained Lease Manager.
#[derive(Clone, Debug, Default)]
pub struct LeaseManager {
    next_lease_seq: u64,
    leases: BTreeMap<LeaseId, LeaseRecord>,
}

impl LeaseManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_lease_seq: 1,
            leases: BTreeMap::new(),
        }
    }

    /// Acquire a spatial lease over a 3D cuboid volume.
    pub fn acquire_spatial_lease(
        &mut self,
        session_id: SessionId,
        cuboid: MapCuboid,
        exclusive: bool,
        current_tick: GameTick,
        ttl_ticks: u64,
    ) -> Result<LeaseId> {
        let expires_at_tick = checked_expiry(current_tick, ttl_ticks)?;
        // 1. Check for conflicts against existing unexpired leases
        for existing in self.leases.values() {
            if existing.expires_at_tick <= current_tick {
                continue; // Expired lease
            }

            match &existing.kind {
                LeaseKind::SpatialExclusive(existing_cuboid) => {
                    if cuboids_intersect(&cuboid, existing_cuboid)
                        && existing.holder_session != session_id
                    {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            format!(
                                "spatial region overlaps with exclusive lease held by session {}",
                                existing.holder_session.get()
                            ),
                        ));
                    }
                }
                LeaseKind::SpatialShared(existing_cuboid)
                    if exclusive
                        && cuboids_intersect(&cuboid, existing_cuboid)
                        && existing.holder_session != session_id =>
                {
                    return Err(DfmcpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "exclusive spatial lease conflicts with shared lease held by session {}",
                            existing.holder_session.get()
                        ),
                    ));
                }
                _ => {}
            }
        }

        // 2. Grant lease
        let (lease_id, next_sequence) = self.next_lease_id()?;

        let kind = if exclusive {
            LeaseKind::SpatialExclusive(cuboid)
        } else {
            LeaseKind::SpatialShared(cuboid)
        };

        let record = LeaseRecord {
            lease_id,
            holder_session: session_id,
            kind,
            acquired_tick: current_tick,
            expires_at_tick,
        };

        if self.leases.insert(lease_id, record).is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "lease identifier collision",
            ));
        }
        self.next_lease_seq = next_sequence;
        Ok(lease_id)
    }

    /// Acquire an entity lease over a specific EntityId.
    pub fn acquire_entity_lease(
        &mut self,
        session_id: SessionId,
        entity_id: EntityId,
        exclusive: bool,
        current_tick: GameTick,
        ttl_ticks: u64,
    ) -> Result<LeaseId> {
        let expires_at_tick = checked_expiry(current_tick, ttl_ticks)?;
        for existing in self.leases.values() {
            if existing.expires_at_tick <= current_tick {
                continue;
            }

            match &existing.kind {
                LeaseKind::EntityExclusive(target) => {
                    if *target == entity_id && existing.holder_session != session_id {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            format!(
                                "entity {} is exclusively leased by session {}",
                                entity_id.get(),
                                existing.holder_session.get()
                            ),
                        ));
                    }
                }
                LeaseKind::EntityShared(target)
                    if exclusive
                        && *target == entity_id
                        && existing.holder_session != session_id =>
                {
                    return Err(DfmcpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "exclusive entity lease on {} conflicts with shared lease held by session {}",
                            entity_id.get(),
                            existing.holder_session.get()
                        ),
                    ));
                }
                _ => {}
            }
        }

        let (lease_id, next_sequence) = self.next_lease_id()?;

        let kind = if exclusive {
            LeaseKind::EntityExclusive(entity_id)
        } else {
            LeaseKind::EntityShared(entity_id)
        };

        let record = LeaseRecord {
            lease_id,
            holder_session: session_id,
            kind,
            acquired_tick: current_tick,
            expires_at_tick,
        };

        if self.leases.insert(lease_id, record).is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "lease identifier collision",
            ));
        }
        self.next_lease_seq = next_sequence;
        Ok(lease_id)
    }

    /// Verify the live manager record, not a caller-supplied copy of a lease.
    /// The full shared write area must fit an exclusive, currently held lease.
    /// This is process-local ownership; callers must separately bind a fortress
    /// and preserve durable unresolved-work fencing across process restarts.
    pub fn verify_exclusive_spatial(
        &self,
        lease_id: LeaseId,
        holder: SessionId,
        area: MapCuboid,
        current_tick: GameTick,
    ) -> Result<()> {
        MapCuboid::new(area.min, area.max)?;
        let record = self.leases.get(&lease_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::LeaseDenied, "spatial lease is no longer retained")
        })?;
        if record.holder_session != holder
            || current_tick < record.acquired_tick
            || current_tick >= record.expires_at_tick
        {
            return Err(DfmcpError::new(
                ErrorCode::LeaseDenied,
                "spatial lease holder or validity interval does not match",
            ));
        }
        if let LeaseKind::SpatialExclusive(held) = &record.kind {
            MapCuboid::new(held.min, held.max)?;
            if held.contains_cuboid(area) {
                return Ok(());
            }
        }
        Err(DfmcpError::new(
            ErrorCode::LeaseDenied,
            "an exclusive spatial lease covering every affected block is required",
        ))
    }

    /// Extend the TTL of an active lease.
    pub fn renew_lease(
        &mut self,
        lease_id: LeaseId,
        current_tick: GameTick,
        extension_ticks: u64,
    ) -> Result<()> {
        let record = self.leases.get_mut(&lease_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::LeaseDenied,
                format!("lease {:?} not found", lease_id),
            )
        })?;

        if record.expires_at_tick <= current_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "cannot renew expired lease",
            ));
        }

        if extension_ticks == 0 {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "lease renewal extension must be nonzero",
            ));
        }
        record.expires_at_tick = GameTick(
            record
                .expires_at_tick
                .0
                .checked_add(extension_ticks)
                .ok_or_else(|| {
                    DfmcpError::new(ErrorCode::BudgetExceeded, "lease expiry tick overflow")
                })?,
        );
        Ok(())
    }

    /// Explicitly release a lease.
    pub fn release_lease(&mut self, lease_id: LeaseId, session_id: SessionId) -> Result<()> {
        let record = self.leases.get(&lease_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::LeaseDenied,
                format!("lease {lease_id} not found"),
            )
        })?;
        if record.holder_session != session_id {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "cannot release lease owned by another session",
            ));
        }
        self.leases.remove(&lease_id);
        Ok(())
    }

    /// Release all leases owned by a specific session (e.g. on disconnect).
    pub fn release_session_leases(&mut self, session_id: SessionId) {
        self.leases
            .retain(|_, record| record.holder_session != session_id);
    }

    /// Prune expired leases.
    pub fn cleanup_expired_leases(&mut self, current_tick: GameTick) -> usize {
        let mut pruned = 0;
        self.leases.retain(|_, record| {
            if record.expires_at_tick <= current_tick {
                pruned += 1;
                false
            } else {
                true
            }
        });
        pruned
    }

    /// Number of active leases.
    #[must_use]
    pub fn active_lease_count(&self) -> usize {
        self.leases.len()
    }

    fn next_lease_id(&self) -> Result<(LeaseId, u64)> {
        let next_sequence = self.next_lease_seq.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "lease identifier space exhausted",
            )
        })?;
        Ok((LeaseId::new(u128::from(self.next_lease_seq)), next_sequence))
    }
}

fn checked_expiry(current_tick: GameTick, ttl_ticks: u64) -> Result<GameTick> {
    if ttl_ticks == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "lease TTL must be nonzero",
        ));
    }
    current_tick
        .0
        .checked_add(ttl_ticks)
        .map(GameTick)
        .ok_or_else(|| DfmcpError::new(ErrorCode::BudgetExceeded, "lease expiry tick overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MapCoord;

    #[test]
    fn test_spatial_lease_mutual_exclusion() -> Result<()> {
        let mut manager = LeaseManager::new();
        let s1 = SessionId::new(1);
        let s2 = SessionId::new(2);

        let cuboid = MapCuboid::new(
            MapCoord { x: 0, y: 0, z: 100 },
            MapCoord { x: 5, y: 5, z: 100 },
        )?;

        let l1 = manager.acquire_spatial_lease(s1, cuboid, true, GameTick(100), 50)?;
        assert_eq!(manager.active_lease_count(), 1);

        // Overlapping lease from s2 must fail
        let overlapping_cuboid = MapCuboid::new(
            MapCoord { x: 3, y: 3, z: 100 },
            MapCoord { x: 8, y: 8, z: 100 },
        )?;
        let result = manager.acquire_spatial_lease(s2, overlapping_cuboid, true, GameTick(100), 50);
        assert!(result.is_err());

        // Disjoint lease from s2 must succeed
        let disjoint_cuboid = MapCuboid::new(
            MapCoord {
                x: 10,
                y: 10,
                z: 100,
            },
            MapCoord {
                x: 15,
                y: 15,
                z: 100,
            },
        )?;
        let _l2 = manager.acquire_spatial_lease(s2, disjoint_cuboid, true, GameTick(100), 50)?;
        assert_eq!(manager.active_lease_count(), 2);

        // Releasing l1 allows s2 to acquire overlapping cuboid
        manager.release_lease(l1, s1)?;
        let l3 = manager.acquire_spatial_lease(s2, overlapping_cuboid, true, GameTick(100), 50)?;
        assert!(l3.get() > 0);

        Ok(())
    }

    #[test]
    fn test_entity_lease_exclusive_and_shared() -> Result<()> {
        let mut manager = LeaseManager::new();
        let s1 = SessionId::new(1);
        let s2 = SessionId::new(2);
        let dwarf = EntityId::new(55);

        // Shared lease from s1
        let l1 = manager.acquire_entity_lease(s1, dwarf, false, GameTick(100), 50)?;

        // Another shared lease from s2 succeeds
        let l2 = manager.acquire_entity_lease(s2, dwarf, false, GameTick(100), 50)?;
        assert_eq!(manager.active_lease_count(), 2);

        // Exclusive lease from s1 fails while s2 holds shared
        let result = manager.acquire_entity_lease(s1, dwarf, true, GameTick(100), 50);
        assert!(result.is_err());

        // Cleanup
        manager.release_lease(l1, s1)?;
        manager.release_lease(l2, s2)?;
        assert_eq!(manager.active_lease_count(), 0);

        Ok(())
    }
}

#[cfg(test)]
mod spatial_verification_tests {
    use super::*;
    use crate::MapCoord;

    fn area() -> MapCuboid {
        MapCuboid { min: MapCoord::new(0, 0, 2), max: MapCoord::new(31, 31, 2) }
    }
    #[test]
    fn spatial_verification_checks_holder_scope_and_half_open_lifetime() -> Result<()> {
        let mut book = LeaseManager::new();
        let owner = SessionId::new(1);
        let id = book.acquire_spatial_lease(owner, area(), true, GameTick(100), 10)?;
        for tick in [100, 109] { book.verify_exclusive_spatial(id, owner, area(), GameTick(tick))?; }
        for tick in [99, 110, u64::MAX] {
            assert!(book.verify_exclusive_spatial(id, owner, area(), GameTick(tick)).is_err());
        }
        assert!(book.verify_exclusive_spatial(id, SessionId::new(2), area(), GameTick(100)).is_err());
        let outside = MapCuboid { max: MapCoord::new(32, 31, 2), ..area() };
        assert!(book.verify_exclusive_spatial(id, owner, outside, GameTick(100)).is_err());
        let reversed = MapCuboid { min: area().max, max: area().min };
        assert!(book.verify_exclusive_spatial(id, owner, reversed, GameTick(100)).is_err());
        Ok(())
    }
    #[test]
    fn stale_tokens_cannot_follow_release_or_reacquisition() -> Result<()> {
        let mut book = LeaseManager::new();
        let owner = SessionId::new(1);
        let old = book.acquire_spatial_lease(owner, area(), true, GameTick(0), 10)?;
        book.release_lease(old, owner)?;
        let current = book.acquire_spatial_lease(owner, area(), true, GameTick(0), 10)?;
        assert_ne!(old, current);
        assert!(book.verify_exclusive_spatial(old, owner, area(), GameTick(0)).is_err());
        book.verify_exclusive_spatial(current, owner, area(), GameTick(0))?;
        book.cleanup_expired_leases(GameTick(10));
        assert!(book.verify_exclusive_spatial(current, owner, area(), GameTick(10)).is_err());
        Ok(())
    }
    #[test]
    fn shared_entity_or_fabricated_tokens_do_not_cover_spatial_writes() -> Result<()> {
        let mut book = LeaseManager::new();
        let owner = SessionId::new(1);
        let shared = book.acquire_spatial_lease(owner, area(), false, GameTick(0), 10)?;
        let entity = book.acquire_entity_lease(owner, EntityId::new(1), true, GameTick(0), 10)?;
        for id in [shared, entity, LeaseId::new(999)] {
            assert!(book.verify_exclusive_spatial(id, owner, area(), GameTick(0)).is_err());
        }
        Ok(())
    }
}
