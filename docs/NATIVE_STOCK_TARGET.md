# Finite native stock targets

Owner: `df-native-stock-target-3m9` (WP-05/WP-13/WP-21).

The native furniture path can calculate a finite order from independently
qualified current inventory. The reusable adapter entry is
`InventoryBaselineProjection::stock_target`. This is an explicitly bounded
stock-sizing policy, not a background stock-maintenance service or a native
recipe-feasibility proof. The combined MCP consumer exposes `kind="stock_target"` through the existing
Plan tool and additive `kind="stock_targets"` for a bounded complete target set;
see [COMBINED_PRODUCTION_MCP.md](COMBINED_PRODUCTION_MCP.md) for the
complete request, no-work result, native guards and recovery flow.

## Qualified sizing

The caller supplies a closed furniture recipe, a minimum of 1 through 1,000,000
observed stack units, and a maximum finite order of 1 through 100. The count
comes from the complete operations/1.3 Item census at the exact paused canonical
fortress tick. Query and Observe authority, a complete Item-domain policy and
independently accepted type/stack fields are required. An omitted field or
incomplete domain cannot become zero stock.

The deficit is `max(target - observed, 0)`. A zero deficit returns a typed
already-satisfied assessment with no action or positive-growth objective. A
positive deficit above the selected maximum refuses the whole proposal; it is
never silently reduced to the limit. An eligible deficit produces a one-unit
per-furniture-order sizing proposal and the existing typed inventory-increase
objective with that exact baseline and deficit. Its terminal target is the
requested minimum. The policy does not certify recipe output, inputs, material,
ownership, usability, access, available labor, workshop suitability or eventual
production. The actual inventory goal must still be observed independently.

For a positive deficit, a currently observed matching furniture job refuses
the proposal. A CustomReaction job or nonempty reaction also refuses because
its output is not classified by this bounded policy. Suspended and repeating
jobs are not ignored. None of these jobs is credited as future inventory.
Existing inventory that already meets the target needs no insertion and does
not require a no-competing-job claim.

## Preserve the original basis

The immutable assessment binds recipe, requested target, order limit, qualified
count, deficit, complete canonical anchor, operations source digest, native
generation, allocation counters and item-row count. Its canonical bytes and
witness are deterministic. Changing any of those inputs changes the assessment.
The existing inventory-increase predicate retains the source-qualified baseline
without changing its established encoding.

Revalidation independently qualifies a new complete projection and requires
the exact original paused census and assessment. This deliberately uses a
coarse witness: even an unrelated change in that operations capture can require
a new original plan. Revalidation cannot update the count, shrink or enlarge an
already sealed amount, move the target or renew the deadline.

The complete manager-order membership observation is a separate source.
Protocol 1.10 supplies no recipe or remaining-quantity detail for those rows.
The single-target combined path therefore requires its queue to be empty before
stock-target preparation and insertion. The vector path permits only the exact
receipt-backed prefix of earlier original orders from that same sealed plan.
Neither assumes that an unclassified existing order is unrelated, subtracts
expected future output, or changes protocol 1.10.

## Explicit labor prerequisite

The combined consumer defaults to observed already-enabled workers. A closed
`assign_labor=true` option composes the same qualified stock assessment with
the existing reviewed labor-enable and dependent-order executor. It uses a
separate `Stock target/2` original identity; the `Stock target/1` existing-worker
encoding remains unchanged. The assignment action and complete original
citizen preconditions are sealed with the inventory target, finite deficit and
timing. Each native step requires its own review and confirmation.

A met target never creates the assignment, even when requested. A positive
target requires current labor authority through the ordinary planner. Historical
assignment evidence releases the dependency but cannot replace current source
checks or the inventory goal. Stock and queue drift after labor changes refuses
the order under its original identity, without a new count or replacement key.
The complete parent monitor still checks current labor and stock together.

## A bounded original target portfolio

The `stock_targets` consumer assesses one to four distinct furniture recipes
against a single independently qualified census. It refuses the entire proposal
if any positive deficit is over its limit, creates no original when every target
is met, and emits original orders only for positive deficits. All requested
minima remain explicit complete-parent requirements, including already-met stock.
The selected eligible workers must already have the requested labor enabled.

The separate `Inventory targets/1` archive identity binds the common census,
all target assessments, workers and unchanged timing. Positive leaves retain
typed inventory-increase objectives; distinct complete-parent minimum predicates
preserve every target without fabricating an order. Before each real insertion,
the complete census must still match and the native queue must contain exactly
the prior original receipt-backed insertion prefix.

Current complete-parent proof requires independently qualified quantities for
all targets and the original cadence/stability across complete cohorts. A leaf's
historical success cannot supply the missing stability of an initially met target
that was later consumed. An unfinished cohort streak resets on False, Unknown or
failure; a completed in-process achievement remains historical while current
truth determines current proof. Reopening restores no cohort achievement or new
deadline. Original per-order recovery, read-only custody and independently
authorized clocks retain their normal boundaries.

## Evidence boundary

The assessment itself observes no source, persists no effect, inserts no order
and grants no mutation or clock authority. Its positive result is source-bound
planning data. The host must preserve it in the original identity and perform
fresh checks at the real native boundaries. A queue insertion receipt remains
distinct from current inventory truth and from a complete original goal proof.
The existing registry and production runner map are unchanged.

The machine contract is
[native_stock_target_v1.json](../architecture/native_stock_target_v1.json).
Executed checks and current-source limitations are recorded in
`IMPLEMENTATION_STATUS.md`; test-source presence does not imply execution.

## Focused acceptance source

The `stock_target_request` and `stock_target_binding` test modules cover
explicit opt-in, exact assignment/order layout, original identities, archive
round trips, bounded deficits and rejection of weakened recovery predicates.
The vector request/binding/runtime and presentation sources additionally cover
whole-proposal bounds, no-action minima, exact original queue prefixes, lost-reply
recovery and whole-parent temporal stability. These new sources are unexecuted
while the execution workspace is unavailable.

The runtime suite uses the real combined Runtime and native clients against
the existing TCP fixtures. Its assignment campaigns cover eleven cases,
including independent step confirmation, zero work, missing current labor
authority, action budgets, source drift after assignment and original-key
lost-reply recovery without redispatch. Presentation tests retain assignment
intent and actual StepId through compaction and overflow while withdrawing
current proof and review.

Run the focused suite in a working latest-nightly environment with the locked
dependencies:

```bash
cargo +nightly test --locked --offline -p dfmcp-mcp --lib stock_target -- --nocapture
```

These are regression sources awaiting execution for this increment. They are
not real-game production evidence; see the exact execution limits in
`IMPLEMENTATION_STATUS.md`.
