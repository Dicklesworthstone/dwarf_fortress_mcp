//! Bounded, generation-fenced joins inside an observed projection. These are
//! one-hop memberships, not transitive containment, job readiness or authority.
//! Bind every referenced root before visiting rows: an empty population or a
//! decisive boolean branch must not hide an unresolved or recycled reference.
use super::*;
use dfmcp_world::{EdgeKind, EdgeRecord};

const MAX_INDEX_ENTRIES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum Relation { ContainedIn, Uses, Performs }
impl Relation {
    fn matches(self, kind: &EdgeKind) -> bool {
        // EdgeKind's canonical encoding uses its semantic string, including
        // Custom aliases. Equal canonical bytes must not yield different joins.
        kind.as_str() == match self {
            Self::ContainedIn => "contained_in", Self::Uses => "uses", Self::Performs => "performs",
        }
    }
}

/// Direction is relative to the explicitly named root, not to the candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum Direction { Incoming, Outgoing }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Selector { root: EntityId, generation: u32, relation: Relation, direction: Direction }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingState { Ready, Missing, GenerationMismatch, IdentityUnestablished, GraphUnestablished }
impl BindingState {
    fn text(self) -> &'static str {
        match self {
            Self::Ready => "bound", Self::Missing => "root_not_observed",
            Self::GenerationMismatch => "generation_mismatch",
            Self::IdentityUnestablished => "root_identity_unestablished",
            Self::GraphUnestablished => "edge_endpoint_unestablished",
        }
    }
}

struct Binding {
    state: BindingState,
    current_generation: Option<u32>,
    members: BTreeMap<EntityId, Truth>,
}

#[derive(Default)]
pub(super) struct Bindings { entries: BTreeMap<Selector, Binding> }
impl Bindings {
    pub(super) fn ready(&self) -> bool {
        self.entries.values().all(|binding| binding.state == BindingState::Ready)
    }
    pub(super) fn invalid_generation(&self) -> bool {
        self.entries.values().any(|binding| binding.state == BindingState::GenerationMismatch)
    }
    pub(super) fn guard(&self, truth: Truth) -> Truth {
        if self.ready() { truth } else { Truth::Unknown }
    }
    pub(super) fn row_truth(&self, entity: &EntityRecord, entity_id: &str, generation: u32,
        relation: Relation, direction: Direction) -> Result<Truth> {
        let selector = Selector { root: positive_id(entity_id)?, generation, relation, direction };
        let Some(binding) = self.entries.get(&selector) else { return Ok(Truth::Unknown); };
        if binding.state != BindingState::Ready || entity.generation == 0 || entity.revision == 0 {
            return Ok(Truth::Unknown);
        }
        // This is absence of an edge in the named observed projection ONLY.
        // No complete-world relationship or native job-completion claim follows.
        Ok(binding.members.get(&entity.id).copied().unwrap_or(Truth::False))
    }
    pub(super) fn annotate(&self, fact: &mut Value) {
        if self.entries.is_empty() { return; } // Keep old evidence bytes unchanged.
        let references: Vec<_> = self.entries.iter().map(|(selector, binding)| json!({
            "entity_id":selector.root.to_string(),"generation":selector.generation,
            "current_generation":binding.current_generation,"relation":selector.relation,
            "direction":selector.direction,"status":binding.state.text(),
            "indexed_endpoints":binding.members.len(),
        })).collect();
        fact["relationship_selection"] = json!({"references":references,
            "references_established":self.ready(),"generation_mismatch":self.invalid_generation(),
            "scope":"observed_projection","max_hops":1,"duplicate_edges_count_once":true,
            "complete_world_relationships_proven":false});
    }
}

