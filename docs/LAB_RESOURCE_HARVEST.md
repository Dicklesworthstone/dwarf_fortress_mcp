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

When observed FOOD or DRINK falls below four consumption rounds for the known
living population, observation can propose one `production` blueprint through
the existing `fortress.plan` tool. All currently known deficient output quotas
share that source. A joint request budgets their common PLANT supply together;
an isolated labor or harvest action no longer replaces the original output goal.

When necessary ingredients are short, the request retains an explicit
`acquire_resources` policy. The ordinary resource-aware compiler sizes the
complete gathering and production program, including consumption during both
stages. A known disabled gathering or production worker can be included through
the corresponding explicit labor-assignment policy. Unknown labor cannot be
treated as disabled. Existing qualified workshops are required: this advice
does not invent a construction site.

Acquisition requires complete qualified source/task and provisions censuses,
the same stock-ledger identity and generation, and sufficient finite source
capacity. Active harvests or competing ingredient consumers remain blockers;
their future output is never credited as inventory. The retained harvesting
diagnostic may describe one bounded tranche, but only the complete production
compiler determines whether the whole proposed project is feasible.

The server lowers the real production source before marking it available. It
checks the complete source record/byte allowance, current Observe and Plan
authority, Query authority for resource-aware analysis, and each current-stage
effect's actual scope, risk, expiry and checkpoint requirement. All remedies in
one observation share the wall-time allowance. Availability is explicitly
`current_reviewed_stage_only`; it does not authorize or guarantee later stages.

The first deficient supply alert carries the joint remedy. A second affected
alert preserves its hazard and evidence, names `remedy_shared_with`, and carries
no duplicate request. The recommendation retains `goal_scope`, `goal_alerts`
and `separate_stage_review_required` so a fresh agent can see the full purpose.
After committing and observing acquisition, explicitly review `continue_goal`
for the retained plan, then separately commit production. That continuation
preserves every original output quota and the parent/root lineage. Gathered
plants alone never establish the original FOOD or DRINK goal.

If a compatible retained FOOD/DRINK pursuit already exists, the next supply
recommendation carries its exact `continue_goal` request instead of creating
another production root. The newest retained pursuit is selected by its sealed
frontier and then digest. Every currently deficient quota must be covered by the
original minimum; additional original FOOD/DRINK quotas remain binding even if
their immediate alerts have disappeared. Acquisition, staffing, workshop-site
and reserve policies are copied from that original, never from the new alert.
The `retained_goal` metadata exposes the parent/root identity and full original
request in both the remedy and Agent Turn recommendation.

Selection admits the complete retained source and plan archives plus current
world census under one inspection allowance. The ordinary continuation gate
still requires quiet work, known original-goal truth, valid lineage and current
source authority, including Query for resource or reserve analysis. A refusal
for the selected original makes advice unavailable; it cannot fall through to
an older pursuit or a new root. Explicit withdrawal or restore abandonment
excludes that exact pursuit. Verified recovered goals may remain eligible
without restoring old action handles. Each proposed stage still needs its own
Plan review, current effect grants and explicit Commit.

## Bounds and evidence

The complete model admits at most 65,536 canonical entities, 1,024 sources and 4,096 harvest tasks. Public requests retain their existing 16-KiB and 16-step bounds. Source scans and scheduling consume the ordinary effect advancement budget.

Regressions cover conservation, shared workers, wait partition equivalence, cancellation, generation substitution, strict requests, original proof, archive/reopen behavior and harvested inputs feeding actual reference food and furniture production. Exact executed scope is in `IMPLEMENTATION_STATUS.md`; reference tests establish no native behavior.

Contracts: `architecture/laboratory_harvest_v1.json` and `schemas/lab_harvest_action.schema.json`.
