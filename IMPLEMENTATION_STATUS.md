# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with implemented but unadmitted read profiles through citizen-inclusive spatial/1.8
and an explicitly unadmitted pause-control/1.7 development slice. No live tuple is currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- separate unadmitted jobs-only 1.2, operations/1.3, paged operations/1.4, map/1.5 and spatial/1.6 development profiles;
- a citizen-inclusive spatial/1.8 source path that captures strict citizens, jobs, buildings, items and bounded terrain during one native suspension, then publishes one combined anchor;
- a pause-control/1.7 bridge/client/development MCP runtime supporting prepare, durably coordinated commit, and reconcile for `Pause { paused }` only;
- a private hash-chained pause-effect coordinator journal whose source records `CommitStarted` before dispatch and terminal evidence before acknowledgement;
- a protocol-bound V2 production ticket and runtime dispatcher whose map still contains only protocol 1.0.

The checked-in compatibility registry remains empty. Source presence does not imply qualification,
admission, or support. No protocol beyond 1.0 appears in the production runner map.

## Evidence hierarchy

1. **source present** — code, contracts, and tests are checked in;
2. **static/Python checked** — static or design checks passed for one exact source generation;
3. **Rust-qualified** — latest-nightly formatting, warning-denied Clippy, debug/release tests and rustdoc passed;
4. **native-qualified** — exact DFHack plugin built and passed native qualification;
5. **live-qualified** — required disposable-fort campaign passed;
6. **registry-admitted** — reviewed receipt promoted to compatibility registry;
7. **floor-accepted** — deployment host advanced monotonic floor;
8. **artifact-qualified** — exact server executable qualified;
9. **runtime-admitted** — protocol-bound single-use ticket consumed by the exact runner.

Higher rungs apply only to the exact source, binary, protocol, platform and inputs they name.

## Present now

### Coherent citizen + operations + terrain spatial/1.8 source

`docs/LIVE_SPATIAL_CITIZENS.md` describes the new citizen-inclusive read profile. It removes the
cross-observation ambiguity between the old citizen profile and spatial/1.6: the strict citizen
roster and the existing operations/terrain payload are serialized during one native DFHack RPC
suspension and transferred as one immutable retained capture.

- `dfmcp_spatial_v1_8` keeps the two-method `Handshake`/`ReadObservation` waist. Its request adds an
  explicit citizen bound while preserving fixed jobs/buildings/items/terrain/page/byte bounds.
  Native retained-cache ownership binds both the requested terrain region and citizen bound.
- A strict complete citizen component is bounded to 4,096 records, sorted by nonnegative native unit
  ID, and rejects duplicates, non-citizens and residents. Observed fields are bounded visible name,
  race, profession, position, alive/sane/active/visible and developmental status.
- The combined payload is at most 16 MiB and preserves the spatial/1.6 immutable-page semantics.
  Citizens are captured before the native RPC returns; later page transfer never dereferences DF
  pointers or mixes a newer citizen roster into the retained operations/terrain bytes.
- Spatial/1.8 uses a dedicated citizen entity namespace rather than the legacy protocol-1.0 unit
  encoding, which would collide with current job IDs. All projected facts and edges are rebound to
  one spatial/1.8 source digest and one combined observation anchor/generation universe.
- A job whose `Job::getWorker` ID appears in the complete strict-citizen roster gains a
  generation-checked `worker_entity` fact and a `performs` edge from that citizen to the job. A job
  with no worker records absence. An assigned worker outside the strict-citizen roster remains
  explicitly unknown rather than becoming a fabricated placeholder unit.
- The existing route-aware inventory allocator and candidate route queries were generalized over a
  coherent spatial-state interface; spatial/1.6 behavior is preserved while spatial/1.8 can run the
  same algorithms against its larger same-anchor graph.
- `dfmcp-live-spatial-citizens-dev-server` is registered with a distinct process-scoped session
  family, its own credentials/opt-in, two-session limit, read-only Observe/Query/Doctor grants, and
  the frozen eleven top-level tool names. Convenience modes expose citizens/jobs/buildings/items/
  tiles, while ordinary graph/baseline/watch/route/allocation queries all use the combined anchor.
- The profile still does not establish citizen skills, needs, health, labor eligibility, complete
  unit navigation, non-citizen unit details, outside-region terrain, native material requirements,
  durable 1.8 history, or any game effect. Pause-control/1.7 remains separate and gains no authority
  from a spatial observation.

