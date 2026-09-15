# Coherent citizens + operations + terrain: spatial/1.8

Spatial/1.8 closes the largest remaining read-coherence gap in the development profiles: strict fortress citizens, jobs, buildings, items, job-item relationships, and one bounded terrain region are acquired during **one native DFHack RPC suspension** and then transferred as immutable pages.

It is explicitly unadmitted development source. It does not widen the production runner map, compatibility registry, deployment floor, or mutation authority.

## Why a new profile

Citizen protocol 1.0 and spatial/1.6 were independent observations. Matching their fortress IDs or nearby game ticks did not justify joining them into one apparently simultaneous world. In particular, a job's native worker ID could not safely be turned into a citizen handle from another observation.

Spatial/1.8 removes that ambiguity by capturing the complete strict-citizen roster inside the same native operation that creates the operations/terrain payload. The resulting Rust projection has one source digest, one anchor, and one generation universe across all observed domains.

## Native capture

The plugin is `dfmcp_spatial_v1_8` and exposes only `Handshake` and `ReadObservation`, both with flags zero. Its package is `dfmcp.spatial.v1_8`.

The authenticated request fixes:

- jobs: at most 4,096;
- buildings: at most 4,096;
- items: at most 65,536;
- strict citizens: at most 4,096;
- terrain: one region of at most 16,384 cells, each side at most 128;
- total immutable payload: at most 16 MiB;
- transfer page: 16..256 KiB.

The request's citizen bound and terrain region are part of retained-capture ownership. Changing them cannot reuse a previously issued snapshot token.

The strict citizen component uses `Units::getCitizens(..., true, false)`, removes nulls, sorts by native unit ID, and rejects duplicate IDs, non-citizens, or residents. It captures bounded UTF-8 visible name and race, profession, position, and observed status booleans for alive, sane, active, visible, citizen, resident, baby, child, and adult.

The citizen component is a data codec only. It does not issue another RPC. Operations, terrain, and citizens are serialized before the native call returns, and the retained cache owns immutable bytes rather than DF pointers. Normal simulation may advance while those bytes are transferred; every page still describes the capture instant.

Hidden terrain remains presence-only exactly as in map/1.5 and spatial/1.6. Adding citizen coverage does not reveal hidden map attributes.

## Canonical identities and worker joins

Existing development namespaces already use small IDs for jobs, `1 << 40` for buildings, `2 << 40` for items, and `3 << 60` for physical terrain cells. Reusing the legacy protocol-1.0 unit encoding would collide with job IDs, so spatial/1.8 assigns strict citizens their own stable namespace beginning at `3 << 40`.

Each strict citizen projects as a canonical `unit` entity with observed fields:

- `native_unit_id`
- `name`
- `race`
- `profession`
- `position`
- `alive`, `sane`, `active`, `visible`
- `citizen`, `resident`, `baby`, `child`, `adult`

A `member_of` edge connects every strict citizen to the fortress root.

Jobs gain two cross-domain fields:

- `worker_is_strict_citizen`
- `worker_entity`

When `Job::getWorker` names a member of the complete strict-citizen roster, `worker_entity` is a generation-checked unit reference and a `performs` edge connects that citizen to the job.

When a job has no worker, `worker_entity` is explicitly absent. When it has a worker outside the strict-citizen roster, the worker is **not invented** as a placeholder entity: `worker_entity` is unknown with an explicit reason. This distinction matters because animals, visitors, or other non-citizen units may legitimately be outside this profile.

These relations prove only the observed assignment. They do not prove labor enablement, skill suitability, availability, path access, job readiness, or why a job is blocked.

## MCP runtime

The development binary is:

```bash
DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_8=1 \
DFMCP_SPATIAL_CITIZEN_TOKEN='<32..256 bytes>' \
DFMCP_SPATIAL_CITIZEN_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-spatial-citizens-dev-server
```

The runtime uses a distinct process-scoped session family and grants only `Observe`, `Query`, and `Doctor` at read-only risk. It preserves the frozen eleven top-level tool names; plan/commit/cancel/checkpoint/restore remain unavailable for game effects.

Convenience query modes are `summary`, `citizens`, `jobs`, `buildings`, `items`, `tiles`, and `schema`. The regular structured query engine can inspect unit fields, follow `performs` and `member_of` edges, aggregate citizens, capture baselines, and register foreground watches.

`map_route` and `spatial_inventory_plan` reuse the same route/allocation implementation as spatial/1.6, but now execute against the citizen-inclusive snapshot and source digest. No separately timed state is introduced.

For example, a job can be inspected and its assigned strict citizen traversed from the same anchor:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "traverse",
    "roots": ["<citizen entity id>"],
    "edge_kinds": ["performs"],
    "direction": "outgoing",
    "max_depth": 1
  }
}
```

A citizen-only change advances the combined observation sequence. Entity generations remain stable while identities remain continuously present; disappearance/reappearance or a full epoch reset advances generation according to the combined state machine.

## Evidence and limitations

Source and registered tests are present. Added Rust scenarios cover:

- citizen/job worker resolution and `performs` edges;
- non-citizen worker uncertainty without placeholder entities;
- citizen-only advancement with stable generation;
- strict-roster/trailing-data rejection.

`scripts/test_live_spatial_citizens_native_mock.py` is checked in to compile the actual producer and shared codecs against explicit mock DFHack/protobuf interfaces and to exercise strict-roster bounds, hidden-terrain noninterference, retained capture identity, generation invalidation, and fixed RPC registration.

Those new 1.8 Rust/native tests have **not been executed in this editing environment**. There is no Rust toolchain here, and the container does not have a network-mounted repository. No real generated DFHack/protobuf build, live fortress campaign, full repository qualification, or admission is claimed.

Still outside this profile: non-citizen unit details, citizen skills/needs/health, labor configuration, full unit-specific pathfinding, outside-region terrain, complete native material requirements, durable 1.8 history, and all live effects. Pause-control/1.7 remains a separate, unadmitted development boundary and is not implicitly authorized by a spatial/1.8 observation.
