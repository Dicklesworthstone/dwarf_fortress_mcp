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
    /// Complete building-kind domain, including absence of future occupants.
    building_domain: bool,
    /// Aggregate predicates read every entity in these canonical kind domains,
    /// including future members and currently nonmatching selector values.
    entity_domains: BTreeSet<String>,
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
            Predicate::LaboratoryRoomAccess { area, .. }
            | Predicate::LaboratoryRoomAccessPairedStairs { area, .. } => {
                self.region(*area);
                self.building_domain = true;
            }
            Predicate::LaboratoryCivilianReleaseSafety { .. } => {
                // Clearance reads the complete entity domain, including absence.
                // It also has owned history outside a snapshot. Never auto-rebase.
                self.unbounded = true;
            }
            Predicate::EntityQuantity { kind, .. } => {
                self.entity_domains.insert(kind.as_str().to_owned());
            }
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
        if self.building_domain {
            // A new entity can obstruct a route without changing any previously
            // named entity. Compare the complete domain, not just known IDs.
            if base.graph.entities.len() > 65_536 || now.graph.entities.len() > 65_536 {
                return Some(
                    json!({"read":"building_domain", "note":"building census exceeds its bound"}),
                );
            }
            let base_buildings = base
                .graph
                .entities
                .iter()
                .filter(|(_, entity)| entity.kind.as_str() == "building");
            let current_buildings = now
                .graph
                .entities
                .iter()
                .filter(|(_, entity)| entity.kind.as_str() == "building");
            if base_buildings.ne(current_buildings) {
                return Some(json!({"read":"building_domain"}));
            }
        }
        if self.pause && base.paused != now.paused {
            return Some(json!({"read": "pause"}));
        }
        if !self.entity_domains.is_empty() {
            if base.graph.entities.len() > 65_536 || now.graph.entities.len() > 65_536 {
                return Some(
                    json!({"read":"entity_domains", "note":"aggregate census exceeds its bound"}),
                );
            }
            // A single bounded census covers every aggregate kind. Comparing
            // only matching IDs would miss insertions and selector changes.
            let before = base
                .graph
                .entities
                .iter()
                .filter(|(_, entity)| self.entity_domains.contains(entity.kind.as_str()));
            let after = now
                .graph
                .entities
                .iter()
                .filter(|(_, entity)| self.entity_domains.contains(entity.kind.as_str()));
            if before.ne(after) {
                return Some(json!({"read":"entity_domains", "kinds":self.entity_domains}));
            }
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
        let mut value = json!({
            "entities": self.entities.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
            "edges": self.edges.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
            "regions": self.regions.iter().map(|r| json!({"min": [r.min.x, r.min.y, r.min.z], "max": [r.max.x, r.max.y, r.max.z]})).collect::<Vec<_>>(),
            "pause": self.pause,
        });
        // Existing witness certificates keep their exact bytes.
        if self.building_domain {
            value["building_domain"] =
                json!({"kind":"building", "coverage":"complete", "max_entities":65_536});
        }
        if !self.entity_domains.is_empty() {
            value["entity_domains"] = json!({"kinds":self.entity_domains,
                "coverage":"complete", "max_entities":65_536});
        }
        value
    }
}

