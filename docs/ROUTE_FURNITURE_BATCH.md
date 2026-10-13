# Original route constraints at the placement frontier

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-franken-storage-mvcc-54h.4`.

`route_furniture_frontier.assess` connects an immutable original route review to
an exact fixed-order prefix of native furniture/1.19 Placed receipts. Each receipt
must preserve the original selected item, native type, material, subtype, target,
source and batch-derived key. Prepared, indeterminate, refused, cancelled,
foreign, reordered, truncated or corrupt records cannot unlock the next step.
Every predecessor's native sequence, tick and job/building ID horizons are checked
against the next capture. Map and operations generations remain separate from
one another and from the native placement generation.

Earlier verified insertions are reported as `historical_placed`, not as current
loose inventory or completed construction. Every remaining original selection
is revalidated against the complete candidate inventory and current captured
route graph. No solver chooses replacement items. The next native capture must
agree with that inventory's paused tick, job/building horizons, map dimensions
and selected item's position. The full original plan remains visible as history.

The checker returns `ready`, `route_blocked`, or `route_unestablished`; none is
permission to dispatch. Evidence digests cover the original review, complete
remaining result, receipt prefix, native capture and source identities. They do
not vary with a narrowed computational allowance when the evidence is identical.
One bounded allowance covers receipt checks and the existing full revalidation.
Results are at most 96 KiB. No private storage, native call, mutation or production
admission is provided by this pure module.

## Executed development evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_frontier -v
```

All 14 actual Python tests pass. Coverage includes post-placement inventory,
blocked later routes, exact receipt identity, every receipt truncation and byte
corruption, source/clock/horizon differences, no replacement, exact shared work
bounds and every cooperative callback failure. The five unchanged dependencies
were reconstructed and checked against their exact Git blob identities before
execution. This is not a live DFHack, Rust or full-workspace qualification.

## Durable route-aware placement workflow

`route_furniture_batch.py` now enforces these checks around the existing
`build_placement_client.start` coordinator instead of exporting another advisory
plan. It creates a separate `dfmcp.route-furniture-batch/1` root; legacy furniture,
room and terrain batches are not migrated or reinterpreted. The original native
placement client, transport, receipt codec and effect journals remain unchanged.

The operator must already have the existing map/1.5, operations/1.4 and
furniture/1.19 plugins configured. This command never installs them or changes
native configuration. Use an isolated environment for initialization, review and
advance:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH=1
DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_TOKEN=<existing furniture credential>
DFMCP_MAP_TOKEN=<existing map read credential>
DFMCP_OPERATIONS_PAGED_TOKEN=<existing operations read credential>
DFMCP_ROUTE_FURNITURE_ALLOW_PLACE=1
```

The endpoint is optional; the shown address is its default, not an established
game connection. Tokens are 32..256 UTF-8 bytes without NUL and never appear in
stored files or output. `ALLOW_PLACE` may be absent or zero for init/review, but
must be exactly one for advance. All other DFMCP environment variables, including
production admission state, are refused. Every native effect rechecks the same
current authority. Changing any retained credential, endpoint or permission
invalidates the call rather than loading a replacement authority silently.

First obtain a complete original review using `review_route_furniture.py create`
in its separate read-only environment, as documented in
[ROUTE_FURNITURE_REVIEW.md](ROUTE_FURNITURE_REVIEW.md). Retain its canonical review
bytes and independently retained review digest. In the route-batch environment:

```sh
# The operator provides an existing EMPTY private directory (mode 0700).
python3 scripts/route_furniture_batch.py init \
  --directory /absolute/private/route-batch \
  --review-file original-review.json --expected-review <retained-review-digest>

# Use the batch_id returned by init for every later operation.
python3 scripts/route_furniture_batch.py review \
  --directory /absolute/private/route-batch --batch-id <original-batch-id>

# Only a ready review supplies both of these exact confirmation values.
python3 scripts/route_furniture_batch.py advance \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --expected-plan <reviewed-native-plan-digest> \
  --confirm-review <paired-route-and-native-review-seal>
```

Initialization records the complete original review and initial native source,
not a route-success or placement claim. `review` inspects the next original
selection and every remaining route. An unestablished or blocked frontier returns
a report with null confirmation values. `advance` attempts at most that ONE
reviewed original step. There is no automatic run-to-completion loop, reallocation,
substitution, unpause or retry. Reopening does not change the original request,
region, max route length, item identities or dependency order.

### Persistence and dispatch ordering

The exact root contains `batch.json`, `index.jsonl`, `effects/`, and optionally
`stop.json`. The complete definition includes the original review, retained
review digest, native manifest and initial capture, a batch nonce, and root and
effect-directory identities. Its exact file SHA-256 is the externally selected
batch ID. Child keys are `frt-<batch-id>-<original-slot-name>`.

The root is exclusively locked. Parent traversal uses the existing no-follow
private-directory implementation; root and files must have exact modes 0700 and
0600, appropriate owner, regular file type and single-link file custody. The root
and child directory identities must remain unchanged. Checks re-read parent
bytes and the complete native journal inventory, not only cached checksums.
Missing registered children, copied directories, replaced paths, foreign files,
corrupt or torn indexes and altered original reviews refuse without repair.
The index pins each native child's inode and exact initial intent digest. Writes
synchronize both file and directory before byte-exact readback. Ambiguous writes
fence the current owner; reopening verifies surviving bytes without completing
or repeating a dispatch.

One `advance` performs these effect boundaries in order:

1. The unchanged native client observes the exact selected item/target and queries
   its original key. A fresh complete map/inventory bracket rechecks every
   remaining selection. The result must match the operator's paired review seal.
   Complete result space is reserved before creating a child journal.
2. The native coordinator durably creates its original intent. The batch then
   synchronizes its child/index/route-evidence association **before native prepare**.
3. The native coordinator obtains fresh preparation and synchronizes its existing
   Prepared and Dispatch records. Only then does the batch acquire a second
   complete route bracket and require unchanged route evidence. A final native
   selected-item capture must equal the original complete BEFORE capture too.
4. The existing client consumes its connection-owned one-shot permit and sends
   the sole CommitPlacement call. Strict native readback is retained in the
   original child journal; the batch returns the complete resulting inventory.

A route failure after Dispatch was recorded keeps the journal pending, even
when this particular call sent no commit. It cannot reset that journal to retry.
A lost preparation or commit response also preserves the original key and
uncertainty. Historical Placed receipts advance the prefix; they do not establish
completed construction or current room usability. A refusal or cancellation
halts the original batch rather than skipping a required furnishing.

The durable index retains the original route evidence digest, not all raw
re-observation bytes. It binds the original coordinator intent to that reviewed
frontier but is not an independently replayable proof of acquiring native data.
The full original request/review and all native receipt bytes do survive restart.

### Inspection, stop and recovery

`inspect` and `stop` require no native credentials or connection. Stop only
prevents future batch advances; it does not cancel already prepared/placed work.
The complete original review, plan, all step states and pending native key remain
visible in the normal result. Files are never pruned to make new work fit.

For `query` or `cancel`, use the **existing build-only recovery environment**,
not the combined route environment:

```text
DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1
DFMCP_BUILD_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_TOKEN=<existing furniture credential>
```

```sh
python3 scripts/route_furniture_batch.py query \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> --step bed

