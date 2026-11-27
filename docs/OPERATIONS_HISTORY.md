# Durable operations observations and historical queries

The operations/1.3 and paged operations/1.4 development servers can keep their
canonical observation sequences in optional, profile-specific local journals. It records source observations before
publishing their live anchors and reconstructs the exact version chain after a
restart. Historical queries reuse the existing typed query engine without
replacing the current snapshot or evaluating a live watch against old facts.

The runtime integration and its exact verification scope are recorded in
`IMPLEMENTATION_STATUS.md`. Full MCP execution, filesystem crash qualification
and live-game validation have not been established. The journal is
not the prospective FrankenSQLite MVCC backend, an effect ledger, a game/save
checkpoint, a durable watch service, signed provenance or an anti-rollback floor.
No production admission, dependency pin or native bridge byte changed. The
current implementation and executed verification scope are recorded in
`IMPLEMENTATION_STATUS.md`.

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

For paged operations/1.4, use `DFMCP_OPERATIONS_PAGED_JOURNAL` with
`dfmcp-live-operations-paged-dev-server` and its existing 1.4 opt-in/token. Its
optional recovery flag is `DFMCP_OPERATIONS_PAGED_JOURNAL_REPAIR=1`. Each server
rejects the other profile's journal variables, and each file is decoded only as
the server's fixed profile. An operations/1.3 file is not a 1.4 archive.

The path is operator process configuration, never an MCP argument. Journalled
sessions require both Query and Observe grants. File locking is exclusive and
nonblocking for the lifetime of the session; a second writer refuses rather
than sharing or racing the stream. The current process-configured path therefore
supports one journal-owning session at a time. Explicit session release drops
this lock so a fresh session can reopen the same journal without restarting the
process. Automatic rotation remains separate unfinished work.

The file and parent are checked before and after opening and at I/O boundaries.
Their inode identities, owner relationships, modes and link count must remain
consistent. Ancestors and the owning account/root are trusted. These checks and
advisory locking are not protection against a hostile owner rewriting storage.
Non-Unix platforms refuse this backend instead of silently dropping custody.

## Release and reopen a session

Both operations/1.3 and operations/1.4 accept this call through the existing
`fortress.cancel` tool:

```json
{
  "session_id": "<current operations session handle>",
  "scope": "session"
}
```

Release waits for any foreground request holding the session, closes its native
connection, drops the observation-journal lock, clears its cached projection and
returns its session capacity. Paged sessions return both the shared slot and the
paged slot immediately, including when another request still holds an old
session reference. Subsequent calls with the closed handle cannot read the old
snapshot or resume the connection. A new session performs the normal fresh
bootstrap and journal recovery described below.

Retained process-local baselines or condition-watch records make the call refuse
with `conflict`. Release those records through their existing query operations
first, or explicitly include `"discard_process_local_work": true` to discard
them during teardown. This consent applies even to completed watch records.
The full close receipt must fit the session's output budget before anything is
released; a refusal leaves the connection, journal lock and query records owned
by the original session.

Cleanup is available after a source failure, exhausted request IDs or expired
read grants. It performs no native acquisition, watch evaluation, journal
validation, append, repair, save, restore or game-effect cancellation. The receipt
reports resource counts and partial continuity without a game-state anchor.
Journal bytes are untouched, so damaged archives still require the explicit
recovery policy on their next open. Ordinary cancellation without `scope` keeps
its existing unavailable response. The most recent 32 successful close receipts
are retained in process memory for exact repeat acknowledgements; older closed
handles return `session_not_found`. These receipts do not survive process exit.

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

