# Security

## Security objective

A client may observe or operate only the fortress state, action classes, entities, map regions,
resources, files, protocols, and time windows explicitly granted to it. Untrusted text, bridge data,
recommendations, local artifacts, and compatibility metadata must never become ambient authority.
Failure or uncertainty reduces authority; it never expands it.

The current live protocols are read-only. The checked-in compatibility registry has no admitted
entries. No current deployment is authorized merely because source exists or a development server
runs.

## Trust posture

### Safe-Rust domain

The Rust workspace uses `unsafe_code = "forbid"`. The semantic core, adapters, MCP presentation,
canonical state machinery, and admission-ticket consumer perform no C/C++ FFI or direct Dwarf
Fortress memory scraping.

### Native game domain

Dwarf Fortress and DFHack are external native processes. Current bridge generations use DFHack’s
supported protobuf RPC service over loopback and expose exactly:

```text
Handshake
ReadObservation
```

Protocol 1.1 adds retained-announcement fields inside `ReadObservation`; it does not add a method.
Neither generation exposes mutation, arbitrary command, Lua, keyboard injection, client-selected
method, filesystem path, native address, or remote-service flag.

### Local deployment domain

Compatibility, protocol identity, anti-rollback custody, executable qualification, and process
admission are separate proofs:

```text
native/live evidence
→ exact registry entry
→ owner-private monotonic floor
→ exact manifest and entry fence
→ exact bridge protocol
→ source-bound server receipt
→ descriptor-bound launcher
→ protocol-bound V2 single-use ticket
→ exact private runner
```

The floor and ticket defend against accidental bypass, stale local generations, cross-process reuse,
protocol confusion, permissive custody, path replacement, and same-inode executable byte
substitution within the documented same-host model. They do not defend against simultaneous
compromise of the owning account or root, launcher, floor, executable, process, and evidence.

## Protocol identity is authority identity

A compatibility entry is not merely a game/version tuple. It includes one exact bridge protocol and
implementation. Selecting a different runtime protocol after admission changes semantics and may
change observed fields, coverage, canonical identity, and available code paths.

The V2 admission contract therefore requires exact equality across:

```text
deployment manifest protocol
compatibility decision protocol
launch record bridge_protocol
ticket bridge_protocol
DFMCP_ADMITTED_BRIDGE_PROTOCOL
Rust admission context and provenance
final runner lookup
```

Both launch and ticket digests cover the protocol. The production map currently contains only
protocol 1.0. Protocol 1.1, unknown protocols, mismatches, and legacy V1 tickets fail before server
startup. An environment variable cannot widen the map.

A future protocol-1.1 runner requires independent source, native, live, registry, floor, server
artifact, and process evidence before the map can be changed.

## Development-runtime isolation

`dfmcp-live-v1-1-dev-server` is an explicitly unadmitted evidence-capture surface. It starts only
when `DFMCP_ALLOW_UNADMITTED_LIVE_V1_1` is exactly `1` and refuses production admission state,
including:

```text
DFMCP_ADMISSION_TICKET
DFMCP_ADMITTED_BRIDGE_PROTOCOL
DFMCP_COMPATIBILITY_*
DFMCP_SERVER_RECEIPT_DIGEST
DFMCP_ADMITTED_LAUNCH_DIGEST
```

The public `dfmcp-mcp` wrapper rejects the production protocol marker before entering the private
development server. The runtime uses a distinct session-ID namespace and exposes no production
admission provenance. Development execution is not compatibility or runtime admission.

## Safe defaults

- stdio or numeric loopback only;
- read-only live sessions;
- one exact fortress identity per session;
- one exact bridge protocol per compatibility and process identity;
- conservative multidimensional budgets;
- no arbitrary shell, Lua, DFHack command, memory write, path, or outbound network;
- unknown compatibility or protocol grants no capability;
- no registry entry means no admitted process;
- diagnostics redact secrets and bound untrusted text;
- direct `serve-live` invocation fails closed;
- no path-based fallback after descriptor qualification;
- development binaries cannot consume production admission state.

