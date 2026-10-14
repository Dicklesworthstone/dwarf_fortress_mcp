# Explicit bounded execution of an original route batch

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`, `df-franken-storage-mvcc-54h.4`.

`route_furniture_run` composes the existing route-batch review and single-step
advance operations. A separately obtained run confirmation authorizes only a
fixed prefix of the original batch, up to 32 placements, within explicit aggregate
limits. This is an opt-in bulk control operation, not a change to ordinary
single-step confirmation or automatic resume. No new native method is added.

## Review, consent, execute

An embedding owner creates `Limits(max_steps, timeout_ms)` and calls `make_budget`
once, before opening the original `Batch`. `review_run(batch, authority, limits)`
performs the existing next-step native/route review and returns `run_review`.
Its `confirm_run` is null if that first frontier is not ready. Review performs no
placement and grants no native dispatch permit.

The confirmation binds the policy, batch and retained-review identities, complete
local effect inventory, first native plan and paired route confirmation, exact
approved original slot names, step cap and all aggregate resource limits.
Changing any of these requires a new review. The caller must independently retain
and explicitly confirm this value; do not obtain consent from the execution
call's own newly generated review.

For execution, open the same original batch writable with one fresh aggregate
budget and call `run(batch, authority, limits, confirmed)`. It reacquires the first
review and requires the supplied confirmation to match before creating any child.
That explicit prefix consent also covers fresh per-step captures of the remaining
approved original items, subject to every unchanged original selector/source and
route constraint. It does not authorize substituted items, new targets, changed
dependencies, larger regions, another native source or a longer run.

Every step still uses the original `review_step` and `advance` functions. The
existing coordinator synchronizes original intent, route association, preparation
and dispatch intent; fresh suffix proof and the final native BEFORE check still
precede the sole native commit. The next step begins only after local original
receipt custody proves `placed`. No helper invokes recovery, cancellation, time
control, reallocation, reordering or commit retry. Read-only recovery connections
are never converted into preparation connections.

## Stops, recovery, and replay

The run stops at its step limit, completion of placement registration, a newly
blocked/unestablished review, insufficient remaining aggregate admission, stopped
batch, refused/cancelled step or unresolved effect. A failed step invocation is
never repeated. A local audit may report its retained result without additional
native contact; unavailable custody/deadline instead produces the enclosing
workflow's normal refusal. Original journals always remain the recovery source.

After any child creation, the inventory commitment changes. Reusing the old run
confirmation cannot continue or repeat that prefix. Resume requires inspection,
explicit original-key recovery where needed, and a new bounded review of the
remaining original suffix. The runner neither automatically reconciles nor
restores a native permit. A lost acknowledgment after a persisted `Placed`
receipt stops the current run too, even though later inspection can see progress.

Compact per-invocation outcomes retain slot, native plan digest, phase, receipt
digest and dispatch-recorded status. Invocation count is not native writer count.
The full original batch and step inventory remain in the result. Construction
completion, current usability, native pathfinding and production admission are
not inferred from historical job-registration receipts.

## Bounds

One 1..60,000 ms deadline spans opening custody, all reviews, placement and final
publication. For N requested steps, the aggregate caps are 840*N RPC calls,
60 MiB*N network bytes, 26,000,000*(N+1)+100,000 work units, and the existing 1 GiB
parent-storage allowance. The extra work slice covers initial review/custody.
These are ceilings, not a promise that N steps fit or that unused reservations
are refunded. Existing child-journal aggregate-disk-accounting limitations remain.
No per-step budget object, wall-time reset, reconnect-on-failure or quota refill
occurs. Every subsequent step is admitted before review. The output must fit the
existing 256 KiB limit with 24 KiB reserved for complete bounded run metadata and
96 KiB for the existing native outcome before any child creation.

## Evidence

The initial increment has 15 passing Python tests of the actual run owner using
explicit step/custody ports. They cover all limits through 32 steps, exact prefix
consent, stale replay, uncertainty/lost replies, route blockage, stop/refusal,
revocation, source changes, shared budget exhaustion and output reservation.
These are NOT execution of private custody, native transport, the full placement
coordinator, CLI, Rust/MCP, DFHack or production-admission qualification. The
existing one-step integration remains unchanged; executable CLI wiring and deeper
integration verification are separate increments.


## Executable CLI and original private custody

The existing `route_furniture_batch.py` command now exposes two explicit modes.
Use an already initialized original batch and the same isolated route-batch
credentials documented in `ROUTE_FURNITURE_BATCH.md`. For example:

```sh
python3 scripts/route_furniture_batch.py review-run \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --max-steps 4 --timeout-ms 10000

