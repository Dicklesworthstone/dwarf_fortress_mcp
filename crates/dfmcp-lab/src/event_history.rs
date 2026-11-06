//! Opt-in, bounded canonical observations of laboratory publication boundaries.
//!
//! This is an explicit reference model, not a native event capture source or a
//! replay of the adapter's control transcript. Existing worlds without its
//! marker retain their original semantics. The marker, counters and immutable
//! retained events travel together in ordinary canonical snapshot objects.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CheckpointId, DfmcpError, Digest32, EntityId, ErrorCode, EventId, GameTick, Result, StateAnchor,
};
use dfmcp_intent::{Action, effects};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value, WorldEvent, WorldEventKind,
    WorldSnapshot,
};

pub const MODEL: &str = "dfmcp.lab-event-history/1";
pub const MODEL_FIELD: &str = "laboratory_event_history_model";
pub const MAX_RETAINED_EVENTS: usize = 128;
pub const MAX_ENTITY_EVENTS_PER_PUBLICATION: usize = 32;
pub const MAX_CHANGED_FIELDS: usize = 8;
pub const MAX_CAPTURED_VALUE_BYTES: usize = 256;
const PREFIX: &str = "laboratory_event_history_";
const TOTAL: &str = "laboratory_event_history_total_events";
const EVICTED: &str = "laboratory_event_history_evicted_events";
const OMITTED_ENTITIES: &str = "laboratory_event_history_omitted_entity_changes";
const OMITTED_FIELDS: &str = "laboratory_event_history_omitted_fields";
const OMITTED_VALUES: &str = "laboratory_event_history_omitted_values";
const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_SCAN_RECORDS: usize = 100_000;
const MAX_SCAN_FIELDS: usize = 200_000;

/// Counters describe this restored branch's retained publication observations.
/// They do not establish complete game history or the authenticity of an
/// imported snapshot that happens to contain the model's marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCoverage {
    pub retained_events: usize,
    pub total_events: u64,
    pub evicted_events: u64,
    pub omitted_entity_changes: u64,
    pub omitted_fields: u64,
    pub omitted_values: u64,
    pub oldest_ordinal: Option<u64>,
    pub newest_ordinal: Option<u64>,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message).retryable(false)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn fact(value: Value, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived(MODEL.to_owned()),
        Digest32::ZERO,
    )
}

fn model_value<'a>(entity: &'a EntityRecord, name: &str, tick: GameTick) -> Result<&'a Value> {
    let value = entity
        .fields
        .get(name)
        .filter(|fact| {
            fact.observed_at <= tick
                && fact.source == FactSource::Derived(MODEL.to_owned())
                && fact.source_digest == Digest32::ZERO
        })
        .and_then(Fact::known_value)
        .ok_or_else(|| {
            invalid("laboratory event history has unavailable or unrecognized metadata")
        })?;
    Ok(value)
}

fn counter(entity: &EntityRecord, name: &str, tick: GameTick) -> Result<u64> {
    match model_value(entity, name, tick)? {
        Value::U64(value) => Ok(*value),
        _ => Err(invalid(
            "laboratory event history counter is not an unsigned integer",
        )),
    }
}

fn value_number(fields: &BTreeMap<String, Value>, name: &str) -> Result<u64> {
    match fields.get(name) {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid(
            "laboratory history event is missing its canonical identity fields",
        )),
    }
}

