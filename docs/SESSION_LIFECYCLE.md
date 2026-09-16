# Spatial session lifecycle and recovery

The spatial/1.8 development server now supports explicit session closure through the existing
`fortress.cancel` tool. Previously its session registry retained connections, two-session capacity
permits and exclusive archive/watch locks until process exit. A fenced session therefore prevented
reopening the same journal. The new path releases those resources without restarting the server.

This is implemented, unadmitted development source. Rust compilation and test execution have not
been established for this increment. It does not change native protocols or add game-effect authority.

## Close the session, not the game work

```json
{
  "session_id": "<spatial session returned by fortress.open_session>",
  "scope": "session"
}
```

Send this to `fortress.cancel`. Opening responses now include the corresponding `session_close`
request. The additional arguments are optional, so omitting `scope` retains the original refusal
of game-effect cancellation. Unknown scopes and a discard flag without `scope="session"` are errors.

A successful close drops the bridge connection, releases the session's observation/watch journal
locks, clears its cached world and grants, removes the registry entry and returns its capacity
permit. It does not acquire a capture, change pause state, cancel native work, evaluate a watch,
append a checkpoint, repair/truncate a journal, or delete an evidence file.

When process-local watches or baselines exist, the default request fails with Conflict and leaves
every resource intact. After deciding that these volatile records can be discarded, use:

```json
{
  "session_id": "<spatial session>",
  "scope": "session",
  "discard_process_local_work": true
}
```

This flag applies only to process-local records. It never authorizes deletion, cancellation or
rewriting of persisted watches. Baselines remain process-local even in a journal-backed session.
A persisted watch is released from memory but its existing journal is left unchanged.

## Reopen using the existing journals

After close succeeds, call `fortress.open_session` normally with the same operator configuration
and requested region. A live reopening still needs normal bridge credentials and fresh grants.
Paired durable watches are recovered through the existing startup path with fresh session-bound
handles; unfinished stability resets and requires fresh observations. Old handles cannot identify
the new session's watches. Use `watches` discovery and `await_watches` to resume monitoring.

An archive-only session can also close and reopen its read-only observation archive without a
process restart. The existing recovery-only configuration restrictions still apply: it cannot
load the watch journal, repair files or promote itself into a live session. Changing an operator
environment configuration still requires launching a process with that configuration.

Releasing custody does not certify journal integrity. Fenced or damaged journals can be closed,
but their bytes remain unchanged and normal reopening still rejects corrupt or incomplete evidence.
The close result explicitly says `watch_evidence_revalidated=false`; it is not a recovery receipt.
No close acknowledgement proves that saved game actions or monitoring outside this session are absent.

## Concurrency and retries

Closure serializes on the session's mutex, waiting for its current foreground call to finish.
It does not interrupt a native read, detach work, or promise hard deadline preemption. Already
resolved callers may still hold a reference after registry removal; the closed-source marker makes
`context()` and `refresh()` refuse them before accessing a world or requesting another capture.
The permit is released exactly once even while those old references remain alive.

All local watch/baseline consent checks and complete response rendering happen before any release.
A refused response leaves the original session, registries, file ownership and capacity intact.
The lock order matches existing baseline publication: baseline store, watch store, watch journals.
There is no ordinary fallible check after release that could report failure instead of the prepared
acknowledgement. This is not a filesystem transaction, power-loss guarantee or out-of-memory proof.

The 32 most recently completed close acknowledgements are retained in memory, in completion order.
A repeated close returns the same tool-payload bytes while its receipt is retained and does not
consume a permit twice. Closing a long-lived session cannot immediately evict its own new receipt
just because its ID is older. After receipt eviction or process exit, a stale handle is not found;
it never selects a newly opened session. The acknowledgement cache is not durable audit evidence.

## Authority and failure containment

Teardown uses the existing process-scoped session resolver, not a new authentication or principal
routing mechanism. It releases resources associated with that resolved session only. Renewed Query,
Observe or Doctor grants are not required to release them: expired grants and exhausted request IDs
must not strand connections. The teardown response exposes resource counts and availability flags,
not cached fortress facts, anchors, definitions, evidence, credentials or filesystem paths.

A poisoned individual session mutex can be recovered solely to drop its owned resources. Semantic
registry poison is not cleared or treated as trusted game evidence; other operations still reject
it. This path is not general repair of poisoned process registries. In particular, failure of the
main session registry itself can still require process restart. Other sessions' work is not removed.

## Validation status

Fourteen Rust regression functions are registered through the actual scoped cancel/query handlers
and existing injected bootstrap fixtures. They cover live and archive reopen, unchanged journal
bytes, durable-watch recovery and fresh handles, volatile-record consent, full-render refusal,
expired grants, exhausted request counters, damaged files, poisoned sessions, concurrent closers,
stale references, receipt eviction/order and reclamation across 130 watch/baseline session cycles.
Existing game-effect refusal and archive-production tests are retained.

These tests have **not been compiled or executed here**. Rust, Cargo and rustfmt were unavailable,
and the repository returned no existing main-branch workflow runs to inspect. No Rust, native,
live-game or production qualification is claimed. The focused command on a configured checkout is:

```bash
cargo test --locked -p dfmcp-mcp session_release -- --test-threads=1
```

The session-family limits, frozen eleven-tool surface, native wire formats, dependencies, production
runner and compatibility admission remain unchanged. Other development profiles, including the
separate pause-control runtime, do not gain this scoped-close operation from this increment.
