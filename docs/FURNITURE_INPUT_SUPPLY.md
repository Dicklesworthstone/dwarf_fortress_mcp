# Conserved input supply for finite furniture production

The adapter can qualify the conservative WOOD supply for one complete original
furniture request. It accounts for every requested bed, door, table and chair
together, plus an optional retained wood reserve. Two individually affordable
orders cannot each count the same log toward their joint requirement.

Owner: `dfmcp-reality-wiring-proof-4bf.14.4`, with combined execution under
`df-dfhack-bridge-plane-c-pic.4.1`. The machine contract is
`architecture/furniture_input_supply_v1.json`.

## The requirement is explicit and bounded

`FurnitureInputRequirements` accepts one to four distinct closed
`WorkOrderRecipe` values, each with a finite amount of 1 through 100. It
canonicalizes their order and retains all amounts. A wood reserve ranges from
zero through 1,000,000 stack units.

The policy `dfmcp.furniture-wood-input-requirement/1` requires one WOOD stack
unit per requested furniture unit, plus the reserve. This is an explicit host
input requirement. It is not a certified interpretation of DF's complete native
ingredient or job-requirement language. The native order writer selects wood
material categories; a supply assessment adds no native ingredient-cost claim.

For example, two beds, three doors and a reserve of four require nine eligible
WOOD stack units. A Created receipt for the bed order does not reduce that
requirement to seven: inserting an order has not consumed its inputs. The
complete original demand remains conserved until all original insertions have
been considered. The check does not create reservations against other game work.

## Supply uses the existing conservative allocation policy

Qualification reuses `conservative-unattached-stack-units/1`, the same bounded
inventory allocator used by production inspection. Its candidate supply has
positive stack sizes and excludes items with these observed conditions:

- removed, forbidden, rotten, trader-owned, marked for dumping or in a job;
- attached to an observed job;
- in an inventory or building, or held by an observed building;
- any of the same conditions inherited from an observed container.

The complete containment graph is checked for missing endpoints and cycles.
Job attachment counts must match the complete attachment roster. The allocator
assigns each eligible stack unit at most once across all orders and the reserve.
Its results include required, eligible, allocated and shortfall quantities,
assignments, and disjoint primary exclusion counts.

This policy does not require the `on_ground` bit. Its name and results therefore
do not assert direct-ground eligibility, reachability, proximity to a workshop,
hauling access, material appropriateness for every native filter, or production
feasibility. A complete observed shortage under this policy is a reason to refuse
the guarded request, not proof that every other strategy is impossible.

## Qualification has its own source boundary

`InputSupplySource` is constructed from independently selected Operations/1.3
world, site, DF version, DFHack version and source-generation pins. It cannot be
derived from a caller's `inventory_plan` JSON report by the combined host.

`InputSupplyAssessment::qualify` requires current Query and Observe authority,
the exact original canonical anchor, the same paused fortress tick, and a complete
typed capture from those pins. The legacy inventory baseline evidence policy
continues to cover only item type and stack quantity. It is not widened to grant
authority over availability flags, containment or job attachments.

The assessment borrows a raw Operations capture and builds no second canonical
world projection. Its 16 MiB work allowance covers the bounded raw eligibility
analysis and source commitment. The complete capture is limited to 4,096 records
(including its two source records), 8,192 total relationships, and the existing
ten-million-unit analysis work ceiling. Arithmetic is checked; incomplete,
malformed, wrong-source, stale, cancelled or over-budget inputs fail rather than
becoming zero supply.

The immutable witness binds the full source digest, independent source policy,
original canonical anchor, complete requirement and reserve, native generation,
job/building/item allocator seed, and allocation totals. A later assessment may
observe changed roster bytes, but it must retain the same original requirement,
source and paused anchor and cannot regress below the initial allocator seed.
The enclosing owner must also retain every higher frontier consumed by other
reads, including failed reads; the assessment is not a durable acknowledgement
of those later observations.

## Require the supply in the combined production host

Add this object to the existing `DFMCP_COMBINED_PRODUCTION_CONFIG`:

```json
{
  "input_supply": {
    "schema": "dfmcp.furniture-input-supply/1",
    "reserve_wood_stack_units": 4
  }
}
```

