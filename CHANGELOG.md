# Changelog

All notable changes will be documented in this file. Until the first stable protocol release,
versions describe design, implementation, and compatibility milestones rather than production
readiness.

## [Unreleased]

### Added

- Executable read-only excavation floor-goal monitoring over unchanged map/1.5:
  strict capture decoding, distinct advancing-tick sampled stability, fixed
  deadlines, unknown terrain and source/clock invalidation without native mutation.
- Durable `track_excavation.py` start/sample/inspect/cancel workflow with complete
  native evidence, read-intent-before-connection synchronization, file/parent
  synchronization before acknowledgement and restart-safe interruption handling.
  Cancellation stops only the monitor; no native effect obligation is cleared.
- Thirty-five actual Python/loopback/POSIX/subprocess tests pass and four weakened
  implementations are rejected by regression assertions. Complete Agent Turn
  output is bounded. This is not Rust/MCP, live DFHack or power-loss qualification;
  see `docs/EXCAVATION_PROGRESS.md` for the standalone workflow and evidence limits.

- Policy-guarded mining control through the separate `dfmcp-dig-control-dev-server`:
  observe, prepare, exact-plan/review-seal commit, query reconciliation and native
  preparation retirement over the existing Rust session/coordinator and six-method wire.
- Actual host spatial-lease verification, protected shared-block footprints and
  explicit checkpoint policy. Required is the default and refuses without a real
  verifier; only trusted disposable-fortress configuration permits uncheckpointed
  development designation. The query-only recovery server remains unchanged.
- Complete Agent Turn output reservation and pending-work discovery, inherited
  Asupersync blocking ownership and one-attempt original-connection dispatch.
  Thirty-two new core/policy/MCP/runtime Rust tests are registered but uncompiled
  and unexecuted. The published query-schema suite passed 28 valid and 102 invalid
  cases; this is not Rust or native execution. See `docs/DIG_CONTROL_MCP.md`.

- Durable dig/1.16 Rust coordination with intent/dispatch-before-native sync,
  terminal-proof-before-acknowledgement sync, non-restorable commit permission,
  cross-key unresolved-work fencing, fixed recovery modes and typed offline
  record discovery. Require supervising runtime guards at native edges.
- Linux private-file backing with exact owned modes, exclusive locking,
  descriptor-pinned opens, append-only checks, file/parent sync and strictly
  read-only offline replay. No tail repair, native wire or admission changes.
  Thirty-two coordinator/storage tests are registered but uncompiled/unexecuted;
  the passing independent Python framing checks are not Rust or filesystem
  execution. Subsequent ownership, recovery and control integrations are documented
  in `docs/DIG_SESSION.md`, `docs/DIG_RECOVERY_MCP.md` and `docs/DIG_CONTROL_MCP.md`.

- Typed dig/1.16 Rust capture, sealed-plan and native-effect validation, plus a fixed
  six-method RPC client with full-halo/shared-block authorization, source/region
  pinning, nonrenewable connection budgets and same-connection one-attempt commit.
  Query/replayed preparations cannot grant dispatch. Twenty-two Rust regression
  groups are registered but UNCOMPILED AND UNEXECUTED; independent Python checks
  reconstruct the four existing native fixture identities only. The subsequent
  coordinator/storage increment is described above; neither increment changes
  native wire or grants production admission. The subsequent MCP route is described above.

- Read-only work-order-progress/1.12 capture and MCP monitoring: complete selected
  presence, approval/activity flags, remaining-work counters, conservative finite
  template recognition, exact-witness foreground refresh and epoch/reset-safe
  endpoint comparisons. No insertion receipt or counter is promoted to completed
  goods. Includes 24 unexecuted Rust regression groups, 1,358 executed C++ assertions
  on each of GCC/Clang with SDK/protobuf doubles and UBSan, three rejected mutants
  per compiler, and 4,800 independent Python phase cases. See
  `docs/WORK_ORDER_PROGRESS_MCP.md`; this is not production or live-game admission.

