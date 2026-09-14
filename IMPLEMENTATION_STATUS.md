# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with implemented but unadmitted announcement, jobs-only, and coherent operations
development read slices. No live tuple is currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- an explicitly unadmitted protocol-1.1 development MCP runtime;
- a separate jobs-only protocol-1.2 native plugin, Rust client, projection, and development MCP binary;
- an operations/1.3 native producer, client, coherent jobs/buildings/items graph, and development MCP binary;
- a protocol-bound V2 production ticket and runtime dispatcher whose map currently contains only
  protocol 1.0.

The checked-in compatibility registry nevertheless has status `no_admitted_live_tuples` and zero
entries. Therefore:

```text
implementation source exists
≠ the current source has a fresh full qualification receipt
≠ the current native plugin passed a named R1 build
≠ the tuple passed its complete live campaign
≠ the tuple is present in the checked-in registry
≠ a deployment floor accepted that registry generation
≠ a server binary is qualified for that source generation
≠ a live process is authorized to start
```

No live mutation RPC or mutation capability is implemented or admitted.

The Asupersync 0.5.0 consumer migration (`dfmcp-k2y`) advances the exact FastMCP
pin to `180a7c88890705217bb8e202d19555adabf24187` and gives all four stdio
composition roots explicit runtime ownership. Its scoped Rust and subprocess
validation is in progress. It does not create a fresh full qualification receipt
or admit a live tuple; historical conformance findings remain recorded in
`docs/DOGFOODING_FASTMCP.md`.

## Evidence hierarchy

Implementation claims must identify their evidence rung:

1. **source present** — code, contracts, and tests are checked in;
2. **static/Python checked** — repository and Python contract gates passed for one exact commit;
3. **Rust-qualified** — latest-nightly formatting, warning-denied Clippy, debug/release tests, and
   warning-denied rustdoc passed for one exact clean commit;
4. **native-qualified** — one exact DFHack plugin built and passed R1 for named DFHack source and
   plugin bytes;
5. **live-qualified** — the required disposable-fort campaign passed for the same exact tuple;
6. **registry-admitted** — reviewed receipts were promoted into the checked-in registry;
7. **floor-accepted** — a deployment host advanced its owner-only monotonic floor to those exact
   registry bytes;
8. **artifact-qualified** — a source-bound release-server receipt identifies the exact executable;
9. **runtime-admitted** — the floor-bound launcher issued and the Rust process consumed one exact
   protocol-bound single-use ticket.

A higher rung applies only to the exact identities it names. It never transfers silently to a later
commit, rebuilt binary, different bridge protocol, or another platform.

## Present now

### Durable operations observations and historical replay

The operations/1.3 development runtime now has an optional synced observation
journal integrated into authenticated bootstrap and live publication. See
`docs/OPERATIONS_HISTORY.md` for configuration, queries and the recovery contract.

- Canonical sources, complete anchors and predecessor digests are persisted before
  a changed observation becomes visible. Restart replays the exact version and
  generation chain; heartbeats do not add duplicate records.
- Private Unix storage uses an exclusive writer lock, exact directory/file modes,
  regular-file/link/identity checks and file/directory fsync. Operator paths and
  explicit incomplete-tail repair are environment configuration, never MCP input.
- Failed writes/syncs fence publication. Default recovery preserves all bytes and
  refuses incomplete tails; explicit repair truncates only a verified incomplete
  suffix. Complete corrupt frames, invalid length checksums and nonreproducible
  projection anchors refuse instead of silently accepting a shorter history.
- `history` lists committed observations with bound whole-row pagination.
  `historical_query` replays an exact record and executes only stateless queries.
  Archived anchors/evidence are explicit, current Query authority is required,
  current active watches remain current, and neither live state nor watches or
  baselines are mutated by a historical read.
- Journal retention defaults to 64 MiB/1,024 records, with no automatic pruning.
  This is a bounded observation archive, not the FrankenSQLite MVCC backend,
  effect journal, game checkpoint, durable watches/baselines, signed provenance,
  anti-rollback floor, complete game history or an admitted production feature.