Four new Rust integration scenarios are registered for worker joins, non-citizen uncertainty,
citizen-only advancement/generation continuity and strict-roster corruption. A reproducible native
mock harness is checked in at `scripts/test_live_spatial_citizens_native_mock.py`, covering immutable
retained bytes, strict citizen limits, hidden-terrain noninterference, generation invalidation and
fixed method registration.

Those new spatial/1.8 Rust/native checks have **not been executed in this editing environment**. The
container has no Rust toolchain and no network-mounted repository. No real generated DFHack/protobuf
build, live-game campaign, full repository qualification or admission is claimed. This section is
therefore evidence rung 1: source present.

### Durable spatial/1.6 history and exact-record analysis

The optional spatial journal is integrated into authenticated bootstrap and
refresh. Complete coherent captures are synced before publishing changed live
anchors, and restart replays the exact combined generation chain. Historical
queries reconstruct an independent typed spatial state, not a replacement live
world. See `docs/SPATIAL_HISTORY.md`.

- The shared journal has sealed operations/1.3, operations/1.4 and spatial/1.6
  codecs. Legacy 1.3 APIs, framing and digest domains remain unchanged. Wrong
  profiles are rejected before incomplete-tail repair; no archive migration is
  inferred. The 1.4 codec is a library API, not 1.4 MCP journal integration.
- Spatial `history` lists committed captures with bound whole-row pagination.
  `historical_query` admits only eight stateless query kinds, including candidate
  routes and route-aware inventory allocation. Allocation route drill-downs stay
  pinned to their exact archived record and digest.
- Current Query authority is required; old captures do not revive expired grants.
  Current watches stay current and are not evaluated against archived facts.
  Acquisition/replay and response budgets remain separate. Native source failure
  still permits verified archive reads in an already-open session.
- Paths and explicit incomplete-tail repair are operator-only settings. Failed
  writes/syncs fence publication; complete corrupt frames are never silently
  discarded. Default retention is 64 MiB/1,024 changed captures with no pruning.
- No offline bootstrap, durable watches/baselines, automatic rotation, effect
  journal, anti-rollback floor, native bridge change, dependency change, or
  production admission is introduced. Concurrent control work is unchanged.

Fourteen new Rust scenarios are registered: eight journal/profile and six Unix
actual-handler scenarios. They have not run because Rust, Cargo and rustfmt were
unavailable. No Rust compilation, Clippy, stdio, filesystem crash campaign, live
DFHack or repository qualification is claimed. JSON Schema meta-validation and
50 new-wrapper/gate component checks passed; full composed schema validation did
not run. Sixteen independent JSON-size checks passed for archived route wrappers.
Those checks are not execution of the Rust implementation.

### Pause-control/1.7 durable development effect boundary

`docs/LIVE_CONTROL.md` describes the first bridge-backed live mutation source. The implementation is
strictly scoped to simulation pause/resume and remains unadmitted development functionality.

- The native bridge exposes only `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause` under
  fixed protocol 1.7 identities. It does not expose a generic command, Lua, keyboard, path, address,
  method selector, or any other action family.
- Prepare binds one idempotency key, 32-byte sealed plan digest, desired pause state, expected game
  tick, and bridge generation. It performs no game mutation. The 16-byte prepare token is derived
  from a fixed SHA-256 domain and that complete identity.
- A private durable coordinator journal is mandatory. It uses an exclusive locked exact-mode `0600`
  file under a canonical exact-mode `0700` directory, a hash-chained transition sequence, bounded
  retention, file sync, and explicit incomplete-tail repair. Complete corrupt records and broken
  predecessor chains are rejected.
- The coordinator syncs `Prepared` after nonmutating bridge prepare. Before any `CommitPause` RPC it
  appends and syncs `CommitStarted`. If this durability boundary fails, no game mutation is called.
  The runtime never retries a mutating commit automatically.
- After one commit dispatch, the bridge observes pause state and returns a generation-bound full
  32-byte SHA-256 receipt. The coordinator must sync `VerifiedApplied` or `VerifiedNotApplied`
  before acknowledging a terminal result. Failure to establish terminal durability is reported as
  `EffectIndeterminate`, even if a bridge reply was received.
