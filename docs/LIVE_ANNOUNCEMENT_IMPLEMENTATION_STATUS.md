# Protocol-1.1 announcement implementation status

This file records the exact source status of the retained-announcement generation. It does not
create compatibility evidence or runtime authority.

## Current status

**Implemented in source, integrated through the protocol-1.1 read-only `GameAdapter`, and still
unadmitted.**

The checked-in compatibility registry remains empty. No protocol-1.1 source revision, native plugin
binary, Dwarf Fortress/DFHack tuple, platform, server binary, monotonic floor, or running process is
admitted merely because the implementation exists.

## Implemented source path

The isolated protocol-1.1 stack now contains:

```text
DfmcpBridgeV1_1.proto
→ dfmcp_bridge_v1_1.cpp
→ DfHackRpcClientV1_1
→ LiveObservationSourceV1_1
→ complete citizen-page assembly
→ bounded retained-announcement page
→ transactional multi-page announcement publication
→ LiveObservationCapsuleV1_1
→ combined citizen + announcement projection
→ LiveReadAdapterV1_1
→ read-only GameAdapter observe/query/doctor surface
```

Protocol 1.0 remains separate and citizen-only. Protocol 1.1 uses a distinct protobuf package,
plugin name, bridge version, native qualifier, source qualification contract, A1-A6 evidence
contract, and diagnostic probe. It inherits no compatibility or runtime admission from protocol
1.0.

## Native boundary

The protocol-1.1 plugin remains a two-method waist:

```text
Handshake
ReadObservation
```

Announcement fields are embedded in `ReadObservation`; there is no standalone `ReadAnnouncements`
method. The plugin exposes no mutation, command, Lua, keyboard, arbitrary forwarding, client-selected
method, filesystem, or memory-write route.

The native bridge:

- authenticates before observing world, citizen, or report state;
- bounds announcement requests to 1–512 records;
- bounds announcement text to 2,048 UTF-8 bytes;
- sorts and deduplicates retained reports by report ID;
- exposes oldest and latest retained IDs;
- echoes the requested cursor;
- marks a gap before the retained window explicitly;
- marks whether the returned suffix reaches the retained high-water;
- rejects a cursor ahead of the retained high-water, including a nonnegative cursor against an
  empty retained window;
- never claims complete fortress announcement history.

The cursor-ahead refusal is important: returning an accepted empty “complete” result for a cursor
beyond the bridge high-water would create false absence evidence.

## Canonical model and publication

`LiveAnnouncementBatch` validates and canonically hashes:

- exact observation generation, pause state, clock, and site;
- requested cursor and retained bounds;
- strict report-ID order;
- record count and continuation progress;
- complete-through-latest status;
- retained-window gap status;
- bounded report metadata and text.

It rejects cursors ahead of the retained high-water and rejects nonnegative cursors against an empty
retained set.

`read_publishable_observation_v1_1` adds the missing publication transaction. A single bridge call
proves one complete citizen roster plus one bounded announcement page. When more announcement pages
are required, the publisher:

1. requires the fortress to be paused before issuing a continuation read;
2. repeats the complete citizen observation for every announcement page;
3. requires byte-identical citizen capsule state across pages;
4. requires stable retained oldest/latest bounds;
5. requires an exact echoed continuation cursor;
6. requires every partial page to fill the requested page size;
7. enforces a total announcement ceiling;
8. preserves an initial retained-window gap;
9. combines pages into one canonical protocol-1.1 capsule only after the suffix reaches the observed
   high-water.

Any failure returns no capsule. Transport pagination therefore cannot become partial canonical
state, and one-page versus multi-page transport produces the same final capsule identity for the
same observation.

The current canonical ceiling is 512 retained announcements per published capsule. A larger retained
suffix fails with `budget_exceeded` and an explicit next cursor; it is not silently truncated or
published as complete.

## Read-only protocol-1.1 adapter

`LiveReadAdapterV1_1` implements the shared `GameAdapter` contract over transactional publication.
It provides:

- exact fortress/session identity fencing;
- bootstrap into one canonical citizen-plus-announcement snapshot;
- heartbeat detection from the combined capsule digest;
- same-epoch sequence advancement for semantic change;
- new observation epochs for bridge restart or game-clock regression;
- world/site/version/method-manifest switch refusal;
- candidate projection and budget validation before state publication;
- exact-current or exact-prior cursor handling only;
- bounded deterministic queries over fortress, citizen, and announcement entities;
- observation evidence bound to the combined capsule digest;
- explicit retained-history partial coverage and gap warnings;
- read-only health diagnostics.

The adapter grants only `Observe`, `Query`, and `Doctor` at its semantic boundary. Prepare, commit,
action polling, cancellation, checkpoint, and restore all reject before reaching an effect path.

The adapter does not accept cross-call announcement continuation. Its configured cursor and total
ceiling define one complete publishable retained suffix. Wider or incremental runtime policy needs a
separately reviewed server/session contract; it must not reinterpret a partial bridge page as a
canonical world snapshot.

## Projection and agent semantics

Announcement records become deterministic event entities in the same `WorldSnapshot` as fortress
and citizens. The projection binds source batch digest, combined capsule digest, bridge generation,
snapshot anchor, and explicit coverage.

Coverage distinguishes:

```text
fortress.announcements.retained_suffix
fortress.announcements.history
```

The retained suffix may be complete through the observed high-water while history remains partial.
No empty result proves that no older announcement ever existed.

The announcement briefing layer can rank bounded attention and summarize newly observed report IDs,
but attention and briefing are derived presentation. They grant no capability and satisfy no
mutation precondition.

## Source qualification

The source-only contract is:

```text
architecture/live_announcement_source_qualification_v1_1.json
```

It binds the exact adapter API/root, V1 and V1.1 bridge source, wire client, batch model, citizen
assembler, transactional publisher, read-only adapter, projection, briefing, source fence,
connector, probe, contracts, checkers, tests, and documentation.

A source qualification run must pass repository integrity, protocol checks, contract and acceptance
tests, Python/shell syntax, locked offline Cargo metadata, rustfmt, warning-denied Clippy, adapter and
workspace tests, warning-denied rustdoc, and diagnostic-probe help for one exact clean commit.

No fresh passing source qualification receipt is implied by this document or by the latest commits.

## Evidence still required

Before protocol 1.1 can be admitted, the same exact source and binary identities must produce:

1. a passing source-only qualification receipt;
2. a native plugin receipt against one named DFHack source revision;
3. a real disposable-fort A1-A6 campaign;
4. a re-executed baseline fortress/citizen campaign under protocol 1.1;
5. a protocol-1.1 server binary and source-bound receipt;
6. an exact compatibility-registry entry;
7. deployment-host monotonic-floor advancement;
8. authority-free artifact preflight;
9. a fresh single-use process admission ticket and descriptor-only launch.

The current normal admitted MCP server is still protocol 1.0. Integrating protocol 1.1 into a
runnable server requires a separate reviewed server configuration and receipt generation. It must
not reuse a protocol-1.0 registry entry, floor acceptance, server receipt, or ticket.

## Explicitly not established

- no current protocol-1.1 native build receipt;
- no current A1-A6 live receipt;
- no current baseline R2-R5 receipt under protocol 1.1;
- no compatibility-registry entry;
- no deployment-floor acceptance;
- no qualified protocol-1.1 server binary;
- no runtime-admitted protocol-1.1 MCP process;
- no complete announcement-history claim;
- no mutation authority.