Both operations profiles expose `history`, `historical_query` and
`historical_changes`. Other live runtimes retain their own query and archive
contracts. Schema discovery first returns the complete annotated schema. If
its descriptions exceed the response allowance, it retries with every validation
keyword intact and marks `schema_annotations.descriptions_omitted_for_budget`.
Literal values, named properties, references and current active work are
preserved. An allowance too small for the complete validation grammar still
refuses explicitly.

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
be revived by selecting a historical tick. All repeated semantic-query checks
use the unchanged current authority context, including after a restore has
made the current game tick smaller than the selected historical tick. Replay,
query execution and final publication share one original foreground deadline.
Source admission uses the full negotiated source allowance; Agent Turn and
active-work reservations narrow only the response allowance.

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

## Compare retained observations

Both operations profiles accept `historical_changes` through the same
`fortress.query` tool. List history first, then copy the exact record numbers
and lowercase digests for the two endpoints. For example:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_changes",
    "from": {"record": 1, "record_digest": "<first record digest from history>"},
    "to": {"record": 2, "record_digest": "<second record digest from history>"},
    "select": {
      "kind": "entities",
      "kinds": ["item"],
      "fields": ["stack_size"]
    },
    "limit": 16
  }
}
```

The result contains complete before/after rows for `changed_in_result`,
`entered_result` and `left_result`, plus counts and digests of both complete
selected projections. These are changes in selection membership: leaving a
selection does not prove an object was destroyed. A reused entity ID with a
new generation produces a separate departure and arrival. Observation-only
provenance refreshes are counted without inventing a value change.

Selectors use the existing entities grammar: `kinds`, `where`, `fields` and
`order`. The comparison owns pagination, so `select.limit` and
`select.continuation` are rejected. Unknown fields, invalid nested predicates
and malformed selectors are rejected before archive replay. A comparison page
has at most 128 change records; each complete endpoint selection is limited to
256 rows and 256 KiB. Narrow fields and filters when a complete selected set
would exceed those bounds. A returned continuation is bound to the exact
records, selection, current session and anchor, archive identity and archive
head. Changing the page width is permitted; appending a new observation or
reopening the session invalidates the old continuation.

Record order must be forward within one observation epoch. Comparing a record
to itself produces an empty comparison. Crossing a reset is refused; a pair
within a retained epoch can still be queried after the current session has
reset, under its current Query grant. The selected ticks never replace the
current authorization tick or renew an expired grant.

Both complete endpoint sources share one negotiated record/byte allowance and
one original foreground deadline, including verified replay, selection,
comparison and publication. The first endpoint is admitted before acquiring
the second; an identical endpoint is replayed and charged once. Complete
source structure has the existing 16 MiB / four-million-work-unit ceiling.
Response reservations account for archive evidence, the Agent Turn and current
active work independently of source admission. A small selected result does
not authorize an oversized source. No partial comparison is returned when
source, selection, deadline or response admission fails.

The owner rechecks the fixed profile, published anchor, exact record digests
and current archive custody. A corrupted archive refuses and fences the live
source. Historical Agent Turns name the target endpoint and explicitly report
partial temporal continuity: they prove neither intervening events nor current
game freshness. Their current active-work section is preserved without
sampling watches, creating baselines, reading the native bridge, changing the
current anchor or authorizing mutations.

## Format and bounds

The version-1 journal header is 80 bytes: eight-byte magic `DFMOJ001` for 1.3
or `DFMPJ001` for 1.4, fortress
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

## Inspect furniture arriving between two records

Use `historical_furniture_arrivals` after observing a production interval. Supply
the two record numbers and full lowercase digests returned by `history`.
Both records must belong to one observation epoch in the same operations journal.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_furniture_arrivals",
    "from": {"record": 1, "record_digest": "<first record digest>"},
    "to": {"record": 2, "record_digest": "<later record digest>"},
    "furniture_kind": "bed"
  }
}
```

Choose `bed`, `chair`, `door` or `table`. An optional `material` pair selects an exact
native material type and index at both endpoints. The report counts non-removed
item records, including installed and held furniture. It gives both endpoint
counts, the signed change, and the number present at the later endpoint whose
native ID is at least the earlier `next_item_id`. Thus an old bed leaving and
a new bed arriving can yield zero net change and one new ID. Each page of up to eight
qualifying IDs includes their entity generations, numeric materials and subtypes.
The count always covers the complete qualifying roster, even when the examples
are capped. An item that existed only between the captures is outside this measure.

