# Live DFHack read path

## Status

This document describes the first game-facing implementation slice. The source now contains:

- a genuine DFHack plugin built with `dfhack_plugin(... PROTOBUFS ...)`;
- exactly two plugin RPC methods, `Handshake` and `ReadObservation`;
- loopback bearer authentication and per-client nonce binding;
- a safe-Rust client for DFHack's native remote protocol;
- dependency-free protobuf-Lite encoding/decoding for the exact admitted messages;
- bounded citizen pagination;
- a canonical `LiveObservationCapsule` whose identity is independent of pagination;
- static gates that reject mutation paths, remote enablement, custom framing, and contract drift.

None of that proves that the plugin has compiled against the current local DFHack checkout or run
against a disposable fortress. Until those acceptance runs produce evidence for the exact source
revision, `IMPLEMENTATION_STATUS.md` remains authoritative: this is unqualified source, not a
supported live adapter.

## Why this path

DFHack already supplies the correct extension mechanism. A plugin may export
`plugin_rpcconnect`, return an `RPCService`, and register typed protobuf functions. The DFHack
remote server performs transport negotiation, method binding, request framing, core suspension,
protobuf parsing, and result framing. Replacing that with a parallel socket daemon would create a
second lifecycle, authentication, framing, compatibility, and suspension protocol without buying
anything useful.

The live path is therefore:

```text
Rust process
  → loopback TCP connection to DFHack remote server
  → DFHack native handshake (DFHack? / DFHack!, protocol 1)
  → CoreService.BindMethod for dfmcp_bridge.Handshake
  → CoreService.BindMethod for dfmcp_bridge.ReadObservation
  → authenticated plugin Handshake
  → one or more authenticated ReadObservation pages
  → strict Rust validation
  → ObservationAssembler
  → immutable LiveObservationCapsule
  → canonical world projection
  → Agent Turn Packet
```

A transport result is never itself an observation. Publication occurs only after the complete
capsule validates.

## Native plugin

Location:

```text
bridge/dfhack-plugin/
├── CMakeLists.txt
├── proto/DfmcpBridge.proto
└── src/dfmcp_bridge.cpp
```

Protocol V1 registers only:

```text
Handshake
ReadObservation
```

There is deliberately no pause, resume, dig, teleport, Lua, command forwarding, keyboard input,
or arbitrary memory mutation method. Mutation will be a separate protocol generation and a
separate acceptance gate. A read-only plugin is much easier to reason about, fuzz, deploy, and
withdraw.

### Data returned

Handshake returns:

- exact bridge protocol and implementation versions;
- exact DFHack and Dwarf Fortress versions;
- bridge process generation;
- world-loaded and fortress-mode posture;
- the exact supported method set;
- the echoed client nonce.

Observation returns:

- bridge generation and nonce;
- world-loaded and fortress-mode posture;
- pause state;
- world year and year tick;
- world name and save folder;
- site ID;
- total citizen count;
- canonical page offset and completeness;
- bounded citizen records sorted by stable unit ID.

The first roster intentionally avoids direct reads through unstable generated military/squad
fields. It uses supported `World`, `Units`, and `Translation` module APIs plus stable unit basics.

## Authentication and threat model

The plugin reads `DFMCP_BRIDGE_TOKEN` from the Dwarf Fortress/DFHack process environment. The
client presents the same 32–256 byte value with every method. Comparison is length-padded and
constant-time with respect to token contents. The token is never returned, logged, formatted by
Rust `Debug`, or included in a capsule.

The client also supplies a 16–64 byte nonce. The bridge echoes it in every response, and the Rust
client rejects any mismatch. The nonce prevents a response for one negotiated client context from
being silently accepted by another.

This is a loopback bearer boundary, not a host-compromise boundary. It protects against accidental
or unauthenticated callers that can reach the local DFHack remote port. It does not protect against
a process that can inspect the DF process environment or memory, attach a debugger, read the
client's memory, or replace either binary. Host isolation and process ownership remain deployment
responsibilities.

The plugin does not request `SF_ALLOW_REMOTE`. The supported posture is loopback only.

## Rust wire client

`crates/dfmcp-adapter/src/dfhack_rpc.rs` implements:

- the 12-byte DFHack handshake header;
- the native-layout 8-byte message header (`i16 id`, two padding bytes, `i32 size`);
- `BindMethod` request/reply messages;
- bounded protobuf varints, ZigZag integers, booleans, strings, bytes, nested messages, and unknown
  field skipping;
- duplicate required-field rejection;
- exact version, method-set, nonce, generation, offset, ordering, and completeness checks;
- 8 MiB ordinary reply ceiling and 64 KiB text-notification ceiling;
- token-redacted credential formatting;
- graceful DFHack quit framing.

