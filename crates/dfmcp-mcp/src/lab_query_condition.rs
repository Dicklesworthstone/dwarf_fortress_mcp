//! Closed laboratory condition syntax shared by explicit foreground watches
//! and stateless preflight. Deserializing original request bytes preserves
//! duplicate-field rejection at every predicate boundary. Source policy and
//! bounded three-valued evaluation remain in the existing semantic engine.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Literal {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    Text(String),
    Fixed { units: i64, scale: u32 },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CountScope {
    ObservedProjection,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CountKind {
    Unit,
    Job,
    Building,
    Item,
    TileFeature,
    Announcement,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CountPredicate {
    Always {},
    Field {
        field: String,
        comparison: Comparison,
        value: Literal,
    },
    All {
        args: Vec<CountPredicate>,
    },
    Any {
        args: Vec<CountPredicate>,
    },
    Not {
        arg: Box<CountPredicate>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SumKind {
    Unit,
    Job,
    Building,
    Item,
    TileFeature,
    Announcement,
    StockLedger,
}

/// Only predicates with an explicit laboratory evidence policy are reachable.
/// Native stack, relationship, terrain, furniture and shelter probes retain
/// their existing independently scoped native entry points.
#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Condition {
    Field {
        entity_id: String,
        generation: u32,
        field: String,
        comparison: Comparison,
        value: Literal,
    },
    EntityCount {
        scope: CountScope,
        kind: CountKind,
        predicate: CountPredicate,
        comparison: Comparison,
        value: u64,
    },
    FieldSum {
        scope: CountScope,
        kind: SumKind,
        field: String,
        predicate: CountPredicate,
        comparison: Comparison,
        value: u64,
    },
    Paused {
        value: bool,
    },
    TickAtLeast {
        value: u64,
    },
    All {
        args: Vec<Condition>,
    },
    Any {
        args: Vec<Condition>,
    },
    Not {
        arg: Box<Condition>,
    },
}