Use the two capture references for follow-up inspection. Classifying a material
as wood and attributing an item to a particular order require independent evidence.

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

### Independently inspect selected furniture materials

For a paused target endpoint, the operator can resolve the material of each
returned example in the running fortress. Copy
`bridge/oracles/dfmcp-furniture-materials.lua` into DFHack's `hack/scripts/`
directory. Save the decoded successful arrivals result as `arrivals.json`.
Print the command for its selected native item IDs with:

```bash
python scripts/verify_furniture_materials.py arrivals.json
```

Run the printed command at the DFHack console while the fortress remains paused.
Save its JSON output as `materials.json` and compare the two observations:

```bash
python scripts/verify_furniture_materials.py arrivals.json materials.json
```

The script reads at most eight selected items under one DFHack suspension.
It resolves each numeric material pair through `dfhack.matinfo.decode` and
reports the raw material token, mode and WOOD flag. The comparison requires
matching DF and DFHack versions, fortress identity, paused clock, item ID
frontier and exact selected item IDs, kinds, subtypes and material pairs. A
mismatch refuses the comparison. Undecodable raws retain an unknown classification.
A plant material with its WOOD flag set contributes to the selected wood count.

The resulting classification describes the independent read. The two calls
cannot establish continuous process identity, production by an original order,
or the materials of items outside the selected page.

### Audit all matching arrivals

The finite work-order catalog yields beds, chairs, doors and tables. Select
`"furniture_kind":"door"` for doors. A query returns at most eight examples
in ascending native item ID order. When more remain, submit the complete
`next_query` envelope returned by the server to the same operations query
handler. It preserves both record numbers and digests, furniture kind and
material filter, and starts at the first matching item omitted from this page.
Repeat until `next_query` is null. Each request replays the selected endpoints
under current authority and the existing source, time and output limits.

`before_item_count`, `after_item_count`, `new_item_count` and the net change
remain totals for the two endpoints on every page. `examples_skipped` counts
matching arrivals below the inclusive `examples_from_item_id` floor. The
existing `examples_truncated` field is true whenever the page alone contains
fewer rows than the complete arrival count, including on the final page of
a multi-page result. Use `next_query` to determine whether another page exists.
An explicit floor beyond the last match returns an empty page with the full
counts and a null continuation.

For an independent material audit, run the verifier on each nonempty page
while the target is still paused. Across a complete traversal, each following
`examples_skipped` equals the number of examples read so far, and the final
sum of page lengths equals `new_item_count`. A material check can return
unknown, including on a complete traversal.

### Assess a minimum new plant-wood inventory

For an explicit numerical goal, supply the page pairs in traversal order:

```bash
python scripts/verify_furniture_goal.py --minimum 3 \
  --page arrivals-1.json materials-1.json \
  --page arrivals-2.json materials-2.json
```

Start at the original item allocator frontier. The verifier checks each
continuation against the next page, including the first matching ID, skipped
count, kind, filter and original record references. It also checks both
source digests, journal identity, profile, anchor lineage and full counts.
A later append to the journal can coexist with unchanged selected records.
A missing middle inventory page, repetition, substitution or conflicting
material classification refuses the assessment. Pass `-` for a page's
missing material observation, including an empty inventory page. A missing
tail leaves a partial result with the next query.

`met` requires at least the requested number of independently verified new
plant-wood items. `insufficient` requires complete arrival coverage and
fewer possible qualifying items than the requested minimum, even if every
unresolved item qualified. Other cases return `unknown`. Counts distinguish
verified wood, verified other materials, selected items without a resolved
material and unseen arrivals. The command reads at most 8,192 pages and
256 MiB across all inputs. It assesses the supplied historical inventory
evidence and carries no current-game authority or original-order attribution.