The module accepts an already connected `Read + Write` stream. It does not create a socket, spawn a
thread, select a runtime, or hide deadline/cancellation policy. The caller must supply those under
the project's structured-concurrency context.

## Pagination and canonical identity

Citizen pages are transport artifacts. `ObservationAssembler` requires:

- one bridge generation;
- identical summary fields on every page;
- contiguous offsets from zero;
- stable total count;
- strict unit-ID order inside and across pages;
- no empty nonterminal page;
- no page after completion;
- complete final coverage.

Canonical bytes contain a domain separator, bridge manifests, summary fields, ordered method set,
and every citizen field in fixed order with little-endian integers and length-prefixed UTF-8.
They do not contain token, nonce, TCP details, method IDs, page size, page boundaries, or response
arrival timing.

Therefore:

```text
same semantic observation + different valid pagination
    ⇒ identical canonical bytes
    ⇒ identical SHA-256 capsule digest
```

That property is tested directly.

## Installation into a DFHack source build

The plugin is intended to be copied or symlinked into a compatible DFHack source tree under its
`plugins/` directory, then built by DFHack's own CMake configuration. A qualified integration
script must eventually automate this without modifying an existing user installation in place.
The required shape is:

```text
<dfhack-source>/plugins/dfmcp_bridge/CMakeLists.txt
<dfhack-source>/plugins/dfmcp_bridge/proto/DfmcpBridge.proto
<dfhack-source>/plugins/dfmcp_bridge/src/dfmcp_bridge.cpp
```

Before starting Dwarf Fortress/DFHack, provision a high-entropy token in the process environment:

```bash
export DFMCP_BRIDGE_TOKEN='<at least 32 random bytes>'
```

Do not place the token in repository configuration, command history, logs, MCP arguments, or a
capsule. The future installer should accept a protected token file or process supervisor secret
and inject it without printing it.

The DFHack remote server must be running on loopback. The project assumes the standard DFHack
remote handshake and default port 5000 unless an explicit local configuration says otherwise.

## Acceptance ladder

Source shape is Gate R0, not live evidence.

### R0: static contract

`python3 scripts/check_dfhack_bridge.py` proves only that:

- the registry, proto, C++, Rust client, and capsule remain aligned;
- exactly two RPC methods are registered;
- both are authenticated and read-only;
- no remote flag or known mutation/command route appears;
- the obsolete socket header is absent;
- bounds and canonical capsule laws remain represented.

### R1: native build

Build the plugin inside the pinned DFHack source tree on Linux. Record:

- DFHack source commit;
- Dwarf Fortress version;
- CMake generator and compiler versions;
- generated protobuf sources;
- plugin binary SHA-256;
- complete compiler output;
- symbol inventory proving the two RPC registrations and absence of mutation exports.

### R2: handshake matrix

Against a disposable DF process, prove:

- missing token rejected;
- configured token shorter than 32 or longer than 256 rejected;
- presented short, long, and wrong token rejected;
- correct token accepted;
- short, long, and mismatched nonce rejected;
- protocol mismatch rejected;
- exact method and version manifests returned;
- no token bytes appear in stdout, stderr, structured logs, doctor bundle, or crashpack.

### R3: read determinism

Pause an unchanged disposable fortress and run:

- two identical one-page reads;
- the same roster using page sizes 1, 2, 7, 64, 256, and 4096;
- reads with names included and omitted as distinct declared projections;
- offset-at-total and offset-beyond-total requests;
- hostile oversize requests.

For identical projections, canonical capsule bytes and digests must match exactly. Page
concatenation must equal a single sufficiently large page.

### R4: restart and drift

Prove that:

- plugin/process restart changes `bridge_generation`;
- an old client rejects new-generation pages;
- world unload and non-fortress modes fail explicitly;
- fortress summary drift between pages aborts the capsule;
- no partial capsule is published;
- a new clean handshake can resume.

### R5: agent orientation

Project one capsule through the semantic world model and verify that a cold agent receives:

- exact live provenance and versions;
- one canonical anchor;
- pause/year/site/citizen summary;
- complete versus partial coverage semantics;
- bounded citizen drill-down references;
- no mutation affordance unless a separately qualified mutation adapter exists;
- explicit uncertainty for every unobserved domain.

Only after R1–R5 pass for named versions may the read-only live adapter become `experimental`.
`Supported` requires the wider compatibility, recovery, security, and release gates in the master
plan.

## Non-goals of V1

- mutation of any kind;
- arbitrary DFHack command execution;
- Lua execution;
- remote network access;
- encryption inside the loopback protocol;
- full unit thoughts, needs, skills, inventory, jobs, military, map, economy, or history;
- claiming absence outside the complete citizen-roster domain;
- treating a transport reply as durable evidence before capsule publication.
