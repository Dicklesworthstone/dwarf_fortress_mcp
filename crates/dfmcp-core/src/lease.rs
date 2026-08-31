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
        let lease_id = LeaseId::new(u128::from(self.next_lease_seq));
        self.next_lease_seq = self.next_lease_seq.saturating_add(1);

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
            expires_at_tick: GameTick(current_tick.0.saturating_add(ttl_ticks)),
        };

        self.leases.insert(lease_id, record);
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

        let lease_id = LeaseId::new(u128::from(self.next_lease_seq));
        self.next_lease_seq = self.next_lease_seq.saturating_add(1);

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
            expires_at_tick: GameTick(current_tick.0.saturating_add(ttl_ticks)),
        };

        self.leases.insert(lease_id, record);
        Ok(lease_id)
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

        if record.expires_at_tick < current_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "cannot renew expired lease",
            ));
        }

        record.expires_at_tick = GameTick(record.expires_at_tick.0.saturating_add(extension_ticks));
        Ok(())
    }

    /// Explicitly release a lease.
    pub fn release_lease(&mut self, lease_id: LeaseId, session_id: SessionId) -> Result<()> {
        if let Some(record) = self.leases.get(&lease_id)
            && record.holder_session != session_id
        {
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
