# Receipt-linked furniture construction conditions

This feature connects a verified furniture/1.19 `Placed` record to later complete
operations/1.4 observations. It is separate from the existing generic
`construction_progress` query: that query deliberately accepts observation-selected
IDs without authenticating a placement receipt. Neither path changes native wire
formats, the eleven-tool MCP surface, production admission or the placement journal.

For an explicit set of 1..32 original placement receipts, use the
[whole-plan construction monitor](RECEIPT_CONSTRUCTION_PLAN.md). It evaluates all
selected targets in one shared capture and requires one global stability streak;
it does not authenticate the completeness of an earlier blueprint or action DAG.

## Implemented condition and replay core

`scripts/construction_receipt.py` strictly decodes the unchanged `DFMO1400` native
capture, including its `DFMJ1200` job component. Full rosters, ordering, ID horizons,
UTF-8, booleans, enum identity consistency, holder/container endpoints, attachment
counts and filters, and acyclic containment are verified before using absence.
Unknown enum keys remain unknown data. Complete acquisition limits are unchanged:
4,096 jobs, 4,096 buildings, 65,536 items, 65,536 attachments and 16 MiB of bytes.
Every traversal invokes the caller's shrinking work/deadline guard.

A goal selects only a valid canonical Placed receipt. It binds the exact original
building ID, one-tile footprint, Bed/Chair/Table type, maximum build stage, item ID,
native item type, subtype and material identity. A successful condition requires
that building at its original maximum stage, no held construction or removal job,
and the original singleton furniture item installed in that building, outside any
container, with no observed job attachment. Job disappearance alone never succeeds.
Missing objects, mismatched identities, horizon/clock/stage regressions invalidate
monitoring. Removal is a failure. Suspended construction, incomplete construction
without a job, and unverified item installation remain explicit pending conditions.

The linked-sample codec carries the original canonical receipt and native manifest
both before and after the complete operations capture. Both furniture generations
must equal the original receipt generation; both record byte strings must equal
the original receipt. Native software versions must agree. Operations generation
is independently pinned, never numerically equated with furniture generation.
These checks are prerequisites, not signatures or authority. A serialized sample
cannot independently prove that network I/O occurred on one connection.

A goal has an absolute exclusive game-time deadline, fixed cadence, sample count,
minimum span, maximum observation gap and a total observation allowance. Repeated
paused observations do not add stability samples. Intermediate false or unknown
conditions reset the streak. Long gaps, interrupted reads and same-tick changed
captures reset stability. Native source changes invalidate the original goal.
Cancellation stops this monitor only. Terminal histories are immutable.

## Evidence and limitations

Run the actual Python implementation and four weakened implementations:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_construction_receipt.py --mutations
```

Sixteen test functions pass, including all 512 item flag words, every truncated
prefix of representative operations/sample/goal records, malformed relationships,
a 4,096-item container chain, original-receipt substitution, source changes,
stability, interruptions and deterministic replay. Four weakened implementations
are rejected by regression assertions. The unchanged furniture fixture and codec
were checked against their actual Git blob identities. Source hashes are retained
in `docs/evidence/construction-receipt-core.json`.

## Same-connection native acquisition

`scripts/construction_monitor_rpc.py` now implements fresh acquisition, binding
only furniture `Handshake`/`QueryPlacement` and operations
`Handshake`/`ReadObservation` on one DFHack TCP connection. It queries the exact
original receipt, reads and validates every immutable operations page, verifies
whole-capture SHA-256 and release acknowledgment, and queries the original receipt
again before returning a sample. Missing original records, changed native source,
mixed pages, malformed replies and lost release/trailing queries cannot publish.
There is no reconnect, automatic retry, background worker or game-effect method.

The caller supplies `Authority`, `Goal` and one shared `Budget`. The operator uses
only `DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR=1`,
`DFMCP_CONSTRUCTION_MONITOR_ENDPOINT`, `DFMCP_BUILD_TOKEN` and
`DFMCP_OPERATIONS_PAGED_TOKEN` in the client process. Numeric loopback is mandatory.
Every other `DFMCP_*` variable, including placement/admission settings, is rejected.
The native game process retains its own existing plugin opt-ins and credentials;
the monitor's isolated environment does not change native configuration.

The wall deadline is 1..60,000 ms for the whole operation, not per page. Caps are
272 RPC calls, 20 MiB of connection bytes, 2 MiB of text notifications and
20 million cooperative work steps. Pages are fixed at 64 KiB and a capture uses
at most 256 pages. Socket ownership always closes in the foreground. Production
registration and native wire bytes are unchanged.

Thirteen actual TCP test functions pass with explicit joined peers, in addition
to the sixteen pure-core functions. These exercise the real Python transport,
fragmented frames, a 2,000-item multi-page capture, both receipt boundaries,
release failures, source/version drift, unknown/duplicate protobuf fields,
configuration revocation and nonrenewable budgets. Exact input hashes are in
`docs/evidence/construction-receipt-transport.json`. These are not real DFHack SDK,
live-game, Rust/MCP or whole-workspace qualification. Durable monitor custody and
the executable foreground workflow are described below.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_construction_receipt test_construction_monitor_rpc -v
```

## Executable durable workflow

`track_construction.py` now owns a separate append-only monitor journal. It accepts
an original canonical furniture/1.19 Placed record as either binary `DFMBR019`
bytes or a small JSON object with exactly one `canonical_record_hex` field. That
field is exposed in the Rust furniture MCP effect record. The input receipt file
must be a regular single-link exact-mode `0600` file under an owned real `0700`
directory. The monitor never modifies this receipt or the original placement
journal. A merely prepared or indeterminate placement is not eligible.

