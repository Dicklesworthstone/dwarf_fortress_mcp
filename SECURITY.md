# Security

## Security objective

A client may observe or operate only the fortress state, action classes, entities, map regions,
resources, files, and time windows explicitly granted to it. Untrusted text, bridge data,
recommendations, local artifacts, and compatibility metadata must never become ambient authority.
Failure or uncertainty reduces authority; it never expands it.

The current live protocol is read-only. The checked-in compatibility registry has no admitted
entries. No current deployment is authorized merely because the source exists.

## Current trust posture

### Rust trust domain

The Rust workspace uses `unsafe_code = "forbid"`. The semantic core, adapter, MCP presentation,
admission-ticket consumer, and canonical state machinery perform no C/C++ FFI or direct Dwarf
Fortress memory scraping.

### Native game domain

Dwarf Fortress and DFHack are external native processes. The current bridge plugin uses DFHack’s
supported native protobuf RPC service over loopback and exposes exactly:

```text
Handshake
ReadObservation
```

Protocol V1 exposes no mutation method, arbitrary command, Lua evaluation, keyboard injection,
client-selected method, filesystem path, native address, or remote-service flag.

### Local deployment domain

Exact compatibility, anti-rollback custody, artifact qualification, and process admission are
separate proofs:

```text
R1-R5 evidence
→ exact registry entry
→ owner-private monotonic floor
→ exact manifest and entry fence
→ source-bound server receipt
→ descriptor-bound launcher
→ single-use Rust ticket
```

The monotonic floor and ticket defend against accidental bypass, stale local generations,
cross-process ticket reuse, permissive custody, path replacement, and same-inode executable byte
substitution within the documented same-host model. They do **not** defend against compromise of
the owning account or root that can replace the floor, launcher, executable, process, and evidence
together.

## Safe defaults

- stdio or numeric loopback only;
- read-only live session;
- one exact fortress identity per session;
- conservative multidimensional budgets;
- no arbitrary shell, Lua, DFHack command, memory write, or path;
- no outbound network from the live bridge path;
- unknown or absent compatibility grants no capability;
- no registry entry means no admitted live process;
- guarded or irreversible registries remain disabled;
- diagnostics redact secrets and bound untrusted text;
- direct `serve-live` invocation fails closed;
- no path-based fallback after descriptor qualification.

## Authentication and credential custody

The V1 bridge uses a 32–256-byte loopback bearer secret and a bounded client nonce. Credentials:

- are process configuration, never MCP arguments;
- are redacted from Rust `Debug` output;
- are not written to compatibility decisions, floor files, server receipts, launch records,
  admission tickets, Agent Turns, or diagnostics;
- are compared only after admitted length checks using a fixed full comparison workload;
- do not protect against a process with access to the DFHack process environment or memory.

The authority-free admission doctor does not read `DFMCP_BRIDGE_TOKEN`, connect to DFHack, execute
the server, or mutate compatibility custody.

## Compatibility and rollback security

The machine registry is content-addressed and exact. Each entry binds source revisions, native
plugin bytes, versions, platform, R1-R5 evidence, capabilities, coverage, omissions, and
limitations. Compatibility never flows automatically to adjacent versions, binaries, commits, or
platforms.

The local monotonic floor additionally binds:

- exact registry file SHA-256;
- canonical registry digest;
- ordered admitted entry IDs;
- monotonic sequence;
- previous floor digest.

Custody requires:

- an absolute floor path;
- a real `0700` parent directory;
- a regular non-symlink `0600` floor file;
- ownership by root or the effective user;
- no-follow reads;
- exclusive initialization;
- compare-and-swap advancement;
- atomic replacement and directory fsync;
- preservation of every previously accepted entry ID.

An older but structurally valid registry therefore cannot silently replace the trusted generation.
Silent entry removal is not revocation; an evidence-bearing revocation schema must be designed
separately.

## Server artifact and execution integrity

A server-binary receipt qualifies only one exact release executable and source generation. It binds
complete local qualification, source-file digests, toolchain, platform, executable checks, size,
and SHA-256.

The admitted launcher:

1. validates credential shape and rejects loader override variables;
2. verifies registry and monotonic floor equality;
3. resolves the exact manifest under the required entry ID;
4. verifies the source-bound server receipt;
5. opens the executable without following a symlink;
6. verifies owner, mode, device, inode, length, and SHA-256;
7. re-reads registry and floor after artifact verification;
8. re-hashes the open executable before ticket issuance;
9. issues an owner-private short-lived ticket containing no bridge secret;
10. re-reads registry and floor immediately before execution;
11. re-hashes the open executable immediately before descriptor-only `execve`.

