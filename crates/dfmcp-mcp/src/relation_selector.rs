//! Shared, positive one-hop selection over one fully admitted immutable source.
//! The caller owns source admission and charges the complete scan before entry.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, EdgeId, EntityId, ErrorCode, Result};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Predicate, PredicateTruth, WorldSnapshot,
    evaluate_truth_for,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{core_query, source};

pub(super) const MAX_WITNESSES: usize = 4;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    edge_kinds: Vec<String>,
    direction: Option<Direction>,
    #[serde(default)]
    target_kinds: Vec<String>,
    #[serde(default)]
    target_ids: Vec<String>,
    #[serde(rename = "where")]
    predicate: Option<Value>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Direction {
    #[default]
    Outgoing,
    Incoming,
    Undirected,
}

impl Direction {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Undirected => "undirected",
        }
    }
}

pub(super) struct Selection {
    pub(super) edge_kinds: BTreeSet<String>,
    pub(super) direction: Direction,
    pub(super) target_kinds: BTreeSet<String>,
    pub(super) target_ids: BTreeSet<EntityId>,
    pub(super) predicate: Option<Predicate>,
    exact_kind_names: bool,
}

pub(super) fn kinds(names: Vec<String>) -> Result<BTreeSet<String>> {
    core_query::bounded_names(names, 32)?
        .into_iter()
        .map(|name| Ok(core_query::entity_kind(&name)?.as_str().to_owned()))
        .collect()
}

impl Request {
    pub(super) fn normalize(self) -> Result<Selection> {
        self.normalize_kind_names(false)
    }

    /// Operational aggregates have always distinguished builtin kinds from
    /// Other/Custom names. Opt in without changing established rq1/rk1 matching
    /// or identity for existing relationship and ranking consumers.
    #[allow(dead_code)]
    pub(super) fn normalize_exact(self) -> Result<Selection> {
        self.normalize_kind_names(true)
    }

    fn normalize_kind_names(self, exact_kind_names: bool) -> Result<Selection> {
        let edge_kinds = core_query::bounded_names(self.edge_kinds, 32)?;
        if edge_kinds.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "relation selection requires at least one explicit edge kind",
            ));
        }
        Ok(Selection {
            edge_kinds: edge_kinds
                .into_iter()
                .map(|name| {
                    Ok(edge_name(&core_query::edge_kind(&name)?, exact_kind_names).into_owned())
                })
                .collect::<Result<_>>()?,
            direction: self.direction.unwrap_or_default(),
            target_kinds: if exact_kind_names {
                core_query::bounded_names(self.target_kinds, 32)?
                    .into_iter()
                    .map(
                        |name| Ok(entity_name(&core_query::entity_kind(&name)?, true).into_owned()),
                    )
                    .collect::<Result<_>>()?
            } else {
                kinds(self.target_kinds)?
            },
            target_ids: core_query::bounded_names(self.target_ids, 64)?
                .into_iter()
                .map(|id| core_query::entity_id(&id))
                .collect::<Result<_>>()?,
            predicate: self.predicate.map(core_query::decode_filter).transpose()?,
            exact_kind_names,
        })
    }
}

impl Selection {
    /// The structural part is separate so the established relation query can
    /// retain its original source-predicate position in its cursor identity.
    pub(super) fn append_identity(&self, bytes: &mut Vec<u8>) {
        fn append(bytes: &mut Vec<u8>, value: &[u8]) {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value);
        }
        for names in [&self.edge_kinds, &self.target_kinds] {
            bytes.extend_from_slice(&(names.len() as u64).to_be_bytes());
            for name in names {
                append(bytes, name.as_bytes());
            }
        }
        bytes.extend_from_slice(&(self.target_ids.len() as u64).to_be_bytes());
        for id in &self.target_ids {
            bytes.extend_from_slice(&id.get().to_be_bytes());
        }
        append(bytes, self.direction.name().as_bytes());
    }

    pub(super) fn append_predicate_identity(&self, bytes: &mut Vec<u8>) {
        match &self.predicate {
            None => bytes.push(0),
            Some(predicate) => {
                bytes.push(1);
                let encoded = predicate.canonical_bytes();
                bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
                bytes.extend_from_slice(&encoded);
            }
        }
    }

    pub(super) fn entity_work(&self) -> usize {
        self.predicate
            .as_ref()
            .map_or(0, Predicate::complexity)
            .saturating_add(self.target_kinds.len())
            .saturating_add(self.target_ids.len())
    }

    pub(super) fn edge_work(&self) -> usize {
        self.edge_kinds.len().saturating_add(4)
    }

    pub(super) fn json(&self) -> Value {
        json!({"direction":self.direction.name(), "edge_kinds":self.edge_kinds,
            "target_kinds":self.target_kinds,
            "target_ids":self.target_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "witnesses_per_row_limit":MAX_WITNESSES})
    }
}

#[derive(Default)]
pub(super) struct Evidence<'a> {
    pub(super) matches: usize,
    pub(super) unknown_targets: usize,
    pub(super) missing_targets: usize,
    witnesses: Vec<(&'a EdgeRecord, &'a EntityRecord)>,
    exact_kind_names: bool,
}

