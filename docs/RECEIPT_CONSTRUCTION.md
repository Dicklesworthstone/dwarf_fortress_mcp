# Receipt-linked furniture construction conditions

This feature connects a verified furniture/1.19 `Placed` record to later complete
operations/1.4 observations. It is separate from the existing generic
`construction_progress` query: that query deliberately accepts observation-selected
IDs without authenticating a placement receipt. Neither path changes native wire
formats, the eleven-tool MCP surface, production admission or the placement journal.

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
a command-line owner are not established by this transport increment yet.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_construction_receipt test_construction_monitor_rpc -v
```

Even a satisfied goal describes a historical receipt-linked sampled condition. It
is not continuous monitoring, causal proof, current usability, terrain safety,
room assignment, checkpoint evidence or permission to discharge/retry an uncertain
placement. Native plugin loss or world restore may make original receipt continuity
unprovable. Preserve the original placement custody in all cases.

Owning beads: `df-dfhack-bridge-plane-c-pic.4` (subsequent-observation postconditions)
and `.5` (interruption/recovery without repeated effects); their broader scope
remains open.