/// Preflight before cloning values or recursively invoking the canonical codec.
fn value_fits(value: &Value, maximum: usize, max_nodes: usize, max_depth: usize) -> bool {
    let mut pending = vec![(value, 0usize)];
    let (mut bytes, mut nodes) = (0usize, 0usize);
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.saturating_add(1);
        bytes = bytes.saturating_add(24);
        if nodes > max_nodes || depth > max_depth || bytes > maximum {
            return false;
        }
        match value {
            Value::Text(value) => bytes = bytes.saturating_add(value.len()),
            Value::Bytes(value) => bytes = bytes.saturating_add(value.len()),
            Value::List(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > max_nodes
                {
                    return false;
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > max_nodes
                {
                    return false;
                }
                for (name, value) in values {
                    bytes = bytes.saturating_add(name.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > maximum {
            return false;
        }
    }
    true
}

fn event_id(event: &WorldEvent) -> Result<EventId> {
    if event.summary.len() > 512 || event.kind.as_str().len() > 128 || event.fields.len() > 32 {
        return Err(invalid("laboratory event exceeds its header bound"));
    }
    // All producer-created records have a small fixed field count. Validate
    // imported records before copying their potentially nested values.
    let mut remaining = MAX_EVENT_BYTES;
    for (name, value) in &event.fields {
        if name.len() > 128 || !value_fits(value, remaining, 512, 12) {
            return Err(invalid("laboratory event exceeds its bounded value shape"));
        }
        let bytes = fact(value.clone(), event.tick).canonical_bytes().len();
        remaining = remaining
            .checked_sub(bytes.saturating_add(name.len()))
            .ok_or_else(|| invalid("laboratory event exceeds its canonical byte bound"))?;
    }
    let mut bytes = b"dfmcp-lab-event-identity/1\0".to_vec();
    bytes.extend_from_slice(&event.tick.0.to_be_bytes());
    bytes.extend_from_slice(&(event.kind.as_str().len() as u64).to_be_bytes());
    bytes.extend_from_slice(event.kind.as_str().as_bytes());
    bytes.extend_from_slice(&event.subject.map_or(0, EntityId::get).to_be_bytes());
    bytes.extend_from_slice(&(event.summary.len() as u64).to_be_bytes());
    bytes.extend_from_slice(event.summary.as_bytes());
    bytes.extend_from_slice(
        &fact(Value::Object(event.fields.clone()), event.tick).canonical_bytes(),
    );
    let id = EventId::new(Digest32::of_bytes(&bytes).first_u128());
    if id.get() == 0 {
        return Err(invalid("laboratory event identity is reserved"));
    }
    Ok(id)
}

/// Validate the exact model, retained identities, counters and bounded shape.
/// Absence of the marker means that this producer does not own the history.
pub fn coverage(snapshot: &WorldSnapshot) -> Result<Option<HistoryCoverage>> {
    let Some(settings) = snapshot
        .graph
        .entities
        .get(&effects::fortress_settings_entity_id(snapshot.fortress_id))
        .filter(|entity| entity.fields.contains_key(MODEL_FIELD))
    else {
        return Ok(None);
    };
    if settings.id != effects::fortress_settings_entity_id(snapshot.fortress_id)
        || settings.kind != EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned())
        || settings.generation == 0
        || settings.revision == 0
        || model_value(settings, MODEL_FIELD, snapshot.tick)? != &Value::Text(MODEL.to_owned())
    {
        return Err(invalid(
            "laboratory event history has an unsupported model or settings identity",
        ));
    }
    let total_events = counter(settings, TOTAL, snapshot.tick)?;
    let evicted_events = counter(settings, EVICTED, snapshot.tick)?;
    let retained_events = snapshot.graph.events.len();
    if retained_events > MAX_RETAINED_EVENTS
        || evicted_events.checked_add(retained_events as u64) != Some(total_events)
    {
        return Err(invalid(
            "laboratory event retention counters disagree with the retained source",
        ));
    }
    let mut ordinals = BTreeSet::new();
    for (id, event) in &snapshot.graph.events {
        let ordinal = value_number(&event.fields, "ordinal")?;
        let epoch = value_number(&event.fields, "publication_epoch")?;
        let sequence = value_number(&event.fields, "publication_sequence")?;
        let content_digest_valid = matches!(
            event.fields.get("publication_content_digest"),
            Some(Value::Text(value)) if Digest32::from_hex(value)
                .is_some_and(|digest| digest.to_hex() == *value)
        );
        if *id != event.id
            || event.tick > snapshot.tick
            || event.fields.get("producer_model") != Some(&Value::Text(MODEL.to_owned()))
            || value_number(&event.fields, "fortress_id")? != snapshot.fortress_id.get()
            || !content_digest_valid
            || epoch > snapshot.cursor.epoch
            || (epoch == snapshot.cursor.epoch && sequence > snapshot.cursor.sequence)
            || ordinal <= evicted_events
            || ordinal > total_events
            || !ordinals.insert(ordinal)
            || event_id(event)? != *id
        {
            return Err(invalid(
                "laboratory event history has a conflicting retained identity",
            ));
        }
    }
    Ok(Some(HistoryCoverage {
        retained_events,
        total_events,
        evicted_events,
        omitted_entity_changes: counter(settings, OMITTED_ENTITIES, snapshot.tick)?,
        omitted_fields: counter(settings, OMITTED_FIELDS, snapshot.tick)?,
        omitted_values: counter(settings, OMITTED_VALUES, snapshot.tick)?,
        oldest_ordinal: ordinals.first().copied(),
        newest_ordinal: ordinals.last().copied(),
    }))
}

/// Enable a new explicit scenario. An existing event source is never silently
/// migrated or pruned into this model. Failure leaves the supplied world intact.
pub fn enable(snapshot: &mut WorldSnapshot) -> Result<()> {
    if !snapshot.hash_is_valid() || !snapshot.graph.events.is_empty() {
        return Err(invalid(
            "event history initialization requires a sealed world without existing events",
        ));
    }
    let mut next = snapshot.clone();
    let id = effects::fortress_settings_entity_id(next.fortress_id);
    let settings = next
        .graph
        .entities
        .entry(id)
        .or_insert_with(|| EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind: EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
            label: "Laboratory fortress settings".to_owned(),
            fields: BTreeMap::new(),
        });
    if settings.kind != EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned())
        || settings.id != id
        || settings.fields.keys().any(|name| name.starts_with(PREFIX))
    {
        return Err(invalid(
            "event history initialization conflicts with existing settings",
        ));
    }
    settings.fields.insert(
        MODEL_FIELD.to_owned(),
        fact(Value::Text(MODEL.to_owned()), next.tick),
    );
    for name in [
        TOTAL,
        EVICTED,
        OMITTED_ENTITIES,
        OMITTED_FIELDS,
        OMITTED_VALUES,
    ] {
        settings
            .fields
            .insert(name.to_owned(), fact(Value::U64(0), next.tick));
    }
    let state =
        coverage(&next)?.ok_or_else(|| invalid("event history model was not initialized"))?;
    let event = notice(
        &next,
        snapshot.anchor(),
        "initialized",
        BTreeMap::from([(
            "publication_content_digest".to_owned(),
            Value::Text(next.compute_hash().to_hex()),
        )]),
    );
    append(&mut next, state, vec![event], Omissions::default())?;
    next.refresh_hash();
    *snapshot = next;
    Ok(())
}

fn anchor_value(anchor: StateAnchor) -> Value {
    Value::Object(BTreeMap::from([
        (
            "fortress_id".to_owned(),
            Value::U64(anchor.fortress_id.get()),
        ),
        ("epoch".to_owned(), Value::U64(anchor.cursor.epoch)),
        ("sequence".to_owned(), Value::U64(anchor.cursor.sequence)),
        ("tick".to_owned(), Value::U64(anchor.tick.0)),
        (
            "state_hash".to_owned(),
            Value::Text(anchor.state_hash.to_hex()),
        ),
    ]))
}

fn common_fields(snapshot: &WorldSnapshot, prior: StateAnchor) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("producer_model".to_owned(), Value::Text(MODEL.to_owned())),
        (
            "fortress_id".to_owned(),
            Value::U64(snapshot.fortress_id.get()),
        ),
        (
            "publication_epoch".to_owned(),
            Value::U64(snapshot.cursor.epoch),
        ),
        (
            "publication_sequence".to_owned(),
            Value::U64(snapshot.cursor.sequence),
        ),
        ("predecessor_anchor".to_owned(), anchor_value(prior)),
    ])
}

