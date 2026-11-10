# Produce from finite resources without losing the original goal

The laboratory can retain a production objective while acquiring the raw
materials it needs. Add `acquire_resources` to the existing production request:

```json
{
  "template": "production",
  "quotas": [{"item": "BED", "minimum": 22}],
  "acquire_resources": {"assign_labor": false}
}
```

Pass this object as the `fortress.plan` **blueprint JSON string**. Use the
`starter_fortress_resources` scenario for the complete finite WOOD/PLANT model.
The output goal remains `BED >= 22` throughout the workflow. The planner can
propose gathering inputs, but gathered wood alone does not prove that goal.

This is deterministic laboratory behavior. It does not provide a new native
DFHack route, a live production controller or compatibility admission.

## Review one stage at a time

The request selects the fixed `resource_aware_v1` compiler. It reads current
qualified stock, recipes, workshop sites, workers and existing physical work,
then chooses one stage:

| `production.resource_workflow.stage` | Proposed work | What happens afterward |
| --- | --- | --- |
| `acquire_resources` | Finite WOOD/PLANT harvests and explicitly permitted harvest-labor enables. | Observe completed inputs and request `continue_goal`. |
| `production` | Ordinary finite production orders and explicitly permitted production setup. | Observe actions, physical work and the original output goal. |

The starter resource scenario has twenty logs and no beds. The example first
proposes acquiring two logs. It does not queue a bed order that could spend
those predicted logs. Review the returned steps and commit the exact plan
digest, then use bounded `fortress.wait` calls to advance and observe work.

After the acquisition is physically quiet, submit:

```json
{
  "template": "continue_goal",
  "plan_digest": "<the retained acquisition-stage plan digest>"
}
```

The server retrieves the original production request. At the new source anchor,
it recompiles the outstanding output quotas using actual observed inventory.
Review and separately commit the new plan. If stock is now sufficient, this
plan produces the beds. If interference or cancellation left a shortage, the
next stage acquires only the currently required residual.

The original output request is never replaced by an inferred recipe or a raw
stock goal. `continue_goal` cannot accept replacement quotas, permissions,
source IDs, parent/root assertions or completion claims. When the original
outputs already hold, no new stage or labor change is proposed.

## Budget all shared inputs and the gathering delay

The diagnostic compiler calculates the complete production program before
deciding whether acquisition is needed. It accounts for:

- raw input demand from every requested output, including shared FOOD/DRINK
  ingredients and the common WOOD supply for furniture;
- remaining demand from existing physical production;
- actual deduplicated workshop construction costs, when the original request
  explicitly permits those sites;
- known queued work, shared workers and the production/setup schedule;
- population consumption during both gathering and the later production
  horizon.

The compiler iterates consumption allowances and required supply monotonically,
with at most 64 rounds. A longer acquisition can require more FOOD/DRINK reserve,
which can require more PLANT and more gathering. Nonconvergence, arithmetic
overflow or an excessive combined horizon refuses the whole proposal.

This analysis uses typed recipe requirements and a diagnostic schedule. It
does not change source inventory to make an unfunded production plan appear
valid. The returned acquisition plan contains finite harvest actions and any
explicitly permitted harvest-labor enables. Production orders and workshop
construction are deferred to a separately reviewed production stage, which must
pass the existing strict funding checks at its own observed anchor. The combined
prediction does not extend the acquisition actions' own deadlines.

Insufficient finite sources, unknown stock, unqualified workers, invalid
workshop sites or missing setup permission remain refusals. Gathering is not
proposed as a remedy for an unrelated infeasible production request. Existing
acquisition overlap rules also apply: unresolved harvests and active consumers
of the requested inputs must be observed or explicitly drained before another
acquisition is prepared. Future work never counts as available inventory.

## Keep permissions separate

`acquire_resources.assign_labor` defaults to false. Setting it to true permits
the proposal of a normal `SetLabor` action when harvesting needs an explicitly
disabled worker. It grants no capability. It is independent of the existing
`prerequisites.assign_labor`, which controls production and setup staffing.

For example:

