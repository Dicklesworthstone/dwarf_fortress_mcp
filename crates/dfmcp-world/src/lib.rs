#![forbid(unsafe_code)]

pub mod atp;
mod attention;
mod canonical;
mod checkpoint;
mod delta;
pub mod franken_fs;
mod ledger;
pub mod merkle;
mod model;
mod query;
pub mod rebase;
pub mod search;
pub mod spatial_index;
pub mod sqlite_ledger;
pub mod topology;

pub use franken_fs::{
    ArchiveBlock, BLOCK_CHUNK_SIZE, SavegameArchive, SavegameScrubber, ScrubReport,
};

pub use search::{FrankenSearchEngine, SearchHit};

pub use atp::{AtpProofCapsule, AtpProofVerifier};
pub use merkle::{MerkleInclusionProof, MerkleStateTree};
pub use rebase::{ConflictCertificate, ConflictKind};

pub use spatial_index::{
    ChunkSpatialIndex, LiquidType, SpatialChunkNode, TemperatureBand, TileProperties, TileType,
};
pub use topology::{
    AbaEntityValidator, detect_cycles, find_reachability, get_transitive_dependencies,
};

pub use attention::{
    AttentionEngine, AttentionLedger, AttentionSignal, AttentionSignalKind, CompletenessStatus,
};
pub use checkpoint::{CheckpointManifest, CheckpointStore, RestoreCertificate};
pub use delta::{
    ContinuationToken, StateDelta, WorldChange, apply_delta, build_delta, compute_snapshot_diff,
    diff_snapshots,
};
pub use ledger::{DurableLedger, EffectJournalRecord, ObservationCapsule, WitnessSet};
pub use model::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    MapChunk, TerrainRun, Value, WorldEvent, WorldEventKind, WorldGraph, WorldSnapshot,
};
pub use query::{
    CompareOp, Predicate, QueryCost, QueryOrder, QueryPlanCost, QueryResult, WorldQuery, evaluate,
    execute_bounded_query, execute_query,
};
pub use sqlite_ledger::{
    CapsuleRow, DeltaRow, SnapshotRow, SqliteLedgerConfig, SqliteProductionLedger,
};
