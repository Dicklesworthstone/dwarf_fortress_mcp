# Rust furniture placement and durable recovery

`dfmcp_adapter::build_placement` connects the existing furniture/1.19 native
protocol to a typed Rust client and an owned, durable placement session. The
supported action is one ordinary bed, chair or table, using one exact existing
item at one exact target tile. No item search, substitute selection, arbitrary
DFHack command or new native wire generation is introduced.

The native effect contract remains
[`BUILD_PLACEMENT_NATIVE.md`](BUILD_PLACEMENT_NATIVE.md). A `Placed` receipt proves
historical stage-zero registration of the selected building, its construction
job and the exact item attachment. It does not prove completed construction,
accessibility, structural safety or present usability.

## Canonical evidence and native ownership

The codec validates the complete bounded capture, including native generation,
sequence, game tick, fortress folder/site, map dimensions, building/job ID
horizons, nine terrain cells, the exact item and its ground tile. Hidden and
missing variants have no attribute payload. Unknown tags, malformed booleans,
out-of-range counts, unordered references and trailing bytes are rejected.

Plans derive their digest and preparation token with the unchanged native hash
domains. A placed record must contain the exact predicted post-state and an
independent insertion record agreeing on building, job, item, kind, location,
material and stage zero. Recomputing a receipt hash cannot make a different
post-state pass this semantic check. Every non-prepared native record, including
`Indeterminate`, is immutable.

The client binds exactly the six published methods over authenticated numeric
IPv4 loopback. It pins native software and source identity, rejects malformed
protobuf and impossible reply shapes, and keeps one shrinking connection byte
allowance and absolute deadline. Later tool calls cannot renew that deadline.
Blocking socket operations check the supplied current runtime authority and
cancellation between bounded I/O slices. Failed exchanges fence the connection.

Only a fresh preparation acquired on the same retained connection permits one
commit attempt. The attempt is consumed before sending. Query, replayed
preparation, a reopened journal or a newly connected client cannot recreate that
permission. The public source commit interface additionally requires a
non-cloneable dispatch value produced by the durable coordinator.

## Durable publication and recovery

The coordinator keeps a bounded append-only journal with a source-bound header,
hash-chained frames and a complete end marker. Its binding includes endpoint,
native software, generation, fortress identity and map dimensions. Each retained
entry carries the complete sealed plan and native receipt, when available.

| Durable state | Meaning | Next permitted work |
|---|---|---|
| `intent` | Exact eligible plan published before native preparation | Original foreground owner may prepare; recovery may query or retire |
| `prepared` | Matching fresh native preparation acknowledged after synchronization | Original owner may publish one dispatch intent |
| `dispatch_started` | Dispatch intent synchronized before native commit | One attempt on the original connection, then receipt recovery |
| `tracking` | A query found matching prepared history | Query or retire; no restored dispatch permission |
| `cancel_requested` | Preparation retirement requested durably | Query or complete retirement; no new placement |
| `terminal` | An immutable native outcome was retained | Historical inspection; an indeterminate outcome still blocks new keys |

`terminal` describes immutable journal custody. The separate `unresolved` flag
distinguishes indeterminate native history from resolved placement, refusal or
cancellation. An absent native key after a disconnect or plugin restart remains
unresolved; absence never proves nonapplication. Every unresolved entry fences
new keys throughout this journal. Resolved keys remain reserved.

The source also exposes a validated native-global summary separately from local
history. A preexisting native uncertainty fence or full native retention blocks a
fresh key after its preflight query and before any local intent is written. This
prevents stranding a local obligation for a preparation that the client already
knows cannot be sent. The session retains the last summary for honest historical
inspection after dropping its connection. Original-key query and preparation
retirement remain available despite an unrelated native uncertainty fence.

Every append is checked and replayed before writing. File and parent-directory
synchronization precede acknowledgment. Runtime, source, current capabilities and
the supplied host policy are checked again after dispatch publication and before
the native call. Receipt publication failure cannot acknowledge a known durable
outcome. Partial or corrupt history is preserved and refused; there is no tail
repair, truncation, automatic retry or eviction.

The session owns the journal, current observation and original native connection
across observe, prepare and commit. Commit takes no connection factory. Any
attempt, cancellation or loss of ephemeral permission drops that permission
without deleting the durable obligation. Recovery creates at most one explicitly
requested query/cancellation connection. There is no background poller or worker.

## Filesystem custody and authority

Linux storage opens each path component without following symlinks, pins parent
and file descriptors, and requires an absolute canonical path, a real owned
`0700` parent directory and a regular single-link `0600` journal. Exclusive
nonblocking locking retains one owner. Every storage boundary checks pathname,
descriptor identity and extent; complete replay also detects changed bytes.
Writable access is append-only and synchronizes both file and parent directory.

The three fixed modes are `control`, `recover` and `offline`. Offline replay is
read-only down to the storage implementation: it cannot create, write, flush,
synchronize or repair a journal. Recover mode cannot observe for planning,
prepare or commit. Neither injected grants nor retained evidence can change a
session's fixed mode. The owning runtime must supply its cancellation and I/O
checks. Native requests require fortress-wide capabilities because they validate
complete building/job registries and an item that can be distant from the target.
The MCP host separately restricts the selected item and target with its configured
spatial scope, protected regions and live lease; the broad native grant does not
remove those host checks. Preparation retirement requires Query authority and
remains available after Construct authority is revoked.

This is cooperative host-local custody. Another journal, another controller or
game UI input is not globally fenced. Hashes do not prevent an authorized owner
from replacing all historical evidence. Filesystem synchronization and injected
failure tests are not physical power-loss qualification.

## Validation and compatibility

Focused checks use the independent existing native golden corpus, adversarial
canonical records, real loopback framing, injected storage/native failures and
Linux private files. Run:

```sh
cargo test --locked -p dfmcp-adapter --lib build_placement
```

Rust evidence applies to the exact compiler, source and tests executed. Native
handler tests still use explicit SDK/protobuf doubles; a real DFHack build and
live fortress campaign remain separate requirements. The existing Python journal
format is separate and is not silently imported or migrated. Production
compatibility registry, protocol runner map and admission remain unchanged.

The current focused run passed all 45 tests with zero ignored on pinned
`nightly-2026-08-31` / rustc `1.100.0-nightly (908501772 2026-08-30)`: ten codec,
sixteen actual TCP and nineteen coordinator/session/Linux storage groups. The
adapter production check passed as well. This is executed Rust development
evidence, not full warning-denied workspace or production qualification.

This implementation advances beads `df-dfhack-bridge-plane-c-pic.4` and
`df-dfhack-bridge-plane-c-pic.5`; their broader mutation and live-campaign scope
remains open.