Fifteen new Rust scenarios are registered but not executed. The independent
Python framing-design oracle passed 5,833 checks; it treats semantic anchors as
opaque and is not Rust execution or projector verification. The new schema passed
JSON Schema meta-validation. No Rust compiler, Cargo or rustfmt was available;
Clippy, stdio, real filesystem crash behavior, live DFHack and full repository
qualification remain unestablished. Existing bridge bytes, dependency pins,
production admission and the active migration bead are unchanged.

### Observed production diagnosis and declared inventory allocation

The operations/1.3 `fortress.query` source now integrates `production_diagnosis`
and `inventory_plan`, with `mode=production` as a diagnostic shortcut. The complete
contract and example workflow are in `docs/PRODUCTION_ANALYSIS.md`.

- Production diagnosis joins jobs, holders, actual attached items and memoized
  container ancestry. It exposes flags, unassigned workers, holder stages,
  unindexed filters and shared inputs with generation-checked drill-downs.
  Repeated attachment roles do not multiply distinct-item counts. Findings remain
  observed conditions, not causal blocker proofs or a claim that a job is ready.
- Declared stack-unit demands use exact type/subtype/raw material selectors and
  conservative ancestry-aware exclusions. The bounded integral allocator uses
  residual rerouting, never double-counts shared supply, and checks flow/min-cut
  equality plus a joint shortage witness when the declared model is deficient.
- Inventory results are conditional on the supplied model and exclusion policy.
  They do not infer native recipes, full material eligibility, path access,
  reservations, executable plans, or permission to mutate the game.
- Both query modes retain the full Agent Turn and active watches, paginate complete
  rows with session/query/policy/snapshot-bound continuations, and reject poisoned
  sources, changed handles, missing authority, malformed input and exhausted budgets.
  Only operations schema discovery gains the two variants; the other sixteen
  variants, eleven tool names and native acquisition bounds are preserved.

Twenty-two Rust tests are registered: seven allocator, eight model, and seven
actual-handler scenarios. Rust compilation, tests, rustfmt, Clippy, stdio and live
execution remain unverified because no Rust toolchain was available. An independent
Python design oracle passed 1,568 exhaustive and 1,000 seeded allocation models;
90 schema-extension cases (38 accepted, 52 rejected) and three documentation
examples passed. Those checks are not Rust execution or repository qualification.
Native producer/wire bytes, dependency pins, production admission and the active
migration bead are unchanged. Full native requirement matching, map reachability,
large-roster acquisition paging, durable supervision and live effects remain absent.

### Coherent operations/1.3 development read path

The operations profile adds source integration from one suspended native read of
jobs, buildings, inventory and their relationships through the common agent query
and foreground-monitoring engines. See `docs/LIVE_OPERATIONS.md`.

- A separate authenticated plugin observes complete bounded native rosters and
  actual container, building-holder and job-item attachment references in one RPC.
  It never merges independently timed citizen, announcement, or jobs snapshots.
- Canonical decoding validates exact type/count/identity bounds, same-roster
  reference endpoints, attachment counts and containment acyclicity before atomic
  multi-domain publication. Every fact and edge shares one source digest/anchor.
- Stable entity namespaces and semantic edge IDs support real graph traversal.
  Observed retirement/reappearance advances generation; native job, building or
  item ID-horizon regression resets the shared epoch. Invisible reuse is not claimed.
- `dfmcp-live-operations-dev-server` is registered with its independently gated
  public entry and the existing owned runtime. It retains eleven tools and only
  Observe/Query/Doctor authority. Its own token, opt-in and process-scoped session
  family keep it separate from existing profiles and production admission.
- Typed inventory/building/job queries, grouping, lexical search, relation paths,
  baselines and condition watches use the combined projection. One await refresh
  can update an inventory baseline and construction watch at the same anchor.
  Authority is rechecked after refresh, terminal watch retry skips I/O, and source
  failure preserves the prior anchor with explicitly stale local management.