The Rust process revalidates the ticket, process ID, expiry, exact read-only capability list,
registry, decision, floor file/content/sequence, receipt, launch digest, executable metadata, and
executable SHA-256. It deletes the ticket and proves its absence before starting MCP.

## Dynamic loader and environment injection

Admitted execution rejects `LD_*`, `DYLD_*`, `GLIBC_TUNABLES`, `LIBPATH`, `LDR_CONFIG`,
`LDR_PRELOAD`, and `SHLIB_PATH`. The launcher refuses a path-based execution fallback. This narrows
the ordinary loader-injection surface but is not a hostile-root sandbox.

## Source and contract integrity

Expected source and contract text must be valid UTF-8, contain no NUL bytes, and remain within an
explicit size bound. The repository integrity checker enforces this before deeper Python or Rust
gates. This rule exists because a previously checked-in Python verifier blob was found corrupted
and unimportable; source text validity is now an explicit security and qualification invariant.

## Capability checks

Checks are repeated at every authority transition that applies:

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
11. executable admission.

A previous check is not proof that a later scope, anchor, policy, floor, binary, or process remains
valid.

## Prohibited default surfaces

The production MCP namespace must not expose:

- arbitrary command strings;
- Lua evaluation;
- shell execution;
- arbitrary filesystem read or write;
- native memory addresses;
- unbounded raw object serialization;
- client-selected bridge method names;
- client-selected plugin or dynamic-library paths;
- client-selected executable paths as authority;
- compatibility or admission mutations through MCP.

Extensions require registered namespaces, typed schemas, explicit capability and risk classes, and
separate evidence gates.

## Prompt injection and tainted content

Names, announcements, books, engravings, mod text, imported Markdown, web content, agent notes,
memories, and model rationales are tainted data. They may be searched or summarized but cannot:

- grant or widen capabilities;
- alter policy or the compatibility floor;
- authorize a plan or process launch;
- select executable code;
- interpolate into commands;
- suppress warnings or omissions;
- choose filesystem or network targets;
- satisfy a live mutation precondition.

## Bridge and wire validation

The Rust client treats every bridge response as untrusted until it validates:

- frame and payload bounds;
- canonical protobuf varints and Booleans;
- duplicate required fields;
- string UTF-8 and byte limits;
- coordinates, counts, IDs, and revisions;
- nonce, protocol, version, method manifest, and bridge generation;
- canonical ordering and pagination offsets;
- completeness and projection consistency;
- text-notification count and aggregate-byte budgets;
- no malformed trailing data.

Bridge restart, version drift, world switch, cursor gap, and transport failure are explicit. A
failed live source is permanently fenced for that session.

## Retry and indeterminacy

Blind retry of a possibly dispatched mutation is a security and integrity failure. Such actions
become `indeterminate`; reconciliation through operation lookup and authoritative observation is
required. The current live protocol has no mutation route, so this remains a target invariant for
future effect generations.

## Filesystem and checkpoints

Checkpoint paths are server capabilities, never client strings. Any implementation must reject
traversal, apply an explicit symlink policy, exclude special devices, stage atomically, checksum
all files, fsync in a documented order, cap file count and bytes, and create a new observation epoch
on restore.

## Network

The current live bridge is loopback-only. Future remote mode requires authenticated encryption,
replay protection, rate limits, request identity, peer policy, and independent capability grants.
Listening on a network interface never implies mutation authority.

## Supply chain

- safe Rust only in the workspace;
- closed minimal dependency universe;
- exact `fastmcp_rust` revision pin;
- locked and offline dependency resolution during qualification;
- no hidden second runtime;
- reproducible generated code or checked generated output;
- source-bound machine-readable receipts;
- exact asset manifests, checksums, signatures, and SBOMs before release claims;
- no correctness reliance on GitHub-hosted Actions.

GitHub workflow files are portable job specifications for controlled local/self-hosted execution,
including `doodlestein_self_releaser`; they are not security or release evidence by themselves.

## Security gates

No mutation family is enabled until it has:

- a threat model and residual-risk statement;
- least-authority capability and scope;
- exact compatibility generation;
- malformed-input and prompt-injection corpora;
- idempotency, duplicate-delivery, and replay tests;
- transport-loss, crash, restart, and bridge-loss schedules;
- denial-of-service bounds;
- prepare/revalidate/commit/observe/prove semantics;
- operation lookup and indeterminate reconciliation;
- checkpoint and compensation policy where applicable;
- disposable-fort evidence for named versions and binaries.

## Reporting vulnerabilities

Use a private GitHub security report to the repository owner. Include the affected commit and exact
artifact identities, preconditions, capability scope, reproducible steps, whether the registry or
floor is involved, and whether game/save or host integrity is affected. Do not attach private saves,
credentials, floor files, or unreleased evidence publicly.
