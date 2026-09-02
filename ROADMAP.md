# Roadmap

Progress is gate-based. Dates are intentionally absent: evidence matters more than calendar theater.

The current source generation is at **Phase 0D-R0**, with an implemented but unadmitted Phase-1
retained-announcement slice. The checked-in compatibility registry is empty. Protocol 1.0 is the
only runner in the production V2 protocol map; protocol 1.1 has an explicitly unadmitted development
runtime. See [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md).

## Evidence notation

- **Source** — implementation and tests are checked in.
- **Static** — repository/Python gates passed for one exact commit.
- **Rust** — latest-nightly formatting, Clippy, debug/release tests, and rustdoc passed.
- **Native** — exact DFHack plugin passed R1.
- **Live** — exact tuple passed its required disposable-fort campaign.
- **Admitted** — reviewed receipts were promoted into the current registry.
- **Floor** — deployment host accepted those exact registry bytes into its monotonic floor.
- **Artifact** — exact release server received a source-bound qualification receipt.
- **Dispatched** — the V2 production map contains the exact protocol runner.
- **Launched** — protocol-bound descriptor launcher and single-use Rust ticket completed.

A milestone is not complete merely because its source exists or a development process runs.

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
  coverage, budget, references, and typed recovery;
- authority-free presentation semantics;
- heartbeat and reset classification;
- exact admitted-launch provenance, including bridge protocol, projected into live Agent Turns;
- bounded session and output behavior.

Remaining:

- durable handoff resources;
- empirical cost, value-of-information, and confidence models;
- complete objective decomposition and candidate comparison;
- durable surprise and evidence-gated learning loop;
- wider live coverage behind exact protocol generations.

## Phase 0C — Protocol-1.0 authenticated live reads

**Source implemented; no current tuple admitted.**

Delivered:

- real DFHack plugin source using supported native protobuf RPC;
- exactly `Handshake` and `ReadObservation`;
- loopback bearer authentication and exact protocol/version manifest;
- safe-Rust wire codec with canonical protobuf validation;
- complete bounded citizen-roster observation;
- optional name projection;
- pagination-independent immutable capsules;
- fortress/citizen graph projection with fact provenance and explicit coverage;
- live identity, version, generation, epoch, sequence, heartbeat, restart, and drift fencing;
- read-only live MCP tools;
- mutation-stage tools fail closed.

Required evidence:

