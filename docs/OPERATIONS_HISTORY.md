# Durable operations observations and historical queries

The operations/1.3 development server can now keep its canonical observation
sequence in an optional local journal. It records source observations before
publishing their live anchors and reconstructs the exact version chain after a
restart. Historical queries reuse the existing typed query engine without
replacing the current snapshot or evaluating a live watch against old facts.

This is source-present functionality. Rust execution, real filesystem crash
qualification and live-game validation have not been established. The journal is
not the prospective FrankenSQLite MVCC backend, an effect ledger, a game/save
checkpoint, a durable watch service, signed provenance or an anti-rollback floor.
No production admission, dependency pin or native bridge byte changed.

## Enable storage as an operator

Leave `DFMCP_OPERATIONS_JOURNAL` absent to preserve process-local observation
storage. To enable it, select an absolute normalized file path in an existing
private directory. The directory must be a real canonical Unix directory with
mode 0700; an existing file must be a regular, non-symlink, single-link file with
mode 0600 and the same owner as its parent. The backend creates a missing file
exclusively with mode 0600. It does not create directories or overwrite files.

With the existing operations token configured, a development invocation is:

```bash
DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_3=1 \
DFMCP_OPERATIONS_JOURNAL=/absolute/private/directory/operations.bin \
cargo run --locked --bin dfmcp-live-operations-dev-server
```

The path is operator process configuration, never an MCP argument. Journalled
sessions require both Query and Observe grants. File locking is exclusive and
nonblocking for the lifetime of the session; a second writer refuses rather
than sharing or racing the stream. The current process-configured path therefore
supports one journal-owning session at a time. Existing sessions end at process
shutdown; session-close and automatic rotation are separate unfinished work.

The file and parent are checked before and after opening and at I/O boundaries.
Their inode identities, owner relationships, modes and link count must remain
consistent. Ancestors and the owning account/root are trusted. These checks and
advisory locking are not protection against a hostile owner rewriting storage.
Non-Unix platforms refuse this backend instead of silently dropping custody.

## Publication and restart

The runtime obtains an authenticated initial native operations observation to
identify the fortress and negotiate fresh session authority. It then opens the
journal, validates every retained frame and replays each observation through the
actual `LiveOperationsState` projector. Every generated anchor must equal the
anchor recorded in that frame. A different fortress or software lineage refuses.

The current initial observation is appended after recovery. If it is an exact
heartbeat, no duplicate record is written. Otherwise its successor or reset is
computed from the recovered state, preserving sequence and generation history.
New sessions have new process-scoped IDs and fresh grants; old handles, watches,
baselines and capabilities are not restored from the archive.

For each later native observation:

1. Validate acquisition limits, source integrity and the candidate projection.
2. Check current and target authority, record/byte capacity and file identity.
3. Write the complete chained frame and call file `sync_all`.
4. Publish the candidate state and journal metadata in memory.

The new file's directory entry is synced before acknowledged appends. A failed
write or sync fences the journal object and the live source; no new in-memory
anchor is reported as published. A complete record whose acknowledgement was
lost can nevertheless be recovered on reopen. An incomplete record requires the
explicit recovery policy below. No automatic blind retry occurs.

A later MCP response-rendering failure does not undo a successfully synced
observation, just as it cannot undo an already performed bridge read. Reopen and
history inspection expose the durable result; query-baseline and watch state
remain governed by their separate render-before-publication boundaries.

## Incomplete-tail recovery

Default recovery is read/verify only. An incomplete header, corrupt complete
frame, wrong predecessor, changed source digest, nonreproducible anchor, unexpected
trailing data or incomplete final record causes refusal without modifying bytes.

The operator may set `DFMCP_OPERATIONS_JOURNAL_REPAIR=1` alongside the path to
permit truncation of an incomplete trailing frame after a completely verified
prefix. Only that suffix is removed, then the file is synced. The response's
history summary reports `repaired_tail_bytes`. An empty or incomplete existing
header is not initialized as a new archive. Complete corrupt frames and invalid
length-header checksums are not reclassified as torn writes and are not repaired.

There is no automatic repair setting exposed to an agent. There is also no
external trusted high-water mark: deletion of an entire valid suffix, replacement
with a valid older archive, or wholesale forgery by the owner is not detected as
rollback. Hash chaining is integrity evidence, not authentication.

## Query the retained history

The operations schema adds two variants to its existing eighteen query forms.
The shared sixteen-variant schema and other live runtimes remain unchanged.

