//! Compare complete native rosters and the earlier item allocator frontier.
//! Item counts include held and installed furniture. They measure identity, not availability.

use super::*;
use dfmcp_adapter::live_operations::item_entity_id;

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Bed,
    Chair,
    Table,
}

impl Kind {
    fn type_key(self) -> &'static str {
        match self {
            Self::Bed => "BED",
            Self::Chair => "CHAIR",
            Self::Table => "TABLE",
        }
    }
}

#[derive(Default)]
struct Census {
    next_id: u32,
    matched: u32,
    arrivals: u32,
    examples: Vec<Value>,
}

pub(super) fn compare(
    journal: &mut Journal,
    endpoints: [&JournalEntry; 2],
    kind: Kind,
    material: Option<[i32; 2]>,
    current: &OperationContext,
    started: Instant,
) -> Result<Value> {
    let mut admission = SourceAdmission::default();
    let mut census = |entry: &JournalEntry, frontier: Option<u32>| -> Result<Census> {
        let state = journal.state_at(entry.number, entry.record_digest, &remaining(current, started)?)?;
        let snapshot = state.snapshot().ok_or_else(|| invalid("archived operations projection absent"))?;
        admission.admit(snapshot, current, started)?;
        let observed = state.observation().ok_or_else(|| invalid("archived operations roster absent"))?;
        let mut summary = Census { next_id: observed.next_item_id, ..Census::default() };
        for (index, item) in observed.items.iter().enumerate() {
            if index.is_multiple_of(256) {
                remaining(current, started)?;
            }
            let pair = [item.material_type, item.material_index];
            if item.type_key != kind.type_key() || item.flags & (1 << 3) != 0
                || material.is_some_and(|filter| filter != pair)
            {
                continue;
            }
            summary.matched += 1;
            if frontier.is_some_and(|next_id| item.native_id >= next_id) {
                summary.arrivals += 1;
                if summary.examples.len() < 8 {
                    let id = item_entity_id(item.native_id);
                    let entity = snapshot.graph.entities.get(&id)
                        .ok_or_else(|| invalid("archived item projection absent"))?;
                    summary.examples.push(json!({"native_item_id":item.native_id,"entity_id":id.to_string(),
                        "generation":entity.generation,"material":pair,"subtype":item.subtype}));
                }
            }
        }
        Ok(summary)
    };
    let [first, last] = endpoints;
    let before = census(first, None)?;
    let after = if first.number == last.number {
        Census { next_id: before.next_id, matched: before.matched, ..Census::default() }
    } else {
        census(last, Some(before.next_id))?
    };
    Ok(json!({"schema":"dfmcp.query.result/1","kind":"historical_furniture_arrivals",
        "basis":anchor_json(first.anchor),"anchor":anchor_json(last.anchor),"native_type_key":kind.type_key(),
        "material_filter":material,"count_unit":"non_removed_item_records","before_item_count":before.matched,
        "after_item_count":after.matched,"net_item_count_change":i64::from(after.matched)-i64::from(before.matched),
        "before_next_item_id":before.next_id,"after_next_item_id":after.next_id,"new_item_count":after.arrivals,
        "examples_truncated":after.arrivals as usize > after.examples.len(),"new_item_examples":after.examples,
        "order_attribution_proven":false,"material_classification_proven":false,"source_work":admission.metadata()}))
}
