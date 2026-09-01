# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and evidence actually
establish.

## Current phase

**Phase 0D-R0: implemented read-only live path, exact admission machinery, and agent-oriented MCP;
no currently admitted live tuple.**

The repository now contains a substantial authenticated read-only DFHack path and a non-bypassable
local launch chain in source. The checked-in compatibility registry nevertheless has status
`no_admitted_live_tuples` and contains zero entries. Therefore:

```text
live-read implementation source exists
≠ current source has passed a fresh R1-R5 campaign
≠ a tuple is admitted by the checked-in registry
≠ a server binary is qualified for this source generation
≠ a live process is authorized to start
```

No mutation RPC or live mutation capability is implemented or admitted.

## Evidence hierarchy

Implementation claims must name their evidence rung:

1. **source present** — types, code, contracts, and tests are checked in;
2. **static/Python checked** — repository and Python contract gates passed for the exact commit;
3. **Rust-qualified** — full latest-nightly formatting, Clippy, tests, release tests, and rustdoc
   passed for the exact clean commit;
4. **native-qualified** — the exact DFHack plugin built and passed R1 for named DFHack source and
   plugin bytes;
5. **live-qualified** — R2-R5 passed against a disposable fortress for the same exact tuple;
6. **registry-admitted** — reviewed receipts were promoted into the checked-in registry;
7. **floor-accepted** — a deployment host advanced its owner-only monotonic floor to those exact
   registry bytes;
8. **artifact-qualified** — a source-bound release-server receipt names the exact executable;
9. **runtime-admitted** — the floor-bound launcher issued and the Rust process consumed a
   single-use ticket.

A higher rung implies only its stated predecessors for the same exact identities. Evidence from an
older source generation does not silently qualify the current one.

## Present now

### Agent-facing MCP

- Modern-only MCP 2026-07-28 presentation through the exact-revision-pinned owned
  `fastmcp_rust` sibling.
- Frozen eleven-tool `fortress.*` waist.
- Deterministic laboratory mode with process-local pause-state effects.
- Authenticated live read-only mode in source.
- Canonical Agent Turn Packet with continuity, briefing, changes, attention, active work,
  affordances, recommendations, uncertainty, coverage, budgets, references, and structured
  recovery.
- Live Agent Turns expose admitted entry, registry, decision, monotonic-floor, server-receipt,
  launch, ticket, and executable digests after successful ticket consumption.
- Live mutation-stage tools remain present for protocol stability but fail closed.

### Live DFHack read path

- Out-of-process DFHack plugin using supported native protobuf RPC facilities.
- Loopback bearer-token authentication with bounded nonce and exact protocol handshake.
- Exactly two plugin methods in protocol V1: `Handshake` and `ReadObservation`.
- No remote-service flag and no arbitrary command, Lua, keyboard, path, or memory-write route.
- Safe-Rust native DFHack wire codec with bounded frames, duplicate-field rejection, canonical
  protobuf checks, notification budgets, nonce/version/generation fencing, and poisoned-stream
  behavior.
- Complete bounded citizen-roster reads with stable unit-ID order, optional name projection, and
  paused-world requirement for multi-page assembly.
- Canonical immutable observation capsule whose identity is independent of pagination.
- Deterministic projection into fortress and citizen entities with fact-level source digests and
  explicit complete/omitted coverage.
- Live fortress identity derivation, observation epochs, heartbeats, change advancement, restart
  reset, clock-regression reset, and world/version switch refusal.
- Live briefing, basic-status attention, query, explain, doctor, and wait surfaces.
- Bounded live-session registry and fail-closed read-only posture.

### R1-R5 qualification machinery

- Native plugin qualification wrapper and source-bound receipt contract.
- R2 authentication matrix, R3 deterministic-read matrix, R4 restart/drift/gap matrix, and R5
  cold-agent orientation evidence contracts.
- Secret scanning, append-only evidence journal, capture guidance, and fail-closed acceptance
  verifier.
- Deterministic exact-tuple promotion and resolution scripts.

These are executable mechanisms in source. They do not mean the current commit has a passing live
receipt.

### Compatibility and local custody

