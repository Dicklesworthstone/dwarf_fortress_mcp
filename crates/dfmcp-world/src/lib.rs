#![forbid(unsafe_code)]

mod canonical;
mod delta;
mod model;
mod query;

pub use delta::{StateDelta, WorldChange, apply_delta, build_delta};
pub use model::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, MapChunk,
    TerrainRun, Value, WorldEvent, WorldEventKind, WorldGraph, WorldSnapshot,
};
pub use query::{
    CompareOp, Predicate, QueryOrder, QueryResult, WorldQuery, evaluate, execute_query,
};