- Rust-process restart replays the exact durable transition chain. `CommitStarted` and
  `Indeterminate` remain reconciliation-required. Read-only reconciliation may reconnect to the
  bridge, but commit is not redispatched as recovery work.
- If the bridge incarnation still knows the effect, reconciliation records its retained result. If
  bridge generation changed or the key is unknown, the durable state remains indeterminate and the
  same effect is never reported safe to retry; a new observation and new plan/idempotency key are
  required. A merely `Prepared` effect may commit once only while its bridge generation still agrees.
- World load/unload advances bridge generation and clears native retained records, preventing native
  idempotency state from crossing a world boundary. The generation is also bound into tokens and
  receipts.
- The safe-Rust client binds only the four fixed methods and profile identity. The development MCP
  runtime retains one isolated mutation session family, grants only `ControlClock` at reversible
  risk, preserves the eleven top-level tool names, and refuses every non-pause mutation surface.
- Agent Turn metadata explicitly keeps `runtime_admitted=false` and `mutation_admissible=false`; the
  unadmitted development effect switch is represented separately. Protocol 1.7 remains absent from
  the production runner map and the compatibility registry remains empty.

The actual native source was compile-checked in the editing environment against explicit mock
DFHack/protobuf interfaces with both GCC and Clang under C++17 and warning-denied flags. A local
state-machine mirror exercised prepare replay, one-shot commit, duplicate suppression, query
reconciliation, generation reset, and fixed method registration. Independent Python `hashlib`
calculations matched the generation-bound token and receipt identities after an embedded-NUL domain
separator bug was corrected. `scripts/test_live_control_native_mock.py` is checked in as the
reproducible mock-native harness.

These checks are not native qualification. The Rust durable journal/client/MCP runtime have not been
compiled or executed because no Rust toolchain was available. No real generated DFHack/protobuf
build, filesystem power-loss campaign, disposable-fort control campaign, registry promotion,
deployment-floor advancement, server-artifact qualification, production runner, or admitted live
mutation capability is established. The source now contains restart-safe coordination semantics;
those semantics still require exact Rust/native/live qualification before broader effects or
production admission are considered.

### Coherent spatial/1.6 capture and route-aware inventory allocation

The spatial profile captures jobs, buildings, items, attachments and one bounded terrain region in
one native suspension, then transfers immutable serialized pages. `docs/LIVE_SPATIAL.md` contains the
full contract.

- Composite decoding requires matching world/site/clock/pause/software/generation identities.
- One source digest, observation cursor and entity-generation universe cover operations and terrain.
- Bounded terrain reachability feeds the existing integral allocator without double-counting stacks.
- `spatial_inventory_plan` returns same-anchor candidate routes, supply handles, exclusion reasons,
  and a flow/min-cut shortage certificate under the declared stack-unit model.
- The registered spatial development server preserves eleven tools, shared query/watch behavior and
  strict read-only mutation refusal.

Mock-native validation previously passed 786 GCC and 786 Clang checks, including a 40,000-item,
2,200,457-byte immutable capture across 135 pages. Twenty-three Rust scenarios remain registered but
unexecuted in this editing environment. Spatial/1.6 remains unadmitted.

### Bounded map/1.5 terrain observation and candidate routes

The map profile observes one fixed bounded terrain region, preserving hidden cells as redacted and
unallocated blocks as unknown. Deterministic dry cardinal floor/stair candidate routes are available
through the existing query surface. Routes are not unit-specific navigation or global reachability
proofs. The profile remains unadmitted development source.

### Immutable-paged operations/1.4

Operations/1.4 captures one coherent jobs/buildings/items state and transfers immutable bytes through
bounded pages with fixed token/generation/limit identity and whole-payload digest verification. It
remains unadmitted development source and does not inherit 1.3 qualification or archive identity.

### Durable operations/1.3 observation history

Operations/1.3 can optionally persist canonical changed observations before publication, replay the
exact generation chain on restart, list retained history, and execute stateless historical queries.
This is an observation archive, not durable effect-journal recovery, anti-rollback custody, or a
production MVCC claim.

### Query, graph, monitoring and production analysis

Public query continuations are snapshot/query bound. Structured entity inspection, aggregates,
search, graph traversal, SCC/dependency diagnosis, baseline changes, foreground condition watches,
production diagnosis, integral inventory allocation and spatial inventory planning are implemented
as described in their dedicated documentation. These derived layers never grant mutation authority.

