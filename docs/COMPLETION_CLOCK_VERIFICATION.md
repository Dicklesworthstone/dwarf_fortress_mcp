# Advance to the next original construction-stability sample

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`; WP-10.

An all-placed batch can be quiescent at one observation while its original
completion monitor still needs advancing-tick samples. The ordinary construction
progress owners intentionally authorize clocks only for pending construction.
`verify_route_furniture_completion.py` supplies the missing, separately confirmed
clock operation for an active original **candidate** completion history. It does
not make the clock receipt a construction sample or certify a goal by elapsed time.

## Original goal and history, never a replacement policy

Open the original route batch and its existing completion journal read-only.
The original journal must have a complete, uninterrupted candidate sample in
which every original furnishing meets its condition. Terminal, unsampled,
interrupted, incomplete or foreign histories refuse. The complete original goal,
receipts, endpoint and source remain validated by the existing owners.

A review binds the goal digest, exact journal head and frame count, original
count/cadence/span/gap/deadline and consumed observation allowance. It checks a
fresh complete final-construction frontier, one read-only clock observation and
an equal second frontier. Every furnishing must be currently quiescent. The
operations capture must equal the monitor's last accepted capture. A newer tick
or changed capture requires an ordinary completion sample first, not a skipped
observation or an assumed continuation of an old streak.

The bounded pure scheduler chooses an eligible next sample using exact integer
tick intervals. With `m` further eligible samples, it checks the reachable final
interval against the original minimum count/span, remaining observations and
exclusive deadline. It avoids spending the last observation too early or greedily
choosing a tick that prevents later samples fitting. It never extends the goal.
The chosen burst is at most 1,200 ticks and no larger than the original maximum
gap. A cadence that cannot be reached by one admitted burst refuses; no hidden
intermediate bursts or uncounted samples are performed.

## Review, explicitly advance once, then sample separately

Use the existing isolated combined construction-progress credentials: the route
batch opt-in/endpoint and furniture/map/operations read tokens, plus the existing
run/1.13 opt-in/endpoint/token. Route and run endpoints must be the same numeric
loopback address. Placement permission is refused. This does not configure or
qualify a game or native plugin.

Illustrative paths and shell variables below must identify existing original
custody, not new substitute journals:

```sh
python3 scripts/verify_route_furniture_completion.py review \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --completion-journal /absolute/private/monitor/completion.journal \
  --expected-goal "$GOAL_DIGEST" \
  --max-ticks 1200 --wall-ms 1000 --timeout-ms 10000
```

Inspect `result.completion_clock_review.decision`. Only `advance_for_sample`
returns `confirm_clock`. Retain and independently review that confirmation, then
use the same original goal and limits:

```sh
python3 scripts/verify_route_furniture_completion.py advance \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --completion-journal /absolute/private/monitor/completion.journal \
  --expected-goal "$GOAL_DIGEST" \
  --max-ticks 1200 --wall-ms 1000 --timeout-ms 10000 \
  --clock-directory /absolute/private/clock-records \
  --confirm-clock "$VERIFICATION_CONFIRMATION"
```

Execution reacquires the review and compares the independently supplied consent.
It uses the same complete prior-clock-history gate and locked private directory
as construction progress. An unresolved earlier effect cannot be bypassed by a
new goal or confirmation. Scope and native intent synchronize before preparation;
original completion history, batch custody, authority and prior clock files are
rechecked at both PrepareRun and CommitRun. There is at most one attempt of each,
no retry, placement or automatic loop. The completion journal is never written.

A native wall stop may occur before the selected target and a native tick stop
may overshoot it. Neither elapsed ticks nor a stopped receipt proves an eligible
sample, a current condition, a stability window or completion. If the result is
running or its reply was lost, use the existing selected-record query under its
**isolated run-only recovery environment**:

```sh
python3 scripts/recover_route_furniture_clock.py query \
  --clock-directory /absolute/private/clock-records --batch-id "$BATCH_ID" \
  --confirm-progress "$VERIFICATION_CONFIRMATION" --wait-ms 1000
```

The unchanged selected recovery owner can inspect/query/cancel a validated
original record without granting another start. Directory discovery advertises
this recovery entrypoint. The ordinary `route_furniture_progress.py query` is not
the recovery entrypoint for this new verification family. Existing batch-inventory
reconciliation can decode the same generic scope and original intent, but that
larger recovery path was not executed in this increment.

Once the original clock has stopped, the unchanged monitor sampler must acquire
and append real construction evidence under its existing isolated monitor-read
environment:

```sh
python3 scripts/track_route_furniture_completion.py sample \
  --journal /absolute/private/monitor/completion.journal \
  --batch-directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --expected-goal "$GOAL_DIGEST"
```

The sampler alone decides whether that observation advances the original streak,
resets it, expires the goal or establishes sampled completion. A changed monitor
head invalidates old clock consent. Further advancement always requires another
explicit review; original timing and total observation allowance remain fixed.

## Distinct compatibility and bounds

The new local scope is `dfmcp.route-furniture-completion-clock/1`, with phase
`completion_verification` and an `frv-<confirmation>` key. It uses the existing
`progress-<confirmation>.run` namespace, immutable scope/intent/outcome custody and
native run/1.13 records. Existing next-step and final-construction scope bytes,
digest domains, keys and semantics are unchanged. Decoding is a closed lazy
extension; old-only readers do not eagerly import the new scheduler.

One operation budget is created before either original owner opens. Its ceilings
are 682 RPC calls (two maximum final-read brackets, one read-only clock review and
one start), 48 MiB of network bytes, the existing 20 million cooperative work
units and 1 GiB of original-custody bytes, within 1..60,000 ms. These are ceilings,
not a guarantee that every maximal journal or capture fits. Additional checks can
exhaust an allowance and refuse. Budgets are never renewed per source or effect.
Output is bounded to 256 KiB and reserved before new clock work.

Original batch and monitor custody remain held through the single stdout attempt.
The clock-directory lock covers scope creation, dispatch, receipt retention and
result construction, not the later stdout call. Historical output is not current
pause or new permission. Lost acknowledgements, torn files and failed publication
retain the original recovery identity; there is no repair, tail truncation or
inferred nonapplication. Local cooperative custody is not a global controller
lease or malicious-owner rollback protection.

## Executed evidence and limits

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_completion_clock.py --mutations
```

All 35 tests pass: 10 pure scheduler, seven scope/history compatibility and 18
workflow/CLI tests. All seven deliberately weakened implementations fail
assertion-based regressions; syntax/import failures do not count. The scheduler
matches an independent exhaustive search over 4,320 small schedules plus seeded
existing-streak and early-poll cases. Tests cover precise deadlines and remaining
observations, original journal identity, changed captures, stale consent, lost
commit replies, revocation after prepare, unresolved prior history, publication
bounds and custody through a single output attempt. Ten executed source/test/
runner inputs are hashed before and after the full suite.

The actual scheduler, scope/context sources, history admission/classification
methods and complete executable owner run unchanged. Native/source codecs,
original batch/monitor replay and file custody, final acquisition, native clock
transport, scope/receipt files and the outer packet use explicit test ports.
The post-clock sample feasibility check uses an independent temporal oracle,
not execution of the existing native monitor sampler. These tests do NOT qualify
native TCP, actual private-journal integration, a DFHack SDK/plugin or live game,
Rust/MCP, full-workspace execution, physical power loss or production admission.
Independent selected recovery and original sampler integration are source-reviewed,
not newly end-to-end qualified. Broad beads remain open.
