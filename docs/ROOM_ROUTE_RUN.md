# Bounded runs preserve original room intent and explain room stops

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` (remain open).

The existing `route_furniture_batch.py review-run` / `run` workflow now accepts
whole-room `room_blocked` and `room_unestablished` observations. Previously these
valid states were absent from the runner's closed vocabulary: an initial review
failed, or a later room obstruction became a generic `review_failed` fault and
lost its diagnosis. A room mismatch is now a read-only stop, not an excuse to
place, skip, change intent, retry or discard already recorded progress.

The original batch owner still authenticates private storage, receipts, complete
room geometry, current construction and every remaining delivery. The run owner
compares the complete parent fields before/after each review and binds both
`room_handoff_digest` and `room_plan_digest` explicitly into run confirmation.
It rejects missing or substituted parents, furniture-only reports on room-backed
runs, inconsistent room diagnoses and attempts to replace earlier route or
construction blockers. Existing one-step preparation, dispatch and commit gates
remain unchanged.

A stop before the first placement has no validated run scope and no outcomes.
A later stop retains all original steps, exact completed outcomes, the next
original step and the room diagnosis. Both withhold further confirmation and
mark `advance_allowed: false`. Failed acquisition never republishes an older
room observation. Final offline audit is not a fresh room check, and reaching
`all_placed` does not establish completed rooms. After progress, the old run
confirmation cannot be reused; a new review is required for the original suffix.

Use the existing isolated route-batch credentials and original batch ID:

```sh
python3 scripts/route_furniture_batch.py review-run \
  --directory /absolute/private/room-route-batch --batch-id ORIGINAL_BATCH_ID \
  --max-steps 3 --timeout-ms 10000

python3 scripts/route_furniture_batch.py run \
  --directory /absolute/private/room-route-batch --batch-id ORIGINAL_BATCH_ID \
  --max-steps 3 --timeout-ms 10000 --confirm-run CURRENT_RUN_CONFIRMATION
```

Paths and identities are illustrative. Only a ready review returns confirmation.
The room-aware scope intentionally requires a fresh room-run confirmation after
this update. Ordinary furniture-only scope bytes, limits and native behavior are
unchanged. No work budget is replenished between steps, no time-control effect
is introduced, and there is no new native method or automatic recovery.

## Evidence scope

Executed on the exact changed runner: all 15 existing `test_route_furniture_run`
tests pass with warnings treated as errors. Twelve additional local room-run
regressions also pass (27 total, no skips), including all 32 step caps, pre/post
progress room stops, lost replies, changed parents, refused stale confirmations,
unknown states, preserved earlier blockers and authorization loss after rendering.
Three deliberately weakened variants fail assertion regressions. Three legacy
scenarios (complete run, route blockage and step cap) produce byte-identical
review/result packets against the predecessor runner.

The new `test_room_route_run.py` upload was blocked by the tool's safety checks;
it is supplied as a separate patch/artifact and is NOT included in this commit.
Its tested Git blob identity is `a38bddd0b16d3a51de21fb28e69d8fc70f84925b`.
The existing test remains executable with:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error \
  -m unittest test_route_furniture_run -v
```

Tests execute the real run owner, digest validation and plan/progress code, but
inject batch custody and per-step effect/observation ports. They do not qualify
real private batch files, default native acquisition, actual TCP, DFHack, the
SDK/plugin, Rust/MCP, automatic clock workflows or the full workspace. No room
completion, atomic map lock, live compatibility or production admission is claimed.
