# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with implemented but unadmitted read profiles through spatial/1.6 and an explicitly
unadmitted pause-control/1.7 development slice. No live tuple is currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- separate unadmitted jobs-only 1.2, operations/1.3, paged operations/1.4, map/1.5 and spatial/1.6 development profiles;
- a new pause-control/1.7 bridge/client/development MCP runtime supporting prepare, commit and reconcile for `Pause { paused }` only;
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

### Pause-control/1.7 development effect boundary

`docs/LIVE_CONTROL.md` describes the first bridge-backed live mutation source. The implementation is
strictly scoped to simulation pause/resume and remains unadmitted development functionality.

- The native bridge exposes only `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause` under
  fixed protocol 1.7 identities. It does not expose a generic command, Lua, keyboard, path, address,
  method selector, or any other action family.
- Prepare binds one idempotency key, 32-byte sealed plan digest, desired pause state, and expected
  game tick. It performs no game mutation and returns a prepare token.
- Commit requires the same key/digest/token. The bridge records the attempt before calling
  `World::SetPauseState`, observes the resulting pause state, and retains a stable receipt. Replays
  of a known effect return the retained result instead of applying it twice.
- Query/reconcile reports whether the bridge knows the key, whether the pause effect was applied,
  the observed pause state/tick, prepare token and retained receipt digest. A lost commit reply is
  treated as `EffectIndeterminate`; callers are directed to reconcile before retry.
- World load/unload advances bridge generation and clears retained effect records, preventing
  idempotency state from crossing a world boundary.
- The safe-Rust client binds only the four fixed methods and profile identity. The development MCP
  runtime grants only `ControlClock` at reversible risk, preserves the eleven top-level tool names,
  and refuses every non-pause mutation surface.
- `dfmcp-live-control-dev-server` requires `DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7=1`, its own token and
  loopback endpoint, rejects unrelated `DFMCP_*` state, and refuses production admission provenance.
- No registry entry, deployment floor, server qualification, production runner, or admitted live
  mutation capability exists for protocol 1.7.

This editing session did **not** compile the Rust client/runtime or build the plugin against real
DFHack/protobuf generated sources, and did not execute a disposable-fort control campaign. The
slice is source-present only. The bridge uses a bounded process-local retained effect map; durable
effect-journal crash recovery is still unfinished. No claim is made that a host crash after effect
dispatch can be fully reconciled across process restart.

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
- protocols 1.1 through 1.7 remain outside the production runner map;
- old or external receipts do not qualify this current source generation.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Agent surface | Agent Turn envelope, eleven-tool waist, structured queries, monitoring, production/spatial analysis | durable handoff, complete counterfactual/VOI models |
| Protocol 1.0 | authenticated citizen read stack and production-runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained announcements and development runtime | current native/live admission chain |
| Jobs/operations/map/spatial | coherent bounded development reads through spatial/1.6 | Rust qualification, real DFHack campaigns, production admission |
| Control/1.7 | pause prepare/commit/reconcile source and isolated development runtime | Rust/native/live qualification, durable effect recovery, admission, any other live effect family |
| World | canonical snapshots, deltas, query/graph/path/allocation, operations history and durable spatial observation replay | admitted production durable backend and complete fortress coverage |
| Intent/effects | sealed plans, in-memory dispatcher laboratory, pause-control live source | production two-phase effect journal, leases/checkpoints, dig/build/labor/etc. live effects |
| Security/admission | closed dependencies, protocol-bound tickets, monotonic floor machinery | admitted current tuple, hostile-host resistance, signed release provenance |

## Explicitly absent

- no current admitted live tuple;
- no supported production compatibility claim for protocols 1.1 through 1.7;
- no admitted live mutation capability;
- no live dig, construction, labor, burrow, stockpile, work-order, military, checkpoint, Lua,
  arbitrary command, keyboard, filesystem, or network effect;
- no durable production effect journal proving restart-safe reconciliation of dispatched effects;
- no proof that the current head passed every Rust qualification gate;
- no signed cross-platform release provenance.

## Next executable milestones

1. Run full Rust verification/qualification for the exact current clean head.
2. Build control/1.7 against a named real DFHack/protobuf generation and exercise prepare/commit/query
   in disposable forts, including lost-response and world-reset campaigns.
3. Replace process-local control receipts with the durable two-phase effect-journal/recovery boundary
   before considering any production admission.
4. Only after that boundary is qualified, add the next narrowly versioned mutation family; do not
   jump directly to broad generic effects.
5. Independently continue the established protocol-1.0 admission chain and keep the production map
   unchanged until exact evidence supports widening it.

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
