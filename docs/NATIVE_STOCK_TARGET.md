# Finite native stock targets

Owner: `df-native-stock-target-3m9` (WP-05/WP-13/WP-21).

The native furniture path can calculate a finite order from independently
qualified current inventory. The reusable adapter entry is
`InventoryBaselineProjection::stock_target`. This is an explicitly bounded
stock-sizing policy, not a background stock-maintenance service or a native
recipe-feasibility proof. Combined MCP integration is a separate functional increment; its public flow
is tracked in [COMBINED_PRODUCTION_MCP.md](COMBINED_PRODUCTION_MCP.md).

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
The combined host therefore requires its queue to be empty before stock-target
preparation and insertion. It does not assume that an unclassified existing
order is unrelated, subtract expected future output, or change protocol 1.10.

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
