# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with implemented but unadmitted read profiles through citizen-inclusive spatial/1.8,
optional archive-bound durable foreground watches, and an explicitly unadmitted pause-control/1.7
development slice. No live tuple is currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- separate unadmitted jobs-only 1.2, operations/1.3, paged operations/1.4, map/1.5 and spatial/1.6 development profiles;
- a citizen-inclusive spatial/1.8 source path that captures strict citizens, jobs, buildings, items and bounded terrain during one native suspension, then publishes one combined anchor;
- optional paired spatial/1.8 observation and watch journals preserving monitoring definitions, outcomes, cancellation and release across restart without claiming downtime continuity;
- a pause-control/1.7 bridge/client/development MCP runtime supporting prepare, durably coordinated commit, receipt-verified reconciliation and bounded foreground recovery for `Pause { paused }` only;
- a private hash-chained pause-effect coordinator journal whose source records `CommitStarted` before dispatch and terminal evidence before acknowledgement;
- offline read-only pause-effect discovery without a bridge connection or mutation authority;
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

### Operational situation and attention in the live spatial loop

`docs/SPATIAL_SITUATION.md` describes the fixed `dfmcp.spatial-situation/1` derivation integrated into
spatial/1.8 session opening, observations and queries. Seven ordered rules summarize observed citizen,
job, building, item and visible-terrain signs without inferring causal blockers, global safety,
starvation or threat exposure. Native field provenance, coherent source digest, tick, type and
consistent known presence are required; unknown and redacted inputs remain explicitly unestablished.

Open/observe/wait and query `mode="situation"` include all signal counts and at most two detailed
attention groups with generation-checked, exact-anchor inspection requests. Ordinary tactical queries
retain one compact priority signal, omitted/unknown group counts and a situation-detail request.
Observe/wait add at most four endpoint-count changes, with explicit omissions and no cross-epoch,
continuous-history, acknowledged-client-cursor or game-effect-success claim. Situation queries acquire
no extra native capture and do not sample watches. Fenced sources and expired Query grants suppress
current findings; historical and archive-only responses do not inherit live operational attention.

Watch result-budget reservation now uses the actual final renderer's watch-specific metadata shape,
including metadata added by the post-refresh callback, rather than unrelated baseline-comparison
metadata. Full output is still checked before watch root publication or durable checkpoint append.
No native protocol, top-level tool, dependency, mutation authority or production admission changes.

Fifteen Rust scenarios are registered: nine projection tests and six actual live/archive handler
tests, including compact/detail navigation, unknown/provenance and hidden-data cases, stale links,
authority expiry, unchanged watch evidence, 8,192-byte pagination and budget-refused registration.
**None has been compiled or executed here**: Rust, Cargo and rustfmt are unavailable. Six independent
Python reference watch-packet shapes fit an 8,192-byte budget, measuring 6,553–6,590 bytes. That sizing
reference does not execute Rust serialization, rules, watch transitions, MCP or native/live DFHack.
No Rust, runtime or whole-repository qualification is established by this increment.

### Restart-safe historical endpoint comparisons

`docs/HISTORICAL_CHANGES.md` describes the new `historical_changes` query in both journal-backed
live and archive-only spatial/1.8 sessions. It compares complete bounded entity selections at two
exact record numbers/digests without creating a process-local baseline. After restart, retained
record references remain usable; continuations remain session-bound and must be restarted.

The operation shares the existing baseline selection and comparison implementation. It separates
selected-set entry, departure and semantic changes by entity ID/generation, discloses provenance-only
refreshes, and preserves unknown/absent/omitted/redacted/source distinctions. ID reuse is a separate
departure and arrival, not an update to the old identity. Cross-epoch, reversed, forked or missing
endpoints fail instead of manufacturing a change history. Before/after rows are emitted only after
both complete selections have been acquired and counted.

