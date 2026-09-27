# Inventory-driven whole-plan furniture allocation

`scripts/furniture_allocation.py` chooses distinct exact items for 1..32 requested
bed/chair/table targets. It emits the existing `dfmcp.furniture-plan/1` format used
by `furniture_batch.py`, or a shortage with **no partial executable plan**.
`scripts/allocate_furniture.py` now connects that allocator to one complete,
authenticated operations/1.4 inventory capture. It returns a source-bound proposal
with exact item selections, or an explicit shortage, through a bounded JSON/Agent
Turn response. It does not reserve items, prepare/commit a placement, unpause the
game, or qualify a production profile.

## Executable native inventory workflow

The operator must already have the unchanged `dfmcp_operations_v1_4` plugin
loaded and configured in the target DFHack process. This client does not load a
plugin or change native configuration. Use an isolated client environment with
only these DFMCP variables:

- `DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION=1` (exact development opt-in).
- `DFMCP_FURNITURE_ALLOCATION_ENDPOINT` (optional canonical numeric IPv4 loopback
  address and port; default `127.0.0.1:5000`).
- `DFMCP_OPERATIONS_PAGED_TOKEN` (the operator-configured 32..256-byte read token).

Other DFMCP variables, including furniture credentials, placement permission and
production-admission configuration, are refused. No mutation authority is needed
or accepted. Credentials are never copied into a proposal, log or error response.

```sh
# In that isolated client environment, using a request in the format below:
python3 scripts/allocate_furniture.py \
  --request-file furnishings-request.json --timeout-ms 10000
```

The command opens one native TCP connection and binds exactly the operations
`Handshake` and `ReadObservation` methods. It reuses the existing transport's
retained-page, nonce, manifest, size, whole-capture SHA-256 and release checks;
there is no duplicate page protocol. The complete immutable capture must be
received and its release acknowledged with the original token before allocation.
Generation or software drift, skipped/replayed pages, wrong digests, lost replies
and release failures return no proposal. There is no reconnect or automatic retry.

The existing full-roster decoder validates **all** jobs, buildings, items,
containment and job-item attachments before projection. Folder and site must match
the operator's request exactly. Items with contradictory or invalid relationships
anywhere in the capture cannot be hidden by selecting only convenient rows.

Candidate projection recognizes only exact `BED`, `CHAIR` and `TABLE` enum keys.
The native nine-bit item projection must contain only `on_ground` (word 64):
forbidden, in-job, dump, removed, rotten, trader, inventory and building flags
exclude the item. Container/holder relationships, being a container for another
item, or any observed job attachment also exclude it even when flags look free.
Candidates must be singleton stacks with known material, valid coordinates and no
explicit operator exclusion. Disjoint rejection counts cover the full item roster.
The full allocator then applies each slot's material, subtype, z-level and distance
constraints and the whole-plan distinct-item requirement.

### Results and handoff to placement

An `ok: true` response is an established report, not necessarily a feasible plan:
`result.status` is `allocated` or `shortage`. Only `allocated` has a non-null
`result.plan`. A valid shortage exits 0 and describes the competing slots and
missing items; a refused read/request exits 2 with `ok: false`, `result: null` and
no partial allocation, native text, credential or path. Consumers must check both
`ok` and `result.status`, not just the process exit code.

For an allocated report, `result.plan` is the complete existing
`dfmcp.furniture-plan/1` object accepted by `furniture_batch.py init --plan`.
Retain the source report and extract that object into the batch's normal plan
input. Use the **separate** furniture-batch environment and its existing
initialization, review and confirmation workflow from `FURNITURE_BATCHES.md`.
The allocator never initiates a batch or copies read credentials into a writer.
Each later placement still requires a fresh native capture, exact confirmation,
original-key custody and all existing uncertainty and policy gates.

The report retains the normalized request, exact capture SHA-256, capture tick,
folder/site, native horizons, generation and software identities, complete
projection counts, chosen source item facts and distances. These identify
historical evidence, not a canonical world generation: the Agent Turn anchor
remains null. Its coverage explicitly says existing placement receipts and active
placement work were not queried. Allocation cannot clear those obligations or
establish that a different controller has no unresolved effect.

The operations projection does not expose wear, all native item flags, terrain,
map dimensions or worker paths. Allocation therefore does **not** establish that
any placement is eligible, accessible or safe. Material and subtype constraints
are assessed at this inventory capture only; the existing exact-item plan format
does not turn those facts into durable future constraints. Inspect each later
native plan, and discard/reallocate rather than silently substitute another item.
Items are not reserved between proposal and execution.

### Input, resource limits and publication

