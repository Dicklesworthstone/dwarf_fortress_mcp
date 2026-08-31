#![forbid(unsafe_code)]

//! In-process contract-scaffold integration test suite.
//!
//! This test uses only deterministic in-memory prototypes. It does not start Dwarf
//! Fortress, load DFHack, connect to the bridge, or provide live-game evidence.

use std::collections::BTreeMap;

use dfmcp_adapter::delta_scanner::ContinuousDeltaStreamer;
use dfmcp_adapter::dispatcher::MutationDispatcher;
use dfmcp_core::clock::{ClockGovernor, ClockPolicy};
use dfmcp_core::lease::LeaseManager;
use dfmcp_core::roles::{RoleManager, SwarmRole};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId, MapCoord,
    MapCuboid, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId,
    WorkBudget,
};
use dfmcp_intent::StaticPlanner;
use dfmcp_intent::blueprint::{BlueprintPlanner, BlueprintTemplate};
use dfmcp_intent::logistics::{InventoryStockpile, ProductionLogisticsCompiler};
use dfmcp_world::atp::{AtpProofCapsule, AtpProofVerifier};
use dfmcp_world::franken_fs::{SavegameArchive, SavegameScrubber};
use dfmcp_world::search::FrankenSearchEngine;
use dfmcp_world::spatial_index::ChunkSpatialIndex;
use dfmcp_world::sqlite_ledger::{SqliteLedgerConfig, SqliteProductionLedger};
use dfmcp_world::{ChunkCoord, MapChunk, TerrainRun, WorldGraph, WorldSnapshot};