Both record replays, both selections, comparison and rendering share one cooperative wall-time
allowance. Full historical Agent Turn metadata, both record witnesses, pagination and current live
watches are reserved before replay. Limits are 256 selected rows/256 KiB per endpoint, 64 acquisition
page attempts/two million source-row visits per endpoint, and 1..128 whole changes per output page.
Current Query authority, cancellation and journal custody are checked before replay and again before
return. Selected record bytes are reverified. The operation neither advances current state nor
samples watches, acquires a native capture, allocates a baseline, repairs history or dispatches effects.
A live source failure still permits verified history comparison. Archive schema discovery now has
thirteen executable variants; nested historical comparisons are not permitted.

Sixteen new logical Rust scenarios are registered: nine shared-comparison tests and seven actual
live/archive handler tests covering pagination, reopen, generation/presence semantics, epoch fences,
authority, unchanged watches, output refusal and same-length record corruption. **None has been
compiled or executed here**: Rust, Cargo and rustfmt are unavailable. The independent Python request
contract checker passed 66 cases (12 accepted, 54 rejected) in explicit `--envelope-only` mode. It
validates the new record/page envelope only, not delegated entity selectors, record existence/order,
digest agreement, comparison, filesystem custody, Rust pagination, MCP or native/live behavior.
Executed script/schema bytes match their committed Git blob identities. Executed script SHA-256:
`67ef27c59d25a5f83203b505c5e80f81fb487af178288f161622029cbdec69a9`.
No dependency, native protocol, production runner, mutation authority or admission is changed.

### Offline spatial/1.8 archive bootstrap and exact-record analysis

`docs/SPATIAL_ARCHIVE_RECOVERY.md` documents `fortress.open_session(recovery_only=true)` on the
existing spatial server. It opens an existing observation journal without DFHack, bridge credentials
or an endpoint. Query and optional Doctor grants are freshly scoped to the verified archive
fortress; Observe, mutation authority, journal creation/repair and watch recovery are refused.
The operator-configured path keeps the existing exclusive lock and private-file custody rules,
using a read-only descriptor whose write, flush, sync and truncate operations explicitly refuse.

The normal fixed-profile replay reconstructs the exact citizen, operations and terrain generation
history. Empty archives cannot bootstrap a world. Requested region/acquisition bounds remain in
force. Cached queries and diagnostics recheck current authority and custody; exact historical reads
also reverify their record bytes. No archive bytes are rewritten or migrated.

Archive queries reuse the ten stateless graph, terrain, inventory and workforce analysis variants,
plus bounded history listing and exact-record historical queries. Their Agent Turns explicitly
mark all facts historical and current freshness unproved. Older-record workforce analysis and
route drill-downs stay pinned to the selected record without replacing the session's latest
retained observation. Whole-row output includes full historical metadata within the response budget.
Archive-only schema discovery excludes watch/baseline mutations and uses session/head-bound history
continuations. No live acquisition or monitor evaluation is routed, even with injected Observe
capability. Persisted watch evidence is not loaded; empty active work is scoped to this archive
session and does not prove that no persisted monitoring or game actions exist.

Fourteen Rust scenarios are registered: six private-file adapter integration tests and eight archive
bootstrap/actual-MCP-handler tests. **None has been compiled or executed here**: Rust, Cargo and
rustfmt are unavailable. An independent Python JSON-size calculation passed 128 boundary cases for
exact-record route wrappers; every wrapper was smaller than its original anchor-bound request
(maximum growth -22 bytes). This limited check does not execute Rust serialization, archive replay,
filesystem custody, query engines, MCP or DFHack. No full qualification, native/live evidence,
production runner, admission, dependency or game-effect capability is changed by this increment.

### Coherent workforce candidates and simultaneous capacity planning

`docs/WORKFORCE_PLANNING.md` describes `workforce_candidates` and `workforce_plan` through the actual
spatial/1.8 `fortress.query` dispatcher. Previously disconnected candidate source is now integrated
with schema discovery, exact anchors, current authority and complete Agent Turn/active-watch budgets.