fn notice(
    snapshot: &WorldSnapshot,
    prior: StateAnchor,
    transition: &str,
    extra: BTreeMap<String, Value>,
) -> WorldEvent {
    let mut fields = common_fields(snapshot, prior);
    fields.insert("transition".to_owned(), Value::Text(transition.to_owned()));
    fields.extend(extra);
    WorldEvent {
        id: EventId::new(0),
        tick: snapshot.tick,
        kind: WorldEventKind::AdapterNotice,
        subject: None,
        summary: format!("Laboratory publication: {transition}"),
        fields,
    }
}

#[derive(Default)]
struct Omissions {
    entities: u64,
    fields: u64,
    values: u64,
}

fn add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| exhausted("laboratory event history counter exhausted"))
}

fn append(
    snapshot: &mut WorldSnapshot,
    mut state: HistoryCoverage,
    events: Vec<WorldEvent>,
    omissions: Omissions,
) -> Result<()> {
    for mut event in events {
        state.total_events = add(state.total_events, 1)?;
        event
            .fields
            .insert("ordinal".to_owned(), Value::U64(state.total_events));
        event.id = event_id(&event)?;
        if snapshot.graph.events.contains_key(&event.id) {
            return Err(invalid(
                "laboratory event identity collision refuses publication",
            ));
        }
        snapshot.graph.events.insert(event.id, event);
    }
    let mut ordered = snapshot
        .graph
        .events
        .values()
        .map(|event| Ok((value_number(&event.fields, "ordinal")?, event.id)))
        .collect::<Result<Vec<_>>>()?;
    ordered.sort_unstable();
    for (_, id) in ordered
        .iter()
        .take(ordered.len().saturating_sub(MAX_RETAINED_EVENTS))
    {
        snapshot.graph.events.remove(id);
        state.evicted_events = add(state.evicted_events, 1)?;
    }
    let settings = snapshot
        .graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(snapshot.fortress_id))
        .ok_or_else(|| invalid("laboratory history settings disappeared during publication"))?;
    settings.revision = settings
        .revision
        .checked_add(1)
        .ok_or_else(|| exhausted("laboratory history settings revision exhausted"))?;
    for (name, value) in [
        (TOTAL, state.total_events),
        (EVICTED, state.evicted_events),
        (
            OMITTED_ENTITIES,
            add(state.omitted_entity_changes, omissions.entities)?,
        ),
        (OMITTED_FIELDS, add(state.omitted_fields, omissions.fields)?),
        (OMITTED_VALUES, add(state.omitted_values, omissions.values)?),
    ] {
        settings
            .fields
            .insert(name.to_owned(), fact(Value::U64(value), snapshot.tick));
    }
    Ok(())
}

