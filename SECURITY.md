# Security

## Security objective

A client should be able to observe and operate only the fortress state, action classes, entities,
map regions, resources, files, and time windows explicitly granted to it. Untrusted text and
bridge data must not become ambient authority. Failure or uncertainty must reduce authority, not
expand it.

## Safe defaults

- localhost or stdio only;
- read-only session;
- one selected fortress;
- conservative budgets;
- no arbitrary shell, Lua, DFHack command, memory write, or path;
- no outbound network;
- unknown compatibility disables affected mutation;
- guarded action requires checkpoint policy;
- irreversible action registry disabled;
- diagnostics redact secrets and bound game content.

## Reporting vulnerabilities

Open a private security report through the repository owner’s preferred GitHub security channel.
Include affected commit/version, preconditions, capability scope, reproducible steps, and whether
game/save or host integrity is affected. Do not attach private saves publicly.

## Capability checks

Checks are repeated at:

1. MCP request intake;
2. plan compilation;
3. prepare;
4. lease acquisition;
5. checkpoint;
6. immediate bridge dispatch;
7. compensation;
8. restore/repair.

A previous check is not proof that a later scope or anchor remains valid.

## Prohibited default surfaces

The production MCP namespace must not expose:

- arbitrary command strings;
- Lua eval;
- shell execution;
- arbitrary filesystem read/write;
- native memory addresses;
- unbounded raw object serialization;
- client-selected bridge method names;
- client-selected dynamic library/plugin paths.

Extensions use registered namespaces and typed schemas.

## Prompt injection

Names, announcements, books, engravings, mod text, imported Markdown, web content, and agent notes
are tainted data. They may be searched or summarized but cannot:

- grant or widen capabilities;
- alter policy;
- authorize a plan;
- select executable extension code;
- interpolate into commands;
- suppress warnings;
- choose filesystem/network targets.

## Bridge protocol

The server validates lengths before allocation, recursion depth, string limits, coordinates,
counts, IDs, revisions, enum/schema versions, instance continuity, and digest sizes. Unknown
required fields fail closed. Bridge restart and journal loss are explicit.

## Retry and indeterminacy

Blind retry of a possibly dispatched mutation is a security and integrity failure. Such actions
become `indeterminate`; reconciliation or sealed operator policy is required.

## Filesystem

Checkpoint paths are server capabilities, never client strings. The implementation must reject
traversal, apply an explicit symlink policy, exclude special devices, stage atomically, checksum
all files, fsync in a documented order, and cap file count/bytes.

## Network

Remote mode requires authenticated encryption, replay protection, rate limits, request identity,
and independent capability grants. Listening on a network interface does not imply mutation
authority.

## Supply chain

- safe Rust only in workspace;
- minimal dependencies;
- pinned lockfile once dependencies exist;
- reproducible release process;
- signed source/release/compatibility manifests;
- no build-time network downloads beyond the package manager’s declared sources;
- generated code checked or reproducibly regenerated.

## Security gates

No mutation family is enabled until it has:

- threat model;
- least-authority scope;
- malformed input corpus;
- prompt-injection tests;
- idempotency/replay tests;
- crash and bridge-loss schedules;
- denial-of-service bounds;
- compatibility evidence;
- documented residual risk.