The shared adapter analysis reads observed sparse skills, job availability and terrain from one
capture. The existing integral allocator gives each citizen one worker slot across all declared
roles, with deterministic rerouting and a distinct-worker shortage certificate. Requests allow
1..16 demands and at most 128 simultaneous slots. The objective is maximum filled slots, not global
skill/travel/priority optimization, native labor eligibility, reservations or actual labor changes.
Unknown skill keys do not recruit every novice. The occupied-citizen endpoint exception relaxes only
unit occupancy, never hidden terrain, walls, liquid, buildings or unknown walkability.

Whole-row continuations bind the session, full anchor, model and work allowance. Current watches
remain attached without sampling. In live mode workforce queries use the latest retained capture;
the subsequent archive-only path described above also supports exact historical workforce analysis.
No native protocol, dependency, top-level tool, mutation capability, compatibility admission or
production map is changed.

Fifteen new Rust tests are registered: eight adapter tests (including all 512 three-worker/three-role
skill graphs against an exhaustive assignment oracle), four query tests and three actual MCP-handler
tests, including 8,192-byte pages with current watches. **None has been compiled or executed here**:
Rust, Cargo and rustfmt are unavailable. The checked-in Python reference passed 88 schema cases
(25 accepted, 63 rejected) and 4,096 independent mathematical allocation-oracle cases. Its executed
SHA-256 is `ff87fdd47943ad6b7dc88ec9cec7148fb0490280cf8ff597995973c050268d04`; committed script and
schema blob identities match the tested bytes. These checks do not execute Rust, the production
allocator, terrain analysis, MCP, native DFHack or a live fortress, and establish no qualification.

### Durable foreground monitoring bound to spatial/1.8 observation history

`docs/DURABLE_WATCHES.md` describes the new restart-safe monitoring path. The existing watch queries
retain their schemas and foreground execution model; no top-level tool, native bridge method,
dependency, game effect, or production admission boundary is added.

- Operator-only `DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL` selects a separate private file paired with the
  required spatial/1.8 observation journal. Query and Observe authority, exact-mode custody and an
  exclusive file lock are required; a process-local observation universe cannot back recovery.
- Registration, changed samples, terminal outcomes, cancellation and release are persisted as bounded
  hash-chained checkpoints. Complete response rendering precedes append/sync; durable sync precedes
  in-memory root publication and acknowledgement. Failed rendering does not commit a transition.
- Startup replays/syncs the fresh observation first, then verifies the exact watch profile/archive
  identity and every stored creation/evaluation/checkpoint anchor against retained observations.
  A wrong archive or missing required observation fails closed without migration or repair.
- Recovered definitions retain their original deadlines, prior samples and evidence links, but use
  fresh session-bound handles. Unfinished stability resets; bootstrap is not counted as a sample.
  `watches` rediscovers the handles and `await_watch` obtains the fresh evidence needed to continue.
- Epoch/identity discontinuity invalidates unfinished watches. Passed deadlines expire rather than
  move forward. Current grants and game-tick horizons are not widened by historical configuration.
  Previously terminal outcomes remain historical, even at an identical bootstrap anchor.
- Repeated unchanged reads do not append or manufacture samples. Historical queries never evaluate
  current watches against past game facts. Scoped persistence metadata does not make query baselines,
  ordinary results, plans or game effects durable. Session drop releases ownership, not durable intent.
- Partial writes and uncertain sync fence publication. A complete uncertain frame may recover on
  reopen; an incomplete watch frame is refused unchanged. There is no automatic tail repair, pruning,
  rotation or compaction. Limits are eight watches/session, 1 MiB/checkpoint, 64 MiB/4,096 checkpoints.
- The actual spatial startup/query paths include recovery metadata and active work. Corrupt watch
  storage is not mislabeled as output overflow when the usual active-work error projection fails.