python3 scripts/route_furniture_batch.py run \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --max-steps 4 --timeout-ms 10000 --confirm-run <reviewed-confirm-run>
```

Paths, limits and placeholders are illustrative. The confirmation comes from
`result.run_review.confirm_run` in a successful, independently reviewed
`review-run` response. A blocked review returns null confirmation. Both invocations
must use the same step cap and timeout; execution needs the existing explicit
`DFMCP_ROUTE_FURNITURE_ALLOW_PLACE=1` permission as well as writable original
custody. Read-only review itself does not need placement permission.

`--max-steps` is required only for these two modes. `--confirm-run` is required
only for `run`. Existing `advance` still requires its separate native plan and
paired route confirmation; existing review, inspect, recovery and stop modes
cannot accidentally opt into bulk execution. Invalid combinations, malformed
confirmation values and limits outside 1..32 steps or 1..60,000 ms refuse before
opening batch custody or contacting a native source.

The CLI passes its own step module to the run owner, so direct script execution
cannot silently import a second Authority class under another module name. That
is a trusted embedding detail, not a caller-selectable module, native method or
permission override. The default Python API still uses the ordinary step module.

Original parent and child directories remain exclusively held through one final
output attempt. Successful reports use the existing bounded Agent Turn envelope.
`result.run.stop_reason` distinguishes limit, all-placed, route blockage,
insufficient aggregate admission and unresolved work. Exit zero means a complete
report, not construction completion. A failed coordinator call returns exit two
and `ok: false`; when local custody and the remaining deadline permit an audit,
the response retains the complete original state and compact failed invocation.
Otherwise the existing sanitized refusal is emitted and journals remain the
source for subsequent inspection. A short write never emits another object,
repeats the step or contacts the source again.

No bulk dispatch permit or new journal format is introduced. The coordinator
still writes original Intent, Prepared and Dispatch records, synchronizes them,
rechecks the mandatory complete delivery suffix and exact native BEFORE, and
consumes the original connection-owned one-shot permit. An explicit run cannot
skip these gates or use an uncertain child to unlock a later item. After progress,
stale bulk consent cannot resume: obtain a fresh review after any necessary
separate original-key recovery.

## Executed integration and limitations

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_route_furniture_run.py --mutations
```

All **32 tests pass**: 15 owner/port tests, 14 real-file integration tests and
3 actual placement-TCP tests. Integration executes the unmodified original
placement coordinator, private parent/index/child journal implementation,
canonical native value codecs, source/selector/frontier/suffix checks and the
new CLI/main functions. File tests include three-step execution, reopened
suffixes, changed final routes after Dispatch sync, lost prepare/commit replies,
uncertain/refused outcomes, torn appends, Prepared/Dispatch synchronization
failures, stale consent and single-output refusal. At each tested writer boundary,
a synchronization spy verifies the exact complete Dispatch file bytes were
synchronized, not merely that an unrelated sync call occurred.

TCP tests execute the actual native client and framing/protobuf decoder against
bounded, joined loopback peers. The peer checks the literal six-method binding
inventory and independently encodes wire fields. A two-step confirmed run performs
two distinct original-key commits; lost commit acknowledgment stops after one
writer without automatic query or another connection. Invalid confirmation is
read-only. Map/inventory acquisition remains injected, with actual frontier and
suffix computations on synthetic captures. Native Capture/Record generation uses
the real value codecs, not a real fortress.

Six weakened implementations fail assertion-based regressions: ignored consent,
ignored step cap, per-step budget replacement, omitted output reservation,
omitted final route gate and omitted child-journal synchronization. Syntax or
import errors do not count as a rejected mutant. The retained runner inventories
all 19 executed production/test/runner inputs before and after the suite and
refuses source changes during execution.

All prefix limits through 32 are exercised in the owner/port tests, and a full
32-slot review is tested with real private files and complete bounded output.
A separate attempted 32-step real-file run was stopped by the editing tool's
45-second execution limit before it completed; it is not a passing scenario or
a benchmark. The successful real-file runs cover up to three placements and the
TCP run covers two. The maximum is a cap, not a promise that every admitted batch
fits one foreground deadline; existing repeated full-custody verification can be
expensive, and its checks were not weakened to make the experiment pass.

The test CLI calls execute the real parser/main/output functions, including a
separately loaded entrypoint module; no real CLI subprocess-to-game campaign is
claimed. No complete native map/inventory TCP acquisition, actual DFHack SDK/plugin,
Rust/MCP build, live game, full-workspace qualification or production admission
was performed for this change. Original native methods, wire formats, grants,
journal formats and registry admission remain unchanged.