- Native pause preparation fencing, bounded token lifetime, map-incarnation invalidation
  and setter/readback exception confinement. The actual handler regressions pass on GCC
  and Clang with native/protobuf doubles, not a real plugin or live game qualification.
  See `docs/NATIVE_PAUSE_FENCING.md` and `CHANGELOG.d/native-pause-fencing.md`.

- Operational situation briefings and deterministic attention in the live spatial/1.8 loop:
  seven native-evidence rules, explicit unknown counts, bounded generation/anchor-bound inspection
  links, compact tactical alerts and `fortress.query` mode `situation` for detailed orientation.
  Observe/wait include bounded endpoint-count changes without inventing causality or event history.
- Current-authority and historical-source separation, watch-specific response reservation through
  the final renderer, and fifteen registered Rust projection/actual-handler scenarios. Those tests
  remain uncompiled/unexecuted; six independent reference JSON packet shapes fit 8,192 bytes.
  Behavior, limitations and evidence status are documented in `docs/SPATIAL_SITUATION.md`.
- `historical_changes` in journal-backed live and archive-only spatial/1.8 sessions: exact record
  pairs, complete selected endpoint projections, generation-aware entered/left/changed rows,
  separate provenance refresh counts, and whole-change pagination without process-local baselines.
  Replay/comparison share one cooperative deadline, current watches are preserved without sampling,
  and full historical Agent Turn metadata is reserved before replay. Sixteen registered Rust
  scenarios remain uncompiled/unexecuted; 66 Python record/page-envelope checks passed without
  validating delegated selectors or runtime semantics. See `docs/HISTORICAL_CHANGES.md`.
- Offline spatial/1.8 archive sessions through `fortress.open_session(recovery_only=true)`, using
  the existing operator-configured observation journal without DFHack, credentials or a connection.
  Fixed-profile replay retains exact entity generations and historical anchors under Query and
  optional Doctor authority; read-only custody refuses creation, repair and write operations.
- Archive-only graph, terrain, inventory and workforce analysis, bounded history discovery and
  exact-record historical queries. Route drill-downs stay pinned to their source record. Full
  Agent Turns label archived evidence as historical and never currently fresh; watches, baselines,
  live acquisition and effects remain unavailable. Fourteen registered Rust scenarios are unrun;
  the independent JSON wrapper-size check passed 128 cases. See `docs/SPATIAL_ARCHIVE_RECOVERY.md`.
- Coherent `workforce_candidates` and `workforce_plan` queries wired into the spatial/1.8 MCP
  dispatcher and schema discovery. Observed skill/readiness and terrain approaches feed the
  existing integral allocator, with capacity one per citizen across 1..16 simultaneous demands,
  at most 128 worker slots, and distinct-worker shortage certificates. These are read-only models,
  not labor assignments, reservations, native eligibility proofs or globally quality-optimal plans.
- Strict occupied-endpoint and unknown-skill handling, whole-row capture/session/model-bound
  pagination, current active-watch reservation and fifteen registered Rust tests. The independent
  Python reference passed 88 schema cases and 4,096 mathematical oracle cases without executing
  Rust. Usage, defaults, exclusions and exact evidence limits: `docs/WORKFORCE_PLANNING.md`.
- Optional restart-safe foreground condition watches for spatial/1.8, paired with its exact
  observation archive through operator-only `DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL` configuration.
  Registration, samples, terminal outcomes, cancellation and release use bounded hash-chained
  checkpoints with render-before-sync and sync-before-publication ordering.
- Recovery under fresh session-bound watch handles, retained definitions/deadlines/evidence,
  reset unfinished stability, explicit downtime uncertainty, current-authority and horizon checks,
  and historical labeling for old terminal outcomes. Existing watch queries and eleven-tool
  interface remain unchanged. Baselines and game effects are not made durable by watch metadata.
- Twenty-five new logical Rust durability/configuration/actual-handler tests and the executable
  independent Python checkpoint framing reference. The reference passed 740 checks; Rust tests
  remain uncompiled and unexecuted here. Full workflow, limits and evidence: `docs/DURABLE_WATCHES.md`.
