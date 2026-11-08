# Original Tree clock

This development component connects one acknowledged Tree/1.21 designation to
separately reviewed finite Run/1.13 intervals. It preserves the original Tree
obligation and its existing deadline while allowing game time to progress.
The adapter and same-session Tree MCP consumer are implemented. The existing
eleven tools now cover the original designation, separate clock review and
commit, bounded reconciliation, and fresh supply observation. The underlying
native Tree and Run protocols are unchanged.

## Immutable original

`Journal::clock_origin` verifies the held private Tree journal and its independent
ACK, reconstructs the canonical original and phase-two designation receipt from
retained bytes, and verifies the ACK again. Mutable public journal view fields
do not define the origin. The descriptor retains current consumed tick,
generation, sequence and item allocator floors, pending publication, continuity
loss and historical supply. Obtaining it performs no native contact or custody
write and restores no designation or clock permission.

`clock::Target::new` binds the Tree journal identity, source binding, canonical
original, exact designation receipt and original deadline. Later acknowledged
samples may advance the retained head and floors without changing that target.
No caller-supplied horizon can replace the original deadline. The original
coordinate/species predicate still is not a stable native plant identity.

## Reviewed finite advancement

The owner uses the existing bounded-run journal. A run first receives durable
Intent, then a fresh original Tree guard before native preparation. Commit
consumes the exact local review and records durable DispatchStarted before a
second fresh Tree guard and the one possible native unpause attempt. Wrong seals,
withdrawn confirmation, source/custody changes or failed guards cannot authorize
another attempt. An uncertain outcome retains its original key for query-only
reconciliation.

Every interval is at most 1,200 game ticks and 60,000 wall milliseconds, further
bounded by the explicit current allowance and remaining original deadline.
The Tree read grants remain scoped to the loaded map in the named fortress.
Named-fortress and native NIL clock authority are independently required;
Tree designation permission does not grant clock control. Opening grants and
current grants both remain relevant. Independently authorized safety stop is
available even after the goal expires; it cannot undo elapsed game time.

A fresh readiness sample must be paused at the exact Run observation tick and
retain the original source, root, dimensions, species and monotone counters.
The matching root, or its absence with outstanding nearby felling work, can
justify another bounded interval. Unknown or contradicted root predicates,
completed observed supply, and an absent root with neither nearby felling jobs
nor eligible new ground wood refuse further advancement. This bounded evidence
does not prove equipment, job assignment, lack of suspension, successful future
scheduling, or safety of every consequence of ordinary world evolution.

## Preserve enough room to observe the outcome

The existing Tree journal allows 16 progress samples, 40 frames and 16 MiB.
A reviewed interval needs one preparation guard, one commit guard and one
post-run goal sample. `TreeClockOrigin::require_capacity(3)` admits the full
interval against the verified snapshot; commit repeats the check for its two
remaining samples. Each check accounts for whole frames and worst-case bytes,
not only a sample counter. This is capacity admission, not a new durable
reservation. The exclusive host owner must reverify capacity at each boundary.
With one initial progress sample, these existing bounds permit at most five
complete three-sample intervals. Extra observations or a failed publication can
reduce the remaining capacity. Limits are never silently increased or reset.

## Time and goal evidence stay distinct

`TreeRpc::require_progress_at_or_after` raises an irreversible consumed-time
floor for subsequent progress reads. A decoded reply older than that floor is
retained in `consumed_progress`, rejected before credit and fences the connection.
The consumer must use the maximum retained Tree and Run floors before sampling.
A later Run receipt therefore cannot be followed by older Tree completion proof.

After a settled interval, only a fresh Tree sample can establish the original
supply conjunction: root predicate absent, no nearby felling jobs and eligible
new ground WOOD above the original item horizon. A run receipt or elapsed time
alone proves none of these. Causal provenance of the wood remains unknown.
Historical supply and the current pause state also remain distinct.

## Use the clock in the Tree MCP session

The clock is optional. Add this closed object to the ordinary Tree configuration:

```json
{
  "clock": {
    "initialize": true,
    "generation": 41,
    "recovery_generation": null,
    "recovery": null
  }
}
```

Use the actual independently pinned Run generation; 41 is illustrative. The
clock inherits the parent Tree endpoint and exact software pins. Its source
incarnation is independent of the Tree generation. Run/1.13 itself does not
identify a named fortress, so matching endpoint/software and joined observations
do not claim independently proven shared-process identity. The configuration
accepts no alternate endpoint, root, source selector, native command or deadline.

