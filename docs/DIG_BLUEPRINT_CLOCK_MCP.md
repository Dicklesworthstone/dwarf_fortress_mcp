# Finite clock advancement for an original mining blueprint

`dfmcp-dig-control-dev-server` advances the game through one separately reviewed
run/1.13 interval while retaining the entire original blueprint and its fixed
excavation monitor. The existing eleven MCP tools connect designation, finite
gameplay, reconciliation and a subsequent whole-blueprint completion sample.
No call automatically repeats an interval or polls until the goal is satisfied.

Start with [original blueprint designation](DIG_BLUEPRINT_MCP.md) and
[whole-blueprint monitoring](DIG_BLUEPRINT_MONITOR.md). All original children
must have verified terminal **Designated** receipts. A new interval also needs
an unstopped parent, an unfinished unfenced monitor, remaining sample capacity,
and current independent clock authority. Native designation receipts and clock
receipts never establish excavation completion.

## Operator configuration

Retain the original dig, parent and monitor configuration, then configure a
fourth private custody file and an independently selected run source:

```bash
export DFMCP_DIG_BLUEPRINT_CLOCK_CONFIG='{"path":"/private/mining/project/clock.journal","initialize":true,"source":{"endpoint":"127.0.0.1:5000","generation":41,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"}}'
export DFMCP_DIG_BLUEPRINT_CLOCK_TOKEN='<matching run token, 32..256 bytes>'
export DFMCP_DIG_ALLOW_CLOCK=1
export DFMCP_DIG_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
```

The clock opt-in must equal exactly `1` for new advancement or a safety cancel.
It is independent of the designation opt-in. The default checkpoint policy is
`required`; it refuses new clock preparation and commit with
`CheckpointRequired`. The explicit disposable-fortress exception acknowledges
that this development workflow has no game checkpoint.

The clock endpoint must be canonical numeric loopback and match the monitor's
endpoint and exact DF/DFHack software versions. The run generation is pinned
independently from the dig and map generations. All four custody paths must be
distinct. Keep each file under an exact-mode `0700` real directory; private
files use exact mode `0600`. Paths, credentials and source selectors are
operator configuration, never caller-selected MCP arguments.

The game process separately enables its run plugin with
`DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1` and `DFMCP_RUN_TOKEN`. These native plugin
settings belong outside the isolated MCP server environment. See
[bounded simulation runs](BOUNDED_SIMULATION_RUN.md) for that setup.

With a newly registered monitor, clock custody is created on the first clock
preparation or fresh monitor sample. If the retained monitor already exists
when the session opens, its configured clock is opened then, including
exclusive creation for a new clock. `initialize:true` cannot adopt an existing
file or forget an uncertain run. Creating an empty clock journal never unpauses
the game.

## Open with an explicit game-time allowance

Use the original dig region and an explicit positive session ceiling:

```text
fortress.open_session(region="[2,2,2,8,1]", mode="control",
  max_wall_millis=60000, max_game_ticks=100)
```

Substitute the actual original region. The default `max_game_ticks` is zero.
A positive value, at most 1,200, is an explicitly retained per-operation ceiling
for operator-configured clock control. Opening the session advances no game
time. A session opened with zero must close and reopen with its exact custody
pins and a positive allowance before it can advance.

Every requested interval must fit the current and original session allowances,
the monitor's maximum sample gap, and its unchanged absolute deadline. The
clock also bounds retained intervals by the smaller of 64 and the monitor's
maximum samples. Cancelling an interval does not reset that count.

## Review and commit one interval

First acquire a normal whole-blueprint monitor sample:

```text
fortress.wait(session_id, monitor_id="<original monitor id>")
```

If the unfinished goal needs gameplay, prepare one interval:

```text
fortress.plan(session_id,
  clock='{"monitor_id":"<original monitor id>","game_ticks":10,"wall_millis":5000}')
```

The closed clock request has only those three fields. Native limits are
1..1,200 game ticks and 1..60,000 wall milliseconds, further narrowed by the
original allowances. Omit native keys, witnesses, blueprint creation and
monitor registration fields. The coordinator derives each run key from the
whole original monitor and retained run history.

Preparation verifies the complete original designation inventory, parent and
monitor custody, then acquires a fresh paused map capture over the entire
original bounding box. Every selected tile must be visible, dry, non-magma,
unoccupied by a building, and either a wall with Mine designated or a cleared
floor. Unselected holes are ignored. The map tick must match the independently
captured paused run witness.

Readiness is not a goal sample. If the fresh terrain would already satisfy the
unchanged goal, preparation refuses an unnecessary run. Retire the retained
intent and publish a normal monitor sample instead.

Review `result.blueprint_clock.active_run`, especially the interval, original
tick, plan digest and separate `review_seal`. Commit exactly that seal:

```text
fortress.commit(session_id, scope="clock",
  monitor_id="<original monitor id>", review_seal="<returned clock seal>")
```

Omit a native key or plan digest. Commit repeats the original-custody and fresh
whole-map checks. After recording its dispatch marker, the run owner verifies
its own journal again following the external guard and before its one native
commit call. The seal is consumed; reopening never restores it. Native
preparations also have a finite lifetime.

