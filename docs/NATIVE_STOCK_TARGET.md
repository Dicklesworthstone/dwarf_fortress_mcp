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
if any positive deficit is over its limit, creates no effect-bearing original
when every target is met, and emits original orders only for positive deficits. A
wholly met request instead retains the read-only original portfolio described
below. All requested minima remain explicit complete-parent requirements,
including already-met stock.
The selected eligible workers must already have the requested labor enabled by
default. Explicit `assign_labor=true` instead creates one reviewed enable at
StepId zero and one to four dependent orders at StepIds one through four. A wholly
met vector creates neither assignment nor order, even with the opt-in.

The separate `Inventory targets/1` archive identity binds the common census,
all target assessments, workers and unchanged timing. The assignment variant
uses `Inventory targets/2` and a separate identity domain, binding the exact
assignment action and original citizen identity guards. Omitted/false assignment
keeps `/1` bytes unchanged. Positive leaves retain
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
authorized clocks retain their normal boundaries. Assignment is not a stability
sample: the complete-parent tracker reads timing only from the original positive
order obligations, and clock requests use the actual order StepIds.

### First project portfolio after restarting before publication

The fixed resource-project owner can reopen its pre-plan custody and then
publish its first stock portfolio with either or both configured clock families.
Its immutable production intent names every additional fixed clock identity
before the parent acknowledges it. Only that successful current publication
can create those exact new headers, using independent current and opening
clock authority and exclusive file creation. Refused acknowledgment writes no
original or additional clock; an existing or partially created slot cannot be
adopted or repaired.

Retained-original recovery continues to require every previously acknowledged
journal and exact identity. It receives no new-slot creation path. A one-order
portfolio reuses the existing first clock owner. See
[NATIVE_RESOURCE_PROJECT_MCP.md](NATIVE_RESOURCE_PROJECT_MCP.md) for the
publication and recovery contract and the current validation limits.

## Read-only monitoring when no order is needed

The reusable `StockTargetWatch` adapter accepts only an independently qualified
already-satisfied `StockTargetAssessment`. It fixes the original target,
source lineage, absolute deadline, cadence and required sample count. It
creates no order, labor assignment, preparation, clock run or effect journal.

The planning census is historical sizing evidence and contributes zero
stability samples. Later `InventoryBaselineProjection` captures require fresh
independent Item-domain and field policy. Complete empty stock is zero; partial
or unqualified stock remains Unknown. Changed stock is evaluated as a minimum,
without applying order-size limits or rejecting unrelated production jobs.

The first eligible sample is at least one original cadence after the initial
census. Repeated ticks and exact replays cannot increase stability. False,
Unknown, acquisition/publication failure and cursor gaps interrupt an
unfinished streak. A missed absolute deadline cannot be renewed. Source
generation, fortress/restore identity, cursor, tick or allocation regression
fences the owner. Decoded rejection floors survive later policy refusal.

Each result separates current truth from the first timely achievement, names
its complete anchor and source digest, and carries a deterministic witness.
Historical achievement cannot make a later false or missing current count
true. This monitor is process-local: reopening restores neither an unfinished
streak nor current proof. It provides no recurring maintenance, material or
usability guarantee, and no causal attribution to production.

The combined single-target consumer now retains this watch after a no-work
Plan. `fortress_wait` or a default `fortress_observe` performs one bounded fresh
citizen/operations observation and independent qualification. The returned
`no_work_stock_goal` reports the exact source, anchor, count, cadence progress,
timely achievement and current proof separately from the unchanged historical
`stock_target` assessment. Query does not acquire another sample. A repeat of
the same Plan retains its timing and progress; changed planning is refused until
the original watch is explicitly cancelled.

This path never dispatches an order, labor change or clock run. The game must
reach later observed ticks for spaced samples to accumulate. Current proof
withdraws after failed publication, cancellation, an unsettled clock, or
contradictory/missing evidence. A later qualified count can restore current
truth after a timely historical achievement. Recovery output carries the
original goal identity with no current sample or proof. The resource-project
parent consumes this typed result, so an initially met single stock target can
complete under its original deadline without inventing production.

### Keep every already-met target stable together