- Full roster/payload bounds are 4,096 jobs, 4,096 buildings, 32,768 items,
  65,536 attachments and 2 MiB. Oversized acquisition refuses rather than paging
  or publishing a partial domain. Native snapshot paging is not implemented.
- Raw material IDs, flags, stack counts, stage values and attachments are not
  material-eligibility, accessibility, blocker-cause or successful-completion proofs.
  No placeholder citizen entities or speculative requirements are introduced.

The actual native producer compiled against mock DFHack/protobuf interfaces with
both GCC and Clang, C++17 and warning-denied flags. Each run passed 135 checks and
matched an independent Python encoder's 423-byte golden frame. Exact source
SHA-256: `ecb555296d3b84111f2acd5290c48797027c12a830d7bb4d2ffdec4e3aa4acb7`.
The reproducible harness is `scripts/test_live_operations_native_mock.py`.

Eighteen new Rust tests are registered but not executed: nine codec/model tests,
four RPC tests and five actual-handler scenarios. No Rust compiler, Cargo or
rustfmt was available. No Clippy, stdio, full repository gate, real generated DF
headers, protobuf generation/linking, actual native loading, live-game behavior
or production qualification has been established. Mock compilation is not native
qualification. Existing native profiles, dependency pins, production map and
migration bead are unchanged; the shared framing module gained client registration.
Citizen/announcement integration, map/path observations, requirement evaluation,
durable monitoring and live mutations remain unfinished.

### Jobs-only protocol 1.2 development read path

The independent jobs profile now has source integration from native DFHack reads
through MCP queries and foreground monitoring. `docs/LIVE_JOBS.md` describes its
build path, entry, fields, examples, binary layout, and limitations.

- `dfmcp_jobs_v1_2` is a separate authenticated plugin with exactly Handshake and
  ReadObservation. It collects the complete bounded global job list in one RPC;
  oversized, cyclic, malformed, or partial source rosters are rejected.
- Records contain job type/reaction, suspension/repeat flags, position, native
  worker/holder identities, raw completion timer, and item-reference/filter counts.
  They do not establish material availability, path access, blocker causes, or
  successful job completion. No placeholder citizen/building entities are created.
- The owned Rust RPC client pins plugin/type/method/protocol identities, bounds
  protobuf and binary input, validates nonce/version/generation, and fences failed
  streams. Numeric-loopback TCP uses an absolute whole-call deadline across
  fragments and notifications; bootstrap and first observation are separate calls.
- Canonical job snapshots have source digests, generation tracking, immutable
  publication, exact heartbeats, ordinary sequence advancement, and epoch resets
  on source-generation, clock, or next-job-ID regression. Observed retirement and
  reappearance advance generation; invisible same-ID reuse is not claimed detected.
- `dfmcp-live-jobs-dev-server` is registered as an automatically discovered Cargo
  binary. Its independently gated public entry uses the existing owned modern MCP
  runtime and preserves the eleven tool names. Only Observe, Query, and Doctor
  grants are available; all game-mutation tools refuse without an effect.
- Shared entity queries, aggregates, lexical search, baselines, and condition
  watches operate on the job projection. The jobs runtime's observe/wait refreshes
  one roster; await_watch validates ownership before refreshing and terminal retry
  skips I/O. Source failure still permits local watch management with stale coverage.
- The jobs profile has its own token, opt-in, session namespace, and explicit
  jobs-only coverage. It is not a citizen/announcement superset, does not merge
  independently timed worlds, and is absent from the production protocol map.

The actual native plugin source was compiled against **mock DFHack/protobuf types**
with both GCC and Clang, C++17, and warning-denied flags. Each run passed 95 checks
and matched an independent Python encoder's 153-byte golden frame. The exact
source SHA-256 was `0458b5548e5bb9891083d29f192bccd3be510107722795f96257855d0c3a7960`.
The reproducible test is `scripts/test_live_jobs_native_mock.py` and the golden
frame is retained for Rust decoder/query tests.