## Authentication and credential custody

The bridge uses a 32–256-byte loopback bearer secret and bounded client nonce. Credentials:

- are process configuration, never MCP arguments;
- are redacted from Rust `Debug` output;
- are absent from compatibility decisions, floors, receipts, source bundles, launch records,
  tickets, Agent Turns, and diagnostics;
- are compared only after admitted length checks using a fixed full comparison workload;
- do not protect against a process with access to DFHack environment or memory.

The authority-free admission doctor does not read `DFMCP_BRIDGE_TOKEN`, connect to DFHack, execute a
server, or mutate compatibility custody.

## Compatibility and rollback security

Each registry entry binds source revisions, native plugin bytes, game/DFHack versions, bridge
protocol/version, platform, evidence, capabilities, coverage, omissions, and limitations.
Compatibility never flows to adjacent versions, protocols, binaries, commits, or platforms.

The local monotonic floor additionally binds:

- exact registry file SHA-256;
- canonical registry digest;
- ordered admitted entry IDs;
- monotonic sequence;
- previous floor digest.

Custody requires an absolute path, real exact-mode `0700` parent, regular non-symlink exact-mode
`0600` file, root/effective-user ownership, no-follow reads, exclusive initialization,
compare-and-swap advancement, atomic replacement, directory fsync, and preservation of every prior
entry ID.

An older valid registry cannot silently replace the trusted generation. Silent entry removal is not
revocation; an evidence-bearing revocation schema must be designed separately.

## Server artifact and execution integrity

A server receipt qualifies one exact release executable and source generation. It binds the complete
local gate order, admission-ticket contract, source-file digests, toolchain, platform, executable
checks, size, and SHA-256.

The admitted launcher:

1. validates credential shape and rejects loader override variables;
2. verifies registry and monotonic floor equality;
3. resolves the exact manifest under the required entry ID;
4. rejects protocols absent from the reviewed production map;
5. verifies the source-bound server receipt;
6. opens the executable without following a symlink;
7. verifies owner, mode, device, inode, length, and SHA-256;
8. re-reads registry and floor after artifact verification;
9. writes a protocol-bound, secret-free launch record;
10. re-hashes the open executable before ticket issuance;
11. issues an exact-mode `0600` ticket under a real exact-mode `0700` directory;
12. re-reads registry/floor and re-hashes the descriptor immediately before descriptor-only
    `execve`.

The Rust process validates ticket digest, process, expiry, protocol, exact read-only capabilities,
registry, decision, floor file/content/sequence, receipt, launch, executable metadata, and
executable SHA-256. It deletes the ticket and proves absence before invoking the protocol-selected
private server.

## Dynamic loader and environment injection

Admitted execution rejects `LD_*`, `DYLD_*`, `GLIBC_TUNABLES`, `LIBPATH`, `LDR_CONFIG`,
`LDR_PRELOAD`, and `SHLIB_PATH`. The launcher refuses path-based execution fallback. This narrows
ordinary loader injection but is not a hostile-root sandbox.

## Source and archive integrity

Expected source and contract text must be valid UTF-8, contain no NUL bytes, and remain within an
explicit bound. Repository traversal rejects symbolic links, special files, unstable replacement,
machine-local placeholders, and recovery debris.

Release source bundles are built from exact Git objects, not copied worktrees. Verification rejects
path traversal, duplicate semantic members, links, special files, missing/extra/reordered members,
mode or ownership drift, unsupported PAX metadata, mixed timestamps, content mismatch, and nonzero
trailing payload. It never extracts or executes archive members.

A source bundle proves source/archive identity only, not compilation, compatibility, signatures, or
runtime authority.

## Capability checks

Checks are repeated at every applicable authority transition:

1. MCP request intake;
2. session capability negotiation;
3. plan compilation;
4. prepare;
5. lease acquisition;
6. checkpoint admission;
7. immediate bridge dispatch;
8. compensation;
9. restore or repair;
10. exact compatibility resolution;
11. protocol selection;
12. executable and process admission.