```text
exact current source
+ exact DFHack source
+ exact protocol-1.0 plugin bytes
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

- canonical exact-tuple registry and deterministic promotion;
- expected-registry compare-and-swap and single-writer lock;
- exact resolver binding full registry digest and required entry ID;
- owner-private monotonic floor with exact `0700`/`0600` custody, no-follow reads, atomic fsynced
  advancement, sequence/digest chain, and prior-entry preservation;
- authority-free admission doctor;
- source-bound server receipt and hardened verifier;
- descriptor-bound launcher with loader-environment rejection and repeated registry/floor/binary
  verification;
- protocol-bound V2 launch and ticket contract;
- exact protocol agreement across manifest, decision, launch, ticket, environment, Rust provenance,
  and final runner lookup;
- production runner map containing protocol 1.0 only;
- legacy V1 ticket, unknown protocol, protocol mismatch, and protocol-1.1 production refusal;
- real exact-mode `0700` ticket directory and exact-mode `0600` single-use file;
- Rust ticket consumer that revalidates protocol, process, custody, capabilities, and executable
  bytes before starting the private server;
- direct `serve-live` bypass fails closed.

### D0 gate: current-head qualification

1. Run `./scripts/verify.sh` on the controlled latest-nightly toolchain.
2. Run `./scripts/qualify_local.sh` on a clean checkout without static-only escape.
3. Fix every source-integrity, schema, shell, format, compile, Clippy, test, release-test, and rustdoc
   failure.
4. Retain the exact qualification receipt and source digests.

### D1 gate: protocol-1.0 server artifact

1. Build the exact release binary from the qualified clean commit.
2. Run `scripts/qualify_live_server_binary.sh`.
3. Independently verify source map, local receipt, executable checks, inode, size, mode, owner, and
   SHA-256.

### D2 gate: first exact protocol-1.0 live tuple

1. Build and qualify the native plugin against one exact DFHack revision.
2. Run the complete R2-R5 disposable-fort campaign.
3. Review receipts and promote one exact entry.
4. Do not infer compatibility for adjacent versions, protocols, or platforms.

### D3 gate: trusted protocol-1.0 launch

1. Initialize or advance the deployment floor to the reviewed registry bytes.
2. Run the authority-free doctor to `artifact_preflight_ready`.
3. Verify the V2 production map resolves the entry’s protocol to the reviewed protocol-1.0 runner.
4. Start only through `scripts/serve_admitted_live.py`.
5. Retain the secret-free launch record and consumed-ticket provenance.
6. Verify Agent Turns expose the exact bridge protocol, floor, registry, decision, receipt, launch,
   ticket, and executable identities.

## Phase 1A — Protocol-1.1 retained announcements

**Implemented in source through an explicitly unadmitted development MCP runtime. No source, native,
live, registry, floor, artifact, dispatch, or launch evidence is implied for the current head.**

Delivered in source:

- distinct protocol-1.1 protobuf package, plugin, bridge version, contracts, and probe;
- retained-announcement fields inside `ReadObservation`; no new method;
- canonical bounded batch with strict IDs, cursor, retained bounds, gap evidence, and
  complete-through-latest semantics;
- transactional citizen-pagination × announcement-continuation publication;
- complete retained suffix versus partial historical coverage;
- deterministic world projection, briefing, attention, and report-ID changes;
- read-only adapter integration;
- single-publication bootstrap with no duplicate underlying bridge read;
- two-dimensional primed replay with cursor, projection, manifest, and snapshot checks;
- exact-opt-in eleven-tool development MCP runtime with distinct session namespace;
- rejection of production admission environment state at the public API seam;
- A1-A6 contract, journal, native receipt issuer, source qualifier, aggregate checker, and mutation
  suites;
- complete local qualification digest coverage of the publication/bootstrap/MCP/isolation graph.

### A0 gate: protocol-1.1 source qualification

1. Run `scripts/qualify_live_announcement_source.sh` on one exact clean commit.
2. Require every core, publication, bootstrap, MCP-isolation, mutation, Rust, and rustdoc gate.
3. Retain the source-only receipt. Do not call it native or live evidence.

### A1 gate: native protocol-1.1 plugin

1. Build `dfmcp_bridge_v1_1` against one exact DFHack source revision.
2. Verify the exact two-method inventory and absence of mutation/command symbols.
3. Issue and independently validate the native receipt.

### A2 gate: live protocol-1.1 evidence

1. Run all 43 A1-A6 cases against disposable forts.
2. Re-run the complete baseline citizen R2-R5 campaign under protocol 1.1.
3. Bind both evidence sets to the same source, native plugin, versions, and platform.
4. Preserve secret-free append-only journals and canonical receipts.

### A3 gate: compatibility and artifact

1. Qualify a protocol-1.1 production server artifact separate from the development binary.
2. Review a proposed exact protocol-1.1 registry entry.
3. Promote only after source, native, A1-A6, and baseline evidence agree exactly.
4. Advance the deployment floor to that reviewed generation.

### A4 gate: production dispatch

1. Add a protocol-1.1 private runner only after A0-A3 are complete.
2. Widen `architecture/live_admission_ticket_v2.json` through reviewed source changes and fresh
   server-artifact qualification.
3. Add protocol-confusion and cross-runner process tests.
4. Launch only through a fresh V2 ticket whose protocol agrees at every representation.

The development server can never substitute for A3 or A4.

## Phase 1B — Wider live observation

After at least one exact read-only tuple is admitted, add each domain as a separate protocol,
canonicalization, coverage, acceptance, and compatibility generation. Preferred order:

1. **Jobs and work orders**
   - stable identities and worker/building/item references;
   - complete versus filtered coverage;
   - dependency graph and blockage evidence.
2. **Items and inventories**
   - stable item identity, stack/material/ownership/container semantics;
   - bounded inventory and stock aggregates;
   - no absence claim without complete-domain witness.
3. **Buildings, stockpiles, zones, burrows, and squads**
   - typed graph relationships;
   - exact spatial extents and assignments.
4. **Bounded map state**
   - chunk/range witnesses;
   - generation-fenced dirty scans;
   - canonical map-block identities;
   - explicit hidden/redacted/omitted semantics.
5. **Welfare, health, military, economy, and history**
   - only after underlying DFHack semantics are understood and bounded.

Acceptance for every domain:

```text
canonical bytes independent of transport pagination
+ strict bounds
+ stable identity/order
+ complete/partial/omitted coverage
+ restart and drift fencing
+ malformed response rejection
+ disposable-fort evidence
+ exact protocol/compatibility generation
```

## Phase 2 — First live mutation: pause/resume

Begins only after live reads provide enough coverage to witness and reconcile the effect.

Required chain:

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

Required failures include stale anchor, expired/reused token, insufficient capability, wrong fortress,
bridge restart, transport loss before/after effect, duplicate delivery, missing postcondition, and
indeterminate effect requiring lookup rather than blind retry.

Pause/resume must be a new bridge protocol generation and exact evidence campaign. It must not be
smuggled into read-only protocol 1.0 or 1.1.

## Phase 3 — Safe action families

Add actions one family at a time only after pause/resume proves the mutation protocol:

1. reversible configuration and alert changes;
2. labor, burrow, and squad assignments;
3. stockpile and work-order configuration;
4. construction and designation with terrain postconditions;
5. military and emergency actions under stricter confirmation/checkpoint policy.

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

No durable backend is complete until restart and corruption campaigns reproduce canonical state and
preserve indeterminate effects correctly.

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
- canonical source bundles;
- signed local/self-hosted release evidence;
- exact binary/source/receipt manifests;
- install, upgrade, rollback, and uninstall verification.

GitHub workflow files remain portable job specifications, not correctness authority. Controlled
local and self-hosted receipts define release evidence.

## Success condition

A future release succeeds only when an agent can manage a long-lived fortress through exact semantic
state, bounded authority, protocol-bound execution, idempotent effects, durable evidence,
deterministic recovery, and honest uncertainty—without trusting hidden retries, brittle screen
automation, development-only paths, or unverifiable compatibility claims.