impl Evidence<'_> {
    pub(super) fn unknown(&self) -> bool {
        self.matches == 0 && (self.unknown_targets > 0 || self.missing_targets > 0)
    }

    pub(super) fn json(&self) -> Value {
        json!({
            "matched_edges":self.matches,
            "witnesses_returned":self.witnesses.len(),
            "witnesses_omitted":self.matches.saturating_sub(self.witnesses.len()),
            "unknown_target_filters":self.unknown_targets,
            "missing_target_records":self.missing_targets,
            "witnesses":self.witnesses.iter().map(|(edge,target)| json!({
                "edge_id":edge.id.to_string(), "edge_revision":edge.revision,
                "edge_kind":edge_name(&edge.kind, self.exact_kind_names),
                "from":edge.from.to_string(), "to":edge.to.to_string(),
                "target":{"entity_id":target.id.to_string(), "generation":target.generation,
                    "revision":target.revision},
                "endpoint_identity_basis":"source_snapshot"
            })).collect::<Vec<_>>(),
            "eligible_as_mutation_evidence":false
        })
    }
}

pub(super) struct Candidate<'a> {
    pub(super) entity: &'a EntityRecord,
    pub(super) evidence: Evidence<'a>,
}

pub(super) struct Prepared<'a> {
    pub(super) candidates: BTreeMap<EntityId, Candidate<'a>>,
    pub(super) selected: usize,
    pub(super) filter_false: usize,
    pub(super) filter_unknown: usize,
    pub(super) selected_edges: usize,
}

fn entity_name(kind: &EntityKind, exact: bool) -> Cow<'_, str> {
    match kind {
        EntityKind::Other(name) if exact => Cow::Owned(format!("other:{name}")),
        _ => Cow::Borrowed(kind.as_str()),
    }
}

fn edge_name(kind: &EdgeKind, exact: bool) -> Cow<'_, str> {
    match kind {
        EdgeKind::Custom(name) if exact => Cow::Owned(format!("custom:{name}")),
        _ => Cow::Borrowed(kind.as_str()),
    }
}

fn matches_kind(kinds: &BTreeSet<String>, entity: &EntityRecord, exact: bool) -> bool {
    kinds.is_empty() || kinds.contains(entity_name(&entity.kind, exact).as_ref())
}

fn truth(snapshot: &WorldSnapshot, id: EntityId, predicate: Option<&Predicate>) -> PredicateTruth {
    predicate.map_or(PredicateTruth::True, |predicate| {
        evaluate_truth_for(snapshot, id, predicate)
    })
}

fn record_edge<'a>(
    subject: EntityId,
    target: EntityId,
    edge: &'a EdgeRecord,
    selection: &Selection,
    targets: &BTreeMap<EntityId, (&'a EntityRecord, PredicateTruth)>,
    candidates: &mut BTreeMap<EntityId, Candidate<'a>>,
) {
    let Some(candidate) = candidates.get_mut(&subject) else {
        return;
    };
    if !selection.target_ids.is_empty() && !selection.target_ids.contains(&target) {
        return;
    }
    let evidence = &mut candidate.evidence;
    match targets.get(&target) {
        Some((entity, PredicateTruth::True)) => {
            evidence.matches += 1;
            if evidence.witnesses.len() < MAX_WITNESSES {
                evidence.witnesses.push((edge, entity));
            }
        }
        Some((_, PredicateTruth::False)) => {}
        Some((_, PredicateTruth::Unknown)) => evidence.unknown_targets += 1,
        None => evidence.missing_targets += 1,
    }
}

/// Evaluate each source and target predicate once, then visit every edge once.
/// Unselected source bytes and total work must already have been admitted.
pub(super) fn prepare<'a>(
    snapshot: &'a WorldSnapshot,
    source_kinds: &BTreeSet<String>,
    source_predicate: Option<&Predicate>,
    selection: &Selection,
    work: &source::Budget,
) -> Result<Prepared<'a>> {
    let mut targets = BTreeMap::new();
    let mut prepared = Prepared {
        candidates: BTreeMap::new(),
        selected: 0,
        filter_false: 0,
        filter_unknown: 0,
        selected_edges: 0,
    };
    for entity in snapshot.graph.entities.values() {
        work.check()?;
        let target = if matches_kind(&selection.target_kinds, entity, selection.exact_kind_names)
            && (selection.target_ids.is_empty() || selection.target_ids.contains(&entity.id))
        {
            truth(snapshot, entity.id, selection.predicate.as_ref())
        } else {
            PredicateTruth::False
        };
        targets.insert(entity.id, (entity, target));
        if !matches_kind(source_kinds, entity, selection.exact_kind_names) {
            continue;
        }
        prepared.selected += 1;
        match truth(snapshot, entity.id, source_predicate) {
            PredicateTruth::False => {
                prepared.filter_false += 1;
                continue;
            }
            PredicateTruth::Unknown => {
                prepared.filter_unknown += 1;
                continue;
            }
            PredicateTruth::True => {}
        }
        prepared.candidates.insert(
            entity.id,
            Candidate {
                entity,
                evidence: Evidence {
                    exact_kind_names: selection.exact_kind_names,
                    ..Evidence::default()
                },
            },
        );
    }
    for (id, edge) in &snapshot.graph.edges {
        work.check()?;
        if *id == EdgeId::NIL || *id != edge.id {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "relation-query edge identity is inconsistent",
            ));
        }
        if !selection
            .edge_kinds
            .contains(edge_name(&edge.kind, selection.exact_kind_names).as_ref())
        {
            continue;
        }
        prepared.selected_edges += 1;
        let mut record = |subject, target| {
            record_edge(
                subject,
                target,
                edge,
                selection,
                &targets,
                &mut prepared.candidates,
            );
        };
        match selection.direction {
            Direction::Outgoing => record(edge.from, edge.to),
            Direction::Incoming => record(edge.to, edge.from),
            Direction::Undirected => {
                record(edge.from, edge.to);
                if edge.from != edge.to {
                    record(edge.to, edge.from);
                }
            }
        }
    }
    Ok(prepared)
}