Set `DFMCP_ALLOW_UNADMITTED_TREE_CLOCK=1`, provide the independent native
`DFMCP_RUN_TOKEN`, and set `DFMCP_RUN_ALLOW_CLOCK=1` for a new control owner.
The existing Tree read credential and designation permission remain separate.
Without a configured clock, the Tree launch still rejects unrelated clock state.
New clock custody exclusively creates `tree-harvest.clock.journal` in the same
private directory. It never repairs or replaces an existing clock file.

Open Control with an explicit `max_game_ticks` between 1 and 1200; the default
remains zero. The logical byte-work default is 512 MiB when a clock is configured
and remains 256 MiB otherwise. The existing public maximum remains 512 MiB.
Smaller explicit byte allowances can inspect custody, but a Plan/Commit clock
operation needs its full 481 MiB admitted before either native source is entered,
including final Tree/ACK and Run publication checks. This is logical work
accounting, not an eager memory allocation.

1. Observe, plan and confirm the original `fell_one_visible_tree` designation
   using the ordinary default goal scope.
2. Call Wait in the default goal scope to obtain fresh original Tree progress.
   If unfinished evidence, deadline, authority and capacity permit advancement,
   `clock.new_run_available` is true and `clock.target_id` names the exact goal.
3. Plan a separate finite clock interval, for example:

   ```json
   {
     "kind": "run_tree",
     "target_id": "<clock.target_id>",
     "game_ticks": 100,
     "wall_millis": 1000
   }
   ```

4. Review `clock.active_run` and confirm with `scope="clock"`, its exact
   `review_seal` and `confirmed=true`. The Tree designation seal cannot confirm
   this interval. Plan never unpauses; each interval gets a separate review.
5. Call Wait with `scope="clock"` once. It queries the original run key and,
   after settlement, acquires a fresh acknowledged Tree supply sample. A running
   or unresolved clock blocks ordinary goal sampling. There is no polling loop
   or repeated unpause attempt.
6. If fresh supply is still unfinished and the original deadline and complete
   remaining sample capacity permit, review another interval under the same
   target. Root absence with neither felling jobs nor eligible wood is a supply
   blocker, not a promise that more time will help.

Cancel with `scope="clock"` retires that owner's undispatched preparation or
requests an independently authorized safety pause of its retained run. It does
not undesignate the tree, reverse elapsed time or renew the original goal.
The default `scope="goal"` retains the existing Tree cancellation behavior;
`scope="session"` releases local ownership while retaining all recovery custody.

The Agent Turn keeps Tree and clock confirmations separate, recommends
reconciliation and an available safety stop for uncertainty, and withholds more
time when capacity or the original source is blocked. `clock_advanced` is null
when a clock exists: a failed or missing reply cannot establish zero exposure.
Retained run receipts, attempts and consumed floors remain visible independently.
Failed encoded publication withdraws a held review before releasing the owner.

## Recovery and evidence

Keep the exact Tree ACK and the independent Run journal identity/head. Offline
inspection verifies retained bytes without either native credential or native
contact. To reopen, set both initializers false, retain the latest ordinary Tree
`acknowledged` pin, and provide `clock.recovery` with the exact Run journal ID and
externally retained head. Recover and Offline restore no review or native clock
control. Recover may append newly queried evidence to local custody and sample
the original Tree after settlement; Offline performs neither native contact nor
custody writes. Unknown outcomes are queried under their original keys without
another unpause; native safety stop belongs to the
independently authorized current Control owner. Missing, replaced or rolled-back
custody refuses. An optional new clock may be attached to an existing acknowledged
Tree only when that clock custody does not already exist; this does not restore
Tree designation permission.

The adapter passes 43 actual repository Tree tests: 20 new guarded clock cases,
eight exact journal/ACK and capacity cases, one native TCP stale-progress case,
and 14 existing regressions. The host's 15 focused tests pass, including seven
configuration/parser cases, six presentation cases and two actual Runtime/TCP
campaigns covering 13 scenarios plus recovery/corruption subcases. The complete
host component suite passes 424 tests, and the actual MCP library and all test
targets pass locked/offline typechecking. The component harness excludes MCP
transport and Asupersync supervision; the real crate typecheck covers those
registrations but does not execute them. These synthetic-peer results do not
claim a live DFHack/game, whole-workspace qualification or production admission.

The machine description is [tree_clock_v1.json](../architecture/tree_clock_v1.json).
