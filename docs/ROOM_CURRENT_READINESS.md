# Check current room conditions without changing historical completion

A satisfied monitor is an immutable historical result. `inspect` and terminal
`sample` remain offline and do not establish that the rooms still satisfy their
requirements. The new `check` operation acquires one fresh complete sample and
reports the whole original condition separately from that historical result:

```sh
python3 scripts/track_room_readiness.py check \
  --journal /private/room-monitor/readiness --timeout-ms 10000
```

Use the same isolated query-only environment as `ROOM_READINESS.md`, including
its original numeric loopback endpoint and build/operations/map credentials.
No new environment variable, credential, native method or MCP tool is added.
The original batch and monitor must both remain available under their existing
private-file custody. A retained journal path does not grant read authority.

## One original selection, one fresh query

Check replays the original goal and opens both the monitor and original batch
read-only. It accepts no replacement batch, deadline, sample policy or circulation
flag. The original `DFMRDG01`/`DFMRDG02` condition determines whether circulation
is required. The existing query client verifies every original placement receipt
around matching paused map endpoints and one released complete operations
capture, then closes its connection before CPU-bound diagnosis.

All original terrain requirements, furnishing identities and occupancy are
checked. A strict original goal additionally checks all required circulation
routes. The stateless construction assessment is the same evaluator used by the
monitor; no replacement deadline or fabricated one-sample goal is introduced.
Check works after satisfaction, cancellation or expiry, and while a prior monitor
read is unresolved. It never clears that unresolved read, contributes a stability
sample, renews a deadline, changes terminal history or reconciles a placement.
An invalidated original identity cannot be revived by a later positive sample.

## Read the two result sections separately

The check response uses `dfmcp.room-readiness-check-result/1`:

- `result.history` retains the original monitor response, complete original
  plans, historical progress and custody information.
- `result.current_readiness` contains the complete fresh diagnosis, original
  goal identity, game tick, source witnesses, per-furnishing assessments,
  terrain/circulation evidence and invalidators.
- `result.current_condition_at_sample` is `true`, `false` or `unknown` for the
  fresh sample, or null when acquisition/publication was refused.

The Agent Turn briefing separately names historical satisfaction and current
condition truth. A historically satisfied room with a newly missing wall can
therefore return historical success and current false without conflating them.
The top-level result never reuses historical `room_readiness_sampled_condition`
as a fresh-success flag. Existing non-check response schemas/shapes are unchanged.

Current status is one of `condition_met`, `condition_not_met`, `blocked_unknown`
or `invalidated_reference`. Hidden/missing terrain and unavailable occupancy
remain unknown. Known pending/removal conditions are false; unverified item
installation is conservatively unknown. Complete domain counts, not the bounded
terrain-deficit preview, determine whole-group truth. All original assessments
remain in the result after a decisive failure.

The last durable monitor sample supplies source/clock/native-allocation-horizon
floors and known building type/stage floors. A changed or missing original
identity, regressing source or clock, or changed map incarnation blocks qualified
positive truth. `domain_truth` still exposes the separately evaluated domains;
`condition_truth` is unknown when original identity cannot be retained.

## Ephemeral, read-only and bounded

Check performs exactly one query acquisition, not a polling loop. A failed
acquisition is not retried automatically. It creates no read-start or sample
journal frame. Failed output, revocation, or lost original custody withholds the
fresh diagnosis and leaves the original history unchanged. Interrupted check
requests therefore cannot be recovered as monitoring samples. This differs
intentionally from durable `sample` and `wait`.

The new diagnosis is not saved in the monitor and does not advance a persistent
source floor for the next check. Each check is independently compared with the
retained monitor history. Separate check calls do not prove uninterrupted
observation, cross-check monotonicity or continuous access. Save the returned
response separately when its diagnostic evidence is needed; it is not a signed
native-acquisition attestation or a new canonical world anchor.

The current report is bounded to 64 KiB and every original furnishing is retained
(up to 32). The CLI reserves the complete historical response plus that maximum
and a 4 KiB presentation margin before native contact; the unchanged 256 KiB
whole-response limit still applies. Native calls, decoding, original-file replay,
diagnosis and final output share the existing shrinking budget. Current authority
and original custody are rechecked after acquisition and rendering, and authority
is checked again after closing the owners. A partial stdout write causes neither
a second object nor another acquisition.

Exit 0 means a valid diagnosis was returned, not that conditions were true.
False, unknown and invalidated-reference diagnoses are valid query results.
Exit 2 means request, read, custody, authority, budget or publication was refused.
Use `sample`/`wait` for durable progress and `inspect` for offline history.

## Evidence boundary

The 15 current-module tests and 12 complete-CLI-module tests executed successfully
with explicit validated-native-producer and file/transport/authority test doubles.
The current evaluator is not replaced in the CLI tests. Cases include all
27 three-valued combinations, every core guard interruption, all terminal phases,
32 targets, lost walls, source regression, lost reads, read-only opens, policy
substitution refusal, post-acquisition custody failure, final revocation and
short stdout writes. This is focused component evidence, not real native codecs,
private-file custody, TCP or game execution.

Eight companion tests in `tests/test_room_readiness_current.py` use the actual
native fixture codecs and real original batch/journal owners (replacing only
network acquisition in CLI cases). They are checked in but could not start in
the editing workspace: its partial source checkout lacks `furniture_batch` and
other upstream modules. Their outcomes are unverified, not passed.

Run in a complete checkout with existing room suites:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_readiness_current_isolated test_room_readiness_check_isolated \
  test_room_readiness_current test_room_readiness test_room_readiness_store -v
```

No Rust/MCP, DFHack SDK, live-fort or full-repository qualification was performed.
Sampled readiness still does not prove native room assignment, structural safety,
native unit pathfinding, present usability or production admission.
Owning beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (remain in progress).
