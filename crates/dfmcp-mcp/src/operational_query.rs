//! Single-pass summaries and lexical search over an authorized snapshot.
//! No bridge reads, mutations, hidden indexes, floating-point sums, or absence proofs.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_NODES: usize = 4096;
const MAX_INPUT_BYTES: usize = 65536;
const MAX_SCAN_ENTITIES: usize = 100000;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK: usize = 4_000_000;
const MAX_GROUPS: usize = 128;
const MAX_METRICS: usize = 16;
const MAX_ROWS: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    expected_anchor: Option<Value>,
    query: Query,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Aggregate {
        #[serde(default)]
        kinds: Vec<String>,
        group_by: Option<GroupBy>,
        #[serde(default)]
        metrics: Vec<Metric>,
        max_groups: Option<usize>,
    },
    Search {
        text: String,
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        text_fields: Vec<String>,
        include_labels: Option<bool>,
        match_mode: Option<MatchMode>,
        limit: Option<usize>,
        continuation: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GroupBy {
    EntityKind,
    Field { field: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metric {
    name: String,
    field: String,
    numeric_type: NumericType,
    scale: Option<u32>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NumericType {
    I64,
    U64,
    Fixed,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MatchMode {
    All,
    Any,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn validate_input(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let (mut nodes, mut bytes) = (0usize, 0usize);
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(32);
        if nodes > MAX_NODES || depth > 24 {
            return Err(exhausted("query input shape exceeds its bound"));
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values
                    .len()
                    .saturating_add(nodes)
                    .saturating_add(pending.len())
                    > MAX_NODES
                {
                    return Err(exhausted("query input node count exceeds its bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(nodes)
                    .saturating_add(pending.len())
                    > MAX_NODES
                {
                    return Err(exhausted("query input node count exceeds its bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > MAX_INPUT_BYTES {
            return Err(exhausted("query input bytes exceed their bound"));
        }
    }
    Ok(())
}

fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id":anchor.fortress_id.to_string(),"epoch":anchor.cursor.epoch,
        "sequence":anchor.cursor.sequence,"game_tick":anchor.tick.0,"state_hash":anchor.state_hash.to_string()})
}

fn names(mut values: Vec<String>, maximum: usize) -> Result<Vec<String>> {
    if values.len() > maximum
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
    {
        return Err(invalid("selector exceeds its name or count bound"));
    }
    values.sort();
    values.dedup();
    Ok(values)
}

fn kind_name(kind: &EntityKind) -> String {
    match kind {
        EntityKind::Other(name) => format!("other:{name}"),
        _ => kind.as_str().to_owned(),
    }
}

fn kinds(values: Vec<String>) -> Result<BTreeSet<String>> {
    let values = names(values, 32)?;
    for value in &values {
        if !matches!(
            value.as_str(),
            "fortress"
                | "unit"
                | "item"
                | "building"
                | "job"
                | "work_order"
                | "stockpile"
                | "zone"
                | "burrow"
                | "squad"
                | "military_order"
                | "tile_feature"
                | "plant"
                | "creature"
                | "historical_figure"
                | "civilization"
                | "announcement"
                | "syndrome"
        ) && !(value.starts_with("other:") && value.len() > 6)
        {
            return Err(invalid(
                "unrecognized entity kind; custom names require other: prefix",
            ));
        }
    }
    Ok(values.into_iter().collect())
}

/// Never expose a stale/redacted backing value or conflicting Known payload.
fn presence(fact: Option<&Fact>) -> (&'static str, Option<&WorldValue>) {
    let Some(fact) = fact else {
        return ("unobserved", None);
    };
    match &fact.presence {
        None => ("known", Some(&fact.value)),
        Some(FactPresence::Known(value)) if value == &fact.value => ("known", Some(value)),
        Some(FactPresence::Known(_)) => ("contradicted", None),
        Some(FactPresence::Absent) => ("absent", None),
        Some(FactPresence::Unknown(_)) => ("unknown", None),
        Some(FactPresence::Unsupported(_)) => ("unsupported", None),
        Some(FactPresence::Omitted(_)) => ("omitted", None),
        Some(FactPresence::Redacted(_)) => ("redacted", None),
        Some(FactPresence::Stale(_)) => ("stale", None),
    }
}

fn scalar(value: &WorldValue) -> Result<Value> {
    Ok(match value {
        WorldValue::Null => json!({"type":"null"}),
        WorldValue::Bool(value) => json!({"type":"bool","value":value}),
        WorldValue::I64(value) => json!({"type":"i64","value":value}),
        WorldValue::U64(value) => json!({"type":"u64","value":value}),
        WorldValue::Fixed { units, scale } => {
            json!({"type":"fixed","value":{"units":units,"scale":scale}})
        }
        WorldValue::Text(value) if value.len() <= 256 => json!({"type":"text","value":value}),
        WorldValue::Entity(value) => json!({"type":"entity","value":value.to_string()}),
        WorldValue::Coord(value) => {
            json!({"type":"coord","value":{"x":value.x,"y":value.y,"z":value.z}})
        }
        _ => {
            return Err(exhausted(
                "grouping requires scalar values with text at most 256 UTF-8 bytes",
            ));
        }
    })
}

/// Explicitly sort object keys; feature-dependent serde_json map order is not identity.
fn canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Object(map) => {
            output.push(b'{');
            let sorted: BTreeMap<_, _> = map.iter().collect();
            for (i, (key, value)) in sorted.into_iter().enumerate() {
                if i != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)
                    .map_err(|_| invalid("cannot encode query key"))?;
                output.push(b':');
                canonical_json(value, output)?;
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    output.push(b',');
                }
                canonical_json(value, output)?;
            }
            output.push(b']');
        }
        _ => serde_json::to_writer(output, value)
            .map_err(|_| invalid("cannot encode query value"))?,
    }
    Ok(())
}

fn identity(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Digest32> {
    let mut spec = input.clone();
    if let Some(query) = spec.get_mut("query").and_then(Value::as_object_mut) {
        query.remove("continuation");
        query.remove("limit");
    }
    let value = json!({"domain":"dfmcp-operational-query-v1","session":context.session_id.to_string(),
        "anchor":anchor_json(snapshot.anchor()),"request":spec});
    let mut bytes = Vec::new();
    canonical_json(&value, &mut bytes)?;
    Ok(Digest32::of_bytes(&bytes))
}

fn finish(
    snapshot: &WorldSnapshot,
    identity: Digest32,
    mut result: Value,
    maximum: usize,
) -> Result<Value> {
    result["schema"] = json!("dfmcp.query.result/1");
    result["anchor"] = anchor_json(snapshot.anchor());
    result["query_digest"] = json!(identity.to_string());
    result["coverage"] = json!({"domain":"observed_projection","absence_proven":false,
        "note":"counts and matches describe the supplied projection, not complete fortress history or unobserved domains"});
    let bytes = serde_json::to_vec(&result).map_err(|_| invalid("cannot encode query result"))?;
    if bytes.len() > maximum {
        return Err(exhausted(
            "query result exceeds its byte budget; narrow kinds, metrics, groups, or search",
        ));
    }
    Ok(result)
}

/// The server supplies its exact published snapshot; this does not refresh it.
pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "query context does not match the published snapshot",
        ));
    }
    if snapshot.graph.entities.len() > (context.budget.max_entities as usize).min(MAX_SCAN_ENTITIES)
    {
        return Err(exhausted("query exceeds the bounded entity scan"));
    }
    validate_input(input)?;
    let request: Request = serde_json::from_value(input.clone()).map_err(|error| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            format!("invalid operational query: {error}"),
        )
    })?;
    if request.schema != "dfmcp.query/1" {
        return Err(invalid("query schema must be dfmcp.query/1"));
    }
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| *anchor != anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "expected_anchor does not match the complete current anchor",
        ));
    }
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query source hash is invalid",
        ));
    }
    let maximum = usize::try_from(
        context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("query byte bound cannot be represented"))?;
    let identity = identity(snapshot, context, input)?;
    match request.query {
        Query::Aggregate {
            kinds: selected,
            group_by,
            metrics,
            max_groups,
        } => {
            let selected = kinds(selected)?;
            let max_groups = max_groups.unwrap_or(32);
            aggregate(
                snapshot, identity, &selected, group_by, &metrics, max_groups, maximum,
            )
        }
        Query::Search {
            text,
            kinds: selected,
            text_fields,
            include_labels,
            match_mode,
            limit,
            continuation,
        } => {
            let spec = SearchSpec {
                text,
                kinds: kinds(selected)?,
                text_fields: names(text_fields, 16)?,
                include_labels: include_labels.unwrap_or(true),
                match_mode: match_mode.unwrap_or(MatchMode::All),
                limit: limit.unwrap_or(4),
                continuation,
            };
            search(snapshot, identity, &spec, maximum)
        }
    }
}