Twenty-five new logical Rust tests are registered: seven binary-journal, thirteen durable-watch,
two configuration and three actual spatial startup/MCP-handler scenarios. They retain the existing
watch tests and cover restart stability, cancellation/release, current authority, paired history,
output refusal, storage faults, historical outcome labeling and fixed framing vectors. **None of
these Rust tests has been compiled or executed here**: Rust, Cargo and rustfmt are unavailable.

The checked-in independent Python framing reference passed 740 checks, including 363 single-byte
corruptions and 361 incomplete prefixes. Its tested source SHA-256 is
`6171594ba703f1b95a458826edcf6acfdf0af002ebe4ce3d8324d99ab3dfac9a`.
The committed script bytes were checked against that executed source. These checks use opaque
payloads and do not execute the Rust serializer, watch state machine, filesystem custody or MCP
runtime. No Rust/Clippy/stdio, power-loss, native/live-game, full-repository or admission claim is
made. This increment supplies development source and independent framing-reference evidence only.

### Pause recovery, verified receipts and bounded foreground reconciliation

The control/1.7 runtime now implements a complete discovery-to-reconciliation source path under the
existing eleven-tool interface. See `docs/LIVE_CONTROL.md` for exact request and evidence semantics.

- `fortress.open_session(recovery_only=true)` opens an existing private journal with Query authority,
  a read-only descriptor, no bridge credentials or connection, and no creation or repair. Query,
  explain and doctor can expose stored evidence while DFHack is unavailable. Recovery sessions
  cannot promote themselves to live sessions or invoke prepare, commit or live wait.
- `fortress.query` lists durable effects without prior knowledge of their keys. Whole-record pages
  and state filters are bounded; continuations bind the session, journal incarnation, exact head,
  filter and offset. Authority and custody are rechecked even for terminal/idempotent lookups.
- Native preparation identity now survives a later commit: desired pause and expected tick are
  immutable, separate from actual observed pause and tick. Duplicate preparation after time advances
  returns the retained token. Setter failure reports the actual opposite pause state, not the target.
- New live prepare tokens and terminal receipts are independently recomputed in Rust. Both applied
  and not-applied terminal results require a complete matching receipt and consistent observation.
  A known native prepare or interrupted commit without a terminal receipt leaves a durable attempt
  indeterminate; it is never interpreted as proven non-application.
- The wire requires explicit outcome/pause/tick fields for known records, exact token/receipt lengths,
  and fences every failed wire call, including unread oversized frames. TCP connect and handshake
  share one deadline allowance.
- `fortress.wait` runs one foreground reconciliation pass over 1..16 selected keys. It validates the
  complete selection and reserves the complete worst-case response before bridge work, processes keys
  canonically, skips prepared/terminal records, and uses a query-only recovery transport interface.
  A shared wall-time allowance and stop-on-first-error preserve earlier durable progress while
  reporting remaining work as deferred. No mutating commit is dispatched or retried.
- Previously persisted terminal records are not migrated or newly qualified by the live receipt
  validator. Historical evidence does not prove current pause state. Hashes are identity checksums,
  not signatures or authority. Synchronous filesystem calls have no claimed hard cancellation bound.

The offline recovery increment registered 14 Rust tests. The receipt/bounded-pass increment adds 19
Rust tests (10 coordinator, seven wire and two response-reservation cases) and extends the existing
MCP-handler tests to refuse live wait in offline mode, including injected clock grants. **None of
these Rust tests was compiled or executed here**: Rust, Cargo and rustfmt are unavailable. No whole
repository qualification is claimed.