pub(crate) enum Publication<'a> {
    Action { action: &'a Action, key: &'a str },
    CancelWork { action: &'a Action, key: &'a str },
    AdvanceTicks { amount: u64 },
}

impl Publication<'_> {
    fn name(&self) -> &'static str {
        match self {
            Self::Action { .. } => "action",
            Self::CancelWork { .. } => "cancel_work",
            Self::AdvanceTicks { .. } => "advance_ticks",
        }
    }

    fn fields(&self) -> BTreeMap<String, Value> {
        match self {
            Self::Action { action, key } | Self::CancelWork { action, key } => BTreeMap::from([
                (
                    "action_digest".to_owned(),
                    Value::Text(Digest32::of_bytes(&action.canonical_bytes()).to_hex()),
                ),
                (
                    "effect_key_digest".to_owned(),
                    Value::Text(Digest32::of_bytes(key.as_bytes()).to_hex()),
                ),
            ]),
            Self::AdvanceTicks { amount } => {
                BTreeMap::from([("advanced_ticks".to_owned(), Value::U64(*amount))])
            }
        }
    }
}

fn presence(fact: Option<&Fact>, tick: GameTick) -> &'static str {
    let Some(fact) = fact else { return "unknown" };
    if fact.observed_at > tick {
        return "future_observation";
    }
    if fact.known_value().is_some() {
        return "known";
    }
    match fact.presence.as_ref() {
        Some(FactPresence::Absent) => "absent",
        Some(FactPresence::Unknown(_)) => "unknown",
        Some(FactPresence::Unsupported(_)) => "unsupported",
        Some(FactPresence::Omitted(_)) => "omitted",
        Some(FactPresence::Redacted(_)) => "redacted",
        Some(FactPresence::Stale(_)) => "stale",
        Some(FactPresence::Known(_)) | None => "inconsistent_known",
    }
}

