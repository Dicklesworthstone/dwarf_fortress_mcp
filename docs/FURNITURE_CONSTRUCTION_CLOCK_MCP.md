# Whole-batch construction clock control through MCP

`dfmcp-build-placement-dev-server` can advance one separately reviewed finite
run/1.13 interval for an existing whole-plan furniture construction monitor.
The agent can place the complete batch, sample its original construction goal,
review a short interval of gameplay, reconcile the original run, and acquire
another ordinary completion sample. Every interval is an explicit action through
the existing eleven tools; there is no background simulation or polling loop.

This is an unadmitted development composition of existing furniture/1.19,
operations/1.4 and run/1.13 sources. The
[adapter contract](FURNITURE_CONSTRUCTION_CLOCK.md) describes its eligibility,
durability and authority rules. Exact executed tests and their limitations belong
to [implementation status](../IMPLEMENTATION_STATUS.md). A clock receipt and a
construction-readiness capture do not establish construction completion.

## Activate after the first ordinary completion sample

Start with the original [furniture batch](FURNITURE_BATCH_MCP.md) and
[whole-plan completion monitor](FURNITURE_COMPLETION_MCP.md). Keep
`DFMCP_BUILD_CLOCK_CONFIG`, `DFMCP_BUILD_CLOCK_TOKEN` and
`DFMCP_BUILD_ALLOW_CLOCK` absent during that initial workflow.

1. Finish every original batch step with a verified retained **Placed** receipt.
2. Create the fixed monitor with query mode `completion_start`. Its absolute
   deadline, cadence, required stable samples and full original target set are
   retained; later clock control cannot replace or renew them.
3. Acquire at least one accepted ordinary sample with
   `fortress.observe(session_id, selection="completion")`. This establishes the
   exact operations/1.4 source independently of the original furniture source.
4. Keep the returned original monitor identity and all custody files. Close the
   session with `fortress.cancel(session_id, scope="session")` and configure the
   next server process as below.
5. Reopen the existing batch with no `selection`, `furniture_plan` or
   `furniture_request`. Clock activation eagerly opens its private journal before
   the session can publish or sample combined state.

Clock configuration cannot accompany a new intent or a monitor without an
accepted sample. It is also refused in the monitor-only `completion-recover` and
`completion-offline` modes. There is no lazy clock initialization during a later
sample that could silently substitute new custody for an unknown earlier run.

## Configure the fourth private file and independent source

Keep `DFMCP_BUILD_JOURNAL`, `DFMCP_BUILD_BATCH`, `DFMCP_BUILD_COMPLETION`, the
original fortress identity and endpoint configuration. Add:

```sh
export DFMCP_BUILD_CLOCK_CONFIG='{"path":"/private/furniture/clock","initialize":true,"source":{"endpoint":"127.0.0.1:5000","generation":41,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"}}'
export DFMCP_BUILD_CLOCK_TOKEN='<matching run credential, 32..256 bytes>'
export DFMCP_BUILD_ALLOW_CLOCK=1
export DFMCP_BUILD_MODE=control
export DFMCP_BUILD_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
```

The closed clock configuration is at most 8 KiB. Its path must be normalized,
absolute and distinct from the three original custody paths. Private files use
exact mode `0600` under owned real directories with exact mode `0700`.
`initialize:true` exclusively creates a new clock journal; it cannot adopt,
truncate or repair an existing file. Opening a journal never advances game time.

The source endpoint must be canonical numeric loopback and match the original
furniture endpoint. The exact DF and DFHack versions must also match. The run
generation is pinned independently: do not substitute the furniture generation
or operations generation merely because the endpoint is the same.

The native game process separately enables run/1.13 with
`DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1` and `DFMCP_RUN_TOKEN`. Those native settings
belong outside the isolated MCP server environment. See
[bounded simulation runs](BOUNDED_SIMULATION_RUN.md). Fresh clock readiness also
needs the original `DFMCP_BUILD_TOKEN` and independent
`DFMCP_OPERATIONS_PAGED_TOKEN` for complete receipt-bracket acquisition.

`DFMCP_BUILD_ALLOW_CLOCK` must equal exactly `1` for advancement or a native
safety cancel. It is independent of `DFMCP_BUILD_ALLOW_PLACE`; completing the
batch does not confer clock permission. Operator configuration is checked again
at native boundaries, and the retained clock connection pins its credential.
Caller-supplied paths, credentials, source selectors and policy exceptions are
never accepted as MCP arguments.

The default checkpoint policy remains `required`, which refuses new clock
preparation and commit because no verified game-checkpoint provider exists.
The explicit disposable-fortress policy is the development exception. It does
not provide a save, rollback protection or restore capability.

## Open with an explicit game-time allowance

```text
fortress.open_session(max_wall_millis=60000, max_game_ticks=100)
```