## Current registry and qualification state

The checked-in compatibility registry remains:

```json
{
  "schema_version": "dfmcp.live-compatibility-registry/1",
  "status": "no_admitted_live_tuples",
  "entries": []
}
```

Consequences:

- no Dwarf Fortress/DFHack/plugin/source/protocol/platform tuple is currently admitted;
- the production launcher cannot authorize any newly added development profile;
- protocols 1.1 through 1.8 remain outside the production runner map;
- old or external receipts do not qualify this current source generation.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Agent surface | Agent Turn envelope, eleven-tool waist, structured queries, monitoring, production/spatial analysis | durable handoff, complete counterfactual/VOI models, durable control-effect listing without a known key |
| Protocol 1.0 | authenticated citizen read stack and production-runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained announcements and development runtime | current native/live admission chain |
| Jobs/operations/map/spatial | coherent bounded development reads through citizen-inclusive spatial/1.8, including same-anchor strict-citizen worker joins | Rust qualification, real DFHack campaigns, citizen skills/needs/health, full unit navigation, durable 1.8 history, production admission |
| Control/1.7 | pause prepare/commit/reconcile source, mandatory private durable coordinator journal, generation-bound tokens/receipts, isolated development runtime | Rust qualification, real DFHack build, crash/disposable-fort campaigns, production admission, any other live effect family |
| World | canonical snapshots, deltas, query/graph/path/allocation, operations history and durable spatial/1.6 observation replay | admitted production durable backend and complete fortress coverage |
| Intent/effects | sealed plans, in-memory dispatcher laboratory, bridge-backed pause effect with durable pre-dispatch/terminal coordinator states | qualified/admitted effect journal, leases/checkpoints tied to live commits, dig/build/labor/etc. live effects |
| Security/admission | closed dependencies, protocol-bound tickets, monotonic floor machinery | admitted current tuple, hostile-host resistance, signed release provenance |

## Explicitly absent

- no current admitted live tuple;
- no supported production compatibility claim for protocols 1.1 through 1.8;
- no admitted live mutation capability;
- no live dig, construction, labor, burrow, stockpile, work-order, military, checkpoint, Lua,
  arbitrary command, keyboard, filesystem, or network effect;
- no Rust-qualified/native-qualified/live-qualified control effect journal or power-loss evidence;
- no proof that the current head passed every Rust qualification gate;
- no signed cross-platform release provenance.

## Next executable milestones

1. Run full Rust verification/qualification for the exact current clean head, including spatial/1.8
   citizen coherence, the durable pause-effect journal, control client, development runtime, and all
   registered recovery tests.
2. Compile spatial/1.8 and control/1.7 against named real DFHack/protobuf generations and execute
   disposable-fort read/control campaigns for the exact plugin bytes.
3. Exercise spatial/1.8 with real citizen/job churn, non-citizen workers, large rosters, hidden terrain,
   immutable multi-page transfers, route/allocation analysis and foreground watches before treating
   its cross-domain joins as live-qualified evidence.
4. Execute control host/bridge failure campaigns at every durability boundary: before
   `CommitStarted` sync, after sync/before dispatch, after dispatch/before reply, after reply/before
   terminal sync, Rust restart with live bridge, DFHack restart, world load/unload, and incomplete/
   corrupt journal tails.
5. Only after those exact semantics are qualified should a control/1.7 admission proposal or another
   narrowly versioned mutation family be considered. Keep the production map unchanged until exact
   evidence supports widening it.

## Status rules

1. Source presence is not qualification, admission, support, or production evidence.
2. Development execution is not production admission.
3. A tuple is admitted only while its exact entry exists in the current registry generation.
4. A deployment is admitted only when its trusted floor matches that registry generation.
5. A protocol executes in production only when the V2 production map contains its reviewed runner.
6. A doctor report is diagnosis, never authority.
7. A server receipt qualifies one executable, not a bridge session or game state.
8. A single-use ticket authorizes one exact process/protocol start and does not grant unstated effects.
9. Negative evidence may reject a claim but cannot certify success.
10. Derived indexes, recommendations, history and path models never grant authority.
11. Unit tests never substitute for disposable-fort evidence where Dwarf Fortress behavior matters.