The command reads a bounded regular request file with a no-follow open for the
final path component, and checks descriptor size and identity/time metadata across
the read. It refuses final symlinks, FIFOs, directories, oversized files and
changed bytes; this is not the placement journal's private directory custody or
an all-parent no-follow policy. JSON duplicate fields, excessive nesting and
invalid requests are refused before any native socket opens. The request file
is not modified. The command creates no plan file or journal; its result is stdout.

All steps share one shrinking 1..60,000 ms wall allowance (default 10,000), 272 RPC
calls including bindings, 20 MiB network allowance and 20,000,000 guarded work
steps. Captures are at most 16 MiB, pages at most 65,536 bytes, and native rosters
are bounded at 4,096 jobs, 4,096 buildings, 65,536 items and 65,536 attachments.
Notifications are bounded per reply and across the connection. These limits are
not renewed for each page, decoding pass or assignment. Filesystem and CPU checks
are cooperative, not hard real-time cancellation guarantees.

Complete JSON/Agent Turn output must fit 65,536 bytes. Allocation facts are fully
serialized before a final authority and deadline recheck, so revocation during
projection or optimization cannot publish cached read evidence. Oversize or
budget failures refuse the complete result rather than truncating it into a
misleading executable plan. A caller may perform another explicit read later,
but this operation never reconnects or grants placement permission.

## Request and pure API

```json
{
  "schema": "dfmcp.furniture-request/1",
  "world_folder": "region1",
  "site": 2,
  "slots": [
    {"name": "bed", "kind": "bed", "target": [15, 15, 2]},
    {"name": "chair", "kind": "chair", "target": [18, 15, 2],
     "material": [419, -1], "max_distance": 100, "after": ["bed"]},
    {"name": "table", "kind": "table", "target": [20, 15, 2],
     "after": ["chair"]}
  ],
  "excluded_items": [123]
}
```

IDs, material pairs, fortress and coordinates are illustrative, not defaults.
A slot can constrain the exact material type/index pair, exact subtype and maximum
horizontal Manhattan distance (0..65,532). Missing material/subtype means any
candidate value; every candidate must be on the target's z-level. Exclusions only
remove candidates and do not claim a global reservation. The original plan
compiler validates names, unique targets, coordinates and dependency acyclicity.
Dependencies still mean historical placement registration, not finished furniture.
The complete normalized request must fit 16 KiB.

```python
from furniture_allocation import Candidate, Request, allocate
request = Request.decode(request_bytes)
# The pure caller supplies already selected candidate facts, not a trusted world.
candidates = (
    Candidate(42, "bed", (10, 10, 2), (419, -1), -1),
    Candidate(43, "chair", (11, 10, 2), (419, -1), -1),
    Candidate(44, "table", (12, 10, 2), (419, -1), -1),
)
result = allocate(request, candidates)
# Only an allocated result contains result['plan']. Later batch review remains
# mandatory: these values never authorize a placement or establish eligibility.
```

The pure API checks all supplied candidate identities and values, including
excluded/irrelevant candidates. It does not authenticate the caller's facts or
verify that those facts came from the requested fortress. The executable native
read adapter establishes those associations before calling it. Every result makes its
non-authority and non-reservation scope explicit.

## Global allocation, objective and shortages

A generic slot must not consume the only material-compatible item for a more
constrained slot merely because it appears first. The implementation solves the
whole bipartite assignment, rather than greedily assigning each target.

The objective is lexicographic: first minimize total same-level Manhattan
distance; then minimize the exact item-ID vector in lexical **slot-name** order.
Dependency execution order is separate. Distance is a geometric heuristic, not a
route-length estimate or proof of worker accessibility. Integer costs represent
both objective levels exactly, including maximum-width item IDs, without floats.
Input row order does not affect the result.

For N slots, keep each slot's N best compatible items by distance and ID. This
preserves an optimum: when a slot uses an item outside its best N, at least one of
those N better items is unused by the other N-1 slots and can replace it. Thus the
solver handles at most N*N distinct reduced items, not a 32-by-65,536 dense matrix.
Full candidate counts are retained. A maximum matching is checked before running
the rectangular Hungarian minimum-cost assignment.

When no perfect assignment exists, a Hall-deficiency witness names a set of slots,
all their compatible candidate IDs and their shortage. Every such set has fewer
than N neighbors, so none of its candidate lists was trimmed; its witness covers
the complete supplied candidate graph. Counts are not substituted for matching:
several individually satisfiable slots can still compete for too few items.
The result includes the maximum assignable count, but never an executable subset
that silently drops the other targets or weakens constraints.

## Solver bounds and executed validation