The default `max_game_ticks` is **zero**. A positive allowance, at most 1,200,
requires configured clock custody, Control mode and the independent clock
opt-in. It is a retained per-operation ceiling, not a request to run the game.
To increase an existing zero allowance, close and reopen with the original
custody and exact clock recovery reference.

An interval must fit both opening and current authority, the original monitor's
maximum sample gap and unchanged absolute deadline. Retained interval intents
are bounded by `min(64, original max_observations)`; cancellation does not erase
an attempt or renew that allowance. The original monitor must remain unfinished,
unfenced and idle, have completed-acquisition allowance remaining, and belong to
an unstopped, unfenced original batch. Choose enough observations in the original
goal for both readiness captures and the ordinary samples required by the
planned intervals; the allowance cannot be increased by clock preparation.

## Prepare and separately commit one interval

If the sampled unfinished goal needs gameplay, prepare a bounded interval:

```text
fortress.plan(session_id,
  clock='{"monitor_id":"<original monitor id>","game_ticks":10,"wall_millis":5000}')
```

The closed request contains exactly these three fields. Native request bounds
are 1..1,200 game ticks and 1..60,000 wall milliseconds, narrowed by the original
monitor and session allowances. Omit placement keys and observation witnesses.
The coordinator derives the run identity from the complete original monitor,
its accepted operations source and the retained clock journal incarnation.

Preparation audits the full original parent, placement journal and monitor.
Its durable guard queries **every original Placed receipt**, acquires one
complete paged operations capture, then queries all original receipts again on
the same connection. Every original target must be either still pending with
viable construction or already meet its condition. Suspended construction,
removal, missing required jobs, changed identity and unverified item conditions
refuse advancement. The complete operations capture must be paused at exactly
the independently observed run tick.

The fresh bracket is retained as readiness evidence in the original monitor.
A synchronized read intent precedes acquisition, and a separate full-sample
readiness frame closes that read before native preparation or unpause. The
original source and exact custody are checked around publication, including after
synchronization. A lost or interrupted acquisition cannot restore permission.

Readiness cannot shrink the target set, earn positive stability credit or satisfy
the construction goal. Even if the same capture would satisfy an ordinary goal
sample, readiness remains uncredited evidence. The clock's pure assessment does
not count that already retained acquisition a second time.

Review `result.construction_clock.active_run`, including the requested interval,
before tick, plan digest and separate `review_seal`. Commit that seal explicitly:

```text
fortress.commit(session_id, scope="clock",
  monitor_id="<original monitor id>", review_seal="<returned clock seal>")
```

Omit placement key and plan digest arguments. Commit consumes the process-local
seal, synchronizes its dispatch marker, and repeats complete original custody
and fresh whole-batch readiness checks before its one native unpause attempt.
The run owner rechecks its own journal after the external guard. A lost reply or
failed synchronization cannot authorize repeating the commit. Reopening restores
history, never a review seal.

## Reconcile the original run, then sample construction

```text
fortress.wait(session_id, scope="clock",
  monitor_id="<original monitor id>", max_wall_millis=5000)
```

This performs at most one native query for the original unresolved run. Terminal
history returns locally. It cannot extend the interval, start another one or
sample construction. Missing or lost-source evidence remains unresolved.

Prepared, pending, unresolved, missing or unverified configured clock custody
blocks fresh completion sampling. Once the clock history is verified and settled,
acquire a separate ordinary sample:

```text
fortress.observe(session_id, selection="completion")
fortress.query(session_id, query='{"mode":"completion"}')
```

Only ordinary observations can add positive credit to the construction stability
window. Fully qualifying paused readiness can preserve an existing streak, so
explicit run/sample cycles can reach the required multi-sample result. Pending,
unknown, unpaused, same-tick changed or excessive-gap readiness clears unfinished
stability. Removal, identity/source regressions and deadline or observation
exhaustion retain their absorbing negative outcomes durably.

The monitor's `observations` field counts completed acquisitions: both ordinary
goal samples and retained readiness. Each run normally needs one readiness
acquisition during preparation and another during commit, plus any separate
ordinary proof sample. All use the original `max_observations` allowance.
Readiness never increments `streak`, establishes `first_tick` or advances
`counted_tick`; those fields describe credited ordinary samples. A monitor that
exhausts its acquisition allowance during readiness refuses new native dispatch.

Successful readiness closes its durable read intent without manufacturing an
interruption. Failed guards, custody, runtime publication and unacknowledged
worker results still withdraw unfinished continuity when required. Restart also
withdraws the live streak while preserving all retained readiness facts and
source/stage/allocation floors. The deadline, total acquisition allowance and
original target set never renew. Terminal historical achievement remains
historical.

## Inspect history or request a safety pause