## Reconcile, then sample the goal separately

Clock wait performs a single bounded reconciliation step:

```text
fortress.wait(session_id, scope="clock", monitor_id="<original monitor id>",
  max_wall_millis=5000)
```

It issues at most one native run query; a retained terminal result replays
locally. It never unpauses, extends an interval or samples terrain. A missing
receipt or uncertain outcome remains pending. Reconcile or cancel that
original interval before preparing another.

Any pending interval, including a merely prepared interval, blocks fresh goal
sampling. Once the retained clock is verified and settled, call
`fortress.wait(session_id, monitor_id=...)` without `scope="clock"`. Only that
separate whole-blueprint acquisition can publish a fresh sampled excavation
condition.

Positive readiness captures contribute no monitor samples or completion proof.
A readiness capture that contradicts an earlier matching window withdraws that
window. The monitor records a reset before its next map acquisition, so later
matching terrain cannot reuse evidence from before a known contradiction.
Failed guards, source checks or runtime publication also withdraw unfinished
continuity. Reopening breaks an unfinished window; the original deadline and
attempt history remain fixed.

## Inspect or stop the retained clock

Clock history queries are local and bounded to eight rows:

```text
fortress.query(session_id,
  query='{"mode":"blueprint_clock","monitor_id":"<original monitor id>","offset":0,"limit":8}')
```

Use the returned `next_query` unchanged; its expected head prevents mixing
pages from different journal states. Rows expose original keys, plan digests,
states, receipts, observed advancement and overshoot. Keep the returned recovery
identity and head. A historical pause receipt is never a claim that the game
is paused now.

To retire or stop the original interval:

```text
fortress.cancel(session_id, scope="clock", monitor_id="<original monitor id>")
```

Before dispatch this retires the interval locally. After possible dispatch it
requests the original native safety pause; only that safety action may repeat.
Cancellation requires current Query and Clock authority in both independent
authority domains. It does not require new planning or observation grants, a
future game-time allowance, an unexpired goal deadline, or intact parent and
monitor custody once this clock owner is already retained. Source loss cannot
prove that the game stopped.

Cancelling the monitor stops monitoring only. Cancelling the blueprint stops
project advancement only. Neither implicitly cancels a clock interval.
Ordinary session close refuses pending or unknown clock effects.
`release_for_recovery:true` releases ownership with an unverified handoff; it
does not pause the game, settle an effect or erase the journal.

## Restart with exact custody pins

Retain clock recovery metadata from successful responses and failures. Reopen
the same file with `initialize:false`:

```json
{
  "path": "/private/mining/project/clock.journal",
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

The acknowledged head must occur in the intact chain. A complete later tail is
accepted; rollback, replacement and a torn tail are refused. Even an empty
existing journal needs its original pin. Creation failures retain the intended
header identity when available; they never authorize replacing an existing
file.

Keep the clock configuration on every combined-session reopen. This optional
configuration is **not an omission-proof custody association** in the older
monitor format. Omitting it cannot establish that no clock history exists.

Offline opening permits historical inspection. Reconcile mode permits native
queries without new advancement. New intervals and safety cancellation require
independently enabled Control authority. Combined restart still requires the
original native dig journal, parent and monitor custody. If those are damaged
but the clock journal remains intact, use the
[standalone bounded-run runtime](BOUNDED_SIMULATION_MCP.md) for offline
inspection, reconciliation or independently authorized safety cancellation
under the same source binding. Do not prepare a replacement interval to bypass
uncertainty about the original. Standalone run recovery does not reconstruct
the blueprint proof. Monitor-only recovery does not open or conceal a
configured clock.

A failed response publication or an unacknowledged worker result withdraws
unfinished monitor continuity and drops the prepared clock seal. The worker
acknowledgement refers to the joined local runtime, not remote network
delivery. Keep all returned references and reconcile the original interval
after an uncertain acknowledgement.

Before publishing a clock result, the handler reserves and performs one final
exact journal check after the last runtime checkpoint. A changed history
withdraws the response and prepared review; commit/cancel failures retain an
explicit possible-effect flag. Recovery references also retain the highest
consumed authority tick from the dig, monitor and clock owners, including a
map capture later rejected by custody verification. Keep that floor with the
original recovery references; it is not permission to replay a commit.

## Meaning of the limits and evidence

Tick and wall limits are native callback stop triggers, not hard real-time or
exact-tick guarantees. Receipts expose observed overshoot. The native owner
outlives an RPC, but it does not fence independent plugins, UI actions or other
controllers.

A run receipt proves neither mining causality, structural safety, worker
access, continuous terrain stability nor current pause state. Run/1.13 has no
named-fortress lineage. Its authority is issued independently, never derived by
rewriting map grants. Endpoint, software and generation checks associate the
sources without proving shared process identity.

This remains an unadmitted development composition of existing protocols. See
[the original-blueprint clock contract](DIG_BLUEPRINT_CLOCK.md) and
[implementation status](../IMPLEMENTATION_STATUS.md) for the evidence boundary.