Nineteen new Rust tests are registered but **not executed**. No Rust compiler,
Cargo, or rustfmt was available in the editing environment. Real generated DF
headers, protobuf generation/linking, native DFHack loading, stdio execution,
live-game behavior, full repository gates, and production admission have not been
established. Mock compilation is not native qualification or a live success claim.
The existing 1.0/1.1 bridges, production map, dependency pins, and migration bead
status are unchanged.

### Foreground condition watches and one-observation waits

Protocol-1.1 `fortress.query` now integrates `watch`, `poll_watch`, `await_watch`,
`watches`, `cancel_watch`, and `release_watch`. The workflow and exact limitations
are described in `docs/CONDITION_WATCHES.md`.

- Watches retain bounded typed success/failure predicates, entity generations,
  game-tick deadlines, sampling cadence, and distinct-observation stability.
- Duplicate anchors cannot manufacture completion. Unknown, incompatible, stale,
  inferred, or unsupported facts cannot satisfy a field condition, including
  under negation. Known failure wins; unknown failure blocks success.
- Missing entities remain unknown. Reused generations, epoch changes, time or
  sequence regressions, and same-cursor forks invalidate nonterminal records.
  Skipped observation sequences reset stability rather than proving continuity.
- `await_watch` validates the session-owned handle before I/O, requires Query and
  Observe authority, performs at most one bounded adapter observation, then
  reauthorizes at the published target before evaluating. Its adapter observation
  may internally require multiple bounded native pages. It never unpauses or
  controls game time. Terminal awaits skip the read entirely.
- Active watches are projected into successful protocol-1.1 query Agent Turns.
  Authorized query errors include them when the complete packet fits. Source
  poisoning still permits local watch listing, cancellation, and terminal release,
  with explicitly stale source continuity.
- Registration, evaluation, cancellation, and release publish only after the full
  response renders within budget. A failed render leaves watch state unchanged,
  although a preceding bridge read may already have published a newer observation.
- Retention is bounded to eight records per session and 128 per process. Terminal
  evidence is immutable; explicit release cannot resurrect an old watch handle.

This tranche adds 25 registered Rust scenarios but remains **source present**:
Rust compilation, Rust tests, rustfmt, Clippy, stdio, native DFHack, and live-game
execution were not available in the editing environment. The self-contained JSON
Schema passed 73 local cases (34 accepted, 39 rejected), preserving all ten prior
query variants; three documentation JSON examples also validated. Those checks
are not Rust or repository qualification. Watches are foreground-only and not
durable, continuous-history proofs, mutation obligations, or admission evidence.
The separate `fortress.wait` tool, other runtimes, bridge methods, dependency pins,
production protocol map, and migration bead status are unchanged.

### Structured live queries and foreground change monitoring

The protocol-1.1 development `fortress.query` implementation now exposes typed
entity filtering, generation-checked inspection, graph traversal/dependency
analysis, grouped aggregates, lexical search, and embedded schema discovery.
These operate on the session's current published projection, not arbitrary
DFHack objects. Other runtime query surfaces are not implicitly upgraded.

The subsequent foreground-history slice adds four query variants behind the same
tool: `capture`, `changes`, `baselines`, and `release_baseline`. Its complete
contract and example workflow are in `docs/QUERY_HISTORY.md`.

- Capture collects the complete selected entity set across bounded pages at one
  exact anchor; no partial page is accepted as a baseline.
- Immutable baselines are process-local and session-owned, with explicit
  idempotent capture keys, game-tick deadlines, 256-row/256-KiB retained-row bounds,
  eight entries per session and 128 entries per process.
- Later queries return deterministic, generation-aware entered/left/changed result
  rows with before/after facts. Leaving a filter is not classified as death or
  deletion. Presence, epistemic class, source kind, and selected values remain
  significant; observation-bookkeeping-only refreshes are counted separately.