The changed actual native producer passed `scripts/test_live_control_outcomes_native_mock.py` under
both GCC and Clang with C++17, `-Wall -Wextra -Werror -pedantic`: 75 C++ assertions and three independent
Python SHA-256 comparisons per compiler. Tested producer SHA-256:
`80bb0c427ce94ecee41b86c52ea721e979cd15b1f371e0a4a7a27f5aa410a6ff`.
The test injects no-op setters, invalid post-set clocks and post-set exceptions, plus time-advanced
prepare replay, duplicate suppression, generation loss and authentication refusal. Independent Python
also checked the Rust test-vector constants. These are mock-interface/source checks, not a real
DFHack/generated-protobuf build, a live campaign, or native qualification. Protocol 1.7 remains
unadmitted and all production admission boundaries are unchanged.

### Coherent citizen + operations + terrain spatial/1.8 source

`docs/LIVE_SPATIAL_CITIZENS.md` describes the citizen-inclusive read profile. It removes the
cross-observation ambiguity between the old citizen profile and spatial/1.6: the strict citizen
roster and the existing operations/terrain payload are serialized during one native DFHack RPC
suspension and transferred as one immutable retained capture.

- `dfmcp_spatial_v1_8` keeps the two-method `Handshake`/`ReadObservation` waist. Its request adds an
  explicit citizen bound while preserving fixed jobs/buildings/items/terrain/page/byte bounds.
  Native retained-cache ownership binds both the requested terrain region and citizen bound.
- A strict complete citizen component is bounded to 4,096 records, sorted by nonnegative native unit
  ID, and rejects duplicates, non-citizens and residents. Observed fields include bounded visible name,
  race, profession, position, alive/sane/active/visible and developmental status. Subsequent readiness
  and skill source fields do not establish qualified labor eligibility or full health coverage.
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
- A separate sealed spatial/1.8 observation codec supports append-before-publication, exact restart
  replay and stateless historical queries. The new paired watch journal is described above; neither
  archive constitutes continuous game history, a game checkpoint, or mutation authority.
- The profile still does not establish complete needs/health, labor eligibility, complete unit
  navigation, non-citizen unit details, outside-region terrain, native material requirements or any
  game effect. Pause-control/1.7 remains separate and gains no authority from a spatial observation.

Four initial Rust integration scenarios are registered for worker joins, non-citizen uncertainty,
citizen-only advancement/generation continuity and strict-roster corruption. A reproducible native
mock harness is checked in at `scripts/test_live_spatial_citizens_native_mock.py`, covering immutable
retained bytes, strict citizen limits, hidden-terrain noninterference, generation invalidation and
fixed method registration.

Those spatial/1.8 Rust/native checks have **not been executed in this editing environment**. The
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
  production admission is introduced by the spatial/1.6 history increment.
  The subsequent spatial/1.8 durable-watch path is separate and described above.

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
- After one commit dispatch, a complete bridge outcome includes observed pause state and a
  generation-bound full 32-byte SHA-256 receipt. The coordinator verifies that identity and must sync
  `VerifiedApplied` or `VerifiedNotApplied` before acknowledging a terminal result. Missing or invalid
  terminal evidence remains unresolved; failed terminal durability is `EffectIndeterminate`.
- Rust-process restart replays the exact durable transition chain. `CommitStarted` and
  `Indeterminate` remain reconciliation-required. Read-only reconciliation may reconnect to the
  bridge, but commit is not redispatched as recovery work.
- If the bridge retains a complete matching receipt, reconciliation records its result. A merely
  known key, missing receipt, generation loss or unknown key cannot prove a terminal outcome. The same
  effect is never reported safe to retry; a new observation and new plan/idempotency key are required.
  A merely `Prepared` effect may commit once only while its bridge generation still agrees.
- World load/unload advances bridge generation and clears native retained records, preventing native
  idempotency state from crossing a world boundary. The generation is also bound into tokens and
  receipts.
- The safe-Rust client binds only the four fixed methods and profile identity. The development MCP
  runtime retains one isolated control/recovery session family, grants only `ControlClock` at
  reversible risk in live mode or `Query` in offline mode, preserves the eleven top-level tool names,
  and refuses every non-pause mutation surface.
