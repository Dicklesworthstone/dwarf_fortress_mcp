# Construction-aware stops and original-run resumption

Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.

The mandatory construction gate narrows `advance_allowed` to false while earlier
original work is unfinished. The bounded runner previously confused this with
invalid local custody: review-run raised instead of returning null consent, and
a run raised after a completed placement instead of retaining its blocking read.
The exact preceding implementation reproduces this `run frontier changed` error.

The runner now separates the original journal's eligible fixed-order prefix from
fresh route/construction readiness. Every review is bound to unchanged batch,
review, plan and complete journal inventory before and after acquisition. Unknown
statuses, contradictory confirmation fields or changed custody still refuse.

`review-run` returns the full unready frontier with null scope and confirm_run.
An unready first `run` observation returns zero invocations and no approved scope;
`confirmation_validated` is false. Well-formed supplied consent was not validated
by that observation and cannot authorize anything. A ready first observation still
requires the exact independently supplied confirmation before the first effect.

After progress, construction_pending, construction_unestablished,
construction_blocked, route_blocked and route_unestablished are normal bounded
stops. The response retains the complete original plan, exact completed outcomes,
next original step, and full blocking route/construction report. It does not retry,
skip, substitute, advance time, weaken the gate or create a new native key.

A subsequent review exception is `review_failed` with faulted=true. Earlier
outcomes survive only when current authority, deadline and original custody still
permit final audit. No cached earlier readiness observation survives that failure.
Lost custody or authority still produces the normal outer refusal.

A final offline journal audit cannot create execution permission: all run returns
clear expected_plan and confirm_review, set advance_allowed=false and require a
fresh execution review. The custody status may still be `ready`; it describes
original journal eligibility, not a new native/route/construction observation.
After any progress the old inventory-bound run consent cannot resume. Inspect,
recover any uncertain original key separately, and obtain a new bounded review.
The separate explicitly confirmed game-clock workflow remains separate authority.

## Executed tests

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_run test_route_furniture_run_readiness -v
```

27 tests pass: 15 unchanged owner tests and 12 new readiness/resumption tests,
including every unfinished prefix of a 32-step plan. Tests execute the actual run
owner and complete unchanged plan/review import closure with explicit native-step
and custody ports. They do not execute native acquisition, private journals,
TCP, DFHack, Rust/MCP or the full workspace. No admission or protocol changes.