- Change pages bind the baseline and exact target snapshot. Reading or retrying a
  page never advances or consumes the baseline. Session, epoch, fork, regression,
  deadline, input, scan, row, and response-budget violations fail explicitly.
- The actual live query route renders the complete Agent Turn before publishing a
  capture or release. Failed response construction leaves retained state unchanged.
- Agent Turns expose the true comparison basis and a compact change summary.
  Advanced endpoint comparisons are explicitly partial temporal coverage, never
  proof of continuous intervening history. Required warnings and source coverage
  remain in the response budget.
- Ten registered Rust regression scenarios exercise the public query dispatcher,
  including an 8192-byte full Agent Turn path with multi-page changes.

This slice is **source present**. Rust compilation, Rust tests, rustfmt, Clippy,
repository qualification, stdio execution, and live-game execution were not run
in its editing environment, which had no Rust toolchain. No fresh receipt or
admission is claimed. Baselines are not durable, background watchers, obligations,
complete fortress history, or mutation authority. The bridge methods, dependency
pins, eleven-tool waist, and production protocol map are unchanged.

### Bounded query and graph execution

The 2026-09-13 query/graph additions are source-present functionality, not a fresh
qualification or admission. See `docs/QUERY_EXECUTION.md` and
`docs/GRAPH_QUERY_EXECUTION.md` for the contracts and limitations.

- Public world query entry points now bind continuation offsets to the exact
  fortress, epoch, sequence, game tick, state hash, filters, and ordering.
  Existing adapter callers use this path. Legacy query cursors require restarting;
  delta cursors are unchanged. Page width and byte budget may change between pages.
- Combined scan/predicate-work and aggregate query-identity limits prevent otherwise
  individually legal inputs from multiplying into unbounded evaluation work.
  Cursor digests are not authentication; every adapter request still needs authority.
- `dfmcp_world::graph_query` provides bounded deterministic multi-source traversal,
  outgoing/incoming/undirected interpretations, shortest-path edge witnesses,
  explicit depth frontiers, and bounded path reconstruction.
- Iterative SCC analysis identifies dependency cycles and their dependent blockers,
  orders the condensation prerequisite-first, and returns a stable entity order
  and longest unweighted dependency chain only when acyclic.
- Graph results carry exact source anchors, caller-supplied scope identities,
  projection/decision digests, and operation counters. They describe observed
  authorized projections, not complete-world absence, game walkability, timed
  production schedules, or permission to mutate.
- Twenty-one added Rust regression tests include an exhaustive directed
  three-vertex graph oracle. An independent Python design oracle passed 512
  exhaustive graphs and 200 seeded multigraphs in the editing environment.

No Rust compiler, Cargo, or rustfmt was available in that editing environment.
The Rust tests, workspace gates, native plugin, and live-game campaign were not
executed for these changes. The Python design check is not repository-gate or
Rust-execution evidence. Graph query modes subsequently gained protocol-1.1
MCP source integration as described above; broader live projection coverage
remains unfinished. The eleven-tool waist, production protocol map, dependency
pins, and migration bead status are unchanged.

### Agent-facing MCP

- Modern-only MCP 2026-07-28 through the exact-revision-pinned owned `fastmcp_rust` sibling.
- Frozen eleven-tool `fortress.*` waist.
- Deterministic laboratory mode with process-local pause-state effects.
- Authenticated protocol-1.0 read-only production server source.
- Explicitly unadmitted protocol-1.1 development server source.
- Separately gated jobs-only protocol-1.2 development server source.
- Separately gated coherent operations/1.3 development server source.
- Canonical Agent Turn Packet with identity, anchor, continuity, briefing, changes, attention,
  active work, affordances, recommendations, uncertainty, coverage, budgets, references, and typed
  recovery.
- An admitted Agent Turn exposes bridge protocol, entry, registry, decision, floor, server receipt,
  launch, ticket, and executable identities after successful V2 ticket consumption.
- Mutation-stage tools remain registered for the frozen waist but fail closed in live read-only
  modes.