The reusable `StockTargetsWatch` accepts one through four distinct, already-met
assessments from the same complete census. Recipe ordering is canonical. Its
immutable witness binds every original assessment and the one shared deadline,
cadence and required sample count. Each sample qualifies every requested minimum
against one `InventoryBaselineProjection`, with one exact anchor, source digest,
generation and allocation horizon. A missing or failed qualification cannot
publish a partially successful target set.

The portfolio owns one joint stability streak. All targets must be True together
in each eligible cohort; independent achievements at different ticks cannot be
combined. For example, two good bed samples followed by two good door samples
do not establish a two-sample portfolio while either stock was missing from the
other samples. The original planning census contributes zero samples. False,
Unknown, cursor gaps and failed acquisition or publication interrupt unfinished
joint stability. Repeated ticks do not advance it. A timely achievement remains
historical, but current proof additionally requires a complete current cohort
in which every original minimum is True. Missing the original deadline cannot
be repaired by repeating Plan or replacing an individual target.

An all-met `kind="stock_targets"` Plan retains this monitor through the existing
`no_work_stock_goal` field, tagged with schema `dfmcp.stock-targets-watch/1` and
scope `original_no_work_inventory_portfolio`. Its `targets` preserve every
original minimum; `sample.targets` carry the independently qualified current
counts or null and each comparison truth. The sample also carries the joint
truth, stability count, exact source and timely achievement. The separate
`stock_targets` assessment remains historical and does not itself prove current
inventory. This inventory-only goal does not establish a workforce prerequisite,
including when the original request used `assign_labor=true`.

Wait and default Observe acquire one complete current portfolio cohort per call.
Query inspects the retained result without native I/O. Repeating the identical
Plan preserves the original target set, deadline and progress; changing a target,
timing field or request mode requires explicit cancellation first. Use the
existing Cancel with its default scope or `scope="step"`; a replacement request
must name a valid retained observation, so Observe again when that selection
has been consumed by sampling. Cancellation and failed publication
withdraw the complete current proof. Decoded native and canonical rejection
floors survive later authority, policy or budget failures, and source or identity
regression fences the complete watch. Recovery output preserves original target
identity without a current sample; restarting restores no joint achievement,
unfinished streak or current proof.

This path creates no insertion keys, effect-bearing original, native preparation,
labor assignment, clock run or goal-history sidecar. It cannot replace consumed
stock with a new order. The root resource-project request remains single-target;
the portfolio monitor is available through the combined production requests.
The adapter sources contain eighteen portfolio regression cases alongside the
fourteen single-target cases. Runtime/native-peer and presentation coverage
address the public sampling, cancellation and proof-withdrawal boundaries.
Execution evidence for these bytes belongs in `IMPLEMENTATION_STATUS.md`;
source presence and authored tests add no qualification or admission.

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
recovery and whole-parent temporal stability. The assignment vector adds exact
StepId offsets, explicit authority and full five-action budget coverage.

The runtime suite uses the real combined Runtime and native clients against
the existing TCP fixtures. The single-target assignment campaigns cover eleven cases,
including independent step confirmation, zero work, missing current labor
authority, action budgets, source drift after assignment and original-key
lost-reply recovery without redispatch. Presentation tests retain assignment
intent and actual StepId through compaction and overflow while withdrawing
current proof and review. The vector campaigns cover eleven existing-worker
cases and fourteen assigned-worker cases, including two and four actual orders,
exact prior-order queue prefixes, no-action target loss and independent clocks.

Run the focused suite in a working latest-nightly environment with the locked
dependencies:

```bash
cargo +nightly test --locked --offline -p dfmcp-mcp --lib stock_target -- --nocapture
```

The previously recorded focused component run passed 78 live-production
adapter tests and 418 Runtime/presentation tests, including 66 presentation tests
and all 25 vector Runtime/native-peer cases. Those historical results do not
validate the subsequently added no-work watch and consumer. Full MCP build/check is still
memory-blocked in the unchanged pinned runtime dependency; component execution
excludes MCP transport/supervision and admission. Execution details and full
qualification limits are recorded in `IMPLEMENTATION_STATUS.md`. Deterministic native-peer results do not establish
real-game production or production admission.
