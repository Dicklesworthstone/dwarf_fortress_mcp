# Explicit construction progress between route-batch placements

The route-batch execution frontier now requires every already-placed original
furnishing to be physically installed and free of held/item-linked work before
another furnishing may be dispatched. That safety gate intentionally stops a
bounded multi-step placement run while construction is still pending.

`scripts/route_furniture_progress.py` provides a separate, explicitly confirmed
way to advance the simulation clock between placement runs. It composes the
existing route-batch read frontier with the existing native run/1.13 watchdog.
It does **not** weaken construction checks, place furniture, retry placement,
loop automatically, or reinterpret elapsed ticks as construction success.

## Review and execute one clock burst

First review the current construction-pending frontier:

```sh
python3 scripts/route_furniture_progress.py review \
  --directory /absolute/private/route-batch \
  --batch-id <batch-id> --ticks 120 --wall-ms 5000
```

Only a current `construction_pending` frontier receives
`progress_review.confirm_progress`. Ready, route-blocked, construction-blocked
and unestablished states do not get clock consent. The confirmation binds the
batch/review identity, complete local inventory identity, next original step,
fresh composed frontier evidence, requested game-tick ceiling and wall watchdog.

To execute exactly that reviewed clock effect:

```sh
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1 \
DFMCP_RUN_ENDPOINT=127.0.0.1:5000 \
DFMCP_RUN_TOKEN='<run/1.13 token>' \
python3 scripts/route_furniture_progress.py progress \
  --directory /absolute/private/route-batch --batch-id <batch-id> \
  --ticks 120 --wall-ms 5000 --confirm-progress <confirmation> \
  --clock-directory /absolute/private/clock-records
```

The ordinary route-batch credentials are still required. The route and run
endpoints must be the same canonical loopback address. This progress process
explicitly refuses furniture placement permission, so the only mutation it can
request is the fixed run/1.13 clock effect.

The private clock directory must already satisfy the run client's owner-private
0700 custody rules. The record filename and run idempotency key are derived from
the reviewed confirmation. Repeating the same reviewed progress effect therefore
hits the same local record and native key instead of manufacturing another
unpause under a different name.

One invocation sends at most one PrepareRun/CommitRun sequence. It reserves
route-batch aggregate call/network/disk allowance before entering the clock
effect and shares the existing foreground wall deadline. The native run profile
still enforces its own 1..1200 game-tick and 1..60000 ms watchdog limits.

If the run returns a fully verified stopped receipt, the process reacquires the
complete route/construction frontier while the game is paused. It reports whether
construction became ready; it never automatically performs a second clock burst
or a furniture placement. If construction remains pending, obtain and review a
new progress confirmation for another bounded burst.

## Lost replies and recovery

A commit transport failure may leave the clock effect indeterminate. Never repeat
`progress` with a different record. Query the exact retained intent instead:

```sh
python3 scripts/route_furniture_progress.py query \
  --directory /absolute/private/route-batch --batch-id <batch-id> \
  --confirm-progress <confirmation> \
  --clock-directory /absolute/private/clock-records --wait-ms 5000
```

Query uses the retained confirmation-derived key and path. It may issue
QueryRun, but never PrepareRun or CommitRun. A retained terminal outcome is read
offline without native contact. Only an exact stopped receipt with verified pause
permits a fresh route/construction review afterward.

The run intent/outcome journal remains separate from the furniture batch. Clock
recovery cannot resolve an uncertain placement receipt, and route readiness
cannot resolve an uncertain clock effect.

## Evidence boundary

This source increment adds no native method, wire field, production runner,
compatibility entry or MCP tool. It reuses run/1.13 and furniture/map/operations
profiles exactly as separate unadmitted development families.

Eight Python source regressions are registered for confirmation binding,
confirmation-derived identity, single-dispatch structure, query-only recovery,
read-only batch custody and explicit no-loop/no-placement claims. They are
source-present but **not executed in this editing environment** because the
repository cannot be materialized locally from GitHub here. No Rust, native
plugin build, live fortress, crash campaign, full workspace qualification or
production admission is claimed.

Beads: df-dfhack-bridge-plane-c-pic.4/.5 and df-franken-storage-mvcc-54h.4.