- Checked-in exact compatibility registry with canonical content-addressed entries.
- Registry promotion protected by expected-generation compare-and-swap and a single-writer lock.
- Exact deployment resolver that binds the complete registry digest and required entry ID.
- Owner-private monotonic compatibility floor:
  - absolute path;
  - exact `0700` parent and `0600` file;
  - root/effective-user ownership;
  - no symlink following;
  - exclusive initialization;
  - atomic fsynced advancement;
  - expected-floor-file compare-and-swap;
  - digest chain and monotonic sequence;
  - prior entry IDs cannot disappear;
  - formatting-only byte changes remain explicit generations.
- Deterministic authority-free admission doctor with fixed stages for registry, floor, exact tuple,
  and optional server artifact.

The monotonic floor is local anti-rollback custody, not distributed consensus, compatibility
evidence, or protection against compromise of the owner/root account.

### Server artifact and process admission

- Source-bound release-server receipt contract sealing:
  - exact clean commit;
  - complete local qualification gate order;
  - source-file digests;
  - toolchain and platform;
  - `contract`, `doctor`, and `demo` executable checks;
  - executable size and SHA-256;
  - empty mutation capability.
- Recovered and hardened receipt verifier using stable no-follow opens, duplicate-key rejection,
  exact schemas, canonical digests, exact gate order, exact source mapping, and opened-inode
  verification.
- Repository-integrity gate now rejects non-UTF-8, NUL-corrupted, and oversized source/contract
  text. This was added after discovering and repairing a corrupted checked-in Python verifier blob.
- Admitted launcher that:
  - requires an exact monotonic floor;
  - resolves the explicitly required entry;
  - verifies the source-bound server receipt;
  - rejects dynamic-loader overrides;
  - verifies owner/mode/device/inode/size/SHA-256 on an opened descriptor;
  - re-reads registry and floor after artifact verification and before execution;
  - re-hashes the opened executable before ticket issue and before descriptor-only `execve`;
  - emits no bridge secret into launch records or tickets.
- Rust single-use ticket consumer that validates process, expiry, exact read-only capabilities,
  registry, decision, floor file/content/sequence, server receipt, launch digest, executable
  metadata, and executable SHA-256 before deleting the ticket and starting MCP.
- Direct `serve-live` invocation without a valid ticket fails closed.

## Current registry and qualification state

The checked-in registry is intentionally empty:

```json
{
  "schema_version": "dfmcp.live-compatibility-registry/1",
  "status": "no_admitted_live_tuples",
  "entries": []
}
```

Consequences:

- no Dwarf Fortress/DFHack/plugin/source/platform tuple is currently admitted;
- the admission doctor cannot produce `compatibility_ready` for a real deployment manifest;
- the launcher cannot authorize a live process from the checked-in registry;
- any old experimental live evidence, if retained outside the repository, does not qualify this
  source generation;
- a deployment floor initialized from the empty registry correctly preserves “no admissions.”

The current direct-editing environment has not run the full latest-nightly Rust qualification for
the final source generation. No fresh qualification receipt is checked in for this head. The
present changes must therefore be described as implemented source with source-level tests and
qualification contracts, not as a newly qualified binary or live configuration.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Architecture | three-plane target, one agent control loop, invariants, machine registries | empirical validation of the full target architecture |
| Agent surface | canonical Agent Turn Packet, eleven-tool waist, live read-only orientation, admission provenance | complete strategic model, durable handoff, empirical VOI/cost/confidence models |
| Core | typed IDs, SHA-256, anchors, capabilities, scopes, budgets, evidence, errors, leases/clock/roles laboratories | production distributed authorization and durable fencing |
| World | canonical graph/facts/snapshots/deltas, coverage, in-memory query/search/Merkle/checkpoint/ATP laboratories | admitted durable FrankenSQLite/FrankenFS/FrankenSearch/FrankenGraphDB implementations |
| Intent | semantic actions, constraints, sealed plans, obligations, laboratory pause effect | qualified live action families and full objective/counterfactual pipeline |
| Adapter | authenticated native DFHack read codec, capsule assembly, identity/version fencing, projection, live read-only adapter | items, jobs, buildings, map, economy, welfare, military, history; live mutation adapter |
| Bridge | real read-only plugin source with `Handshake` and `ReadObservation` | any mutation RPC; wider read domains; current exact R1 receipt |
| MCP | laboratory server and admitted live read-only server source | qualified Streamable HTTP deployment and durable task/session stores |
| Compatibility | exact registry/promotion/resolver, monotonic floor, admission doctor | any entry in the current registry; revocation schema; supported compatibility window |
| Artifact admission | source-bound receipt, hardened verifier, descriptor launcher, single-use Rust ticket | a fresh qualified release binary and launch receipt for current head |
| Security | closed dependency checks, safe Rust policy, secret scan, loader rejection, source-text corruption gate | hostile-host resistance, signed provenance, external review |
| Persistence/transfer | reference in-memory ledgers, checkpoints, Merkle, ATP models | process-crash recovery and admitted persistent/remote transfer backends |
| Release | local qualification and DSR specifications | current cross-platform receipts, signed release assets, installable stable release |