For example, with both native plugins already loaded in the same DFHack process,
configure the isolated client environment with the four variables listed above.
The operator must select a future absolute deadline using observed game ticks;
`900000` below is illustrative, not a recommended deadline for every fortress.

```sh
python3 scripts/track_construction.py start \
  --journal /private/construction/bed-1.construction \
  --receipt-file /private/construction/placed.receipt \
  --deadline-tick 900000 --stable-samples 2 --stable-span-ticks 10 \
  --interval-ticks 10 --max-gap-ticks 1200
python3 scripts/track_construction.py sample \
  --journal /private/construction/bed-1.construction
python3 scripts/track_construction.py inspect \
  --journal /private/construction/bed-1.construction
python3 scripts/track_construction.py cancel \
  --journal /private/construction/bed-1.construction
```

Each `start` or nonterminal `sample` performs one bounded foreground acquisition;
there is no automatic polling or simulation advancement. `start` synchronizes the
goal and endpoint, then a read-start record, before opening the native connection.
The original receipt query, complete capture/release and final receipt query must
all succeed. The complete proposed result is rendered before a sample append;
file and parent-directory synchronization plus complete readback precede
acknowledging that sample. A failed read, render or publication preserves the
unknown read intent. Every response remains within 16 KiB and includes an Agent
Turn with exact receipt/capture references and visible unresolved monitoring work.
Its canonical world anchor remains null: native IDs and captures are not relabeled
as a canonical world generation.

`sample` reopens the original goal; it cannot accept another receipt, endpoint or
policy, extend the deadline, or recover a prior process's read-publication permit.
After a process ends with an incomplete read, the next explicitly started read
resets the stability streak. Prior complete evidence stays in the journal. A
whole interrupted frame is retained as unknown; a torn frame or corrupt history
is refused unchanged, with no repair or truncation. A restored original goal
never renews its observation or game-time allowance.

`inspect` is strictly read-only and needs no credentials or DFHack. `cancel`
records cancellation of this monitor only; it never removes furniture, cancels
a native construction job, unpauses the game or changes placement obligations.
All terminal operations are idempotent: a terminal `sample`, `cancel` or `inspect`
returns retained history without credentials, network calls or file writes.

### Storage contract and limits

The private POSIX owner rejects symlinks in every path component, noncanonical
paths, nonregular files, hard links and noncanonical ownership/modes. It holds an
exclusive nonblocking file lock even during offline inspection. Full file bytes,
file/path identity and parent-directory identity are checked before publication
and return; same-size substitution and replaced pathnames are refused. Locking is
cooperative local custody, not malicious-owner protection or distributed fencing.

The `DFMCJR01` binary journal begins with one goal and numeric endpoint. Every
frame contains a big-endian bounded body length, sequence number, previous-frame
checksum, typed body and domain-separated SHA-256 checksum. Full replay validates
all canonical goal/receipt/capture bytes and rederives every monitor transition;
a saved phase label is never accepted as evidence. Replay streams bounded frames
rather than allocating the entire journal. Limits are 128 MiB, 1,030 frames and
at most 512 accepted observations, with the goal optionally selecting a smaller
observation allowance. The whole operation shares the transport deadline/work
allowance and a 1-GiB custody-byte allowance. A full maximum-sized sample and a
future cancellation frame are reserved before read intent and native access.
Capacity refusal preserves history; there is no compaction or eviction.

All bytes may be present after a synchronization failure, but the failed call
never acknowledges durability. A later owner can inspect fully valid historical
bytes; it does not retroactively claim the failed call acknowledged them.
Filesystem operations have cooperative deadline checks, not hard real-time
cancellation guarantees. The tests exercise process/I/O failures, not physical
power loss. Frame checksums detect corruption; they are not signatures or an
anti-rollback authority.

### Executed end-to-end validation

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_construction_receipt test_construction_monitor_rpc \
  test_construction_monitor_store -v
```

All **49 actual Python test functions pass**: 16 core, 13 transport and 20 durable
custody/CLI functions. They include a new-process completion sample, an interrupted
trailing receipt query followed by fresh stability, file/parent synchronization
failure before any socket opens, terminal synchronization loss, complete replay,
every incomplete prefix and single-byte corruption of a representative journal,
short/partial writes, real subprocess locking, mode/link/FIFO/path replacement
refusals, input/endpoint/policy substitution and offline terminal behavior. Bed,
chair and table conditions and bounded whole Agent Turn outputs are exercised.
Four weakened publication implementations also fail regression assertions: missing
read-intent synchronization, replayed publication permission, skipped old-byte
verification, and missing result reservation. The four core weakened implementations
continue to fail regression assertions.
Exact evidence scope and source hashes are retained in
`docs/evidence/construction-monitor-workflow.json`. Run
`python3 scripts/check_construction_monitor.py --mutations` to repeat the complete
49-function suite, check the machine-contract bounds and reject the four
publication mutants.

Even a satisfied goal describes a historical receipt-linked sampled condition. It
is not continuous monitoring, causal proof, current usability, terrain safety,
room assignment, checkpoint evidence or permission to discharge/retry an uncertain
placement. Native plugin loss or world restore may make original receipt continuity
unprovable. Preserve the original placement custody in all cases.

Owning beads: `df-dfhack-bridge-plane-c-pic.4` (subsequent-observation postconditions)
and `.5` (interruption/recovery without repeated effects); their broader scope
remains open.
