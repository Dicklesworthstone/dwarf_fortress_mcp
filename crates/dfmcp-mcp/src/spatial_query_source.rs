//! Shared admission before hashing or rendering a complete structured-query source.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, OperationContext, Result};
use dfmcp_world::{EntityRecord, Fact, FactPresence, FactSource, Value, WorldSnapshot};

const MAX_WORK: usize = 4_000_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;

pub(super) struct Budget {
    pub(super) bytes: usize,
    pub(super) work: usize,
    maximum_bytes: usize,
    started: Instant,
    wall: Duration,
}

impl Budget {
    pub(super) fn check(&self) -> Result<()> {
        if self.bytes > self.maximum_bytes
            || self.work > MAX_WORK
            || self.started.elapsed() >= self.wall
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "structured query exceeds its source byte, work, or wall-time allowance",
            ));
        }
        Ok(())
    }

    pub(super) fn charge(&mut self, bytes: usize, work: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        self.work = self.work.saturating_add(work);
        self.check()
    }

    fn value(&mut self, root: &Value) -> Result<()> {
        let mut pending = vec![(root, 0usize)];
        while let Some((value, depth)) = pending.pop() {
            if depth > 64 {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "structured-query source value nesting exceeds 64",
                ));
            }
            self.charge(32, 1)?;
            let children = match value {
                Value::List(values) => values.len(),
                Value::Object(values) => values.len(),
                _ => 0,
            };
            // Admit frontier shape before allocating it, including unselected
            // fields and contradictory compatibility representations.
            if self
                .work
                .saturating_add(pending.len())
                .saturating_add(children)
                > MAX_WORK
                || self
                    .bytes
                    .saturating_add(pending.len().saturating_add(children).saturating_mul(32))
                    > self.maximum_bytes
            {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "structured-query source value frontier exceeds its bound",
                ));
            }
            match value {
                Value::Text(text) => self.charge(text.len(), 0)?,
                Value::Bytes(bytes) => self.charge(bytes.len(), 0)?,
                Value::List(values) => {
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                }
                Value::Object(values) => {
                    for (key, value) in values {
                        self.charge(key.len(), 0)?;
                        pending.push((value, depth + 1));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn facts(&mut self, facts: &BTreeMap<String, Fact>) -> Result<()> {
        for (name, fact) in facts {
            self.charge(256usize.saturating_add(name.len()), 1)?;
            match &fact.source {
                FactSource::DfhackField(text)
                | FactSource::Derived(text)
                | FactSource::AgentAssertion(text) => self.charge(text.len(), 0)?,
                FactSource::Replay => {}
            }
            self.value(&fact.value)?;
            match &fact.presence {
                Some(FactPresence::Known(value)) => self.value(value)?,
                Some(
                    FactPresence::Unknown(text)
                    | FactPresence::Unsupported(text)
                    | FactPresence::Omitted(text)
                    | FactPresence::Redacted(text),
                ) => self.charge(text.len(), 0)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn values(&mut self, values: &BTreeMap<String, Value>) -> Result<()> {
        for (name, value) in values {
            self.charge(32usize.saturating_add(name.len()), 1)?;
            self.value(value)?;
        }
        Ok(())
    }
}

/// A lower bound avoids constructing a large typed JSON tree for a row that
/// cannot fit. Final serialization still decides the exact whole-row limit.
pub(super) fn render_fits(
    entity: &EntityRecord,
    fields: &[String],
    geometry_fields: &[String],
    maximum: usize,
    budget: &mut Budget,
) -> Result<bool> {
    let mut minimum = entity
        .label
        .len()
        .saturating_add(entity.kind.as_str().len());
    for name in fields.iter().chain(geometry_fields) {
        minimum = minimum.saturating_add(name.len());
        if let Some(fact) = entity.fields.get(name) {
            let source = match &fact.source {
                FactSource::DfhackField(text)
                | FactSource::Derived(text)
                | FactSource::AgentAssertion(text) => text.len(),
                FactSource::Replay => 0,
            };
            minimum = minimum.saturating_add(source);
            if let Some(
                FactPresence::Unknown(reason)
                | FactPresence::Unsupported(reason)
                | FactPresence::Omitted(reason)
                | FactPresence::Redacted(reason),
            ) = &fact.presence
            {
                minimum = minimum.saturating_add(reason.len());
            }
            let mut pending = fact.known_value().into_iter().collect::<Vec<_>>();
            while let Some(value) = pending.pop() {
                budget.charge(0, 1)?;
                // Even typed null occupies fifteen JSON bytes. Frontier
                // admission prevents an oversized list allocation as well.
                minimum = minimum.saturating_add(15);
                match value {
                    Value::Text(text) => minimum = minimum.saturating_add(text.len()),
                    Value::Bytes(bytes) => minimum = minimum.saturating_add(bytes.len()),
                    Value::List(values) => {
                        if minimum.saturating_add(values.len().saturating_mul(15)) > maximum {
                            return Ok(false);
                        }
                        pending.extend(values);
                    }
                    Value::Object(values) => {
                        if minimum.saturating_add(values.len().saturating_mul(15)) > maximum {
                            return Ok(false);
                        }
                        for (key, value) in values {
                            minimum = minimum.saturating_add(key.len());
                            pending.push(value);
                        }
                    }
                    _ => {}
                }
                if minimum > maximum {
                    return Ok(false);
                }
            }
        }
        if minimum > maximum {
            return Ok(false);
        }
    }
    Ok(minimum <= maximum)
}

/// A narrow region or a one-row limit cannot bypass complete-source custody.
pub(super) fn preflight(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    started: Instant,
) -> Result<Budget> {
    let records = snapshot
        .graph
        .entities
        .len()
        .saturating_add(snapshot.graph.edges.len())
        .saturating_add(snapshot.graph.chunks.len())
        .saturating_add(snapshot.graph.events.len());
    if records > (context.budget.max_entities as usize).min(MAX_RECORDS) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "structured query exceeds its complete-source record allowance",
        ));
    }
    let maximum_bytes =
        usize::try_from(context.budget.max_bytes.min(MAX_BYTES as u64)).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "structured-query source byte allowance cannot be represented",
            )
        })?;
    let mut budget = Budget {
        bytes: 256,
        work: 0,
        maximum_bytes,
        started,
        wall: Duration::from_millis(context.budget.max_wall_millis),
    };
    budget.check()?;
    for (id, entity) in &snapshot.graph.entities {
        if *id == EntityId::NIL || *id != entity.id {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "structured-query source entity identity is inconsistent",
            ));
        }
        budget.charge(
            128usize
                .saturating_add(entity.label.len())
                .saturating_add(entity.kind.as_str().len()),
            1,
        )?;
        budget.facts(&entity.fields)?;
    }
    for edge in snapshot.graph.edges.values() {
        budget.charge(128usize.saturating_add(edge.kind.as_str().len()), 1)?;
        budget.facts(&edge.fields)?;
    }
    for chunk in snapshot.graph.chunks.values() {
        budget.charge(
            128usize.saturating_add(chunk.terrain_runs.len().saturating_mul(16)),
            chunk.terrain_runs.len().saturating_add(1),
        )?;
        for values in chunk.sparse_overlays.values() {
            budget.charge(32, 1)?;
            budget.values(values)?;
        }
    }
    for event in snapshot.graph.events.values() {
        budget.charge(
            128usize
                .saturating_add(event.summary.len())
                .saturating_add(event.kind.as_str().len()),
            1,
        )?;
        budget.values(&event.fields)?;
    }
    Ok(budget)
}