### Protocol 1.0 live read path

- Out-of-process DFHack plugin using supported native protobuf RPC facilities.
- Loopback bearer-token authentication with bounded nonce and exact protocol handshake.
- Exactly two plugin methods: `Handshake` and `ReadObservation`.
- No remote-service flag and no arbitrary command, Lua, keyboard, path, direct memory-write, or
  mutation route.
- Safe-Rust wire codec with bounded frames, duplicate-field rejection, canonical protobuf checks,
  text budgets, nonce/version/generation fencing, and poisoned-stream behavior.
- Complete bounded citizen-roster reads with stable unit-ID order, optional names, and paused-world
  requirements for coherent multi-page assembly.
- Pagination-independent immutable observation capsules.
- Deterministic fortress/citizen projection with fact-level source digests and explicit coverage.
- Fortress identity derivation, observation epochs, heartbeats, ordinary advancement, restart and
  clock-regression resets, and world/version switch refusal.
- Read-only briefing, attention, query, explain, doctor, and wait surfaces.

This is implemented source, not a current admitted tuple.

### Protocol 1.1 retained-announcement read slice

Protocol 1.1 extends the same two-method bridge waist by adding bounded announcement request and
reply fields inside `ReadObservation`. It does not add `ReadAnnouncements` or any mutation method.

Implemented source includes:

- distinct protocol package, plugin name, bridge version, text and count limits,
  retained-window bounds, gap evidence, and complete-through-latest semantics;
- safe-Rust extension codec with canonical protobuf validation;
- combined citizen and announcement capsule assembly;
- transactional publication across citizen pagination and announcement continuation;
- complete retained-suffix versus incomplete historical-coverage separation;
- deterministic world projection, briefing, attention, and report-ID change summaries;
- read-only `GameAdapter` integration;
- single-publication bootstrap that acquires one combined capsule and replays that exact capsule
  into adapter initialization without another underlying bridge read;
- a two-dimensional primed replay contract over citizen pagination and announcement continuation;
- a separately named `dfmcp-live-v1-1-dev-server` preserving the eleven-tool waist;
- exact opt-in and rejection of production admission environment state;
- A1-A6 evidence, journal, native-receipt, probe, source-qualification, and mutation-test tooling.

The development runtime uses a distinct session namespace, exposes only read-only behavior, and
cannot consume a production ticket. It is useful for source testing and live evidence capture. It
is not production admission.

### R1-R5 and A1-A6 qualification machinery

- Protocol-1.0 native plugin qualification and R2-R5 acceptance tooling.
- Protocol-1.1 source-only qualification contract.
- Protocol-1.1 native receipt contract and issuer.
- Protocol-1.1 A1-A6 announcement acceptance contract with 43 exact cases.
- Secret scanning, append-only evidence journals, capture guidance, and fail-closed verifiers.
- Aggregate protocol-1.1 checker that now runs core isolation, transactional publication,
  single-read bootstrap, and development-MCP isolation checkers.
- Mutation tests that reject production-map widening, inherited admission, lost coverage, method
  widening, development guard removal, and mutation contamination.
- Local qualification digest inventory covering the complete protocol-1.1 source graph rather than
  only the wire and batch layers.

These mechanisms do not mean the current commit has passing native or live receipts.

### Compatibility and local custody

- Content-addressed exact compatibility registry.
- Deterministic promotion with expected-generation compare-and-swap and a single-writer lock.
- Resolver binding the complete registry digest, deployment manifest, and required entry ID.
- Owner-private monotonic floor with:
  - absolute path;
  - exact `0700` parent and exact `0600` file;
  - root/effective-user ownership;
  - no-follow reads;
  - exclusive initialization;
  - atomic fsynced compare-and-swap advancement;
  - monotonic sequence and digest chain;
  - preservation of every previously accepted entry ID.
- Deterministic authority-free admission doctor with fixed registry, floor, tuple, and optional
  server-artifact stages.