#[derive(Default)]
struct MetricState {
    samples: u64,
    sum: i128,
    minimum: Option<i128>,
    maximum: Option<i128>,
    excluded: BTreeMap<&'static str, u64>,
    sources: BTreeMap<&'static str, u64>,
}

impl MetricState {
    fn add(&mut self, spec: &Metric, fact: Option<&Fact>) -> Result<()> {
        let (state, value) = presence(fact);
        let number = match (spec.numeric_type, value) {
            (NumericType::I64, Some(WorldValue::I64(value))) => Some(i128::from(*value)),
            (NumericType::U64, Some(WorldValue::U64(value))) => Some(i128::from(*value)),
            (NumericType::Fixed, Some(WorldValue::Fixed { units, scale }))
                if Some(*scale) == spec.scale =>
            {
                Some(i128::from(*units))
            }
            _ => None,
        };
        let Some(number) = number else {
            *self
                .excluded
                .entry(if state == "known" {
                    "type_or_scale_mismatch"
                } else {
                    state
                })
                .or_default() += 1;
            return Ok(());
        };
        self.sum = self
            .sum
            .checked_add(number)
            .ok_or_else(|| exhausted("numeric aggregate overflow"))?;
        self.samples += 1;
        self.minimum = Some(self.minimum.map_or(number, |old| old.min(number)));
        self.maximum = Some(self.maximum.map_or(number, |old| old.max(number)));
        if let Some(fact) = fact {
            let source = match &fact.source {
                FactSource::DfhackField(_) => "observed",
                FactSource::Derived(_) => "derived",
                FactSource::AgentAssertion(_) => "agent_assertion",
                FactSource::Replay => "replay",
            };
            *self.sources.entry(source).or_default() += 1;
        }
        Ok(())
    }
    fn json(&self, spec: &Metric) -> Value {
        json!({"field":spec.field,"numeric_type":match spec.numeric_type {
            NumericType::I64=>"i64",NumericType::U64=>"u64",NumericType::Fixed=>"fixed"},
            "scale":spec.scale,"samples":self.samples,"excluded":self.excluded,"sources":self.sources,
            "sum":if self.samples == 0 { None } else { Some(self.sum.to_string()) },
            "minimum":self.minimum.map(|value|value.to_string()),"maximum":self.maximum.map(|value|value.to_string()),
            "mean":if self.samples == 0 { Value::Null } else { json!({"numerator":self.sum.to_string(),"denominator":self.samples}) },
            "encoding":"decimal integer strings; fixed statistics are in declared-scale units; mean is exact rational"})
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    All,
    Kind(String),
    Presence(&'static str),
    Known(WorldValue),
}

struct Group {
    count: u64,
    metrics: Vec<MetricState>,
}

fn group_key(entity: &EntityRecord, group_by: Option<&GroupBy>) -> Result<GroupKey> {
    Ok(match group_by {
        None => GroupKey::All,
        Some(GroupBy::EntityKind) => GroupKey::Kind(kind_name(&entity.kind)),
        Some(GroupBy::Field { field }) => match presence(entity.fields.get(field)) {
            (_, Some(value)) => {
                scalar(value)?;
                GroupKey::Known(value.clone())
            }
            (state, None) => GroupKey::Presence(state),
        },
    })
}

fn aggregate(
    snapshot: &WorldSnapshot,
    identity: Digest32,
    selected: &BTreeSet<String>,
    group_by: Option<GroupBy>,
    metrics: &[Metric],
    max_groups: usize,
    maximum: usize,
) -> Result<Value> {
    if max_groups == 0 || max_groups > MAX_GROUPS || metrics.len() > MAX_METRICS {
        return Err(exhausted(
            "aggregate group or metric count exceeds its bound",
        ));
    }
    if let Some(GroupBy::Field { field }) = &group_by {
        names(vec![field.clone()], 1)?;
    }
    let mut metric_names = BTreeSet::new();
    for metric in metrics {
        names(vec![metric.name.clone(), metric.field.clone()], 2)?;
        if !metric_names.insert(&metric.name) {
            return Err(invalid("aggregate metric names must be unique"));
        }
        if matches!(metric.numeric_type, NumericType::Fixed) != metric.scale.is_some() {
            return Err(invalid(
                "fixed metrics require scale; non-fixed metrics must omit scale",
            ));
        }
    }
    if snapshot
        .graph
        .entities
        .len()
        .saturating_mul(metrics.len() + 2)
        > MAX_WORK
    {
        return Err(exhausted("aggregate scan exceeds its operation bound"));
    }
    let mut groups = BTreeMap::new();
    if group_by.is_none() {
        groups.insert(
            GroupKey::All,
            Group {
                count: 0,
                metrics: metrics.iter().map(|_| MetricState::default()).collect(),
            },
        );
    }
    let mut matched = 0u64;
    for entity in snapshot.graph.entities.values() {
        if !selected.is_empty() && !selected.contains(&kind_name(&entity.kind)) {
            continue;
        }
        let key = group_key(entity, group_by.as_ref())?;
        if !groups.contains_key(&key) && groups.len() >= max_groups {
            return Err(exhausted(
                "aggregate group cardinality exceeded max_groups; no partial summary was published",
            ));
        }
        let group = groups.entry(key).or_insert_with(|| Group {
            count: 0,
            metrics: metrics.iter().map(|_| MetricState::default()).collect(),
        });
        group.count += 1;
        matched += 1;
        for (state, spec) in group.metrics.iter_mut().zip(metrics) {
            state.add(spec, entity.fields.get(&spec.field))?;
        }
    }
    let rows = groups
        .into_iter()
        .map(|(key, group)| {
            let key = match key {
                GroupKey::All => json!({"kind":"all"}),
                GroupKey::Kind(kind) => json!({"kind":"entity_kind","value":kind}),
                GroupKey::Presence(state) => json!({"kind":"field","presence":state,"value":null}),
                GroupKey::Known(value) => {
                    json!({"kind":"field","presence":"known","value":scalar(&value)?})
                }
            };
            let statistics: BTreeMap<_, _> = metrics
                .iter()
                .zip(&group.metrics)
                .map(|(spec, state)| (spec.name.clone(), state.json(spec)))
                .collect();
            Ok(json!({"key":key,"count":group.count,"metrics":statistics}))
        })
        .collect::<Result<Vec<_>>>()?;
    finish(
        snapshot,
        identity,
        json!({"kind":"aggregate","matched":matched,"groups":rows,
        "truncated":false,"continuation":null,"epistemic_state":"inferred",
        "scanned_entities":snapshot.graph.entities.len(),"metric_evaluations":matched.saturating_mul(metrics.len() as u64)}),
        maximum,
    )
}

struct SearchSpec {
    text: String,
    kinds: BTreeSet<String>,
    text_fields: Vec<String>,
    include_labels: bool,
    match_mode: MatchMode,
    limit: usize,
    continuation: Option<String>,
}

type Rank = (Reverse<u32>, EntityId);
struct Hit<'a> {
    entity: &'a EntityRecord,
    terms: u32,
    fields: Vec<&'a str>,
    source_digests: Vec<String>,
}

fn cursor(identity: Digest32, rank: Rank) -> String {
    let body = format!("ls1:{identity}:{}:{}", rank.0.0, rank.1.get());
    format!("{body}:{}", Digest32::of_bytes(body.as_bytes()))
}

fn parse_cursor(raw: Option<&str>, identity: Digest32) -> Result<Option<Rank>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(exhausted("search continuation exceeds its byte bound"));
    }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 5 || parts[0] != "ls1" {
        return Err(invalid("invalid lexical-search continuation"));
    }
    let terms = parts[2]
        .parse::<u32>()
        .map_err(|_| invalid("invalid search rank"))?;
    let id = parts[3]
        .parse::<u64>()
        .map_err(|_| invalid("invalid search entity ID"))?;
    if terms == 0 || terms > 16 || id == 0 {
        return Err(invalid("search cursor does not name a possible ranked hit"));
    }
    let rank = (Reverse(terms), EntityId::new(id));
    if raw != cursor(identity, rank) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "search cursor changed or belongs to another query, session, or snapshot",
        ));
    }
    Ok(Some(rank))
}

