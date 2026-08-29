#![forbid(unsafe_code)]

mod canonical;
mod delta;
mod model;
mod query;

pub use delta::{apply_delta, build_delta, StateDelta, WorldChange};
pub use model::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, MapChunk,
    TerrainRun, Value, WorldEvent, WorldEventKind, WorldGraph, WorldSnapshot,
};
pub use query::{evaluate, execute_query, CompareOp, Predicate, QueryOrder, QueryResult, WorldQuery};
