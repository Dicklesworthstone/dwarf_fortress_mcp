# Roadmap

Progress is gate-based. Dates are intentionally absent: evidence matters more than calendar theater.

The current source generation is at **Phase 0D-R0**: a substantial authenticated read-only live
path and exact admission machinery exist, but the checked-in compatibility registry is empty and
the final head does not yet have a fresh full Rust qualification receipt. See
[`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md).

## Evidence notation

- **Source** — implementation and tests are checked in.
- **Static** — repository/Python gates passed for the exact commit.
- **Rust** — latest-nightly format, Clippy, debug/release tests, and rustdoc passed.
- **Native** — exact DFHack plugin passed R1.
- **Live** — exact tuple passed R2-R5 against a disposable fortress.
- **Admitted** — receipts were promoted into the current registry.
- **Floor** — deployment host accepted those exact registry bytes into its monotonic floor.
- **Artifact** — exact release server received a source-bound qualification receipt.
- **Launched** — floor-bound descriptor launcher and single-use Rust ticket completed.

A milestone is not complete merely because its source exists.

## Phase 0A — Executable semantic laboratory

**Source implemented. Historical qualification does not automatically apply to the current head.**

Delivered in source:

- safe-Rust workspace and closed dependency policy;
- typed identities, anchors, budgets, capabilities, evidence, and stable errors;
- canonical graph, facts, snapshots, deltas, queries, checkpoints, Merkle, ATP, and search
  laboratories;
- semantic intent normalization, sealed plans, prepare/commit checks, idempotency, obligations,
  cancellation, checkpoint/restore, and deterministic replay laboratories;
- modern-only eleven-tool MCP surface through pinned `fastmcp_rust`;
- canonical Agent Turn Packet and deterministic laboratory adapter.

Remaining evidence gate:

```text
full current-head local qualification with no Rust gate skipped
```

## Phase 0B — Agent-operating model

**Substantial source implemented.**

Delivered:

- synthetic agent control loop;
- continuity, briefing, changes, attention, active work, affordances, recommendations, uncertainty,
  coverage, budget, references, and structured recovery;
- authority-free presentation semantics;
- heartbeat and reset classification;
- exact admitted-launch provenance projected into live Agent Turns;
- bounded session and output behavior.

Remaining:

- durable handoff resources;
- empirical cost, value-of-information, and confidence models;
- full objective decomposition and candidate comparison;
- durable surprise and evidence-gated learning loop;
- profile semantics backed by wider live coverage.

## Phase 0C — Authenticated live read-only path

**Source implemented; current tuple not admitted.**

Delivered:

- real DFHack plugin source using supported native protobuf RPC;
- exactly `Handshake` and `ReadObservation` in protocol V1;
- loopback bearer-token authentication and bounded nonce/version manifest;
- safe-Rust DFHack wire codec with canonical protobuf validation;
- complete bounded citizen-roster observation;
- optional name projection;
- pagination-independent immutable observation capsule;
- fortress/citizen graph projection with fact provenance and explicit coverage;
- live identity, version, generation, epoch, sequence, heartbeat, restart, and drift fencing;
- read-only live MCP tools for open, observe, query, wait, explain, and doctor;
- mutation-stage tools fail closed.

Required gate to move from source to an admitted experimental configuration:

```text
exact current source
+ exact DFHack source
+ exact plugin bytes
+ R1
+ R2
+ R3
+ R4
+ R5
+ reviewed registry promotion
```

The current registry has zero entries.

## Phase 0D — Exact compatibility, custody, and process admission

**Source implemented; deployment evidence not yet produced for current head.**

Delivered:

- canonical exact-tuple compatibility registry;
- deterministic promotion with source/binary/evidence equality checks;
- expected-registry compare-and-swap and single-writer lock;
- deterministic resolver binding the full registry digest and required entry ID;
- owner-private monotonic compatibility floor with:
  - absolute path;
  - exact `0700` parent and `0600` file;
  - root/effective-user ownership;
  - no-follow opens;
  - exclusive initialization;
  - atomic fsynced compare-and-swap advancement;
  - monotonic sequence and digest chain;
  - prior entry IDs cannot disappear;
- authority-free admission doctor with fixed registry/floor/tuple/artifact stages;
- source-bound release-server receipt contract;
- hardened receipt verifier and repository source-text corruption detector;
- descriptor-bound Python launcher with repeated floor/registry and executable SHA checks;
- owner-private, short-lived, single-use process ticket;
- Rust ticket consumer that revalidates process, floor, receipt, executable metadata, and executable
  bytes before starting the private live server;
- direct `serve-live` bypass fails closed.

### Immediate D0 gate: current-head qualification

1. Run `./scripts/verify.sh` on the controlled latest-nightly toolchain.
2. Run `./scripts/qualify_local.sh` on a clean checkout with no static-only escape.
3. Fix every source-integrity, schema, shell, format, compile, Clippy, test, release-test, and rustdoc
   failure.
4. Retain the exact qualification receipt with its commit and source digests.

### D1 gate: current server artifact

1. Build the exact release binary from the qualified clean commit.
2. Run `scripts/qualify_live_server_binary.sh`.
3. Independently verify the source mapping, local receipt, executable checks, inode, size, mode,
   owner, and SHA-256.

### D2 gate: first current exact live tuple

1. Build and qualify the native plugin against one exact DFHack source revision.
2. Run the full R2-R5 disposable-fort campaign.
3. Review receipts and promote one exact entry into the checked-in registry.
4. Do not infer compatibility for adjacent versions or platforms.

### D3 gate: trusted deployment admission

1. Initialize or advance the deployment host’s monotonic floor to the reviewed registry bytes.
2. Run the authority-free doctor to `artifact_preflight_ready`.
3. Start only through `scripts/serve_admitted_live.py`.
4. Retain the secret-free launch record and consumed-ticket provenance.
5. Verify live Agent Turns report the exact floor, registry, decision, receipt, launch, ticket, and
   executable identities.

## Phase 1 — Wider live observation

Begins only after one current read-only tuple is admitted and launched.

Each domain is a separate protocol, canonicalization, coverage, acceptance, and compatibility
generation. Preferred order:

1. **Announcements and reports**
   - bounded stable ordering and continuation semantics;
   - explicit truncation and historical coverage;
   - no global hidden cursor shared across clients.
2. **Jobs and work orders**
   - stable identities and worker/building/item references;
   - complete versus filtered coverage;
   - dependency graph and blockage evidence.
3. **Items and inventories**
   - stable item identity, stack/material/ownership/container semantics;
   - bounded inventory and stock aggregates;
   - no absence claim without complete-domain witness.
4. **Buildings, stockpiles, zones, burrows, and squads**
   - typed graph relationships;
   - exact spatial extents and assignments.
5. **Bounded map state**
   - chunk/range witnesses;
   - generation-fenced dirty scans;
   - canonical map-block identities;
   - explicit hidden/redacted/omitted semantics.
6. **Welfare, health, military, economy, and history**
   - only after the underlying DFHack semantics are understood and bounded.

Acceptance for every domain:

```text
canonical bytes independent of transport pagination
+ strict bounds
+ stable identity/order
+ complete/partial/omitted coverage
+ restart and drift fencing
+ malformed response rejection
+ disposable-fort evidence
+ exact compatibility generation
```

## Phase 2 — First live mutation: pause/resume

Begins only after Phase 1 provides enough read coverage to witness and reconcile the effect.

Required design:

```text
semantic intent
→ exact capability and risk scope
→ anchor and witness set
→ prepared adapter token
→ commit-time revalidation
→ idempotent operation identity
→ bridge-side effect journal
→ authoritative post-state read
→ obligation discharge
→ unknown-outcome reconciliation
```

Required failures:

- stale anchor;
- expired or reused token;
- insufficient capability;
- wrong fortress identity;
- bridge restart;
- transport loss before/after effect;
- duplicate delivery;
- postcondition not observed;
- indeterminate effect requiring lookup, never blind retry.

Pause/resume must be a new bridge protocol generation and a new exact compatibility campaign. It
must not be smuggled into read-only V1.

## Phase 3 — Safe action families

Add actions one family at a time only after pause/resume proves the mutation protocol:

1. reversible configuration and alert changes;
2. labor, burrow, and squad assignments;
3. stockpile and work-order configuration;
4. construction and designation with terrain postconditions;
5. military and emergency actions under stricter confirmation and checkpoint policy.

Every family needs typed preconditions, postconditions, compensation where possible, operation
lookup, cancellation semantics, and disposable-fort fault campaigns.

## Phase 4 — Durable world and effect custody

Admit the Franken substrate behind existing semantic traits:

- `frankensqlite` MVCC, WAL, witnessed reads, negative SSI, rebase, and merge;
- `frankenfs` immutable publication, crash recovery, generation fencing, and retrievability;
- durable effect and obligation journals;
- process-crash and power-loss recovery;
- retained checkpoints and exact restore epochs;
- bounded compaction without authority loss.

No durable backend is “done” until restart and corruption campaigns reproduce canonical state and
correctly preserve indeterminate effects.

## Phase 5 — Search, graph, and cognition

- admitted FrankenSearch generations;
- admitted FrankenGraphDB/FrankenNetworkX projections;
- deterministic attention and affordance scoring;
- graph witnesses and complexity certificates;
- objective decomposition, branch-per-agent candidates, counterfactual comparison;
- surprise records and evidence-gated memory promotion.

All cognition remains derived and revocable. It never gains direct mutation authority.

## Phase 6 — ATP and distributed operation

- content-addressed state/evidence movement;
- resume-capable transfer;
- peer exchange, anti-rollback, and retrievability evidence;
- bandwidth-aware replication classes;
- branch exchange and merge certificates;
- scheduler-aware transfer under structured cancellation.

## Phase 7 — Qualification and release

- Linux/Classic reference qualification;
- Premium Linux;
- Windows named-pipe path;
- macOS/Wine exploratory support only after evidence;
- long-horizon performance and leak campaigns;
- fuzzing and fault injection;
- signed local/self-hosted release evidence;
- exact binary/source/receipt manifests;
- install, upgrade, rollback, and uninstall verification.

GitHub workflow files remain portable job specifications, not correctness authority. Controlled
local and self-hosted receipts define release evidence.

## Success condition

A future release is successful only when an agent can manage a long-lived fortress through exact
semantic state, bounded authority, idempotent effects, durable evidence, deterministic recovery,
and honest uncertainty—without the operator having to trust hidden retries, brittle screen
automation, or unverifiable claims.
