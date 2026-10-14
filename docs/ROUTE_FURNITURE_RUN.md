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