fn same_visible_fact(
    a: Option<&Fact>,
    b: Option<&Fact>,
    before: GameTick,
    after: GameTick,
) -> bool {
    if presence(a, before) != presence(b, after) {
        return false;
    }
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.known_value() == b.known_value()
                && a.source == b.source
                && a.source_digest == b.source_digest
                && match (&a.presence, &b.presence) {
                    (Some(FactPresence::Stale(a)), Some(FactPresence::Stale(b))) => a == b,
                    _ => true,
                }
        }
        _ => false,
    }
}

fn field_state(input: Option<&Fact>, tick: GameTick, omissions: &mut Omissions) -> Value {
    let state = presence(input, tick);
    let mut fields = BTreeMap::from([("presence".to_owned(), Value::Text(state.to_owned()))]);
    let Some(input) = input else {
        fields.insert(
            "reason".to_owned(),
            Value::Text("field_not_in_retained_record".to_owned()),
        );
        return Value::Object(fields);
    };
    fields.insert("observed_at".to_owned(), Value::U64(input.observed_at.0));
    fields.insert(
        "source_digest".to_owned(),
        Value::Text(input.source_digest.to_hex()),
    );
    let (kind, label) = match &input.source {
        FactSource::DfhackField(label) => ("dfhack_field", Some(label)),
        FactSource::Derived(label) => ("derived", Some(label)),
        FactSource::AgentAssertion(label) => ("agent_assertion", Some(label)),
        FactSource::Replay => ("replay", None),
    };
    fields.insert("source_kind".to_owned(), Value::Text(kind.to_owned()));
    if let Some(label) = label {
        if label.len() <= 128 {
            fields.insert("source_label".to_owned(), Value::Text(label.clone()));
        } else {
            fields.insert("source_label_omitted".to_owned(), Value::Bool(true));
        }
    }
    if state == "known"
        && let Some(value) = input.known_value()
    {
        let included = value_fits(value, MAX_CAPTURED_VALUE_BYTES, 32, 4);
        fields.insert("value_included".to_owned(), Value::Bool(included));
        if included {
            fields.insert("value".to_owned(), value.clone());
        } else {
            omissions.values = omissions.values.saturating_add(1);
        }
    }
    if let Some(FactPresence::Stale(anchor)) = input.presence.as_ref() {
        fields.insert("last_anchor".to_owned(), anchor_value(*anchor));
    }
    Value::Object(fields)
}

fn kind(entity: &EntityRecord) -> WorldEventKind {
    match entity.kind {
        EntityKind::Job | EntityKind::WorkOrder => WorldEventKind::JobChanged,
        EntityKind::Unit | EntityKind::Creature => WorldEventKind::UnitChanged,
        EntityKind::Building => WorldEventKind::ConstructionChanged,
        EntityKind::Other(ref name) if name == effects::DIG_DESIGNATION_KIND => {
            WorldEventKind::ConstructionChanged
        }
        _ => WorldEventKind::Other("laboratory_entity_changed".to_owned()),
    }
}