fn edge_truth(edge: &EdgeRecord, snapshot: &WorldSnapshot, budget: &mut EvaluationBudget) -> Result<Truth> {
    if edge.revision == 0 || edge.fields.is_empty() { return Ok(Truth::Unknown); }
    let mut source = None;
    let mut established = true;
    for fact in edge.fields.values() {
        budget.charge()?;
        let same_source = source.is_none_or(|digest| digest == fact.source_digest);
        source = Some(fact.source_digest);
        established &= same_source && fact.source_digest != Digest32::ZERO
            && fact.observed_at == snapshot.tick
            && matches!(&fact.source, FactSource::DfhackField(_))
            && match &fact.presence {
                None => true,
                Some(FactPresence::Known(value)) => value == &fact.value,
                _ => false,
            };
    }
    Ok(if established { Truth::True } else { Truth::Unknown })
}

pub(super) fn bind(predicate: &Predicate, snapshot: &WorldSnapshot,
    budget: &mut EvaluationBudget) -> Result<Bindings> {
    let mut pending = vec![predicate];
    let mut selectors = std::collections::BTreeSet::new();
    let mut nodes = 0usize;
    while let Some(predicate) = pending.pop() {
        nodes += 1;
        if nodes > MAX_CONDITIONS { return Err(bounded("relationship predicate exceeds the node bound")); }
        match predicate {
            Predicate::Related { entity_id, generation, relation, direction } => {
                budget.charge()?;
                if *generation == 0 { return Err(invalid("relationship root generation must be positive")); }
                selectors.insert(Selector { root: positive_id(entity_id)?, generation: *generation,
                    relation: *relation, direction: *direction });
            }
            Predicate::All { args } | Predicate::Any { args } => {
                if nodes.saturating_add(pending.len()).saturating_add(args.len()) > MAX_CONDITIONS {
                    return Err(bounded("relationship predicate exceeds the node bound"));
                }
                pending.extend(args);
            }
            Predicate::Not { arg } => pending.push(arg),
            Predicate::Always {} | Predicate::Field { .. } => {}
        }
    }
    let mut result = Bindings::default();
    let mut indexed = 0usize;
    for selector in selectors {
        budget.charge()?;
        let root = snapshot.graph.entities.get(&selector.root);
        let state = match root {
            None => BindingState::Missing,
            Some(root) if root.id != selector.root || root.generation == 0 || root.revision == 0 =>
                BindingState::IdentityUnestablished,
            Some(root) if root.generation != selector.generation => BindingState::GenerationMismatch,
            Some(_) => BindingState::Ready,
        };
        let mut binding = Binding { state, current_generation: root.map(|root| root.generation),
            members: BTreeMap::new() };
        if state == BindingState::Ready {
            for (edge_id, edge) in &snapshot.graph.edges {
                budget.charge()?;
                if !selector.relation.matches(&edge.kind) { continue; }
                let endpoint = match selector.direction {
                    Direction::Incoming if edge.to == selector.root => edge.from,
                    Direction::Outgoing if edge.from == selector.root => edge.to,
                    _ => continue,
                };
                budget.charge()?;
                if !snapshot.graph.entities.get(&endpoint).is_some_and(|entity|
                    entity.id == endpoint && entity.generation != 0 && entity.revision != 0) {
                    binding.state = BindingState::GraphUnestablished;
                    continue;
                }
                let truth = if edge.id == *edge_id { edge_truth(edge, snapshot, budget)? } else { Truth::Unknown };
                if let Some(previous) = binding.members.get_mut(&endpoint) {
                    // Several native attachment roles may connect the same two
                    // entities. Existential membership is true if ANY is proven.
                    if truth == Truth::True { *previous = Truth::True; }
                } else {
                    if indexed >= MAX_INDEX_ENTRIES {
                        return Err(bounded("relationship joins exceed 65536 indexed endpoints"));
                    }
                    indexed += 1;
                    binding.members.insert(endpoint, truth);
                }
            }
        }
        result.entries.insert(selector, binding);
    }
    budget.check()?;
    Ok(result)
}

#[cfg(test)]
#[path = "query_watch_relationship_tests.rs"]
mod tests;