- Agent Turn metadata explicitly keeps `runtime_admitted=false` and `mutation_admissible=false`; the
  unadmitted development effect switch is represented separately. Protocol 1.7 remains absent from
  the production runner map and the compatibility registry remains empty.

The earlier native source was compile-checked in the editing environment against explicit mock
DFHack/protobuf interfaces with both GCC and Clang under C++17 and warning-denied flags. A local
state-machine mirror exercised prepare replay, one-shot commit, duplicate suppression, query
reconciliation, generation reset, and fixed method registration. Independent Python `hashlib`
calculations matched the generation-bound token and receipt identities after an embedded-NUL domain
separator bug was corrected. `scripts/test_live_control_native_mock.py` is checked in as the
reproducible baseline mock-native harness; current outcome-fix evidence is recorded above.

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
as described in their dedicated documentation. Optional spatial/1.8 watch persistence is now present;
query baselines remain process-local. These derived layers never grant mutation authority.

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
| Agent surface | Agent Turn envelope, eleven-tool waist, structured queries, restart-safe spatial/1.8 watches, production/spatial analysis, bounded durable control-effect discovery and recovery passes | complete durable handoff, durable baselines, complete counterfactual/VOI models |
| Protocol 1.0 | authenticated citizen read stack and production-runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained announcements and development runtime | current native/live admission chain |
| Jobs/operations/map/spatial | coherent bounded development reads through citizen-inclusive spatial/1.8, same-anchor strict-citizen worker joins, exact spatial/1.8 history and paired durable watches | Rust qualification, real DFHack campaigns, complete needs/health/labor coverage, full unit navigation, production admission |
| Control/1.7 | pause prepare/commit, receipt-verified live reconciliation, bounded wait, mandatory private journal and offline Query-only recovery | Rust qualification, real DFHack build, crash/disposable-fort campaigns, production admission, any other live effect family |
| World | canonical snapshots, deltas, query/graph/path/allocation, operations history and durable spatial/1.6 and spatial/1.8 observation replay | admitted production durable backend and complete fortress coverage |
| Intent/effects | sealed plans, in-memory dispatcher laboratory, bridge-backed pause effect with durable pre-dispatch/terminal coordinator states | qualified/admitted effect journal, leases/checkpoints tied to live commits, dig/build/labor/etc. live effects |
| Security/admission | closed dependencies, protocol-bound tickets, monotonic floor machinery | admitted current tuple, hostile-host resistance, signed release provenance |

## Explicitly absent

- no current admitted live tuple;
- no supported production compatibility claim for protocols 1.1 through 1.8;
- no admitted live mutation capability;
- no live dig, construction, labor, burrow, stockpile, work-order, military, checkpoint, Lua,
  arbitrary command, keyboard, filesystem, or network effect;
- no Rust-qualified/native-qualified/live-qualified control effect journal or power-loss evidence;
- no Rust-qualified durable-watch implementation, continuous downtime monitoring, or automatic watch-journal compaction/repair;
- no proof that the current head passed every Rust qualification gate;
- no signed cross-platform release provenance.

## Next executable milestones

1. Run full Rust verification/qualification for the exact current clean head, including spatial/1.8
   citizen coherence, paired durable watches and their actual handler tests, the durable pause-effect
   journal, receipt-verified coordinator, control wire, bounded wait and all registered recovery tests.
2. Compile spatial/1.8 and control/1.7 against named real DFHack/protobuf generations and execute
   disposable-fort read/control campaigns for the exact plugin bytes.
3. Exercise spatial/1.8 with real citizen/job churn, non-citizen workers, large rosters, hidden terrain,
   immutable multi-page transfers, route/allocation analysis and foreground watches, including paired
   journal restart/failure cases, before treating its cross-domain joins as live-qualified evidence.
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