/// An endpoint comparison deliberately ignores observation-time-only refreshes.
/// Missing fields remain unknown; unavailable compatibility values never escape.
fn entity_event(
    before: &WorldSnapshot,
    after: &WorldSnapshot,
    id: EntityId,
    scan_fields: &mut usize,
    capture: bool,
    omissions: &mut Omissions,
) -> Result<Option<WorldEvent>> {
    let a = before.graph.entities.get(&id);
    let b = after.graph.entities.get(&id);
    let Some(entity) = b.or(a) else {
        return Ok(None);
    };
    let empty = BTreeMap::new();
    let before_fields = a.map_or(&empty, |entity| &entity.fields);
    let after_fields = b.map_or(&empty, |entity| &entity.fields);
    *scan_fields = scan_fields
        .saturating_add(before_fields.len())
        .saturating_add(after_fields.len());
    if *scan_fields > MAX_SCAN_FIELDS {
        return Err(exhausted(
            "laboratory event comparison exceeds its field-scan bound",
        ));
    }
    let names: BTreeSet<_> = before_fields.keys().chain(after_fields.keys()).collect();
    let settings = effects::fortress_settings_entity_id(after.fortress_id);
    let mut changed = 0u64;
    let mut changes = Vec::new();
    for name in names {
        if (id == settings && name.starts_with(PREFIX))
            || same_visible_fact(
                before_fields.get(name),
                after_fields.get(name),
                before.tick,
                after.tick,
            )
        {
            continue;
        }
        changed += 1;
        if capture && changes.len() < MAX_CHANGED_FIELDS && name.len() <= 128 {
            changes.push(Value::Object(BTreeMap::from([
                ("field".to_owned(), Value::Text(name.clone())),
                (
                    "before".to_owned(),
                    field_state(before_fields.get(name), before.tick, omissions),
                ),
                (
                    "after".to_owned(),
                    field_state(after_fields.get(name), after.tick, omissions),
                ),
            ])));
        }
    }
    let identity_changed = match (a, b) {
        (Some(a), Some(b)) => {
            a.generation != b.generation || a.kind != b.kind || a.label != b.label
        }
        _ => true,
    };
    if changed == 0 && !identity_changed {
        return Ok(None);
    }
    if !capture {
        omissions.entities = add(omissions.entities, 1)?;
        return Ok(None);
    }
    let omitted_fields = changed.saturating_sub(changes.len() as u64);
    omissions.fields = add(omissions.fields, omitted_fields)?;
    let transition = if a.is_none() {
        "created"
    } else if b.is_none() {
        "removed"
    } else {
        "updated"
    };
    let mut fields = common_fields(after, before.anchor());
    fields.extend([
        ("change".to_owned(), Value::Text(transition.to_owned())),
        (
            "generation_before".to_owned(),
            a.map_or(Value::Null, |entity| {
                Value::U64(u64::from(entity.generation))
            }),
        ),
        (
            "generation_after".to_owned(),
            b.map_or(Value::Null, |entity| {
                Value::U64(u64::from(entity.generation))
            }),
        ),
        (
            "revision_before".to_owned(),
            a.map_or(Value::Null, |entity| Value::U64(entity.revision)),
        ),
        (
            "revision_after".to_owned(),
            b.map_or(Value::Null, |entity| Value::U64(entity.revision)),
        ),
        ("changed_field_count".to_owned(), Value::U64(changed)),
        ("omitted_fields".to_owned(), Value::U64(omitted_fields)),
        ("changes".to_owned(), Value::List(changes)),
        (
            "label_changed".to_owned(),
            Value::Bool(a.map(|e| &e.label) != b.map(|e| &e.label)),
        ),
    ]);
    if entity.kind.as_str().len() <= 128 {
        fields.insert(
            "entity_kind".to_owned(),
            Value::Text(entity.kind.as_str().to_owned()),
        );
    }
    Ok(Some(WorldEvent {
        id: EventId::new(0),
        tick: after.tick,
        kind: kind(entity),
        subject: Some(id),
        summary: format!("Laboratory entity {id} {transition}"),
        fields,
    }))
}

fn changed_records<K: Ord, V: PartialEq>(a: &BTreeMap<K, V>, b: &BTreeMap<K, V>) -> u64 {
    a.iter()
        .filter(|(key, value)| b.get(key) != Some(value))
        .count() as u64
        + b.keys().filter(|key| !a.contains_key(key)).count() as u64
}

