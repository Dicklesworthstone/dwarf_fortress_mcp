# Select furniture for a complete layout through MCP

The `furniture_allocation` query chooses distinct observed items for every target
in a requested bed/chair/table layout. It returns a complete
`dfmcp.furniture-plan/1` artifact, or explains which targets compete for too few
compatible items. Agents supply the layout and constraints instead of manually
selecting every native item ID.

This query is available through the existing `fortress.query` tool in the
operations/1.3, paged operations/1.4 and citizen-inclusive spatial/1.8 development
runtimes. Schema mode advertises the request. It uses the already published
coherent capture and requires current Query authority; it performs no additional
native read. The eleven top-level tools remain unchanged. The operations
runtime's stdio wire names use underscores: `fortress_open_session`,
`fortress_query`, and the other existing names.

## Request a layout

Open and observe the selected development profile as described in
`OPERATIONS_PAGING.md` or `LIVE_SPATIAL_CITIZENS.md`. Supply the following as the
`query` argument to `fortress.query`, with `session_id` supplied separately:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "furniture_allocation",
    "world_folder": "region1",
    "site": 2,
    "slots": [
      {"name": "bed", "kind": "bed", "target": [15, 15, 2]},
      {"name": "chair", "kind": "chair", "target": [18, 15, 2],
       "material": [419, -1], "max_distance": 100, "after": ["bed"]},
      {"name": "table", "kind": "table", "target": [20, 15, 2],
       "after": ["chair"]}
    ],
    "excluded_items": [123],
    "maximum_work": 10000000
  }
}
```

Fortress identity, material identifiers and coordinates above are illustrative.
Use the identity and material values from the actual published observation.
An optional top-level `expected_anchor` pins the query to a previously returned
canonical anchor. The request always checks its world folder and site against
the source, and every allocated or shortage result carries its exact source
digest and anchor.

Each slot has a unique ASCII name and target, a furniture kind and optional
constraints. `material` selects an exact type/index pair; `subtype` selects an
exact native subtype. Candidates must be on the target's z-level and within
`max_distance`, measured by horizontal Manhattan distance. The default distance
ceiling is 65,532. Dependencies express placement order; they do not prove
finished construction. Explicit exclusions remove named items from consideration.
Duplicate targets, cycles, unknown fields and invalid values are refused.

## Complete assignment and shortage results

An allocated result has `status="allocated"`, `model_feasible=true`, one
assignment per slot, `plan`, `plan_digest`, and `total_distance`. Assignments name
the exact native item together with its canonical entity ID, generation and
revision. Every target and dependency appears in the plan. The solver first
minimizes the total same-level distance, then resolves ties by the item-ID vector
in lexical slot-name order. A general slot cannot greedily consume the only
material-compatible item needed by a more constrained slot.

An infeasible result has `status="shortage"`, `model_feasible=false`, null plan
and plan digest, and no assignments. Its shortage names a group of competing
slots, **all** candidate items compatible with that group and the number missing.
The summary still gives the maximum number of assignable slots. Even if some
targets could be filled, the response cannot silently omit the others and turn
that subset into an executable plan.

Both outcomes include the normalized request and its digest, complete compatible
counts per slot, disjoint inventory classification counts, and an analysis digest
bound to the session, source and selection policy. Request and plan digests match
the existing Python codecs, including ASCII escaping of non-ASCII world folders.
Input row order does not change the request, plan or selection objective.

## Candidate policy and placement handoff

The conservative `direct-ground-unattached-singleton-furniture/1` policy selects
supported singleton furniture directly on the ground, with known material,
bounded coordinates, no container/building holder, no observed contained items
and no actual job attachment.
Only the ground bit may be set among the observed flags. Actual attachment
relations exclude an item even if its raw in-job flag is clear. All other observed
items remain accounted for under a primary exclusion reason.

The query preserves the enclosing source and canonical generations. An item that
disappears and later reappears cannot retain a stale generation simply because
its native integer ID is reused. When operations are embedded in spatial/1.8,
the result keeps that combined capture's anchor and source identity.

Save the complete returned `plan` object for the existing exact furnishing-batch
workflow described in `FURNITURE_BATCHES.md`, retaining the allocation's
source and item evidence for review. The artifact is accepted by that workflow's
existing plan compiler and has the same digest. Each placement still requires a
fresh exact observation, review and the separately configured furniture/1.19
effect session. Once the
original batch has registered every placement, `FURNITURE_COMPLETION.md` describes
monitoring the complete original layout.

Use the returned fortress identity when opening the placement workflow. Material,
subtype and distance constraints are evaluated at allocation time; the existing
exact-item plan format does not retain them as future mutation preconditions.
Review each later native placement against the retained request and evidence.

The allocation creates no reservation or prepared MCP effect. Its candidate
policy does not establish target terrain/map bounds, quality/wear, unobserved
native flags, route access, worker eligibility, continued availability or
successful construction. A shortage applies to this observed candidate subset.
All exposed runtimes remain explicitly unadmitted development profiles.

## Whole-result budgets

The request accepts 1..32 slots and at most 4,096 excluded IDs; its complete
normalized artifact must fit 16 KiB. The source must fit the current complete
entity allowance and the profile's item/attachment ceilings. One shared work
allowance covers inventory reading, normalization, matching and evidence
collection; the optional `maximum_work` can lower the 10,000,000-unit ceiling.
Current authority, cancellation and the call's wall allowance remain effective.

The full assignment or shortage must fit the byte/token budget left after the
required Agent Turn and active work are reserved. `limit` and `continuation` are
not accepted for this query. Insufficient output budget produces an explicit
refusal without a partial plan. A smaller layout or a larger permitted output
budget is required before retrying the read-only analysis.

## Verification scope

The solver is checked against an independent exhaustive matching/cost oracle,
including all 4,096 three-slot/four-item graphs, unpruned inventories, maximum-width
IDs, interrupted work and the 65,536-item/32-slot boundary. Separate projection
tests cover all observed flag words, real attachment relations, identity reuse,
fortress/anchor/authority restrictions and the enclosing spatial source. MCP tests
exercise complete plans, shortage evidence, canonical digest parity, schema
discovery, registered sessions, active-work budgeting and source fencing.

The actual paged operations server passes five modern MCP stdio/TCP scenarios.
The 32-slot case reads a 2,000-item source in six verified native pages and returns
the complete 18,630-byte Agent Turn. The same layout under an 8,192-byte output
allowance refuses without a partial plan, and a one-slot retry succeeds from the
same published capture. Run this suite against the built development binary:

```bash
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 scripts/test_furniture_allocation_mcp.py \
  --binary target/debug/dfmcp-live-operations-paged-dev-server -v
```

Exact commands, source identities and measured results are recorded in
`docs/evidence/furniture-allocation-mcp.json`.

These are development checks over explicit observations and protocol peers.
They do not establish a real DFHack SDK build, a live fortress campaign, full
workspace qualification or production admission. The broader owning beads
`df-dfhack-bridge-plane-c-pic.3` and `.4` remain open.