The floor is local anti-rollback custody, not distributed consensus, compatibility evidence,
revocation, or protection against compromise of the owner/root account.

### Protocol-bound V2 process admission

The previous ticket boundary did not carry the bridge protocol. A future protocol-1.1 compatibility
entry could therefore have reached the always-protocol-1.0 Rust runner. That protocol-confusion bug
is now closed by `architecture/live_admission_ticket_v2.json`.

The exact bridge protocol is bound across:

```text
deployment manifest
→ compatibility decision
→ launch record
→ single-use ticket
→ DFMCP_ADMITTED_BRIDGE_PROTOCOL
→ Rust admission context and retained provenance
→ final private runner lookup
```

Both launch and ticket digests cover the protocol. The production map currently contains only:

```text
1.0 → dwarf-fortress-mcp serve-live → private protocol-1.0 server
```

Protocol 1.1, unknown protocols, mismatched representations, and legacy V1 tickets fail before live
server startup. The development protocol-1.1 server rejects the production protocol marker at its
public API seam.

The launcher and Rust consumer additionally enforce:

- exact registry and monotonic-floor generation;
- exact entry fence and source commit;
- source-bound server receipt;
- loader-environment hygiene;
- no-follow executable opening;
- executable owner, mode, device, inode, size, and SHA-256;
- repeated registry/floor and descriptor revalidation;
- exact `0700` ticket directory and exact `0600` ticket file;
- process and expiry binding;
- single-use deletion before server startup;
- no path-based execution fallback;
- empty mutation capability.

The server-binary receipt source map now includes the V2 ticket contract, launcher, Rust consumer,
Agent Turn projection, and focused tests.

### Source and release custody

- Canonical clean-commit source-bundle contract.
- Git-object-derived deterministic archive with canonical file modes and metadata.
- Independent hostile archive verification without extraction.
- Atomic sibling-directory publication after complete verification.
- Stable no-follow repository-file reader.
- Repository integrity rejection for symbolic links, special files, invalid UTF-8, NUL corruption,
  oversized source text, machine-local placeholders, recovery debris, and files that
  change while being inspected.
- Local qualification and DSR release specifications.

A source bundle proves source/archive identity only. It does not prove compilation, tests,
compatibility, binary reproducibility, or runtime admission.

## Current registry and qualification state

The checked-in registry remains:

```json
{
  "schema_version": "dfmcp.live-compatibility-registry/1",
  "status": "no_admitted_live_tuples",
  "entries": []
}
```

Consequences:

- no Dwarf Fortress/DFHack/plugin/source/protocol/platform tuple is currently admitted;
- the production launcher cannot authorize a process from the checked-in registry;
- protocol 1.1, jobs-only 1.2 and operations/1.3 profiles cannot enter the production runner map;
- an empty-registry floor correctly preserves “no admissions”;
- old or external receipts do not qualify the current source generation unless they match every
  exact identity and are reviewed and promoted.