/// Called on the unpublished shadow, after effects and cursor allocation and
/// before its final hash. The caller publishes nothing if this returns an error.
pub(crate) fn record_publication(
    before: &WorldSnapshot,
    after: &mut WorldSnapshot,
    publication: Publication<'_>,
) -> Result<()> {
    let Some(state) = coverage(before)? else {
        return Ok(());
    };
    if !before.hash_is_valid()
        || before.fortress_id != after.fortress_id
        || before.cursor.checked_next() != Some(after.cursor)
        || before.tick > after.tick
        || coverage(after)?.as_ref() != Some(&state)
        || before.graph.events != after.graph.events
    {
        return Err(invalid(
            "laboratory event publication does not extend the exact retained source",
        ));
    }
    let record_count = [before, &*after]
        .into_iter()
        .fold(0usize, |count, snapshot| {
            count
                .saturating_add(snapshot.graph.entities.len())
                .saturating_add(snapshot.graph.edges.len())
                .saturating_add(snapshot.graph.chunks.len())
        });
    if record_count > MAX_SCAN_RECORDS {
        return Err(exhausted(
            "laboratory event comparison exceeds its record-scan bound",
        ));
    }
    // This hashes the complete effect-applied shadow with the previously
    // retained history, before any new history/counter append. Naming that
    // stage explicitly avoids a self-referential final canonical hash and
    // distinguishes equal-count transitions on different hypothetical forks.
    let content_digest = after.compute_hash();
    let ids: BTreeSet<_> = before
        .graph
        .entities
        .keys()
        .chain(after.graph.entities.keys())
        .copied()
        .collect();
    let mut events = Vec::new();
    let mut omissions = Omissions::default();
    let mut scan_fields = 0usize;
    for id in ids {
        if let Some(event) = entity_event(
            before,
            after,
            id,
            &mut scan_fields,
            events.len() < MAX_ENTITY_EVENTS_PER_PUBLICATION,
            &mut omissions,
        )? {
            let mut event = event;
            event.fields.insert(
                "publication_content_digest".to_owned(),
                Value::Text(content_digest.to_hex()),
            );
            events.push(event);
        }
    }
    let mut fields = publication.fields();
    fields.extend([
        (
            "publication_content_digest".to_owned(),
            Value::Text(content_digest.to_hex()),
        ),
        (
            "entity_changes_recorded".to_owned(),
            Value::U64(events.len() as u64),
        ),
        (
            "entity_changes_omitted".to_owned(),
            Value::U64(omissions.entities),
        ),
        ("fields_omitted".to_owned(), Value::U64(omissions.fields)),
        ("values_omitted".to_owned(), Value::U64(omissions.values)),
        (
            "changed_edge_records".to_owned(),
            Value::U64(changed_records(&before.graph.edges, &after.graph.edges)),
        ),
        (
            "changed_chunk_records".to_owned(),
            Value::U64(changed_records(&before.graph.chunks, &after.graph.chunks)),
        ),
        ("paused_before".to_owned(), Value::Bool(before.paused)),
        ("paused_after".to_owned(), Value::Bool(after.paused)),
        (
            "intermediate_states_observed".to_owned(),
            Value::Bool(false),
        ),
    ]);
    events.push(notice(after, before.anchor(), publication.name(), fields));
    append(after, state, events, omissions)
}

pub(crate) enum Boundary {
    Recovery {
        from: StateAnchor,
    },
    Restore {
        prior: StateAnchor,
        checkpoint: CheckpointId,
        checkpoint_anchor: StateAnchor,
    },
}

/// Restore retains the checkpoint's branch, then records the discontinuity.
/// Recovery retains the durable head. Neither imports abandoned future events.
pub(crate) fn record_boundary(snapshot: &mut WorldSnapshot, boundary: Boundary) -> Result<()> {
    let Some(state) = coverage(snapshot)? else {
        return Ok(());
    };
    let (prior, source, name, mut fields) = match boundary {
        Boundary::Recovery { from } => (from, from, "recovery", BTreeMap::new()),
        Boundary::Restore {
            prior,
            checkpoint,
            checkpoint_anchor,
        } => (
            prior,
            checkpoint_anchor,
            "restore",
            BTreeMap::from([(
                "checkpoint_id".to_owned(),
                Value::Text(checkpoint.to_string()),
            )]),
        ),
    };
    if prior.fortress_id != snapshot.fortress_id
        || source.fortress_id != snapshot.fortress_id
        || prior.cursor.checked_reset_epoch() != Some(snapshot.cursor)
        || source.tick != snapshot.tick
    {
        return Err(invalid(
            "laboratory event reset boundary has a different lineage",
        ));
    }
    fields.insert("retained_history_anchor".to_owned(), anchor_value(source));
    fields.insert(
        "publication_content_digest".to_owned(),
        Value::Text(snapshot.compute_hash().to_hex()),
    );
    fields.insert("continuous_from_predecessor".to_owned(), Value::Bool(false));
    let event = notice(snapshot, prior, name, fields);
    append(snapshot, state, vec![event], Omissions::default())
}

#[cfg(test)]
#[path = "event_history_tests.rs"]
mod tests;
