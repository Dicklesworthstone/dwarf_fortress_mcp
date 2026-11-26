//! Immutable inventory ownership for material-backed carpentry and furniture.
//! The default ledger chooses a new owner only at dispatch. Retained work never
//! selects another owner from a later census, including after an ABA replacement.

use super::*;

pub const INVENTORY_LEDGER_FIELD: &str = "inventory_ledger";
pub const INVENTORY_LEDGER_GENERATION_FIELD: &str = "inventory_ledger_generation";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LedgerBinding {
    pub(super) id: EntityId,
    generation: u32,
}

impl LedgerBinding {
    /// Preserve the ordinary lowest-ID selection for new work, while requiring
    /// an actual canonical identity before recording its immutable ownership.
    pub(super) fn capture(snapshot: &WorldSnapshot) -> Result<Self> {
        if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
            return Err(advance_budget_error(
                "inventory custody exceeds the bounded canonical entity domain",
            ));
        }
        let (id, record) = snapshot
            .graph
            .entities
            .iter()
            .find(|(_, record)| record.kind.as_str() == STOCK_LEDGER_KIND)
            .ok_or_else(|| precondition("inventory stock ledger is not established"))?;
        let binding = Self {
            id: *id,
            generation: record.generation,
        };
        binding.ledger(snapshot)?;
        Ok(binding)
    }

    /// Both absent fields describe unbound historical state. A partial,
    /// unavailable or malformed pair is never equivalent to that absence.
    pub(super) fn read(record: &EntityRecord, tick: GameTick) -> Result<Option<Self>> {
        if !record.fields.contains_key(INVENTORY_LEDGER_FIELD)
            && !record
                .fields
                .contains_key(INVENTORY_LEDGER_GENERATION_FIELD)
        {
            return Ok(None);
        }
        let Some(Value::Entity(id)) = field_value(record, INVENTORY_LEDGER_FIELD, tick) else {
            return Err(precondition(
                "inventory custody has no qualified ledger identity",
            ));
        };
        let generation = u32::try_from(field_u64(record, INVENTORY_LEDGER_GENERATION_FIELD, tick)?)
            .ok()
            .filter(|generation| *generation != 0)
            .ok_or_else(|| precondition("inventory custody has no positive ledger generation"))?;
        if *id == EntityId::NIL {
            return Err(precondition("inventory custody cannot name a nil ledger"));
        }
        Ok(Some(Self {
            id: *id,
            generation,
        }))
    }

    pub(super) fn fields(self) -> [(String, Value); 2] {
        [
            (INVENTORY_LEDGER_FIELD.to_owned(), Value::Entity(self.id)),
            (
                INVENTORY_LEDGER_GENERATION_FIELD.to_owned(),
                Value::U64(u64::from(self.generation)),
            ),
        ]
    }

    /// A direct bounded lookup, deliberately independent of the current default
    /// ledger. A new unrelated ledger cannot steal or invalidate this custody.
    pub(super) fn ledger(self, snapshot: &WorldSnapshot) -> Result<&EntityRecord> {
        let record =
            snapshot.graph.entities.get(&self.id).ok_or_else(|| {
                precondition("original inventory stock ledger is no longer observed")
            })?;
        if self.id == EntityId::NIL
            || record.id != self.id
            || self.generation == 0
            || record.generation != self.generation
            || record.kind.as_str() != STOCK_LEDGER_KIND
        {
            return Err(precondition(
                "inventory requires its original stock-ledger identity and generation",
            ));
        }
        Ok(record)
    }
}