No fresh full latest-nightly qualification receipt is checked in for the final current head. The
present tranche is therefore described as implemented and source-bound, not as a newly qualified
binary or live configuration.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Agent surface | canonical Agent Turn, eleven-tool waist, read-only orientation, protocol-bound admission provenance, protocol-1.1 structured queries, endpoint change monitoring and foreground condition watches, jobs/operations profile integration | durable handoff, complete objectives/counterfactuals, empirical VOI/cost/confidence models |
| Protocol 1.0 | authenticated citizen read stack and private production runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained-announcement bridge, codec, publication, adapter, bootstrap, dev MCP, A1-A6 tooling | source receipt for current head, native/live receipts, production artifact, registry/floor/runtime admission |
| Jobs-only 1.2 | native job roster service, bounded client, canonical projection, shared queries/monitoring, development binary, native-source mock tests | Rust execution, real DFHack build, live campaign, admission, coherent combined citizen/job/inventory projection |
| Operations/1.3 | same-read jobs/buildings/items/attachment producer, closed client, atomic graph publication, shared queries/monitoring, production diagnosis, declared inventory allocation, optional synced observation archive, historical queries and registered development binary | Rust execution, actual DF headers/protobuf/native build, live campaign, snapshot paging, native material/path feasibility, admission |
| Compatibility | exact registry, promotion, resolver, monotonic floor, authority-free doctor | any current entry, evidence-bearing revocation, supported compatibility window |
| Process admission | V2 protocol-bound launch/ticket/environment/Rust dispatch, exact custody and executable checks | a fresh qualified current binary and successful admitted launch receipt |
| World | canonical snapshots, facts, deltas, bound query pagination, witnessed BFS, SCC/dependency analysis, graph/search/Merkle/checkpoint/ATP laboratories | native validation of current query/graph changes, broader live observations, admitted durable FrankenSQLite/FrankenFS/FrankenSearch/FrankenGraphDB backends |
| Intent | semantic actions, sealed plans, witnesses, idempotency, obligations, lab pause effect | any qualified live mutation family |
| Security | safe Rust, closed deps, secret scan, loader refusal, source/archive integrity, protocol-confusion defense | hostile-host resistance, signed provenance, external review |
| Release | local qualification, source bundles, server receipts, DSR specifications | current signed cross-platform release assets and install/rollback evidence |

## Explicitly absent

- no current admitted live tuple;
- no current supported or production compatibility claim;
- no admitted protocol-1.1, jobs-only protocol-1.2 or operations/1.3 runtime;
- no live mutation RPC;
- no pause/resume, dig, construction, labor, burrow, stockpile, work-order, military, keyboard, Lua,
  arbitrary command, arbitrary filesystem, or arbitrary network effect;
- no proof that the final current head passed every Rust qualification gate;
- no admitted production MVCC/WAL, effect-journal crash recovery, game-checkpoint custody or ATP deployment;
- no signed release provenance or hostile-host security claim.

## Next executable milestones

1. Run `./scripts/verify.sh` and `./scripts/qualify_local.sh` for one exact clean current head with no
   Rust gate skipped.
2. Produce the protocol-1.0 source-bound release-server receipt for that exact commit.
3. Build the exact protocol-1.0 native plugin against a named DFHack revision and run R1-R5.
4. Review and promote the first exact protocol-1.0 tuple, advance the deployment floor, run the
   authority-free preflight, and launch only through the V2 protocol-bound boundary.
5. Separately run protocol-1.1 source qualification, native qualification, A1-A6, and baseline R2-R5
   for one exact generation.
6. Qualify a protocol-1.1 production server artifact and review a protocol-1.1 compatibility entry.
7. Only after all protocol-1.1 evidence exists, add an explicit production runner to the V2 protocol
   map, advance the floor, and execute through a fresh protocol-bound ticket.
8. Validate jobs-only 1.2 and operations/1.3 against Rust and a real DFHack build; expand citizen,
   map/path and requirement coverage only under separately versioned coherent observation contracts.
9. Design pause/resume only after the widened read path is stable; mutation must be separately
   versioned, witnessed, idempotent, reconciled, and disposable-fort qualified.

## Status rules

1. This file, exact receipts, the current registry, and local floor bytes define status.
2. Source presence is not qualification, admission, support, or production evidence.
3. Development execution is not production admission.
4. A tuple is admitted only while its exact entry exists in the current registry generation.
5. A deployment is admitted only when its trusted floor matches that registry generation.
6. A protocol can execute in production only when the V2 production map contains its reviewed
   runner and every launch/ticket representation agrees.
7. A doctor report is diagnosis, never authority.
8. A server receipt qualifies one executable, never a bridge or game session.
9. A ticket authorizes one exact process/protocol start and is single-use.
10. Negative evidence may reject a claim but cannot certify success.
11. Derived indexes, attention, recommendations, memory, and counterfactuals are never more
    authoritative than canonical source evidence.
12. Unit tests do not substitute for disposable-fort evidence where Dwarf Fortress behavior
    matters.