/// Whether two sealed plans preserve the complete reviewed execution contract.
/// Identifiers derived from the anchor may differ; goals, guards, temporal
/// obligations, compensation and authority requirements must remain identical.
pub(crate) fn same_actions(a: &PreparedPlan, b: &PreparedPlan) -> bool {
    // Replaying at a later anchor can resynthesize deadlines and preparation
    // expiry without changing any action or read witness. Only identities and
    // derived keys may change automatically; a changed execution contract must
    // be reviewed through the ordinary stale-plan replay path.
    a.summary == b.summary
        && a.terminal_condition == b.terminal_condition
        && a.max_risk == b.max_risk
        && a.required_capabilities == b.required_capabilities
        && a.requires_checkpoint == b.requires_checkpoint
        && a.expires_at_tick == b.expires_at_tick
        && a.steps.len() == b.steps.len()
        && a.steps.iter().zip(&b.steps).all(|(x, y)| {
            x.id == y.id
                && x.action == y.action
                && x.preconditions == y.preconditions
                && x.postconditions == y.postconditions
                && x.compensation == y.compensation
                && x.obligation == y.obligation
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

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{FortressId, GameTick, ObservationCursor, Result};
    use dfmcp_world::{EntityKind, EntityRecord, WorldGraph};

    fn replay_fixture(
        tick: u64,
        sequence: u64,
        temporal: bool,
    ) -> Result<(WorldSnapshot, PreparedPlan)> {
        use dfmcp_core::{
            Capability, CapabilityGrant, CapabilityScope, IntentId, OperationContext, RequestId,
            RiskTier, SessionId, WorkBudget,
        };
        use dfmcp_intent::{Action, Constraint, DigMode, Intent, RequestedAction, StaticPlanner};
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor { epoch: 1, sequence },
            false,
            WorldGraph::default(),
        );
        let action = if temporal {
            Action::DesignateDig {
                area: MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(0, 0, 0))?,
                mode: DigMode::Mine,
            }
        } else {
            Action::Pause { paused: true }
        };
        let intent = Intent {
            id: IntentId::new(u128::from(sequence) + 1),
            anchor: snapshot.anchor(),
            summary: "retain the reviewed execution contract".into(),
            terminal_condition: Predicate::All(dfmcp_intent::effects::default_postconditions(
                &action,
                "fixture",
                snapshot.fortress_id,
            )),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };
        let context = OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget::default(),
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        };
        let plan = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)?;
        Ok((snapshot, plan))
    }

    #[test]
    fn replay_rejects_a_resynthesized_obligation_deadline() -> Result<()> {
        let (base, original) = replay_fixture(10, 0, true)?;
        let (current, replayed) = replay_fixture(20, 1, true)?;
        assert!(
            ReadWitness::of(&original)
                .first_change(&base, &current)
                .is_none()
        );
        assert_eq!(original.steps[0].action, replayed.steps[0].action);
        assert!(
            original.steps[0]
                .obligation
                .as_ref()
                .zip(replayed.steps[0].obligation.as_ref())
                .is_some_and(|(old, new)| new.deadline_tick.0 == old.deadline_tick.0 + 10)
        );
        // Even equal preparation lifetimes cannot hide a changed temporal goal.
        let mut same_expiry = replayed.clone();
        same_expiry.expires_at_tick = original.expires_at_tick;
        assert!(!same_actions(&original, &same_expiry));
        assert!(!same_actions(&original, &replayed));
        Ok(())
    }

    #[test]
    fn replay_cannot_revive_an_expired_preparation() -> Result<()> {
        let (base, original) = replay_fixture(10, 0, false)?;
        let (current, replayed) = replay_fixture(original.expires_at_tick.0 + 1, 1, false)?;
        assert!(
            ReadWitness::of(&original)
                .first_change(&base, &current)
                .is_none()
        );
        assert_eq!(original.steps[0].action, replayed.steps[0].action);
        assert!(current.tick > original.expires_at_tick);
        assert!(replayed.expires_at_tick > original.expires_at_tick);
        assert!(!same_actions(&original, &replayed));
        Ok(())
    }

    #[test]
    fn same_tick_replay_requires_the_complete_reviewed_contract() -> Result<()> {
        use dfmcp_core::{Capability, RiskTier};
        use dfmcp_intent::Action;
        let (base, original) = replay_fixture(10, 0, true)?;
        let (current, replayed) = replay_fixture(10, 1, true)?;
        assert!(
            ReadWitness::of(&original)
                .first_change(&base, &current)
                .is_none()
        );
        assert_ne!(original.anchor, replayed.anchor);
        assert_ne!(
            original.steps[0].idempotency_key,
            replayed.steps[0].idempotency_key
        );
        assert!(same_actions(&original, &replayed));
        let mut changes = Vec::new();
        let mut changed = replayed.clone();
        changed.summary.push_str(" changed");
        changes.push(("summary", changed));
        let mut changed = replayed.clone();
        changed.terminal_condition = Predicate::Paused(true);
        changes.push(("whole goal", changed));
        let mut changed = replayed.clone();
        changed.max_risk = RiskTier::ReadOnly;
        changes.push(("risk ceiling", changed));
        let mut changed = replayed.clone();
        changed.required_capabilities.insert(Capability::Query);
        changes.push(("capability summary", changed));
        let mut changed = replayed.clone();
        changed.requires_checkpoint = !changed.requires_checkpoint;
        changes.push(("checkpoint", changed));
        let mut changed = replayed.clone();
        changed.steps[0].preconditions.push(Predicate::Paused(true));
        changes.push(("preconditions", changed));
        let mut changed = replayed.clone();
        changed.steps[0]
            .postconditions
            .push(Predicate::Paused(true));
        changes.push(("postconditions", changed));
        let mut changed = replayed.clone();
        changed.steps[0].compensation = Some(Action::Pause { paused: true });
        changes.push(("compensation", changed));
        let mut changed = replayed.clone();
        changed.steps[0].obligation = None;
        changes.push(("removed obligation", changed));
        for field in ["terminal", "failure", "cadence", "stability"] {
            let mut changed = replayed.clone();
            if let Some(obligation) = &mut changed.steps[0].obligation {
                match field {
                    "terminal" => obligation.terminal = Predicate::Paused(true),
                    "failure" => obligation.failure = Some(Predicate::Paused(false)),
                    "cadence" => obligation.poll_interval_ticks += 1,
                    _ => obligation.stable_for_observations += 1,
                }
            }
            changes.push((field, changed));
        }
        for (field, changed) in changes {
            assert!(
                !same_actions(&original, &changed),
                "accepted changed {field}"
            );
        }
        Ok(())
    }

    #[test]
    fn room_access_witness_detects_phantom_buildings_and_kind_aliases() -> Result<()> {
        let base = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(0),
            ObservationCursor {
                epoch: 1,
                sequence: 0,
            },
            false,
            WorldGraph::default(),
        );
        let mut witness = ReadWitness::default();
        witness.predicate(&Predicate::LaboratoryRoomAccess {
            area: MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(3, 3, 0))?,
            entry: MapCoord::new(0, 0, 0),
            targets: vec![MapCoord::new(3, 3, 0)],
        });
        assert!(witness.first_change(&base, &base).is_none());
        for kind in [
            EntityKind::Building,
            EntityKind::Other("building".to_owned()),
        ] {
            let mut now = base.clone();
            let id = EntityId::new(900);
            now.graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation: 1,
                    revision: 1,
                    kind,
                    label: "new obstruction".to_owned(),
                    fields: Default::default(),
                },
            );
            now.refresh_hash();
            assert_eq!(
                witness.first_change(&base, &now),
                Some(json!({"read":"building_domain"}))
            );
            assert_eq!(
                witness.first_change(&now, &base),
                Some(json!({"read":"building_domain"}))
            );
        }
        assert_eq!(witness.to_json()["building_domain"]["coverage"], "complete");
        Ok(())
    }

    #[test]
    fn legacy_witness_certificate_does_not_acquire_an_empty_building_domain() {
        assert_eq!(
            ReadWitness::default().to_json(),
            json!({
                "entities":[], "edges":[], "regions":[], "pause":false,
            })
        );
    }

    #[test]
    fn quantity_witness_detects_phantoms_selectors_and_replaced_contributors() {
        use dfmcp_world::{CompareOp, Fact, FactSource, Value as WorldValue};
        let mut base = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(0),
            ObservationCursor::ORIGIN,
            false,
            WorldGraph::default(),
        );
        let item = |id, item_type: &str, kind| EntityRecord {
            id: EntityId::new(id),
            generation: 1,
            revision: 1,
            kind,
            label: "item".into(),
            fields: std::collections::BTreeMap::from([
                (
                    "type_key".into(),
                    Fact::known(
                        WorldValue::Text(item_type.into()),
                        GameTick(0),
                        FactSource::DfhackField("type".into()),
                        Digest32::of_bytes(b"source"),
                    ),
                ),
                (
                    "stack_size".into(),
                    Fact::known(
                        WorldValue::U64(1),
                        GameTick(0),
                        FactSource::DfhackField("stack".into()),
                        Digest32::of_bytes(b"source"),
                    ),
                ),
            ]),
        };
        base.graph
            .entities
            .insert(EntityId::new(1), item(1, "BED", EntityKind::Item));
        base.graph
            .entities
            .insert(EntityId::new(2), item(2, "DOOR", EntityKind::Item));
        base.refresh_hash();
        let mut witness = ReadWitness::default();
        witness.predicate(&Predicate::EntityQuantity {
            kind: EntityKind::Item,
            filter_field: "type_key".into(),
            filter_value: WorldValue::Text("BED".into()),
            quantity_field: Some("stack_size".into()),
            op: CompareOp::Ge,
            value: 1,
        });
        assert!(witness.first_change(&base, &base).is_none());
        for kind in [EntityKind::Item, EntityKind::Other("item".into())] {
            let mut phantom = base.clone();
            phantom
                .graph
                .entities
                .insert(EntityId::new(3), item(3, "BED", kind));
            phantom.refresh_hash();
            assert!(witness.first_change(&base, &phantom).is_some());
        }
        let mut selector = base.clone();
        selector
            .graph
            .entities
            .insert(EntityId::new(2), item(2, "BED", EntityKind::Item));
        selector.refresh_hash();
        assert!(witness.first_change(&base, &selector).is_some());
        let mut replacement = base.clone();
        replacement.graph.entities.remove(&EntityId::new(1));
        replacement
            .graph
            .entities
            .insert(EntityId::new(3), item(3, "BED", EntityKind::Item));
        replacement.refresh_hash();
        assert!(witness.first_change(&base, &replacement).is_some());
        let mut unrelated = base.clone();
        unrelated
            .graph
            .entities
            .insert(EntityId::new(3), item(3, "BED", EntityKind::Unit));
        unrelated.refresh_hash();
        assert!(witness.first_change(&base, &unrelated).is_none());
        assert_eq!(
            witness.to_json()["entity_domains"]["kinds"],
            json!(["item"])
        );
    }
}
