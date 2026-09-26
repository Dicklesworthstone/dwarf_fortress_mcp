# Whole-plan furniture construction goals

The shared foreground watch engine now accepts the closed `furniture_set`
condition. One watch can cover 1..32 explicitly selected beds, chairs and tables,
with optional exact installed items. This removes the need to consume one of the
eight per-session watch slots per building. Source is present; Rust/MCP compilation
and execution of this increment are not established in the editing environment.

## Semantics

Use `test: "all_complete"` for success and a second condition with the identical
targets and `test: "any_removal"` for failure. All selected construction predicates
must hold in the same observation, for the same watch's stable sample streak.
Independently successful historical watches are not combined into current success.
Normal cadence, fixed deadline, epoch/generation invalidation, cancellation,
terminal immutability and configured spatial/1.8 watch persistence still apply.

Each target has `building_native_id`, `building_generation`, `kind` (`bed`, `chair`,
`table`) and `max_stage` (1..32). Optional `item_native_id` and `item_generation`
must both be supplied or both absent/null. Generations are canonical generations
from the current observing session, not native plugin incarnation counters.
Targets must be strictly ordered by native building ID. Duplicate building IDs,
duplicate selected items, incomplete item identities and invalid bounds are refused.

For each target the implementation expands the same fixed stage/type/job/item
recipe used by the original per-building construction proposal. It invokes the
existing field, count and relationship evaluator with the SAME shrinking work
budget. It does not replace unknown values with success, change the condition
language's old limits, take another native capture or create a second state machine.
Explicit building and item roots are checked even in removal-only conditions and
even after an earlier decisive target. A recycled late identity invalidates the
watch; missing/unestablished roots cannot yield a successful target.

Per-target evidence is condensed to its truth, root-binding and generation status,
plus a digest covering the full predicate trace and exact anchor. Every selected
target remains in the result; there is no first-eight-target truncation. Use the
existing construction_progress diagnosis for the native IDs to inspect detailed
stages, jobs and item relationships. These observations do not verify an external
placement receipt, original footprint, native causality, usability or safety.
Native effect obligations are unchanged.

## Bounds and compatibility

A condition accepts at most 32 targets; a success/failure definition may reference
at most 64 targets across all furniture-set nodes. This admits a 32-target success
plus 32-target failure predicate but not arbitrary nested multiplication. Existing
64-node/depth-eight limits still apply to the outer condition tree. Input remains
bounded to 1,024 nodes and 32 KiB of conservative accounting. Evaluation still has
one million shared work units and the enclosing cooperative deadline. Large
rosters can exhaust this bound: the engine refuses instead of skipping targets.
Complete rendering must succeed before registration or progress is published.

The new op is added to discovered watch schemas, including condition inspection
and the existing registration paths. Old condition encodings are unchanged.
Stored definitions use the existing serde/checkpoint path; an older binary rejects
the unknown op rather than weakening it. This does not establish a tested migration
or power-loss campaign, and downgrading a journal with new definitions is unsupported.

## Evidence

Nine registered Rust test functions cover fixed-recipe equivalence with the
unchanged construction predicate fixture, full 32-target evaluation, item flags,
late generation changes, unknown roots/facts, removal, joint stability, one-slot
registration, denied authority and failed output publication. They have not been
compiled or executed here. No native code, dependency, production runner,
compatibility admission or game mutation is changed.

Beads: `df-action-coordinator-exec-ero.4`, `df-dfhack-bridge-plane-c-pic.3`.