```json
{
  "template": "production",
  "quotas": [
    {"item": "DRINK", "minimum": 100},
    {"item": "FOOD", "minimum": 100}
  ],
  "acquire_resources": {"assign_labor": true},
  "prerequisites": {
    "assign_labor": true,
    "workforce": "complete_project_v1"
  }
}
```

Each proposal and its commit require current Observe, Query and Plan authority
and allowance for the complete source. Actual actions retain their usual
Designate, Checkpoint, ConfigureLabor, ConfigureProduction and, where needed,
construction requirements. Clock advancement has its own current authority.
An enabled policy with insufficient grants remains a refusal.

The existing `orders: "dependency_preserving_v1"` and explicit workshop sites
remain part of the original source. Unused permissions and sites are sealed too;
equivalent action arrays do not erase differences in the original request.

`resource_aware_v1` deliberately rejects `reserve_until_tick`. To combine finite
acquisition with an explicit future tick, submit both fields without naming
the older compiler. New intake selects the additive
[`resource_reserve_v1` workflow](LAB_RESOURCE_PRODUCTION_RESERVES.md), retaining
the same absolute tick through continuation `/5`. Historical production
requests without `acquire_resources` keep their existing compiler selection,
source bytes and strict shortage behavior.

## Cancellation, shared sessions and durable recovery

Cancellation stops future harvesting and preserves completed yield. Continuing
the original production goal recomputes the needed inputs from the current
stock, rather than repeating the original harvest amount. A canceled original
harvest's exact proof stays canceled even if a later child produces every
requested output. Current semantic output truth is reported separately, so a
completed child suppresses redundant gathering without rewriting history.
That truth remains bound to the original stock ledger's ID and generation.
A replacement ledger cannot satisfy or continue the old objective. Durable
recovery reconstructs this binding from the original sealed snapshot.

Every retained pursuit sharing the same root must be physically quiet. Active,
deferred or unknown sibling work blocks another stage. Commit repeats the
current eligibility checks, so two sessions cannot dispatch independently
reviewed stages for the same active root.

New continuations use the flat `dfmcp.production-continuation/4` source. It
preserves the complete original request, immediate parent and root digest in
the existing durable `production_continuation` kind. Historical `/1`, `/2` and
`/3` retain their original generations and bytes and cannot import acquisition
permission. Exact source replay reconstructs the same reviewed stage from its
original snapshot.

Verified durable reopen retains the original objective and source while
requiring fresh grants and observed continuity. It does not dispatch old work.
Checkpoint restore abandons the old pursuits; arbitrary epoch changes cannot
make them eligible again.

## Source changes and complete publication

A proposal is tied to one complete source anchor. If that anchor changes before
commit, the server keeps the reviewed pending plan and returns
`fresh_resource_production_required` with an explicit fresh Plan request. Root
plans return the original production request; continuations return the public
`continue_goal` request for their retained parent. Observation alone cannot make
the stale proposal commit-ready.

Planning holds the session/world owner through complete Agent Turn publication.
The analysis, current work and query catalogs must fit before pending state is
replaced. Output, source, authority, no-work and deadline refusals preserve the
previous pending plan. Full step proof appears once in top-level `steps`; the
pending catalog uses a checked same-response `steps_reference` and count.
The complete component material budget appears at
`production.resource_workflow.requirements`; source provenance, combined
horizon, iteration count and consumption assumptions appear at
`production.consumption`. This generation keeps those values once so the
ordinary continuation can retain its complete review within the default budget.

Candidate comparison and selection use the same fixed source compiler. Their
forecasts execute only the current reviewed stage on discarded worlds. A
forecast that finishes harvesting while the bed quota is still false is not a
prediction of completed production, and comparison cannot dispatch the next
stage.

The decoded request schema is
[`lab_resource_production.schema.json`](../schemas/lab_resource_production.schema.json),
and the machine contract is
[`laboratory_resource_production_v1.json`](../architecture/laboratory_resource_production_v1.json).
Owning bead: `df-lab-resource-harvest-h2m.3`. Current execution evidence and
qualification limits are recorded in `IMPLEMENTATION_STATUS.md`.
