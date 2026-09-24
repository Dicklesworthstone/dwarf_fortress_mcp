#![forbid(unsafe_code)]

pub mod atp;
mod attention;
mod canonical;
mod checkpoint;
mod delta;
pub mod franken_fs;
pub mod graph_query;
pub mod inventory_allocation;
pub mod journal_delta;
mod ledger;
pub mod map_connectivity;
pub mod map_reachability;
pub mod map_region;
pub mod merkle;
mod model;
mod query;
pub mod query_page;
pub mod rebase;
pub mod search;
pub mod spatial_index;
pub mod sqlite_ledger;
pub mod topology;
pub mod workforce_allocation;

pub use franken_fs::{
    ArchiveBlock, BLOCK_CHUNK_SIZE, SavegameArchive, SavegameScrubber, ScrubReport,
};

pub use search::{FrankenSearchEngine, SearchHit};

pub use atp::{AtpProofCapsule, AtpProofVerifier};
pub use merkle::{MerkleInclusionProof, MerkleStateTree};
pub use rebase::{ConflictCertificate, ConflictKind, RebaseOutcome, SemanticRebaseEngine};

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
};
pub use query_page::{execute_bounded_query, execute_query};
pub use sqlite_ledger::{
    CapsuleRow, DeltaRow, SnapshotRow, SqliteLedgerConfig, SqliteProductionLedger,
};