## What works without a live tuple

The repository can still:

- compile and exercise the deterministic laboratory when a valid latest-nightly toolchain and
  locked dependencies are available;
- validate source, architecture, dependency, bridge, compatibility, floor, doctor, receipt,
  launcher, and ticket contracts;
- build and test the read-only bridge and live adapter source;
- produce proposed native/live/server receipts and proposed registry generations;
- initialize a deployment floor that truthfully records an empty registry;
- diagnose why a candidate deployment is not ready without reading a bridge secret or executing a
  server.

It cannot truthfully claim a currently runnable admitted live configuration until the complete
fresh chain is produced.

## Explicitly absent

- no live mutation RPC;
- no pause/resume authority;
- no dig, construction, labor, burrow, stockpile, work-order, military, keyboard, Lua, command,
  arbitrary RPC, arbitrary filesystem, or arbitrary network effect;
- no current admitted live tuple;
- no current supported or production compatibility claim;
- no durable production MVCC/WAL, checkpoint custody, crash recovery, or ATP deployment;
- no signed release provenance or hostile-host security claim;
- no proof that the final current head passes all Rust gates.

## Next executable milestones

1. Run `./scripts/verify.sh` and `./scripts/qualify_local.sh` on a controlled latest-nightly machine
   with no Rust gates skipped; fix every format, compile, Clippy, test, release-test, and rustdoc
   failure.
2. Qualify the release server binary for that exact clean commit with
   `scripts/qualify_live_server_binary.sh`.
3. Build the exact native plugin against the selected DFHack source and capture R1.
4. Run the complete R2-R5 disposable-fort campaign for the same source/plugin/platform tuple.
5. Review and promote the receipts into a new registry generation.
6. Advance a deployment host’s owner-private monotonic floor through compare-and-swap.
7. Run the authority-free admission doctor with the exact manifest, entry fence, source receipt,
   and executable.
8. Launch only through `scripts/serve_admitted_live.py` and retain the secret-free launch record.
9. After one current read-only tuple is admitted, expand read coverage in separate protocol and
   evidence generations: announcements/events, jobs, items, buildings, then bounded map state.
10. Design pause/resume only after the expanded read path is stable; mutation must be a separately
    versioned, witnessed, idempotent, reconciled, and disposable-fort-qualified generation.

## Status rules

1. Only this file plus exact receipts and registries define implementation status.
2. Source presence is not qualification, admission, support, or production evidence.
3. A feature is `experimental` only after it executes under a named receipt.
4. A tuple is admitted only while its exact entry is present in the current registry generation.
5. A deployment is admitted only when its trusted floor exactly matches that registry generation.
6. A structurally valid old registry cannot override a newer trusted floor.
7. A doctor report is diagnosis, never authority.
8. A server receipt qualifies an executable, never a bridge or game session.
9. A launch ticket grants only the exact read-only process start it names and is single-use.
10. Negative evidence may reject a claim but cannot certify success.
11. Derived indexes, attention, recommendations, memory, and counterfactuals are never more
    authoritative than their canonical source evidence.
12. Unit tests do not substitute for disposable-fort evidence where Dwarf Fortress behavior
    matters.