At most 65,536 supplied candidate items, 32 slots, 4,096 explicitly excluded IDs,
1,024 reduced item identities, and 49,152 serialized allocation-result bytes are
accepted. The normalized request's 16-KiB bound usually limits large exclusion and
dependency lists sooner. An injected guard runs during scans, matching and cost
optimization; budget refusal returns no partial result. The pure API has finite
structural bounds even when no external deadline guard is supplied.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_furniture_allocation test_furniture_inventory -v
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 scripts/check_furniture_allocation.py
```

All **40 actual test functions pass**, with no skips: 14 pure allocator functions
and 26 projection, real TCP and CLI/process functions. The first increment's
14-function evidence remains in `docs/evidence/furniture-allocation-core.json`;
current exact source hashes, counts and mutations are in
`docs/evidence/furniture-allocation-integration.json`.

Tests exhaust all 4,096 three-slot/four-item bipartite graphs against an independent
cardinality/cost/permutation oracle and compare 150 random inventories against an
unpruned oracle. They execute the 65,536-item/32-slot and 1,024-item reduced-union
bounds, all 512 native projected flag words, strict complete-roster parsing,
material starvation, deterministic tie breaks, exact Hall shortage witnesses,
foreign-fortress refusal and compatibility with the actual existing plan compiler.

The joined TCP peer uses an independent wire encoder and asserts both read-only
bindings and every native request field. Its 65,536-item test transmits a
**3,080,323-byte capture across 48 pages**, with a job-item attachment in the final
page that must exclude an otherwise attractive item before solving all 32 slots.
Fault tests exercise binding, framing, source/software/token/digest drift, lost
pages, release acknowledgments, notification limits, shrinking budgets, real wall
timeout and post-read revocation. Real CLI subprocess tests establish the native
read-to-plan handoff, valid shortages and sanitized failures without altering the
request or creating placement files.

Ten deliberately weakened implementations fail **regression assertions**, not
syntax/import errors: ignored distance, greedy candidate trimming, cross-level
candidates, partial shortage plans, ignored flags/job attachments, foreign-world
adoption, missing release verification, missing capture digest verification, and
missing final authority recheck. The checker repeats these executions from copied
source and emits exact source identities. The largest measured complete response
in this campaign, including maximum-width source fields and 32 slots, is
**31,572 bytes** of the 65,536-byte limit; larger outputs are refused, not presumed
safe from that measurement.

The native TCP peer is a test double, not DFHack. No real native SDK/plugin build,
live fortress, Rust/MCP integration, whole-workspace qualification or production
admission is established. Native protocols, dependencies and existing mutation
paths are unchanged. Broader beads `df-dfhack-bridge-plane-c-pic.4/.5` remain open;
this workflow implements inventory-driven proposals, not all construction or
logistics acceptance criteria.

## Rust allocation from a published inventory

`dfmcp_adapter::furniture_allocation` implements the same global objective in
safe Rust with the existing dependencies. Its public `allocate` function takes a
typed request, candidate slice and cooperative guard. Checked fixed-width
lexicographic costs preserve the exact distance/ID objective without floats or
an added big-integer dependency. A refused guard propagates through scanning,
maximum matching and optimization without returning a partial assignment.

`dfmcp_adapter::furniture_supply::plan` connects this model to the existing sealed
`OperationsStateView`. It takes the operation context, expected world folder/site,
request and total work allowance. The reader requires Query authority, the
current published anchor, sufficient entity budget and the exact fortress. It
retains the enclosing source digest and canonical item generation/revision,
including when operations are embedded in a spatial capture. It does not create
another projection or perform another native read.

The supply policy is `direct-ground-unattached-singleton-furniture/1`. Candidate
items must have a supported furniture type, exclusively the observed ground flag,
no container or building holder, no observed contained items, no actual job
attachment, a singleton stack,
known material and bounded ground coordinates. Explicitly excluded IDs are then
removed. Every observed item receives one primary classification; compatible
counts refer to the complete resulting candidate graph. Selected item evidence
and any complete Hall witness retain their original canonical handles.

One work allowance covers normalization, attachment scanning, inventory scanning,
matching and evidence collection. At most 10,000,000 work units, the caller's
remaining wall budget, 65,536 items and 65,536 observed attachment rows are allowed.
The result contains either every assignment or a complete shortage. This supply
subset establishes neither target terrain nor native placement eligibility,
quality/wear, route access, reservations or future availability. A later exact
placement observation and review remain necessary.

All 32 focused adapter Rust tests passed on `nightly-2026-08-31`, with zero
ignored. The 15 solver tests include all 4,096 small graphs, 200 unpruned random
inventories, maximum bounds and interruption at every guard boundary. The 17
projection tests exercise every observed flag word, actual attachments, source
and item reuse, enclosing spatial generations, current authority and shared
budgets. This is focused Rust development evidence, not full workspace, real SDK,
live-game or production qualification. Exact scope and source hashes are in
`docs/evidence/furniture-allocation-rust.json`.
