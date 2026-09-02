# Live announcement generation implementation status

Protocol 1.1 is **implemented in source** as an isolated, read-only, explicitly **not admitted**
bridge generation. It does not widen protocol 1.0 authority and does not inherit any protocol 1.0
compatibility evidence.

## Implemented source

The integrated generation now contains:

- a separate `dfmcp_bridge_v1_1` native plugin and protobuf package;
- the same two-method native waist as protocol 1.0:
  - `Handshake`;
  - `ReadObservation`;
- additive announcement request fields 8-9 and reply fields 20-25 inside `ReadObservation`;
- a safe-Rust protocol-1.1 native RPC client with exact plugin, protocol, nonce, method-manifest,
  bridge-version, generation, frame, field, text-notification, and payload fencing;
- a bounded canonical batch with strict report-ID ordering, retained-window bounds, cursor
  continuation, explicit historical gaps, 512-record limit, 2,048-byte text limit, and 2 MiB
  canonical-byte limit;
- an atomic combined citizen-and-announcement capsule whose identity is independent of citizen page
  size and whose multi-page path requires a paused world plus byte-identical announcement evidence;
- a permanently poisoned source fence after any ambiguous transport failure;
- a numeric-loopback-only connector;
- deterministic world projection into source-bound announcement entities plus explicit retained
  suffix and incomplete-history coverage domains;
- authority-free agent briefing, attention, heartbeat/change summary, bounded recent-record display,
  and explicit continuation guidance;
- an explicitly unqualified diagnostic probe guarded by
  `DFMCP_ALLOW_UNQUALIFIED_ANNOUNCEMENT_PROBE=1`;
- native qualification, source-only qualification, A1-A6 acceptance, evidence-journal, secret-scan,
  and generation-qualification machinery.

The crate root compiles and exports this complete typed stack. Protocol-1.1 source qualification
binds every implementation, contract, test, wrapper, probe, and status document named by its
machine contract.

## Not established

No checked-in evidence currently establishes:

- a successful native protocol-1.1 plugin build against a named clean DFHack revision;
- plugin load inside a named Dwarf Fortress process;
- a completed A1-A6 disposable-fort campaign;
- a completed baseline R1-R5 campaign re-executed for protocol 1.1;
- exact compatibility-registry admission;
- a trusted deployment-floor generation containing a protocol-1.1 entry;
- source-bound release-server qualification for a protocol-1.1 MCP configuration;
- admitted runtime launch;
- complete fortress announcement history;
- any mutation capability.

The checked-in compatibility registry remains empty. Therefore protocol 1.1 cannot start through
the admitted live launcher and must not be described as supported, compatible, or runnable merely
because the source exists.

## MCP posture

The diagnostic probe can exercise the typed protocol-1.1 read, canonical capsule, and world
projection when explicitly enabled. The admitted MCP live server remains protocol-1.0 read-only
source and is not silently switched to protocol 1.1. Promoting announcement data into the normal
MCP Agent Turn requires a separately reviewed server configuration, source-bound server receipt,
exact compatibility entry, monotonic-floor acceptance, and fresh single-use process admission.

## Historical prototype cleanup

The superseded standalone `ReadAnnouncements` contract, checker, qualification wrapper, and design
documents have been removed. Some older prototype code remains in the protocol-1.0 Rust/native
files and is not part of the protocol-1.1 source qualification identity. That prototype must be
removed or permanently isolated before protocol 1.1 can claim one unambiguous implementation path.

## Next evidence gates

1. Run the complete source-only protocol-1.1 qualifier on a clean latest-nightly checkout.
2. Fix every format, compile, Clippy, test, rustdoc, checker, and probe-help failure.
3. Build `dfmcp_bridge_v1_1` against one exact clean DFHack revision and issue its native receipt.
4. Execute all 43 A1-A6 cases against a disposable fortress while retaining secret-free evidence.
5. Re-execute the baseline R1-R5 citizen/fortress campaign for the same exact source, plugin,
   versions, and platform.
6. Review and promote one exact protocol-1.1 tuple into a new compatibility-registry generation.
7. Advance the deployment floor, qualify the exact server configuration, and launch only through
   the admitted descriptor-and-ticket chain.

Until every applicable gate passes, the correct status is: **implemented source, not admitted,
read-only, incomplete historical coverage, and zero mutation authority**.