fn match_text(
    text: &str,
    terms: &[String],
    found: &mut [bool],
    scanned: &mut usize,
) -> Result<bool> {
    *scanned = scanned
        .checked_add(text.len())
        .ok_or_else(|| exhausted("search byte counter overflow"))?;
    if *scanned > MAX_SCAN_BYTES {
        return Err(exhausted("search exceeds its UTF-8 source byte budget"));
    }
    let lowered = text.to_lowercase();
    let mut matched = false;
    for (term, found) in terms.iter().zip(found) {
        if lowered.contains(term) {
            *found = true;
            matched = true;
        }
    }
    Ok(matched)
}

fn search(
    snapshot: &WorldSnapshot,
    identity: Digest32,
    spec: &SearchSpec,
    maximum: usize,
) -> Result<Value> {
    if spec.text.is_empty()
        || spec.text.len() > 512
        || spec.text.contains('\0')
        || spec.limit == 0
        || spec.limit > MAX_ROWS
    {
        return Err(invalid(
            "search requires text of 1..512 UTF-8 bytes and limit of 1..128",
        ));
    }
    if !spec.include_labels && spec.text_fields.is_empty() {
        return Err(invalid("search requires labels or explicit text_fields"));
    }
    let terms: BTreeSet<_> = spec
        .text
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    if terms.is_empty() || terms.len() > 16 || terms.iter().any(|term| term.len() > 128) {
        return Err(exhausted(
            "search accepts 1..16 distinct terms of at most 128 UTF-8 bytes",
        ));
    }
    let terms: Vec<_> = terms.into_iter().collect();
    if snapshot
        .graph
        .entities
        .len()
        .saturating_mul(spec.text_fields.len() + 1)
        .saturating_mul(terms.len())
        > MAX_WORK
    {
        return Err(exhausted("search exceeds its term/field scan bound"));
    }
    let after = parse_cursor(spec.continuation.as_deref(), identity)?;
    let mut after_found = after.is_none();
    let (mut matched, mut remaining, mut scanned, mut unavailable) = (0u64, 0usize, 0usize, 0u64);
    let mut best: BTreeMap<Rank, Hit<'_>> = BTreeMap::new();
    for entity in snapshot.graph.entities.values() {
        if !spec.kinds.is_empty() && !spec.kinds.contains(&kind_name(&entity.kind)) {
            continue;
        }
        let mut found = vec![false; terms.len()];
        let mut fields = Vec::new();
        let mut source_digests = Vec::new();
        if spec.include_labels && match_text(&entity.label, &terms, &mut found, &mut scanned)? {
            fields.push("$label");
        }
        for field in &spec.text_fields {
            let fact = entity.fields.get(field);
            match presence(fact) {
                (_, Some(WorldValue::Text(text))) => {
                    if match_text(text, &terms, &mut found, &mut scanned)? {
                        // Borrow the key from the canonical entity, never a temporary selector.
                        if let Some((name, fact)) = entity.fields.get_key_value(field) {
                            fields.push(name.as_str());
                            source_digests.push(fact.source_digest.to_string());
                        }
                    }
                }
                _ => unavailable += 1,
            }
        }
        let count = found.iter().filter(|&&value| value).count();
        if count == 0 || (matches!(spec.match_mode, MatchMode::All) && count != terms.len()) {
            continue;
        }
        matched += 1;
        let rank = (Reverse(count as u32), entity.id);
        if after == Some(rank) {
            after_found = true;
        }
        if after.is_some_and(|after| rank <= after) {
            continue;
        }
        remaining += 1;
        best.insert(
            rank,
            Hit {
                entity,
                terms: count as u32,
                fields,
                source_digests,
            },
        );
        if best.len() > spec.limit + 1 {
            best.pop_last();
        }
    }
    if !after_found || (after.is_some() && remaining == 0) {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "search continuation no longer names a hit in this exact result set",
        ));
    }
    let ordered: Vec<_> = best.into_iter().collect();
    let build = |count: usize| -> Result<Value> {
        let truncated = remaining > count;
        let continuation = if truncated && count > 0 {
            Some(cursor(identity, ordered[count - 1].0))
        } else {
            None
        };
        let rows:Vec<_>=ordered.iter().take(count).map(|(_,hit)|json!({
            "entity_id":hit.entity.id.to_string(),"generation":hit.entity.generation,"revision":hit.entity.revision,
            "kind":kind_name(&hit.entity.kind),"label":hit.entity.label,"matched_terms":hit.terms,
            "matched_fields":hit.fields,"source_digests":hit.source_digests,
            "inspect":{"kind":"inspect","entity_id":hit.entity.id.to_string(),"generation":hit.entity.generation,
                "fields":hit.fields.iter().filter(|&&field|field!="$label").collect::<Vec<_>>()}
        })).collect();
        finish(
            snapshot,
            identity,
            json!({"kind":"search","matched":matched,"returned":count,
            "truncated":truncated,"continuation":continuation,"rows":rows,"terms":terms,
            "ranking":"distinct matching terms descending, then stable entity ID ascending",
            "matching":"Unicode lowercase substring terms, not stemming or semantic search",
            "scanned_text_bytes":scanned,"unavailable_or_nontext_fields":unavailable}),
            maximum,
        )
    };
    if ordered.is_empty() {
        return build(0);
    }
    // Largest complete page that fits, including the envelope and next cursor.
    let (mut low, mut high) = (1usize, spec.limit.min(ordered.len()));
    let mut result = None;
    while low <= high {
        let middle = low + (high - low) / 2;
        match build(middle) {
            Ok(page) => {
                result = Some(page);
                low = middle + 1;
            }
            Err(error) if error.code == ErrorCode::BudgetExceeded => {
                high = middle - 1;
            }
            Err(error) => return Err(error),
        }
    }
    result.ok_or_else(|| exhausted("one search row and its envelope exceed the response budget"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
        SessionId, WorkBudget,
    };
    use dfmcp_world::WorldGraph;

    fn fact(value: WorldValue) -> Fact {
        Fact::known(
            value,
            GameTick(1),
            FactSource::DfhackField("fixture".to_owned()),
            Digest32::of_bytes(b"fixture"),
        )
    }
    fn fixture() -> (WorldSnapshot, OperationContext) {
        let mut graph = WorldGraph::default();
        for (id, label, text) in [
            (1, "Urist", "Cavern flood"),
            (2, "Domas", "Flood at workshop"),
            (3, "Child", "secret flood"),
            (4, "Report", "Cavern notice"),
        ] {
            let mut fields = BTreeMap::from([(
                "message".to_owned(),
                fact(WorldValue::Text(text.to_owned())),
            )]);
            if id < 3 {
                fields.insert("alive".to_owned(), fact(WorldValue::Bool(id == 1)));
                fields.insert("stock".to_owned(), fact(WorldValue::U64(u64::MAX)));
            } else if id == 3 {
                fields.insert(
                    "alive".to_owned(),
                    Fact::with_presence(
                        FactPresence::Omitted("not read".to_owned()),
                        GameTick(1),
                        FactSource::Replay,
                        Digest32::ZERO,
                    ),
                );
                let mut secret = fact(WorldValue::U64(9000));
                secret.presence = Some(FactPresence::Redacted("private".to_owned()));
                fields.insert("stock".to_owned(), secret);
                if let Some(message) = fields.get_mut("message") {
                    message.presence = Some(FactPresence::Redacted("private".to_owned()));
                }
            } else {
                fields.insert(
                    "stock".to_owned(),
                    Fact::with_presence(
                        FactPresence::Absent,
                        GameTick(1),
                        FactSource::Replay,
                        Digest32::ZERO,
                    ),
                );
            }
            graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: 1,
                    kind: if id == 4 {
                        EntityKind::Announcement
                    } else {
                        EntityKind::Unit
                    },
                    label: label.to_owned(),
                    fields,
                },
            );
        }
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );
        let ctx = OperationContext {
            session_id: SessionId::new(9),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget {
                max_wall_millis: 5000,
                max_game_ticks: 1000,
                max_entities: 1000,
                max_bytes: 1024 * 1024,
                max_output_tokens: 65536,
                max_actions: 1,
            },
            grants: vec![CapabilityGrant {
                capability: Capability::Query,
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
        (snapshot, ctx)
    }
    fn envelope(query: Value) -> Value {
        json!({"schema":"dfmcp.query/1","query":query})
    }
    fn numeric_query() -> Value {
        envelope(json!({"kind":"aggregate","metrics":[{
            "name":"stock","field":"stock","numeric_type":"u64"
        }]}))
    }
    fn search_query() -> Value {
        envelope(
            json!({"kind":"search","text":"cavern flood","text_fields":["message"],"match_mode":"any","limit":1}),
        )
    }

    #[test]
    fn sums_are_exact_beyond_u64_and_unknown_samples_are_accounted_for() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let result = execute(&snapshot, &ctx, &numeric_query())?;
        let stats = &result["groups"][0]["metrics"]["stock"];
        assert_eq!(result["matched"], 4);
        assert_eq!(stats["samples"], 2);
        assert_eq!(stats["sum"], "36893488147419103230");
        assert_eq!(stats["minimum"], u64::MAX.to_string());
        assert_eq!(stats["mean"]["numerator"], "36893488147419103230");
        assert_eq!(stats["mean"]["denominator"], 2);
        assert_eq!(stats["excluded"]["redacted"], 1);
        assert_eq!(stats["excluded"]["absent"], 1);
        assert_eq!(stats["sources"]["observed"], 2);
        assert_eq!(result["coverage"]["absence_proven"], false);
        Ok(())
    }

    #[test]
    fn grouping_separates_known_false_from_unobserved_and_omitted() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let result = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"aggregate","group_by":{"kind":"field","field":"alive"}})),
        )?;
        let groups = result["groups"]
            .as_array()
            .ok_or_else(|| invalid("missing groups"))?;
        assert_eq!(groups.len(), 4);
        let states: Vec<_> = groups
            .iter()
            .filter_map(|group| group["key"]["presence"].as_str())
            .collect();
        assert!(states.contains(&"unobserved"));
        assert!(states.contains(&"omitted"));
        assert_eq!(states.iter().filter(|&&state| state == "known").count(), 2);
        assert!(groups.iter().all(|group| group["count"] == 1));
        Ok(())
    }

    #[test]
    fn empty_domain_and_type_mismatch_never_invent_zero_stock() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let mut empty = numeric_query();
        empty["query"]["kinds"] = json!(["item"]);
        let result = execute(&snapshot, &ctx, &empty)?;
        assert_eq!(result["matched"], 0);
        assert!(result["groups"][0]["metrics"]["stock"]["sum"].is_null());
        let mut mismatch = numeric_query();
        mismatch["query"]["metrics"][0]["numeric_type"] = json!("i64");
        let result = execute(&snapshot, &ctx, &mismatch)?;
        let stats = &result["groups"][0]["metrics"]["stock"];
        assert_eq!(stats["samples"], 0);
        assert_eq!(stats["excluded"]["type_or_scale_mismatch"], 2);
        assert!(stats["mean"].is_null());
        Ok(())
    }

    #[test]
    fn fixed_scale_is_explicit_and_mean_keeps_exact_units() -> Result<()> {
        let (mut snapshot, mut ctx) = fixture();
        for (id, units, scale) in [(1, -9, 2), (2, 2, 2), (3, 99, 3)] {
            let entity = snapshot
                .graph
                .entities
                .get_mut(&EntityId::new(id))
                .ok_or_else(|| invalid("missing fixture entity"))?;
            entity
                .fields
                .insert("price".to_owned(), fact(WorldValue::Fixed { units, scale }));
        }
        snapshot.refresh_hash();
        ctx.anchor = snapshot.anchor();
        let result = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"aggregate","metrics":[{
                "name":"price","field":"price","numeric_type":"fixed","scale":2
            }]})),
        )?;
        let stats = &result["groups"][0]["metrics"]["price"];
        assert_eq!(stats["sum"], "-7");
        assert_eq!(stats["minimum"], "-9");
        assert_eq!(stats["maximum"], "2");
        assert_eq!(stats["mean"]["denominator"], 2);
        assert_eq!(stats["excluded"]["type_or_scale_mismatch"], 1);
        assert_eq!(stats["excluded"]["unobserved"], 1);
        Ok(())
    }

    #[test]
    fn group_cardinality_and_duplicate_metric_names_fail_atomically() {
        let (snapshot, ctx) = fixture();
        let request = envelope(
            json!({"kind":"aggregate","group_by":{"kind":"field","field":"alive"},"max_groups":1}),
        );
        assert!(
            matches!(execute(&snapshot,&ctx,&request),Err(error) if error.code==ErrorCode::BudgetExceeded)
        );
        let mut duplicate = numeric_query();
        duplicate["query"]["metrics"] = json!([
            {"name":"same","field":"stock","numeric_type":"u64"},
            {"name":"same","field":"other","numeric_type":"u64"}
        ]);
        assert!(execute(&snapshot, &ctx, &duplicate).is_err());
    }

    #[test]
    fn grouping_builtin_and_custom_kind_names_does_not_alias() -> Result<()> {
        let (mut snapshot, mut ctx) = fixture();
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or_else(|| invalid("missing fixture entity"))?
            .kind = EntityKind::Other("unit".to_owned());
        snapshot.refresh_hash();
        ctx.anchor = snapshot.anchor();
        let result = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"aggregate","group_by":{"kind":"entity_kind"}})),
        )?;
        let groups = result["groups"]
            .as_array()
            .ok_or_else(|| invalid("missing groups"))?;
        assert_eq!(groups.len(), 3);
        assert!(
            groups
                .iter()
                .any(|group| group["key"]["value"] == "other:unit" && group["count"] == 1)
        );
        assert!(
            groups
                .iter()
                .any(|group| group["key"]["value"] == "unit" && group["count"] == 2)
        );
        Ok(())
    }

    #[test]
    fn search_ranking_pagination_and_resize_are_deterministic() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let mut query = search_query();
        let first = execute(&snapshot, &ctx, &query)?;
        assert_eq!(first["matched"], 3);
        assert_eq!(first["rows"][0]["entity_id"], "1");
        assert_eq!(first["rows"][0]["matched_terms"], 2);
        assert_eq!(first, execute(&snapshot, &ctx, &query)?);
        query["query"]["continuation"] = first["continuation"].clone();
        query["query"]["limit"] = json!(2);
        let next = execute(&snapshot, &ctx, &query)?;
        assert_eq!(next["rows"][0]["entity_id"], "2");
        assert_eq!(next["rows"][1]["entity_id"], "4");
        assert_eq!(next["truncated"], false);
        assert!(next["continuation"].is_null());
        Ok(())
    }

    #[test]
    fn all_terms_and_unicode_lowercase_search_work_without_exposing_hidden_text() -> Result<()> {
        let (mut snapshot, mut ctx) = fixture();
        let mut query = search_query();
        query["query"]["match_mode"] = json!("all");
        let all = execute(&snapshot, &ctx, &query)?;
        assert_eq!(all["matched"], 1);
        assert_eq!(all["rows"][0]["entity_id"], "1");
        query["query"]["text"] = json!("secret");
        assert_eq!(execute(&snapshot, &ctx, &query)?["matched"], 0);
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or_else(|| invalid("missing fixture entity"))?
            .label = "ÄRGER".to_owned();
        snapshot.refresh_hash();
        ctx.anchor = snapshot.anchor();
        let result = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"search","text":"ärger"})),
        )?;
        assert_eq!(result["rows"][0]["label"], "ÄRGER");
        assert_eq!(result["rows"][0]["matched_fields"], json!(["$label"]));
        Ok(())
    }

    #[test]
    fn search_cursor_cannot_cross_query_session_or_same_cursor_fork() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let mut query = search_query();
        query["query"]["continuation"] = execute(&snapshot, &ctx, &query)?["continuation"].clone();
        let mut other = ctx.clone();
        other.session_id = SessionId::new(10);
        assert!(
            matches!(execute(&snapshot,&other,&query),Err(error) if error.code==ErrorCode::StaleAnchor)
        );
        let mut changed = query.clone();
        changed["query"]["text"] = json!("workshop");
        assert!(
            matches!(execute(&snapshot,&ctx,&changed),Err(error) if error.code==ErrorCode::StaleAnchor)
        );
        let mut fork = snapshot.clone();
        fork.paused = false;
        fork.refresh_hash();
        other = ctx.clone();
        other.anchor = fork.anchor();
        assert!(
            matches!(execute(&fork,&other,&query),Err(error) if error.code==ErrorCode::StaleAnchor)
        );
        Ok(())
    }

    #[test]
    fn search_pages_fit_whole_rows_and_never_continue_from_a_terminal_hit() -> Result<()> {
        let (snapshot, mut ctx) = fixture();
        let mut request = search_query();
        let one = execute(&snapshot, &ctx, &request)?;
        ctx.budget.max_bytes = serde_json::to_vec(&one)
            .map_err(|_| invalid("fixture JSON"))?
            .len() as u64;
        request["query"]["limit"] = json!(128);
        let bounded = execute(&snapshot, &ctx, &request)?;
        assert_eq!(bounded["returned"], 1);
        assert!(
            serde_json::to_vec(&bounded)
                .map_err(|_| invalid("fixture JSON"))?
                .len() as u64
                <= ctx.budget.max_bytes
        );
        let identity = identity(&snapshot, &ctx, &request)?;
        request["query"]["continuation"] = json!(cursor(identity, (Reverse(1), EntityId::new(4))));
        assert!(
            matches!(execute(&snapshot,&ctx,&request),Err(error) if error.code==ErrorCode::CursorGap)
        );
        ctx.budget.max_bytes = 1;
        assert!(
            matches!(execute(&snapshot,&ctx,&search_query()),Err(error) if error.code==ErrorCode::BudgetExceeded)
        );
        Ok(())
    }

    #[test]
    fn authorization_cancellation_scope_and_anchor_checks_cover_both_operations() {
        let (snapshot, ctx) = fixture();
        for request in [numeric_query(), search_query()] {
            for change in 0..5 {
                let mut denied = ctx.clone();
                match change {
                    0 => denied.grants.clear(),
                    1 => denied.cancellation_requested = true,
                    2 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
                    3 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
                    _ => denied.anchor.cursor.sequence += 1,
                }
                assert!(execute(&snapshot, &denied, &request).is_err());
            }
            let mut anchored = request.clone();
            anchored["expected_anchor"] = anchor_json(ctx.anchor);
            assert!(execute(&snapshot, &ctx, &anchored).is_ok());
            anchored["expected_anchor"]["game_tick"] = json!(2);
            assert!(
                matches!(execute(&snapshot,&ctx,&anchored),Err(error) if error.code==ErrorCode::StaleAnchor)
            );
        }
    }

    #[test]
    fn malformed_queries_are_rejected_instead_of_silently_narrowed() {
        let (snapshot, ctx) = fixture();
        for query in [
            json!({"kind":"search","text":"   "}),
            json!({"kind":"search","text":"x","limit":0}),
            json!({"kind":"search","text":"x","include_labels":false}),
            json!({"kind":"search","text":"x","unexpected":true}),
            json!({"kind":"search","text":"x","kinds":["unt"]}),
            json!({"kind":"aggregate","max_groups":129}),
            json!({"kind":"aggregate","metrics":[{"name":"x","field":"x","numeric_type":"fixed"}]}),
            json!({"kind":"aggregate","metrics":[{"name":"x","field":"x","numeric_type":"u64","scale":2}]}),
            json!({"kind":"search","text":"x".repeat(513)}),
        ] {
            assert!(execute(&snapshot, &ctx, &envelope(query)).is_err());
        }
        let mut version = search_query();
        version["schema"] = json!("dfmcp.query/99");
        assert!(execute(&snapshot, &ctx, &version).is_err());
    }
}
