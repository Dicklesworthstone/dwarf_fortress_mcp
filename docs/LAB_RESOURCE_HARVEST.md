# Finite laboratory resource harvesting

The `starter_fortress_resources` scenario extends the finite workforce, construction-material and provisions models with explicit WOOD and PLANT sources. Agents acquire supplies, observe the resulting inventory, and then use the existing furniture, workshop and food-production planners.

`starter_fortress_multilevel_resources` retains those same entities, finite
sources and opening stocks while providing the existing five-level terrain
at z=8 through z=12. It supports the complete
[resource-aware room and paired-stair workflow](LAB_RESOURCE_ROOM_PROJECTS.md#connected-rooms-and-stairs)
through an ordinary built-in scenario. Earlier scenario snapshots are unchanged.

This is a deterministic reference economy. Resource units and timing are laboratory rules; the action has no native DFHack route or admission.

For an original WOOD/PLANT stock target, use the
[`resource_acquisition` blueprint](LAB_RESOURCE_ACQUISITION.md). It calculates
current deficits, allocates multiple finite sources and optionally proposes
explicitly permitted labor enables through these same actions. The direct
action form below remains available for an intentionally selected source.

## Request

Pass this array as the existing `fortress.plan` actions string:

```json
[
  {"action":{"kind":"harvest_resource","source":"8001","source_generation":1,"resource":"WOOD","amount":2}},
  {"action":{"kind":"harvest_resource","source":"8002","source_generation":1,"resource":"PLANT","amount":2}}
]
```

The closed action requires a canonical source ID, its exact positive generation, one resource token (`WOOD` or `PLANT`) and an amount from 1 through 100. Unknown fields, tokens and malformed identities are refused. The action is guarded, requires `designate`, and uses ordinary checkpoint policy. Observation and clock grants remain necessary to supervise and advance work.

The planner seals source identity, the unique stock ledger and its generation, finite amount, worker model, mandatory completion predicates and original deadline. Existing active demand and all requests in the new plan count against remaining source units. Planning reserves no source units and credits no inventory.

Harvest actions append canonical/archive tag 11. Earlier action tags and scenario bytes remain unchanged. The existing Actions plan source retains the request for replay and recovery.

## Source world

| Source | Generation | Resource | Initial units | Initially enabled worker |
|---|---:|---|---:|---|
| `8001` | 1 | `WOOD` | 32 | `1004`, `WOODCUTTING` |
| `8002` | 1 | `PLANT` | 40 | `1005`, `PLANT_GATHERING` |

Sources are canonical Plant records under `finite-harvest/1`. Complete eligible facts establish initial, remaining and harvested amounts. `initial = remaining + harvested` must hold, and harvested quantities must agree with retained task history. Missing, stale, future, asserted, unsupported or conflicting values cannot establish usable supply. The unique ledger must expose qualified WOOD and PLANT counters.

The model does not replenish its own sources. A separately qualified canonical source update may change declared initial/remaining supply while preserving generation and yielded history; existing tasks then stall or resume under their original amount and identity. No spatial route is inferred from these abstract resource records.

## Work and conservation

Each completed harvest unit consumes 50 eligible worker ticks, debits one source unit and credits one unit to the ordinary WOOD or PLANT stock field. Source debit, inventory credit and task progress settle together on the ordinary transaction shadow. Overflow, malformed history and exhausted budgets refuse publication.

Harvesting shares the finite living worker pool with mining, construction, cooking, brewing and carpentry. It requires the corresponding enabled labor and no workshop. One multi-skilled worker cannot serve two jobs simultaneously. New inputs affect subsequent causal intervals; they never grant production service for earlier blocked time.

Partial work creates no inventory. Waiting without a worker or usable source earns no service. Competing completions recheck finite supply in canonical order; a loser retains previously earned partial work. Successful different wait partitions preserve the same physical outcome.

## Observe, cancel and recover

Open and briefing responses expose bounded source/task rows under `harvesting`: exact source identities, remaining units, active demand, produced units, task status and earned service. Totals and explicit omitted counts distinguish the bounded display from the complete validation census. Action progress selects its original task from that complete census. Forecasts expose `predicted_harvesting` on a discarded world.

Cancellation stops future work and preserves completed yield and depletion. It refunds no materials. A retry of the same committed action retains its original identity. Checkpoint restore and durable reopening retain source depletion and task history. Recovered proof independently validates source/ledger generations and mandatory original postconditions. Unknown or malformed completed history remains unresolved physical work; losing an action handle does not restore dispatch authority.

## Acquire, then produce

Food, furniture and workshop planners still require observed current inputs. A harvest forecast or pending task cannot satisfy that requirement. After acquisition, observe the ledger and prepare the existing production or construction request against the new anchor.

## Low-input recovery advice

When a low FOOD or DRINK reserve lacks PLANT ingredients, observation can
propose one finite acquisition through the existing `fortress.plan` Actions
source. The quantity covers pending ingredient debt plus the minimum new
batches, less observed current PLANT stock, bounded by available source units
and the 100-unit action limit. It is an acquisition proposal, not a guarantee
that the subsequent reserve-production plan fits its complete service horizon.

The proposal requires a complete qualified source/task census, the same stock
ledger and generation as provisions, and a qualified living worker with
`PLANT_GATHERING` enabled. It prefers the largest useful bounded tranche and
the lowest source ID on ties. Unknown, depleted or unstaffed supply remains a
diagnostic blocker. Existing active PLANT work yields `awaiting_harvest` until
its output is observed. No pending work is credited as inventory.

The server's ordinary remedy qualifier parses the real Actions source and
checks current Plan authority, guarded Designate authority, Checkpoint,
source scope, expiry, anchor and budget. A proposal without that authority is
diagnostic rather than an executable recommendation.

Identical food/drink acquisition actions appear once in an observation packet.
The second alert retains its shortage and source evidence, identifies
`remedy_shared_with`, and has no duplicate action. Distinct requests remain
separate choices. After completed acquisition, the agent observes the ledger
and replans production against current stock, pending demand and consumption.

## Bounds and evidence

The complete model admits at most 65,536 canonical entities, 1,024 sources and 4,096 harvest tasks. Public requests retain their existing 16-KiB and 16-step bounds. Source scans and scheduling consume the ordinary effect advancement budget.

Regressions cover conservation, shared workers, wait partition equivalence, cancellation, generation substitution, strict requests, original proof, archive/reopen behavior and harvested inputs feeding actual reference food and furniture production. Exact executed scope is in `IMPLEMENTATION_STATUS.md`; reference tests establish no native behavior.

Contracts: `architecture/laboratory_harvest_v1.json` and `schemas/lab_harvest_action.schema.json`.
