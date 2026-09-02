# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with an implemented but unadmitted Phase-1 announcement read slice. No live tuple is
currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- an explicitly unadmitted protocol-1.1 development MCP runtime;
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

### Agent-facing MCP

- Modern-only MCP 2026-07-28 through the exact-revision-pinned owned `fastmcp_rust` sibling.
- Frozen eleven-tool `fortress.*` waist.
- Deterministic laboratory mode with process-local pause-state effects.
- Authenticated protocol-1.0 read-only production server source.
- Explicitly unadmitted protocol-1.1 development server source.
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

- distinct protocol package, plugin name, bridge version, and native build path;
- canonical retained-announcement batch with strict report-ID order, text and count limits,
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
  oversized source text, machine-local placeholders, and unstable reads.
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
- protocol 1.1 cannot enter the production runner map;
- an empty-registry floor correctly preserves “no admissions”;
- old or external receipts do not qualify the current source generation unless they match every
  exact identity and are reviewed and promoted.

No fresh full latest-nightly qualification receipt is checked in for the final current head. The
present tranche is therefore described as implemented and source-bound, not as a newly qualified
binary or live configuration.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Agent surface | canonical Agent Turn, eleven-tool waist, read-only orientation, protocol-bound admission provenance | durable handoff, complete objectives/counterfactuals, empirical VOI/cost/confidence models |
| Protocol 1.0 | authenticated citizen read stack and private production runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained-announcement bridge, codec, publication, adapter, bootstrap, dev MCP, A1-A6 tooling | source receipt for current head, native/live receipts, production artifact, registry/floor/runtime admission |
| Compatibility | exact registry, promotion, resolver, monotonic floor, authority-free doctor | any current entry, evidence-bearing revocation, supported compatibility window |
| Process admission | V2 protocol-bound launch/ticket/environment/Rust dispatch, exact custody and executable checks | a fresh qualified current binary and successful admitted launch receipt |
| World | canonical snapshots, facts, deltas, graph/query/search/Merkle/checkpoint/ATP laboratories | admitted durable FrankenSQLite/FrankenFS/FrankenSearch/FrankenGraphDB backends |
| Intent | semantic actions, sealed plans, witnesses, idempotency, obligations, lab pause effect | any qualified live mutation family |
| Security | safe Rust, closed deps, secret scan, loader refusal, source/archive integrity, protocol-confusion defense | hostile-host resistance, signed provenance, external review |
| Release | local qualification, source bundles, server receipts, DSR specifications | current signed cross-platform release assets and install/rollback evidence |

## Explicitly absent

- no current admitted live tuple;
- no current supported or production compatibility claim;
- no admitted protocol-1.1 runtime;
- no live mutation RPC;
- no pause/resume, dig, construction, labor, burrow, stockpile, work-order, military, keyboard, Lua,
  arbitrary command, arbitrary filesystem, or arbitrary network effect;
- no proof that the final current head passed every Rust qualification gate;
- no durable production MVCC/WAL, checkpoint custody, crash recovery, or ATP deployment;
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
8. Expand read coverage next through jobs, items, buildings, and bounded map state, each as a
   separate protocol and evidence generation.
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