A previous check is not proof that a later scope, anchor, policy, floor, protocol, binary, or process
remains valid.

## Prohibited default surfaces

The production MCP namespace must not expose:

- arbitrary command strings;
- Lua evaluation;
- shell execution;
- arbitrary filesystem read or write;
- native memory addresses;
- unbounded raw object serialization;
- client-selected bridge method or protocol names;
- client-selected plugin, dynamic-library, or executable paths as authority;
- compatibility, floor, runner-map, or admission mutations through MCP.

Extensions require registered namespaces, typed schemas, explicit capability/risk classes, exact
protocol generations, and separate evidence gates.

## Prompt injection and tainted content

Names, announcements, books, engravings, mod text, imported Markdown, web content, agent notes,
memories, and model rationales are tainted data. They may be searched or summarized but cannot:

- grant or widen capability;
- alter policy, registry, floor, or protocol map;
- authorize a plan or process;
- select executable code or a runner;
- interpolate into commands;
- suppress warnings or omissions;
- choose filesystem/network targets;
- satisfy a live mutation precondition.

## Bridge and wire validation

The Rust client treats every bridge response as untrusted until it validates:

- frame and payload bounds;
- canonical protobuf varints and Booleans;
- duplicate required fields;
- UTF-8 and string bounds;
- coordinates, counts, IDs, revisions, and report cursors;
- nonce, protocol, bridge version, method manifest, and generation;
- canonical ordering, pagination offsets, retained-window bounds, and continuation progress;
- completeness and projection consistency;
- text-notification and announcement budgets;
- no malformed trailing data.

Bridge restart, version drift, protocol drift, world switch, cursor gap, retained-history gap, and
transport failure are explicit. A failed live source is permanently fenced for that session.

## Retry and indeterminacy

Blind retry of a possibly dispatched mutation is a security and integrity failure. Such actions
become `indeterminate`; reconciliation through operation lookup and authoritative observation is
required. Current live protocols have no mutation route.

## Filesystem and checkpoints

Checkpoint paths are server capabilities, never client strings. Implementations must reject
traversal, enforce an explicit symlink policy, exclude special devices, stage atomically, checksum
all files, fsync in documented order, cap file count and bytes, and create a new observation epoch
on restore.

## Network

Current live bridges are loopback-only. Future remote mode requires authenticated encryption,
replay protection, rate limits, request identity, peer policy, and independent capability grants.
Listening on an interface never implies mutation authority.

## Supply chain

- safe Rust only in the workspace;
- closed minimal dependency universe;
- exact `fastmcp_rust` revision pin;
- locked and offline dependency resolution during qualification;
- no hidden second runtime;
- reproducible generated code or checked output;
- source-bound machine-readable receipts;
- exact source bundles and asset manifests;
- checksums, signatures, SBOMs, and install/rollback evidence before release claims;
- no correctness reliance on GitHub-hosted Actions.

Workflow files are portable specifications for controlled local/self-hosted execution, including
`doodlestein_self_releaser`; they are not security or release evidence by themselves.

## Security gates for future mutation

No mutation family is enabled until it has:

- a threat model and residual-risk statement;
- least-authority capability and scope;
- exact protocol and compatibility generation;
- malformed-input and prompt-injection corpora;
- idempotency, duplicate-delivery, and replay tests;
- transport-loss, crash, restart, and bridge-loss schedules;
- denial-of-service bounds;
- prepare/revalidate/commit/observe/prove semantics;
- operation lookup and indeterminate reconciliation;
- checkpoint and compensation policy where applicable;
- disposable-fort evidence for named versions and binaries.

## Reporting vulnerabilities

Use a private GitHub security report to the repository owner. Include affected commit and exact
artifact/protocol identities, preconditions, capability scope, reproducible steps, whether registry
or floor custody is involved, and whether game/save or host integrity is affected. Do not attach
private saves, credentials, floors, tickets, or unreleased evidence publicly.
