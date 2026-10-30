//! Retain opening restrictions inside every delegated native boundary. A check
//! before a call cannot protect a later tick consumed inside that call.
use super::*;
use dfmcp_core::{CapabilityScope, MapCoord, MapCuboid};

const MAX_GRANTS: usize = 64;
const MAX_SCOPE_ENTITIES: usize = 4096;

pub(super) fn intersect(
    c: &OperationContext,
    opening: &[CapabilityGrant],
) -> Result<Vec<CapabilityGrant>> {
    if c.grants.len() > MAX_GRANTS
        || opening.len() > MAX_GRANTS
        || c.grants
            .iter()
            .chain(opening)
            .any(|grant| grant.scope.entity_ids.len() > MAX_SCOPE_ENTITIES)
    {
        return Err(bounded());
    }
    let mut out = Vec::new();
    for current in &c.grants {
        for retained in opening {
            // This owner has no durable limited-use counter. Cloning a finite
            // grant would create replayable consumption authority.
            if current.is_limited() || retained.is_limited() {
                continue;
            }
            let capability = match (current.capability, retained.capability) {
                (a, b) if a == b => a,
                (Capability::Admin, b) => b,
                (a, Capability::Admin) => a,
                _ => continue,
            };
            if [current, retained].iter().any(|grant| {
                grant
                    .scope
                    .fortress_id
                    .is_some_and(|id| id != c.anchor.fortress_id)
                    || grant
                        .scope
                        .map_area
                        .is_some_and(|area| MapCuboid::new(area.min, area.max).is_err())
            }) {
                continue;
            }
            let entity_ids = match (
                current.scope.entity_ids.is_empty(),
                retained.scope.entity_ids.is_empty(),
            ) {
                (true, _) => retained.scope.entity_ids.clone(),
                (_, true) => current.scope.entity_ids.clone(),
                (false, false) => {
                    let intersection: std::collections::BTreeSet<_> = current
                        .scope
                        .entity_ids
                        .intersection(&retained.scope.entity_ids)
                        .copied()
                        .collect();
                    // Empty means unrestricted in CapabilityScope, so an empty
                    // intersection must remove the entire grant instead.
                    if intersection.is_empty() {
                        continue;
                    }
                    intersection
                }
            };
            let map_area = match (current.scope.map_area, retained.scope.map_area) {
                (Some(a), Some(b)) => {
                    let min = MapCoord::new(
                        a.min.x.max(b.min.x),
                        a.min.y.max(b.min.y),
                        a.min.z.max(b.min.z),
                    );
                    let max = MapCoord::new(
                        a.max.x.min(b.max.x),
                        a.max.y.min(b.max.y),
                        a.max.z.min(b.max.z),
                    );
                    let Ok(area) = MapCuboid::new(min, max) else {
                        continue;
                    };
                    Some(area)
                }
                (Some(area), None) | (None, Some(area)) => Some(area),
                (None, None) => None,
            };
            let expires_at_tick = match (current.expires_at_tick, retained.expires_at_tick) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(tick), None) | (None, Some(tick)) => Some(tick),
                (None, None) => None,
            };
            let grant = CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(c.anchor.fortress_id),
                    entity_ids,
                    map_area,
                },
                max_risk: current.max_risk.min(retained.max_risk),
                expires_at_tick,
                remaining_uses: None,
            };
            if !out.contains(&grant) {
                if out.len() == MAX_GRANTS {
                    return Err(bounded());
                }
                out.push(grant);
            }
        }
    }
    Ok(out)
}
