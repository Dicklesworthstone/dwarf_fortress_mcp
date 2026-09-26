# Whole-plan furniture construction goals

The shared foreground watch engine accepts the closed `furniture_set` condition.
One watch can cover 1..32 explicitly selected beds, chairs and tables, with optional
exact installed items. All targets share one sample history, fixed deadline and
stability streak, instead of consuming one of the eight session watch slots per
building. The existing spatial/1.6 and citizen/spatial/1.8 `construction_progress`
query can generate the complete request with `monitor.mode = "all_targets"`.

This increment is source present with executed independent Python reference
checks. Its Rust tests have **not been compiled or executed here**. It adds no game
mutation, native protocol, dependency, production admission, receipt import or
native-effect discharge. Existing implementation evidence does not qualify it.

## Request one proposal for the complete selection

In an existing spatial session, submit the following as the `query` argument of
`fortress.query`. IDs are illustrative; select them from the current observation.
The deadline must be future, fit the session horizon, and allow the requested
cadence and sample count. Use the returned proposal's session ID when registering.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "construction_progress",
    "targets": [
      {"building_native_id": 10, "item_native_id": 20},
      {"building_native_id": 11, "item_native_id": 21}
    ],
    "monitor": {
      "mode": "all_targets",
      "key_prefix": "bedrooms",
      "deadline_tick": 100801200,
      "poll_interval_ticks": 1,
      "stable_observations": 2
    },
    "limit": 1
  }
}
```

The whole-plan proposal is at **`result.monitoring.watch_request`**, not inside a
row. Submit that complete envelope explicitly through the existing watch query to
register it. Its key is `<key_prefix>.all`. Preview alone creates no watch, reads
no new native capture and modifies no game state. The proposal's exact anchor must
still be current at registration; do not silently remove a stale anchor.

Every page repeats the identical complete-selection proposal, even when only one
row fits. A missing, unsupported or mismatched building, or an unestablished exact
item identity, makes `monitoring.available=false` and lists all affected building
IDs. It returns **no smaller replacement goal**. A complete proposal plus at least
one row and summary must fit the enclosing output allowance or the query refuses.

Omitting `mode`, or setting it to null, retains the previous per-building proposal
behavior and normalized query identity. Other mode values are rejected. Switching
mode, target selection, key, cadence or deadline invalidates a continuation.
Changing page width or input target order does not change the normalized selection.

## One simultaneous condition, not accumulated individual successes

The generated success predicate uses `test: "all_complete"`; the failure predicate
uses `test: "any_removal"` over the same complete target set. Every selected
construction predicate must hold in the same observation for the same watch's
stable sample streak. A building matching yesterday and another matching today do
not establish that both match now. A later mismatch resets the common streak;
repeated polls of one anchor do not add qualifying samples.

Normal watch cadence, fixed deadline, epoch/generation invalidation, cancellation,
terminal immutability and configured spatial/1.8 persistence still apply. A true
removal predicate fails the goal. Missing or unestablished roots remain unknown,
including in removal checks; recycled generations invalidate the whole watch even
if another target already supplied a decisive boolean result. Cancellation stops
only monitoring, not construction jobs, miners, or the game clock.

Each target has `building_native_id`, `building_generation`, `kind` (`bed`, `chair`,
`table`) and `max_stage` (1..32). Optional `item_native_id` and `item_generation`
must both be supplied or both absent/null. Generations are canonical generations
from the observing session, not native plugin incarnation counters. Targets in a
`furniture_set` must be strictly ordered by native building ID. Duplicate building
IDs, duplicate selected items and incomplete identities are refused.

For each target the evaluator expands the same fixed stage/type/job/item recipe
used by the original per-building construction proposal. It invokes the existing
field, count and relationship evaluator with the **same shrinking work budget**.
It does not substitute a second truth evaluator or monitoring state machine.

Multi-target evidence retains each target's truth, root-binding and generation
status, plus a digest covering its complete predicate trace and exact anchor. No
target summary is dropped. A single-target `condition_evaluation` additionally
returns the full predicate trace for drill-down. The normal `construction_progress`
rows remain the route to detailed stages, jobs and item relationships.

These are conditions on observed entities. They do not verify an external
placement receipt, original footprint, common incarnation across plugins,
causality, current usability, continuous history, safety or game checkpoint.
Native effect obligations remain unchanged.

## Bounds, persistence and compatibility

A condition accepts at most 32 targets; one success/failure definition may contain
at most 64 target occurrences across all furniture-set nodes. This admits a
32-target success plus a 32-target failure predicate, not arbitrary nested
multiplication. The existing outer 64-node/depth-eight limits remain unchanged.
Input stays within 1,024 nodes and 32 KiB conservative accounting; evaluation keeps
its one million shared work units and cooperative deadline. Because the existing
population evaluator scans the captured projection, large rosters may exhaust
this allowance. The operation refuses rather than skipping targets or extending
the deadline. Complete rendering precedes visible registration or progress.

Schema discovery includes the new op for condition inspection and existing watch
registration. Old condition encodings are unchanged. Definitions pass through the
existing serde/checkpoint/recovery path; no separate store, timer or background
worker is added. Older binaries reject the unknown op rather than weakening it.
Downgrading a journal containing new definitions is unsupported. This increment
does not establish an executed migration, recovery or physical power-loss campaign.

## Validation scope

Run the retained independent reference checker:

```sh
python3 scripts/check_furniture_set_reference.py
```

It executes 20 accepted and 24 rejected furniture-condition schema examples,
nine accepted and ten rejected query-mode examples, three semantic identity/order
refusals, and 19,680 three-valued set-reduction cases. It also verifies that removing
only the additive mode property reconstructs the exact prior query-schema Git
blob. Its maximum-width paired 32-target request has 465 input nodes, 20,348
conservatively accounted bytes and 9,976 serialized bytes. This is a modeled watch
request, **not** an actual MCP/Agent Turn response measurement or Rust execution.

Fifteen new Rust test functions are registered: ten shared condition tests and
five shared spatial-query tests. They exercise the actual existing evaluator,
router and watch transitions when run. Coverage includes fixed-recipe equality
with the unchanged predicate fixture, complete 32-target plans, item flags,
late generation changes, unknown facts, removal, joint stability, one-slot
registration, missing off-page identities, complete pagination, old-mode identity,
no implicit registration, denied authority and rejected publication. The existing
ten per-building query tests are retained unchanged. **None of these Rust tests
was compiled or executed in this editing environment.**

Exact checked source hashes and reference counts are in
`docs/evidence/furniture-set-reference.json`. No real DFHack, live-fortress,
full-workspace, native ABI or production qualification is claimed.

Beads: `df-action-coordinator-exec-ero.4`, `df-dfhack-bridge-plane-c-pic.3`.