Pass this envelope to the existing `fortress.query` tool, with the session ID
supplied normally. `mode="history"` is a convenience equivalent:

```json
{"schema":"dfmcp.query/1","query":{"kind":"history","limit":8}}
```

Rows identify record number, exact anchor, source digest, frame digest,
predecessor digest and encoded size. They come from the verified in-memory index.
Pages are whole-row bounded. A continuation binds the journal ID/head, current
session and complete current anchor, so a refresh that changes the head rejects
an old listing continuation. Page width may change; default is eight, maximum 64.
A missing journal produces a typed refusal, not a misleading empty history.

For a historical read, use an exact record number and digest from a returned row.
For example, given `history_response` from the preceding query:

```python
record = history_response["rows"][0]
request = {
    "schema": "dfmcp.query/1",
    "query": {
        "kind": "historical_query",
        "record": record["record"],
        "record_digest": record["record_digest"],
        "query": {"kind": "entities", "kinds": ["item"], "fields": ["stack_size"], "limit": 2}
    }
}
```

The nested query may be `entities`, `inspect`, `traverse`, `dependencies`,
`aggregate` or `search`. It cannot capture a baseline, register/poll a watch,
refresh the bridge, mutate the game or recursively query history. Generation
checks and query continuations operate on the exact archived snapshot. An outer
`expected_anchor`, when supplied, means the current session anchor, not the past
anchor. Current Query authority is checked before replay; an old grant cannot
be revived by selecting a historical tick.

Historical reads re-read and verify the header and every frame needed to reach
the selected record, rather than trusting a cached result. They return the
historical anchor with matching source evidence, `historical=true`,
`current_live_anchor` and `current_freshness_proven=false`. Agent Turn continuity
is explicitly partial/historical. Current-session active watches are retained
but labelled as current work, not archived obligations. Their state does not
advance and the live snapshot is not replaced.

An existing session can inspect valid archived observations after its native
source fails. This does not make the source healthy. The development binary
still requires live bootstrap when opening a new session; an offline replay CLI
or offline-only server has not been added.

## Format and bounds

The version-1 journal header is 80 bytes: eight-byte magic `DFMOJ001`, fortress
u64, journal identity and domain-separated SHA-256 header digest. All journal
integers are big-endian. Each frame has a 44-byte length header (`DFMOREC1`, u32
body length, header digest), body, 32-byte frame digest and `DFMOEND1` footer.

The body contains ordinal, predecessor digest, complete anchor, source digest,
bridge generation, bounded DF/DFHack version strings and the existing canonical
operations payload. The length header has its own digest so a corrupt length
cannot masquerade as a valid incomplete record. Replay rejects duplicate
heartbeat records, missing/reordered predecessors, trailing body data and any
mismatch between retained source and reconstructed projection.

Runtime retention defaults to 64 MiB and 1,024 changed observations; the library
allows up to 256 MiB and 4,096 records. Capacity exhaustion refuses further
publication rather than deleting history. Existing native acquisition ceilings
are unchanged. Replay checks each source against the current acquisition budget
and checks wall time between steps. Filesystem reads and fsync are synchronous;
these checks do not promise preemption of a stalled kernel/storage operation.
Historical replay is prefix replay, not an indexed constant-time snapshot read.
No compaction, migration, automatic retention or checkpoint acceleration exists.

This stores observed endpoints only. Heartbeats are deduplicated and intervals
between observations are not recorded. It does not prove continuous conditions,
complete game history, successful effects, resource availability or path access.

## Evidence

Fifteen Rust regression scenarios are registered: ten journal/storage tests and
five actual operations-handler scenarios. They cover exact reopen, every torn
frame prefix, corruption, injected write/sync failures, generations and resets,
authority, capacity, real-file custody/locking, historical inventory inspection,
restart identity, 8192-byte pagination, current active work and stale sources.
They have not been compiled or executed in this editing environment.

An independent Python framing-design oracle passed 5,833 checks, including every
truncated prefix, one-bit mutation at every byte, reordered/duplicated/spliced
records and 250 seeded payload sizes. It treats semantic anchors as opaque and
is not execution of the Rust code or verification of the world projector. JSON
Schema meta-validation and source-byte checks are also development checks only.
No Rust, Clippy, rustfmt, stdio, real storage crash campaign, live DFHack or full
repository qualification receipt is claimed for this source generation.