The optional reserve defaults to zero. Unknown fields, an explicit null object,
unsupported schema, negative reserve and reserves above the bound are refused.
The operator supplies no native item IDs, allocation claims or ingredient
coefficients. The host derives all demands from the actual original finite-order
amounts. It uses the existing independently pinned semantic work-order Operations
source and credential; no new native plugin, credential or public tool is added.

The policy applies to ordinary assignment-and-order requests, `existing_labor`,
`portfolio`, `existing_labor_portfolio`, `stock_target` and `stock_targets`, with
either stock-target labor-assignment mode. It composes with both the original
Manager and Workshop prerequisites. Omitting the object preserves the existing
request identities and behavior.

A positive original Plan needs a fresh complete supply assessment and an empty
native manager queue. Unclassified queued orders have unknown input costs and
cannot be credited as harmless. Stock-target sizing and input assessment share
one raw Operations census; the input check does not perform another full world
projection. Already-satisfied stock targets retain their independent no-work
result, without input qualification or an extra input queue read, including when
an input-only continuity fence remains visible.

The initial requirement, source policy, full capture commitment, quantities and
allocator seed enter original identity before intent IDs and step keys are
derived. A strict 400-hex-character `dfmcp-input-supply/1` marker resides in the
existing original-plan archive. No additional input sidecar is created. Only
input-configured compilers use a 2048-byte string limit for the complete marked
summary; existing archive limits and no-input compiler limits remain unchanged.

Every original order is checked again at its preparation and post-DispatchStarted
evidence boundaries. The fresh queue must be precisely the receipt-backed prefix
of original Created orders. Missing, retired, replaced or unrelated members
refuse insertion. Every original input demand remains in the allocation, even
after some of those orders have been inserted. Insert all original leaves at the
original paused tick before advancing the production clock. Each assignment,
order and clock still needs its own ordinary review and confirmation.

## Shortages, source continuity and recovery

The public `input_supply` view reports the original order amounts, reserve,
required stack units, binding commitment and complete checkpoint. A historical
assessment includes eligible and shortfall quantities plus disjoint exclusion
counts. Shortage explanations survive error and compact responses; a retained
successful assessment cannot become a fresh dispatch permit or current supply
claim. Neither result changes the original output goal.

Decoded source floors are recorded before later authority, budget or qualification
checks. Tick or job/building/item allocation regression and source mismatch fence
the owner. An ambiguous native connection/read failure also fences it, since the
source may have consumed a newer capture before rejecting its response. Shortage
alone does not erase the original or reduce the conserved demand.

Before an original exists, preserve the returned checkpoint as
`input_supply_checkpoint` in the parent configuration. It contains
`origin_witness`, `tick`, `generation`, `next_job_id`, `next_building_id`,
`next_item_id` and `fenced`. Its pre-plan witness binds the exact configured
policy, reserve and independent source pins. After original publication,
preserve the complete `recovery` object, including `input_supply_digest` and
`input_supply_floor`. That digest commits to the archived original marker, not
to a separate sidecar file.

Reopening checks the original source, full demand, reserve, marker and complete
floor. It cannot drop the policy, change the reserve, restore confirmation or
redispatch an uncertain original key. Offline reopening needs no input-source
credential and provides historical custody only. Unavailable status cannot
overwrite a previously retained complete checkpoint. Higher fresh-read floors
are caller-held: a newly opened owner cannot independently detect every rollback
of the entire external checkpoint beyond the sealed original seed.

The existing 8 GiB public default remains sufficient for four positive stock
targets with Manager, Workshop and input prerequisites and the three earlier
Created receipts. Each shared input guard reserves 16 MiB for assessment plus
1 MiB for its final control checks. The maximum combined dispatch custody
reservation is 3786 MiB, with separate allowances retained for the complete
receipt prefix, native frontier and public view.

## Evidence

Regression coverage includes shared shortages, reserves, all exclusion classes,
container and job inheritance, malformed relationships, independent source and
authority checks, bounded work, large exact stack totals, allocator regression,
and parity with the existing inspection allocator. Runtime/native-peer scenarios
also exercise all request routes, full portfolio demand, both durable markers,
queue-prefix changes, all-met requests, complete default-budget dispatch and
lost-reply Offline/reconciliation without redispatch. Executed results and exact
scope are recorded in `IMPLEMENTATION_STATUS.md`. Component evidence does not
qualify full MCP transport, a DFHack plugin build, a live fortress or admission.