Use the clock head returned by the latest response, including open-session
recovery metadata. Even the first history page requires its exact head:

```text
fortress.query(session_id,
  query='{"mode":"construction_clock","monitor_id":"<original monitor id>","head":"<clock head>","offset":0,"limit":8}')
```

The query is local, with offset 0..63 and page size 1..8. Use the returned
`runs_page.next_query` unchanged; a changed head refuses mixed history pages.
Responses keep pending and unresolved state visible independently of pagination.
Rows expose original keys, plan digests, native receipts, observed advancement
and overshoot. `readiness` is labeled historical pre-dispatch evidence, with
`goal_sample_published=false`. A retained pause receipt does not prove that the
game is paused now.

```text
fortress.cancel(session_id, scope="clock", monitor_id="<original monitor id>")
```

Before dispatch, cancellation retires the local intent. After possible dispatch,
it requests the original native safety pause; only that safety action may repeat.
It requires current and retained Query and ControlClock authority in both the
named-fortress and independent native-source domains. It does not require new
Plan/Observe authority, a future game-time budget or an unexpired construction
deadline. Once the owner is retained in the current session, original batch or
monitor damage does not disable reconciliation or this safety path. Native
source loss cannot establish that the game stopped.

Cancelling `scope="completion"` stops only the monitor. Cancelling `scope="batch"`
stops only new batch advancement. Neither cancels the clock. Ordinary session
close refuses unsettled clock effects. Explicit `release_for_recovery=true`
releases local ownership while preserving original evidence; it does not pause
the game or settle an unknown effect.

## Restart without losing original run identity

Keep clock recovery references from successes and failures. Reopen the same
clock file with `initialize:false` and its original journal identity and an
acknowledged head:

```json
{
  "path": "/private/furniture/clock",
  "initialize": false,
  "source": {
    "endpoint": "127.0.0.1:5000",
    "generation": 41,
    "df_version": "<exact DF version>",
    "dfhack_version": "<exact DFHack version>"
  },
  "recovery": {
    "journal_id": "<original clock journal id>",
    "head": "<acknowledged clock head>"
  }
}
```

The acknowledged head must occur in the verified intact chain. A complete later
tail can recover a lost response; rollback, unrelated replacement and torn tails
are refused. Existing empty journals also need their original recovery pin.
Keep `DFMCP_BUILD_CLOCK_CONFIG` on every combined-session reopen. The older
monitor format does not durably require this optional association: omitting the
configuration cannot prove that no clock history exists.

Normal `offline` mode inspects existing history without native calls or writes.
Normal `recover` mode can query an original unresolved run but cannot start an
interval or acquire safety-cancel authority. These combined modes still require
the three original furniture files plus the exact clock file and accepted
original operations source. Monitor-only recovery refuses configured clock custody.

If the placement journal or furniture parent is unavailable, use
[`clock-offline` or `clock-recover`](FURNITURE_CONSTRUCTION_CLOCK_RECOVERY.md).
Those independent modes open only the exact original monitor and clock histories,
using `result.completion.recovery` and `result.construction_clock.recovery`.
Query-only recovery can reconcile one original outcome; separately enabled
retained/current clock authority permits original safety cancellation. Future
game ticks remain zero, and recovery cannot sample construction, prepare or
commit a new run. The first accepted operations source remains the original
clock association even if a later sample invalidated the monitor after a
source change. The linked guide specifies configuration and close/release behavior.

Before publishing an opening response, the server prepays and rechecks the
original placement journal, batch custody, complete monitor and exact clock
recovery head after the final runtime checkpoint. These checks keep the opening
request's original absolute deadline and cannot renew their allowances. Later
combined responses also reserve and recheck exact clock history after their
final checkpoint. Current and retained Query authority governs cached handoffs.
Every clock projection includes the greatest consumed authority floor, including
a decoded construction capture rejected by a later original-custody check. Durable
run evidence reconstructs its floor on restart; rejected unjournaled read floors
remain process-local. Worker acknowledgement covers the joined local runtime,
not delivery to a remote MCP client.

## Evidence boundary

Tick and wall bounds are native callback stop triggers, not exact-tick or hard
real-time guarantees. Overshoot remains visible. The native run owner outlives
one RPC but does not fence unrelated plugins, UI actions or other controllers.
Endpoint, exact software and independently pinned generations associate the
sources without proving shared process identity. Run/1.13 has no named-fortress
lineage; its grants are independently issued, never derived by rewriting scoped
furniture authority.

This workflow does not prove construction causality, hauling or worker
reachability, continuous stability, current pause state or present furniture
usability. It adds no native wire generation, compatibility admission or
production runner. The fixed whole-plan completion monitor remains the sole
publisher of its sampled construction condition, and game checkpoint/restore
remains unavailable.