- Foreground condition watches integrated into protocol-1.1 `fortress.query`:
  `watch`, `poll_watch`, `await_watch`, `watches`, `cancel_watch`, and
  `release_watch`. Typed predicates retain generation fences, failure guards,
  game-tick deadlines, sampling cadence, distinct-observation stability, and
  immutable terminal evidence. Unknown facts never become false under negation.
- A one-observation await path with pre-I/O handle/anchor validation, Query plus
  Observe authorization, and post-refresh reauthorization. It performs at most
  one adapter observation, never controls game time, and skips terminal reads.
- Active-watch projection in query Agent Turns, output-budget reservation,
  render-before-publication state changes, and source-stale metadata/cancellation
  after bridge poisoning. This is not background work or durable mutation state.
- Twenty-five registered Rust watch/refresh/packet scenarios, including an
  8192-byte integrated response path. JSON Schema validation passed 73 local
  cases and preserved every prior definition and ten original query variants;
  the embedded schema now has sixteen variants. Rust execution remains unverified.
- The condition-watch workflow and limitations in `docs/CONDITION_WATCHES.md`.
- Foreground query baselines and endpoint change monitoring through the existing
  protocol-1.1 `fortress.query` tool: `capture`, `changes`, `baselines`, and
  `release_baseline`. Captures retain complete bounded entity selections, carry
  exact anchors and game-time deadlines, and are isolated by session.
- Generation-aware entered/left/changed results with before/after facts, immutable
  baselines, target-bound whole-change pagination, explicit endpoint-only history
  coverage, and separate accounting for selected-view provenance refreshes.
- Transactional response rendering before baseline creation or release. Failed
  full Agent Turn construction does not create, consume, advance, or release a
  baseline. Agent Turns carry the true comparison basis and compact change summary.
- Ten registered public-dispatcher history tests, including full 8192-byte response
  pagination, filtering, generation reuse, presence changes, retries, session/epoch/
  deadline fences, retention bounds, and failed-publication recovery. These tests
  have not been compiled or executed in the editing environment.
- Foreground monitoring workflow and limits in `docs/QUERY_HISTORY.md`; embedded
  query-schema discovery includes the original ten structured query variants.
- Bounded `dfmcp_world::graph_query` reference APIs for canonical multi-source BFS,
  outgoing/incoming/undirected traversal, edge-revision path witnesses, explicit
  depth frontiers, and bounded path reconstruction.
- Iterative strongly connected components, prerequisite-first condensation order,
  cycle-member/dependent blocker diagnosis, and deterministic longest unweighted
  dependency chains for acyclic projections. Results retain exact source and
  authorization-scope identities without granting authority or claiming game
  walkability, complete-world absence, or timed production scheduling.
- Twenty-one query/graph Rust regression tests, including exhaustive directed
  three-vertex graph checks against independent closure/distance oracles, plus
  executable pagination coverage in the existing query truth-table suite.
- Query and graph execution contract documentation in `docs/QUERY_EXECUTION.md`
  and `docs/GRAPH_QUERY_EXECUTION.md`.
