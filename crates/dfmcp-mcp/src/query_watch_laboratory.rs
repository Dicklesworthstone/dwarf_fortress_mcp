//! Explicit reference-world evidence for foreground laboratory monitors.
//!
//! This is an internal owner-issued policy, never a JSON trust selector. The
//! same bounded state machine serves native and laboratory monitors, while each
//! retained watch keeps its original source policy and evidence domain. Native
//! checkpoint bytes and native field eligibility remain unchanged.

use super::*;
use dfmcp_core::GameTick;
use dfmcp_world::{Fact, PredicateEvidence, laboratory_fact_value};
use std::hash::{BuildHasher, Hasher};

// Laboratory session IDs and watch serials restart with the process. Binding
// their handles to this private incarnation prevents an old handle from naming
// a newly registered watch. This is not an authority token or a secret grant.
static INCARNATION: LazyLock<(u64, u64)> = LazyLock::new(|| {
    let scope = || {
        let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
        hash.write(b"dfmcp-laboratory-watch-store-v1");
        hash.finish()
    };
    (scope(), scope())
});

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SourcePolicy {
    #[default]
    Native,
    LaboratoryReference,
}

impl SourcePolicy {
    pub(super) fn fact_value<'a>(
        self,
        fact: &'a Fact,
        tick: GameTick,
    ) -> std::result::Result<&'a WorldValue, &'static str> {
        let known = match &fact.presence {
            None => true,
            Some(FactPresence::Known(value)) => value == &fact.value,
            _ => false,
        };
        if !known {
            return Err("field_not_consistently_known");
        }
        match self {
            Self::Native => {
                if !matches!(&fact.source, FactSource::DfhackField(_)) {
                    Err("source_is_not_an_observed_dfhack_field")
                } else if fact.source_digest == Digest32::ZERO {
                    Err("source_digest_not_established")
                } else if fact.observed_at != tick {
                    Err("field_is_not_observed_at_this_game_tick")
                } else {
                    Ok(&fact.value)
                }
            }
            Self::LaboratoryReference => laboratory_fact_value(fact, tick)
                .ok_or("source_is_not_eligible_laboratory_reference_evidence"),
        }
    }

    pub(super) fn validate_snapshot(self, snapshot: &WorldSnapshot) -> Result<()> {
        if self == Self::LaboratoryReference {
            // Only this private path's laboratory owner may supply the complete
            // MemoryAdapter snapshot. The constructor also binds canonical map
            // keys, record identities and the exact hash before field evidence.
            PredicateEvidence::laboratory(snapshot)?;
        }
        Ok(())
    }

    pub(super) fn check_session(self, store: &Store, session: SessionId) -> Result<()> {
        if store
            .entries
            .iter()
            .any(|((owner, _), watch)| *owner == session && watch.source_policy != self)
        {
            return Err(failure(
                ErrorCode::Conflict,
                "watch session belongs to another immutable evidence policy",
            ));
        }
        if self == Self::LaboratoryReference {
            durability::require_unattached(session)?;
        }
        Ok(())
    }

    pub(super) fn validate_definition(self, definition: &Definition) -> Result<()> {
        if self == Self::Native {
            return Ok(());
        }
        let mut pending = vec![&definition.condition];
        if let Some(condition) = &definition.failure_condition {
            pending.push(condition);
        }
        while let Some(condition) = pending.pop() {
            match condition {
                Condition::Field { .. }
                | Condition::Paused { .. }
                | Condition::TickAtLeast { .. } => {}
                Condition::EntityCount { predicate, .. } => validate_population(predicate)?,
                Condition::All { args } | Condition::Any { args } => {
                    pending.extend(args);
                }
                Condition::Not { arg } => pending.push(arg),
                Condition::ShelterSet { .. }
                | Condition::FurnitureSet { .. }
                | Condition::ItemQuantity { .. }
                | Condition::TerrainCount { .. } => {
                    return Err(invalid(
                        "laboratory monitors support fields, projected entity counts, pause, ticks and boolean conditions",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) const fn identity_domain(self) -> &'static str {
        match self {
            Self::Native => "dfmcp-condition-watch-v1",
            Self::LaboratoryReference => "dfmcp-laboratory-condition-watch/1",
        }
    }

    pub(super) fn identity(
        self,
        context: &OperationContext,
        serial: u64,
        definition: &Definition,
    ) -> Result<Digest32> {
        let incarnation = match self {
            Self::Native => (0, 0),
            Self::LaboratoryReference => *INCARNATION,
        };
        self.identity_with_incarnation(context, serial, definition, incarnation)
    }

    // Explicit incarnation input keeps the identity calculation pure and lets
    // tests simulate restarts without replacing a process-global random value.
    fn identity_with_incarnation(
        self,
        context: &OperationContext,
        serial: u64,
        definition: &Definition,
        incarnation: (u64, u64),
    ) -> Result<Digest32> {
        let mut value = json!({"domain":self.identity_domain(),
            "session":context.session_id.to_string(), "serial":serial,
            "anchor":anchor(context.anchor), "definition":definition});
        if self == Self::LaboratoryReference {
            value["incarnation"] = json!(incarnation);
        }
        // Native identity bytes stay exactly as they were, including the lack
        // of an incarnation field, so durable native recovery is unchanged.
        digest(&value)
    }

    pub(super) const fn evidence_domain(self) -> &'static str {
        match self {
            Self::Native => "dfmcp-condition-watch-evidence-v1",
            Self::LaboratoryReference => "dfmcp-laboratory-condition-watch-evidence/1",
        }
    }

    pub(super) const fn inspection_predicate_domain(self) -> &'static str {
        match self {
            Self::Native => "dfmcp-condition-inspection-predicate/1",
            Self::LaboratoryReference => "dfmcp-laboratory-condition-inspection-predicate/1",
        }
    }

    pub(super) const fn inspection_evidence_domain(self) -> &'static str {
        match self {
            Self::Native => "dfmcp-condition-inspection-evidence/1",
            Self::LaboratoryReference => "dfmcp-laboratory-condition-inspection-evidence/1",
        }
    }
}

