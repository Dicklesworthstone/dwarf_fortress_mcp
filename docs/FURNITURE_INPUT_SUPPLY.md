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

## Execution integration and evidence

This adapter checkpoint supplies the reusable qualified assessment. Wiring it
into the combined host's original identity, queue-prefix checks, effect
boundaries and recovery is a separate consumer increment. The adapter does not
insert an order, grant ConfigureProduction, create a current-world fact or prove
that the original output goal has been achieved.

Regression coverage includes shared shortages, reserves, all exclusion classes,
container and job inheritance, malformed relationships, independent source and
authority checks, bounded work, large exact stack totals, allocator regression,
and parity with the existing inspection allocator. Executed results and exact
scope are recorded in `IMPLEMENTATION_STATUS.md`. Component evidence does not
qualify full MCP transport, a DFHack plugin build, a live fortress or admission.