- Protocol-bound V2 production admission contract
  (`architecture/live_admission_ticket_v2.json`). The exact bridge protocol now travels from the
  deployment manifest through the compatibility decision, launch record, single-use ticket,
  `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, Rust admission provenance, and final private runner lookup.
  Launch and ticket digests both cover the protocol. The production map currently contains only
  protocol 1.0; protocol 1.1 and unknown protocols fail before live-server startup.
- Protocol-1.1 retained-announcement read generation with a distinct protobuf package, plugin,
  bridge version, source qualification contract, native receipt contract, A1-A6 acceptance
  contract, evidence journal, diagnostic probe, source qualification contract, and development MCP runtime.
- Canonical retained-announcement batches with strict report-ID ordering, bounded UTF-8 text,
  retained-window oldest/latest identities, explicit gap evidence, continuation progress, and
  complete-through-latest semantics without a complete-history claim.
- Transactional protocol-1.1 publication across citizen pagination and announcement continuation.
  No combined capsule is published until the citizen roster and configured retained suffix are
  complete and every page reproduces the same observation state.
- Single-publication protocol-1.1 bootstrap. One complete combined capsule now supplies fortress
  identity, source digest, initial world projection, and adapter bootstrap through a primed replay
  layer without a duplicate underlying bridge read.
- Explicit two-dimensional primed replay over citizen pagination and announcement continuation,
  with cursor, projection, limit, source-manifest, and final-snapshot drift checks.
- `dfmcp-live-v1-1-dev-server`, a separately named, exact-opt-in, read-only development runtime
  preserving the frozen eleven-tool waist and a protocol-specific session namespace. It rejects
  production admission state and cannot consume or impersonate a production ticket.
- Protocol-1.1 world projection, briefing, bounded attention, certified-derived report-ID change
  summaries, and query modes for `summary`, `citizens`, `announcements`, and `all`.
- Aggregate protocol-1.1 source checker that executes the core isolation, transactional
  publication, single-read bootstrap, and development-MCP isolation checkers.
- Mutation suites for production-protocol-map widening, inherited admission, method-waist widening,
  history-coverage overclaim, development-guard removal, process-test loss, and mutation
  contamination.
- Canonical clean-commit source-bundle contract, stable no-follow repository-file reader,
  Git-object-derived deterministic tar creation, hostile archive verification without extraction,
  create-only receipts, and atomic sibling-directory publication after complete verification.
- Authenticated read-only DFHack bridge protocol 1.0 with exactly `Handshake` and
  `ReadObservation`, bounded loopback bearer authentication, canonical protobuf validation,
  generation/version/nonce fencing, stable citizen pagination, and no mutation RPC surface.
- Canonical immutable live-observation capsules whose identity is independent of transport
  pagination, plus deterministic fortress/citizen graph projection with fact provenance and
  explicit complete, conditional, and omitted coverage.
- Live read-only adapter and MCP path covering session bootstrap, observation, heartbeat, query,
  wait, explain, doctor, restart/reset classification, and fail-closed mutation-stage tools.
- R1 native-plugin qualification and R2-R5 disposable-fort acceptance machinery, including
  source/binary receipts, secret scanning, append-only evidence journal, capture guidance,
  deterministic exact-tuple promotion, and registry-generation-bound resolution.
- Owner-private monotonic compatibility floor with exact `0700`/`0600` custody, no-follow reads,
  exclusive initialization, expected-file-digest compare-and-swap, atomic fsynced advancement,
  monotonic sequence and digest chain, and preservation of all previously accepted entry IDs.
- Deterministic authority-free live-admission doctor with fixed registry, floor, exact-tuple, and
  optional server-artifact stages; canonical reports expose `compatibility_ready` or
  `artifact_preflight_ready` without reading a bridge secret or executing a process.
- Source-bound release-server receipt contract sealing the exact clean commit, complete local gate
  order, admission machinery, source digests, platform, executable checks, size, and SHA-256.
- Descriptor-bound admitted launcher that repeatedly verifies registry/floor generation and
  executable bytes, rejects dynamic-loader override variables, emits a secret-free launch record,
  and refuses path-based execution fallback.
- Owner-private, short-lived, single-use admission tickets bound to process ID, bridge protocol,
  exact compatibility entry, registry, decision, monotonic-floor file/content/sequence, server
  receipt, launch digest, executable identity/SHA-256, read-only capabilities, and an empty mutation
  set.
- Rust V2 ticket consumer that validates protocol, rejects legacy V1 tickets, revalidates and hashes
  the current executable, consumes and proves deletion of the ticket, retains admission provenance,
  and only then invokes the exact reviewed private runner; direct `serve-live` invocation fails
  closed.
- Live Agent Turn provenance exposing bridge protocol plus exact compatibility, floor,
  server-receipt, launch, ticket, and executable identities after successful admission.
- Repository source-integrity checks rejecting symbolic links, special files, invalid UTF-8,
  NUL-corrupted or oversized source, machine-local placeholders, recovery debris, and files that
  change while being inspected.
- Adopted the owned `fastmcp_rust` sibling as the MCP presentation plane (ADR-013): modern-only MCP
  2026-07-28, `default-features = false` with `tasks`, pinned to an exact upstream revision for
  dogfooding; upstream defects return through `docs/DOGFOODING_FASTMCP.md`.
- Adopted the owned `eidetic_engine_cli` sibling as an advisory, evidence-linked agent campaign
  memory layer with no canonical-state or authority path.
- Deep source-level audit of asupersync, FrankenSQLite, FrankenFS, FrankenSearch,
  FrankenMarkdown, FrankenGraphDB, FrankenNetworkX, and Doodlestein Self-Releaser.
- Three-plane architecture separating authoritative world/evidence state, derived cognition, and
  narrowly fenced DFHack effects, plus an explicit deployment-admission boundary.
- One observation-capsule version universe for history, projections, subscriptions, branches,
  checkpoints, evidence, and replicas.
- Multi-version world-state specification with positive, negative, range, aggregate, spatial, and
  epoch witnesses; hierarchical conflict refinement; deterministic semantic rebase; and
  proof-carrying merge.
- Canonical fortress graph algorithm plan with explicit tie-break policies, complexity witnesses,
  tiered projections, incremental maintenance, and capability non-interference.
- ATP state/evidence plane covering content-addressed manifests, RaptorQ repair, path racing,
  anti-rollback rules, proof of retrievability, and an explicit prohibition on mutation authority.
- Root-last immutable publication primitives, same-binary performance experimentation doctrine,
  local-only qualification, and DSR release specifications with machine-readable receipts.

### Changed

- Public world query execution now uses `q1` continuations binding the exact
  fortress, epoch, sequence, game tick, state hash, selector set, predicate tree,
  and ordering. This closes cross-query and same-cursor-fork reuse. Legacy query
  cursors require restart; delta encoding remains unchanged. Page width and byte
  budget can vary without changing query identity. Digests are not authentication.
- Query execution verifies the snapshot hash and bounds combined scan/predicate
  work, aggregate identity bytes, and nested kind names before evaluation.
- Migrated the pinned FastMCP dependency to `180a7c88890705217bb8e202d19555adabf24187`
  and published Asupersync 0.5.0. Four modern-only stdio entry points now retain an
  explicit runtime owner and preserve inherited caller context restrictions.
  The existing real subprocess harness has bounded response waits and joined
  reader cleanup; conformance validation is tracked in `dfmcp-k2y`.
- Closed a protocol-confusion defect in process admission: an admitted compatibility decision can no
  longer execute an implicitly selected protocol-1.0 server. Production startup now requires exact
  protocol agreement at every representation and an explicit runner in the V2 production map.
- Hardened ticket custody so both Python issuance and Rust consumption require a real exact-mode
  `0700` directory and exact-mode `0600` ticket. Owner-only but noncanonical modes such as `0500`
  fail closed.
- Moved the protocol-1.1 production-environment refusal to the public `dfmcp-mcp` development API
  seam. External callers cannot bypass the guard by invoking the library wrapper directly.
- Replaced the stale monolithic announcement checker coupling with an aggregate that executes all
  specialized layers while the core checker owns shared protocol, source-map, native, model, and
  qualification invariants.
- Protocol-1.1 source identity now includes the bootstrap checker, development MCP contract/server/
  binary/process tests, and the production V2 admission contract whose runner map it must remain
  outside.
- Local verification compiles every specialized announcement checker and runs the protocol-1.1 MCP
  mutation suite explicitly.
- Local qualification receipts now hash the complete protocol-1.1 publication, adapter, bootstrap,
  transaction-test, development-MCP, and production-isolation source graph rather than only the
  initial wire and batch layers.
- Recovered a corrupted checked-in server-receipt verifier from a known-good source generation,
  rebuilt its exact contract validation, and hardened it around stable no-follow opens, duplicate
  JSON-key rejection, exact gate/source maps, opened-inode verification, and repeated SHA-256
  checks.
- Local verification and qualification now include source-bundle, announcement, compatibility-floor,
  admission-doctor, server-qualification-wrapper, launcher, V2 ticket, source-integrity, and binary
  process gates in one exact source-bound order.
- Workspace moved to Rust 2024 and the latest nightly channel.
- GitHub workflow files target controlled self-hosted machines and serve as locally executable
  specifications; GitHub-hosted execution is not release evidence.
- README, architecture, security, agent rules, roadmap, live-admission documentation, announcement
  documentation, changelog, and implementation status distinguish source presence, development
  execution, qualification, compatibility admission, local floor acceptance, artifact
  qualification, production-protocol dispatch, and runtime admission.

### Security

- Bridge-protocol confusion across manifest, launch, ticket, environment, Rust provenance, and
  runtime dispatch now fails closed.
- Legacy V1 tickets, unknown bridge protocols, protocol 1.1 production attempts, and mismatched
  protocol representations fail before server startup.
- Protocol-1.1 development execution refuses every production admission marker, including the V2
  protocol environment field.
- Compatibility registry rollback relative to an accepted local generation fails closed.
- Same-size executable byte substitution is checked before ticket issuance, before descriptor
  execution, and again by the Rust process before live MCP startup.
- Dynamic-loader injection variables, permissive or symbolic custody, stale compare-and-swap
  identities, direct live-server bypass, and mixed registry/floor generations fail closed.
- Credentials remain absent from receipts, floor files, decisions, launch records, tickets, Agent
  Turns, and admission-doctor reports.

### Current evidence status

- Spatial situation/attention has fifteen registered but uncompiled and unexecuted Rust scenarios.
  Six independent watch-packet JSON reference cases fit the minimum output budget; this is not
  execution of Rust rules, watch transitions, actual MCP or native/live-game behavior.
- Historical endpoint comparisons are source-present with sixteen registered but uncompiled and
  unexecuted Rust scenarios. The 66 Python checks validate the record/page envelope only, not the
  delegated selector schema, Rust comparison, journal replay, MCP or native/live qualification.
- Offline spatial archive recovery is source-present with fourteen registered but uncompiled and
  unexecuted Rust scenarios. The 128 independent JSON-size checks cover route-wrapper sizing only,
  not Rust, replay, filesystem custody, MCP, native/live-game behavior or qualification.
- Workforce queries are source-present with fifteen registered but uncompiled/unexecuted Rust
  tests. The 88 passing Python schema cases and 4,096 independent allocation-oracle cases do not
  execute the production allocator or establish Rust, MCP, native/live or admission evidence.
- Durable spatial/1.8 watches are source-present with 25 newly registered Rust tests, not executed
  here. The 740 passing Python framing-reference checks do not execute Rust or establish runtime,
  filesystem power-loss, native/live-game, repository qualification or admission evidence.
- Condition-watch schema checks passed 73 local cases (34 accepted, 39 rejected),
  plus three documentation examples. The 25 original Rust scenarios are registered but
  unexecuted: no Rust compiler, Cargo, or rustfmt is available in this editing
  environment. These checks do not establish Rust, stdio, native/live-game,
  repository qualification, or admission evidence.
- The foreground-history tranche is source-present, including ten registered
  regression scenarios. No Rust compiler, Cargo, or rustfmt was available;
  compilation, Rust tests, Clippy, repository qualification, stdio execution, and
  live-game execution have not been established for it.
- The initial query/graph tranche includes an independent Python design check over
  512 exhaustive graphs and 200 seeded multigraphs. That is not Rust-execution or
  qualification evidence. Graph modes subsequently gained protocol-1.1 MCP source
  integration alongside typed inspection, aggregate/search queries, and baselines;
  broader live observation domains remain unfinished.
- The checked-in compatibility registry remains `no_admitted_live_tuples` with zero entries.
- No current live tuple is admitted by that empty registry.
- The V2 production runtime map contains protocol 1.0 only; later profiles remain explicitly
  unadmitted development source.
- No admitted live mutation capability exists. The separate control/1.7 pause-only development
  source and its exact mock-test limitations are recorded in `IMPLEMENTATION_STATUS.md` and
  `CHANGELOG.d/2026-09-15-pause-reconciliation.md`; it is not production admission.
- A fresh full latest-nightly qualification receipt, exact native/live evidence, registry
  promotion, deployment-floor advancement, qualified server artifact, and admitted launch evidence
  are still required for the final current head.

## [0.0.1] - 2026-08-29

### Added

- Initial public design corpus and executable contract scaffold.