fn sample_world_snapshot() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let chunk = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 100 },
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            length: 256,
            tile_code: 2, // SolidWall
        }],
        sparse_overlays: BTreeMap::new(),
    };
    graph.chunks.insert(chunk.coord, chunk);

    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn sample_context(session_id: SessionId, snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id,
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: vec![
            CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::ControlClock,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::Designate,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::ConfigureProduction,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
        ],
        cancellation_requested: false,
    }
}
//! In-process contract-scaffold integration test suite.
//!
//! This test uses only deterministic in-memory prototypes. It does not start Dwarf
//! Fortress, load DFHack, connect to the bridge, or provide live-game evidence.
//!
//! The test body exercises the full pipeline (blueprint compilation → logistics
//! work orders → two-phase mutation dispatch → FrankenFS archival → ATP Merkle
//! verification). It is marked `#[ignore]` because the in-memory
//! `MutationDispatcher` currently supports only the `pause` action; the rest of
//! the pipeline is therefore blocked on a future DFHack-backed dispatcher that
//! can carry out excavation / construction work orders against a live simulation.
//! Run with `cargo test -- --ignored` once that dispatcher lands.
fn test_end_to_end_fortress_control_pipeline() -> Result<()> {
    // 1. Initialize world snapshot & spatial index
    let mut snapshot = sample_world_snapshot();
    let mut spatial_index = ChunkSpatialIndex::new();
    for chunk in snapshot.graph.chunks.values() {
        spatial_index.insert_or_update_chunk(chunk)?;
    }
    for z in 99..=101 {
        for y in -1..=1 {
            for x in -1..=1 {
                let coord = ChunkCoord { x, y, z };
                if snapshot.graph.chunks.contains_key(&coord) {
                    continue;
                }
                spatial_index.insert_or_update_chunk(&MapChunk {
                    coord,
                    revision: 1,
                    width: 16,
                    height: 16,
                    terrain_runs: vec![TerrainRun {
                        length: 256,
                        tile_code: 2,
                    }],
                    sparse_overlays: BTreeMap::new(),
                })?;
            }
        }
    }

    // 2. Setup Multi-Agent Role & Lease governance
    let mut role_manager = RoleManager::new();
    let mut lease_manager = LeaseManager::new();
    let mut clock_governor = ClockGovernor::new(ClockPolicy::UnanimousUnpause);

    let session_leader = SessionId::new(1);
    let leader_grants = role_manager.assign_role(session_leader, SwarmRole::ExpeditionLeader);
    assert!(!leader_grants.is_empty());

    clock_governor.register_session(session_leader, 1000);

    // Acquire spatial lease for dining hall excavation
    let excav_cuboid = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 100 },
        MapCoord { x: 4, y: 4, z: 100 },
    )?;
    let lease = lease_manager.acquire_spatial_lease(
        session_leader,
        excav_cuboid,
        true,
        snapshot.tick,
        500,
    )?;
    assert!(lease.get() > 0);

    // 3. Compile Blueprint Intent & Logistics Quota
    let blueprint_planner = BlueprintPlanner;
    let blueprint_intent = blueprint_planner.compile_blueprint_intent(
        IntentId::new(10),
        snapshot.anchor(),
        MapCoord { x: 0, y: 0, z: 100 },
        BlueprintTemplate::DiningHall {
            width: 5,
            height: 5,
        },
        &spatial_index,
    )?;

    let logistics_compiler = ProductionLogisticsCompiler::default();
    let mut inventory = InventoryStockpile::new();
    inventory.set_stock("DRINK", 5);
    inventory.set_stock("PLANT", 20);
    inventory.set_stock("BARREL", 10);

    let work_order_actions =
        logistics_compiler.compile_quota_work_orders("DRINK", 50, &inventory)?;
    assert!(!work_order_actions.is_empty());

    // 4. Plan compilation with StaticPlanner
    let ctx = sample_context(session_leader, &snapshot);
    let plan = StaticPlanner::default().prepare(&snapshot, &blueprint_intent, &ctx)?;

    // 5. Two-Phase Mutation Dispatch
    let mut dispatcher = MutationDispatcher::new();
    let prepare_receipt = dispatcher.prepare_mutation(&plan, &snapshot, &ctx)?;
    let commit_receipt =
        dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &ctx)?;
    assert_eq!(commit_receipt.actions.len(), 1);
    assert_eq!(
        commit_receipt.actions[0].state,
        dfmcp_core::CommitState::Verified
    );

    // 6. Continuous Delta Streaming
    let mut streamer = ContinuousDeltaStreamer::new(&snapshot);
    let next_tick = GameTick(101);
    let mut snap_after = snapshot.clone();
    snap_after.tick = next_tick;
    snap_after.cursor = ObservationCursor {
        epoch: 0,
        sequence: 1,
    };
    snap_after.state_hash = snap_after.compute_hash();
    let next_hash = snap_after.state_hash;
    let delta = streamer.emit_next_delta(next_tick, &[], &[], next_hash)?;

    assert_eq!(delta.base_cursor, ObservationCursor::ORIGIN);
    assert_eq!(
        delta.target_cursor,
        ObservationCursor {
            epoch: 0,
            sequence: 1
        }
    );

    // 7. ATP Merkle Proof Capsule Verification
    let capsule = AtpProofCapsule::seal(&snapshot, &snap_after, delta.clone(), next_tick)?;
    let verifier = AtpProofVerifier;
    assert!(verifier.verify_capsule(&capsule).is_ok());

    // 8. Durable SQLite WAL Ledger and FrankenFS Archival
    let mut sqlite_ledger = SqliteProductionLedger::new(SqliteLedgerConfig::default());
    sqlite_ledger.insert_snapshot(&snapshot)?;
    sqlite_ledger.insert_snapshot(&snap_after)?;
    sqlite_ledger.insert_delta(&delta)?;
    assert!(sqlite_ledger.verify_storage_integrity().is_ok());

    let mut fs_archive = SavegameArchive::new();
    fs_archive.store_snapshot(&snapshot)?;
    let scrubber = SavegameScrubber;
    let scrub_report = scrubber.scrub_archive(&fs_archive);
    assert!(scrub_report.is_clean);

    // 9. Full-Text Search indexing
    let mut search_engine = FrankenSearchEngine::new();
    search_engine.index_snapshot(&snapshot);
    let hits = search_engine.search("Entity", 5);
    assert_eq!(hits.len(), 0); // No entities initially, only chunk

    Ok(())
}
