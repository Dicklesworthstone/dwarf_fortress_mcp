//! Read witnesses of sealed plans and witness-based revalidation.
//!
//! A sealed plan is bound to the exact anchor it was planned at. In a shared
//! fortress any member's action moves that anchor, which used to force an
//! explicit replay round-trip even for unrelated work. A plan's read witness
//! is everything its decision depended on: every entity, edge and pause flag
//! its predicates (preconditions, postconditions, obligations, terminal
//! condition) or action scopes name, and every terrain region they touch,
//! widened by a one-tile hazard halo. If none of that changed between the
//! version the plan was sealed on and now, the change was unrelated: the
//! intent is replayed at the current anchor and, when the replay yields the
//! very same actions, committed directly with a deterministic certificate.
//!
//! Refinement may only remove false conflicts: anything the witness cannot
//! bound (an oversized region, a version outside the retained history, a
//! different epoch) falls back to explicit replay, never to a guess.

use std::collections::BTreeSet;

use dfmcp_core::{Digest32, EdgeId, EntityId, MapCoord, MapCuboid};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::{Predicate, WorldSnapshot};
use serde_json::{Value, json};

/// Largest region (after the halo) a witness compares tile by tile.
const MAX_WITNESS_REGION_TILES: u64 = 65_536;

/// Everything a sealed plan read.
#[derive(Debug, Default)]
pub(crate) struct ReadWitness {
    entities: BTreeSet<EntityId>,
    edges: BTreeSet<EdgeId>,
    regions: Vec<MapCuboid>,
    pause: bool,
    /// Set when some read cannot be bounded; the witness then proves nothing.
    unbounded: bool,
}

fn haloed(area: MapCuboid) -> Option<MapCuboid> {
    let grow = |c: MapCoord, d: i32| -> Option<MapCoord> {
        Some(MapCoord::new(
            c.x.checked_add(d)?,
            c.y.checked_add(d)?,
            c.z.checked_add(d)?,
        ))
    };
    let area = MapCuboid::new(grow(area.min, -1)?, grow(area.max, 1)?).ok()?;
    area.tile_count()
        .filter(|tiles| *tiles <= MAX_WITNESS_REGION_TILES)
        .map(|_| area)
}

impl ReadWitness {
    fn region(&mut self, area: MapCuboid) {
        match haloed(area) {
            Some(area) => self.regions.push(area),
            None => self.unbounded = true,
        }
    }

    fn predicate(&mut self, predicate: &Predicate) {
        match predicate {
            Predicate::True | Predicate::False => {}
            Predicate::EntityExists(id)
            | Predicate::EntityKind { entity_id: id, .. }
            | Predicate::FieldCompare { entity_id: id, .. } => {
                self.entities.insert(*id);
            }
            Predicate::EdgeExists { edge_id, .. } => {
                self.edges.insert(*edge_id);
            }
            Predicate::Paused(_) => self.pause = true,
            Predicate::RegionTerrain { area, .. } => self.region(*area),
            Predicate::All(children) | Predicate::Any(children) => {
                for child in children {
                    self.predicate(child);
                }
            }
            Predicate::Not(child) => self.predicate(child),
        }
    }

    /// The read witness of a sealed plan.
    pub(crate) fn of(plan: &PreparedPlan) -> Self {
        let mut witness = Self::default();
        witness.predicate(&plan.terminal_condition);
        for step in &plan.steps {
            let scope = step.action.scope();
            witness.entities.extend(scope.entity_ids);
            if let Some(area) = scope.map_area {
                witness.region(area);
            }
            for predicate in step.preconditions.iter().chain(&step.postconditions) {
                witness.predicate(predicate);
            }
            if let Some(obligation) = &step.obligation {
                witness.predicate(&obligation.terminal);
                if let Some(failure) = &obligation.failure {
                    witness.predicate(failure);
                }
            }
        }
        witness
    }

    /// The first read that differs between two versions, or `None` when
    /// every read is unchanged.
    pub(crate) fn first_change(&self, base: &WorldSnapshot, now: &WorldSnapshot) -> Option<Value> {
        if self.unbounded {
            return Some(
                json!({"read": "unbounded", "note": "a read region exceeds the witness bound"}),
            );
        }
        if base.fortress_id != now.fortress_id || base.cursor.epoch != now.cursor.epoch {
            return Some(
                json!({"read": "lineage", "note": "the fortress or observation epoch changed"}),
            );
        }
        if self.pause && base.paused != now.paused {
            return Some(json!({"read": "pause"}));
        }
        for id in &self.entities {
            if base.graph.entities.get(id) != now.graph.entities.get(id) {
                return Some(json!({"read": "entity", "entity_id": id.get().to_string()}));
            }
        }
        for id in &self.edges {
            if base.graph.edges.get(id) != now.graph.edges.get(id) {
                return Some(json!({"read": "edge", "edge_id": id.get().to_string()}));
            }
        }
        for area in &self.regions {
            for coord in dfmcp_world::terrain::region_tiles(*area) {
                if base.tile_code_at(coord) != now.tile_code_at(coord) {
                    return Some(json!({"read": "terrain", "at": [coord.x, coord.y, coord.z]}));
                }
            }
        }
        None
    }

    /// Canonical description, for certificates.
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "entities": self.entities.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
            "edges": self.edges.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
            "regions": self.regions.iter().map(|r| json!({"min": [r.min.x, r.min.y, r.min.z], "max": [r.max.x, r.max.y, r.max.z]})).collect::<Vec<_>>(),
            "pause": self.pause,
        })
    }
}

/// Whether two sealed plans perform the same actions in the same structure
/// (identifiers derived from the anchor may differ).
pub(crate) fn same_actions(a: &PreparedPlan, b: &PreparedPlan) -> bool {
    a.steps.len() == b.steps.len()
        && a.steps.iter().zip(&b.steps).all(|(x, y)| {
            x.id == y.id
                && x.action == y.action
                && x.depends_on == y.depends_on
                && x.required_capability == y.required_capability
                && x.risk == y.risk
        })
}

/// Deterministic certificate of a witness rebase.
pub(crate) fn certificate(
    from: &PreparedPlan,
    to: &PreparedPlan,
    base: &WorldSnapshot,
    now: &WorldSnapshot,
    witness: &ReadWitness,
) -> Value {
    let body = json!({
        "schema": "dfmcp.witness-rebase/1",
        "from_digest": from.digest.to_hex(),
        "to_digest": to.digest.to_hex(),
        "base_state_hash": base.state_hash.to_hex(),
        "current_state_hash": now.state_hash.to_hex(),
        "witness": witness.to_json(),
        "policy": "intent_replay_with_identical_actions_after_unchanged_read_witness",
    });
    let digest = Digest32::of_bytes(body.to_string().as_bytes()).to_hex();
    json!({"certificate": body, "certificate_digest": digest})
}