python3 scripts/route_furniture_batch.py cancel \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> --step bed
```

The step name is illustrative and must belong to the original plan. These paths
use the unchanged native `recover` function only: they never acquire routes,
prepare, commit or issue another key. A crash-created unregistered child may be
indexed for recovery with null route-evidence identity; that is not a permit.
Prepared or absent native query results remain pending. Terminal retained
receipts are inspected offline; an uncertain effect is never cleared merely
because independent route checks happen to pass. Native cancellation follows
the existing protocol; it cannot undo a historical placement.

Exit zero and `ok: true` mean a complete report was produced, not that the batch
is complete or a route is currently safe. Inspect `result.status`, original step
states and `placement`/`recovery` evidence. Errors exit two with null result and
a sanitized `ROUTE_BATCH_REFUSED` error. Keep the original batch ID and inspect
its journals after errors; do not retry placement. Short stdout writes never
start another native call or emit a second object. Root custody remains held
through the one final output attempt.

### Shared bounds and remaining limitations

The foreground operation has one shrinking 1..60,000 ms wall allowance (default
10,000), 560 RPC calls and 40 MiB network allowance across both read brackets and
native placement. The parent metadata reads/writes share a 1-GiB disk allowance;
unchanged native child-journal I/O keeps its existing bounded file/inventory and
shared-deadline checks rather than claiming byte-exact aggregate disk accounting.
All new CPU passes share 20 million work units. Each route analysis reserves six
million of those units after projection; that reservation is not refunded or
renewed, even for smaller results. Parent definitions are at most 128 KiB, indexes
32 KiB, reviews 49,152 bytes, and complete output 256 KiB. Native per-file and
transport bounds remain unchanged. Final hashing/serialization, filesystem
synchronization and injected synchronous I/O are bounded/cooperatively checked,
not preemptible hard real-time operations.

This owner enforces fresh route checks immediately around the existing native
control boundary. It does **not** lock the entire observed map atomically with
CommitPlacement. Another actor can alter remote terrain after the last route
capture; the native protocol revalidates its existing local selection, not the
whole routing graph. No atomic route/placement, native pathfinding, safety,
global controller exclusion or continuous-preservation proof is claimed. Routes
are exact only in the existing captured floor/stair model. Prior furnishings can
block later work; revalidation then stops the remaining batch without replacing
its intent. Local custody does not protect against a malicious owner rolling
back all files or recreating the plan in a different executor. Read-source floors
come from the original review and retained placements, not a durable log of every
failed read or a global anti-rollback clock.

The only change to existing source is permitting an `Authority` subtype in the
read client's constructor for a trusted embedding-owned stricter environment
guard. That subtype carries only the same two read credentials; the four native
read bindings are unchanged. The ordinary read-only CLI still uses its original
exact authority loader and rejects combined mutation environments. No native
protocol, plugin, production runner, compatibility entry or Rust MCP surface is
added or modified.

### Executed integrated tests

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_frontier test_route_furniture_batch \
  test_route_furniture_batch_runtime -v
```

All **42 actual Python test functions pass**, with zero skips: 14 frontier tests,
18 durable-owner/coordinator tests and 10 real TCP/CLI tests. The durable tests use
real private files and the unchanged placement coordinator with injected native
I/O. The TCP tests execute the actual client classes and complete CLI subprocess
flows, with joined test-peer connections. Map/inventory and framing/protobuf are
independently encoded; placement receipts use the existing strict fixture codec.
They cover multi-page inventory, two-step reopen, no substitution, index/dispatch
ordering, lost prepare/commit acknowledgments, source/nonce/release failures,
final route obstruction, final native capture change, build-only recovery,
no second dispatch, original read-only profile compatibility and offline inspect.
Other tests exercise every small receipt truncation/byte corruption, wrong or
missing children, torn index, sync failure, source and authority loss, exact work
limits and short stdout. All imported existing source bytes were matched to their
Git blob IDs before changes; ten existing source files remain byte-identical.

These are development tests, not a native DFHack game, SDK/plugin build, Rust
execution or full-workspace qualification. The broad implementation beads remain
open; no registry or production admission is widened.