fn validate_population(predicate: &counts::Predicate) -> Result<()> {
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            counts::Predicate::Always {} | counts::Predicate::Field { .. } => {}
            counts::Predicate::Not { arg } => pending.push(arg),
            counts::Predicate::All { args } | counts::Predicate::Any { args } => {
                pending.extend(args);
            }
            counts::Predicate::Related { .. } => {
                return Err(invalid(
                    "laboratory population monitors do not establish native relationship evidence",
                ));
            }
        }
    }
    Ok(())
}

fn records(store: &Store, context: &OperationContext) -> Result<Value> {
    let records: Vec<_> = store
        .entries
        .iter()
        .filter(|((owner, _), _)| *owner == context.session_id)
        .map(|(_, watch)| watch.summary(context.anchor))
        .collect();
    if records.len() > MAX_PER_SESSION {
        return Err(failure(
            ErrorCode::InternalInvariantViolation,
            "laboratory watch catalog exceeds its session bound",
        ));
    }
    Ok(json!(records))
}

/// The candidate catalog is read under the same lock as the state transition.
/// Its complete response must be accepted before the caller swaps the root.
pub(super) fn publish<F>(
    store: &Store,
    context: &OperationContext,
    mut value: Value,
    publisher: F,
) -> Result<String>
where
    F: FnOnce(Value) -> Result<String>,
{
    value["_query_watch_work"] = records(store, context)?;
    value["evidence_policy"] = json!("laboratory_reference/1");
    value["native_captures"] = json!(0);
    value["mutation_success_proven"] = json!(false);
    publish_plain(context, value, publisher)
}

/// Only metadata is read: no world scan, observation, sample or checkpoint.
/// The owner must still reserve and publish the complete returned catalogs.
pub(crate) fn retained_work(context: &OperationContext) -> Result<Value> {
    retained_work_in(&WATCHES, context)
}

/// Pin an owned original definition before foreground clock progression.
/// This reads retained metadata only and never samples the source world.
pub(crate) fn selected_record(context: &OperationContext, handle: &str) -> Result<Value> {
    let budget = counts::EvaluationBudget::new(context.budget.max_wall_millis);
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_handle(handle)?;
    let store = lock(&WATCHES)?;
    SourcePolicy::LaboratoryReference.check_session(&store, context.session_id)?;
    let watch = store.entries.get(&(context.session_id, handle.to_owned()))
        .ok_or_else(|| invalid("watch is not retained by this session"))?;
    let record = watch.detail(context.anchor);
    let maximum = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4);
    if record.to_string().len() as u64 > maximum {
        return Err(bounded("selected watch definition exceeds its inspection allowance"));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    budget.check()?;
    Ok(record)
}

fn retained_work_in(storage: &Mutex<Store>, context: &OperationContext) -> Result<Value> {
    let budget = counts::EvaluationBudget::new(context.budget.max_wall_millis);
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let store = lock(storage)?;
    SourcePolicy::LaboratoryReference.check_session(&store, context.session_id)?;
    let result = json!({"records":records(&store, context)?,
        "active_work":active_work(&store, context)});
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    budget.check()?;
    Ok(result)
}

/// Evaluate current predicates without installing a handle or counting a sample.
pub(crate) fn condition_query(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    inspection::query_with_policy(snapshot, context, input, SourcePolicy::LaboratoryReference)
}

/// A current-only foreground transaction. It neither refreshes a source nor
/// advances the game; every selected watch shares one exact supplied snapshot.
pub(crate) fn execute<F>(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    publisher: F,
) -> Result<String>
where
    F: FnOnce(Value) -> Result<String>,
{
    execute_in(&WATCHES, snapshot, context, input, publisher)
}

fn execute_in<F>(
    storage: &Mutex<Store>,
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    publisher: F,
) -> Result<String>
where
    F: FnOnce(Value) -> Result<String>,
{
    let budget = counts::EvaluationBudget::new(context.budget.max_wall_millis);
    let publish = |value| {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        budget.check()?;
        let encoded = publisher(value)?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        budget.check()?;
        Ok(encoded)
    };
    match input["query"]["kind"].as_str() {
        Some("watch" | "poll_watch" | "watches" | "cancel_watch" | "release_watch") => {
            super::execute_in_policy(
                storage,
                snapshot,
                context,
                input,
                SourcePolicy::LaboratoryReference,
                publish,
            )
        }
        Some("poll_watches") => {
            let prepared = batch::prepare_in_policy(
                storage,
                snapshot,
                context,
                input,
                SourcePolicy::LaboratoryReference,
                |value| Ok(value.to_string()),
            )?;
            budget.check()?;
            batch::complete_in(storage, snapshot, context, prepared, false, publish)
        }
        _ => Err(invalid(
            "laboratory watch publication requires a supported foreground lifecycle query",
        )),
    }
}

#[cfg(test)]
#[path = "query_watch_laboratory_tests.rs"]
mod tests;
