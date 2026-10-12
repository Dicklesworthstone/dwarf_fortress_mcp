# Native shelter arrival status through fortress.query

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The existing unadmitted spatial/1.8 development MCP server accepts a
`shelter_status` query. It checks actual citizen positions in its last coherent
native capture, not the laboratory protection workflow's membership settings.
No bridge method, game mutation, extra MCP tool or production runner is added.

## Select citizens and inspect actual positions

Open the existing spatial/1.8 development session and inspect its `citizens`
query for canonical entity IDs and generations. Use those exact identities in
the same session. Do not use laboratory IDs such as `1001`, raw native IDs,
or an old session's generation numbers. Declare one or more inclusive shelter
cuboids within the observed fortress's map dimensions. Geometry is a desired
location, not a claim that the area is safe or registered as a native burrow.

Call `fortress.query` with the session ID and this object as its `query` argument
(the IDs below are synthetic examples, not a selection from your fortress):

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "shelter_status",
    "citizens": [
      {"entity_id": "3298534883338", "generation": 1},
      {"entity_id": "3298534883339", "generation": 1}
    ],
    "shelters": [{"min": [0, 0, 5], "max": [1, 1, 5]}],
    "limit": 16
  }
}
```

The checked-in envelope is `schemas/examples/shelter_status_v1.json`.
`fortress.query(mode="schema")` includes the closed query variant. Do not mix
`mode` with `query`. This is the native server's object-valued query argument,
not the laboratory's JSON-string `mode` interface. The archive-only branch and
its historical query vocabulary are not extended by this increment.

`arrival_truth` is the conjunction over the entire nonempty named selection.
A row is `inside`, `outside`, `not_observed`, `incarnation_changed`, `not_alive`
or `position_unknown`. All selected living incarnations must have known positions
inside the union to yield `true`. Any observed outside/dead target yields `false`;
otherwise unresolved targets yield `unknown`. Missing citizens are never dropped.
Children, babies and citizens unavailable for jobs remain in the selected goal.
Overlapping shelters cannot double-count a citizen.

Each response binds its complete aggregate counts to an exact anchor,
`source_digest`, bridge generation, `selection_digest` and `model_digest`.
`assessment_complete` means all named targets were classified, not that every
position is known. `complete_fortress_population_selected` remains false: the
query does not certify that the caller named everyone who needs protection.
`all_named_inside_at_capture` is positional truth at that observation, not
proof of a successful evacuation command, threat clearance or shelter safety.

## Whole rows, stable selection and explicit refresh

`limit` is 1..128. Rows are canonical entity-ID ordered, and the original
selection's aggregate is repeated unchanged on every page. A first page with
only inside citizens cannot hide an outside citizen on a later page.
`returned`, `total_rows`, `truncated` and `continuation` describe the actual page.

Copy a non-null `continuation` into the same query. Its `ss1` identity binds the
session, exact capture, full selected generations and geometry, policy and
`max_work`. Changing any of those invalidates the token. The page limit can be
adjusted without changing the underlying assessment. A page must fit its complete
summary and at least one whole row; insufficient output refuses instead of
returning a misleading empty success. The existing Agent Turn publisher reserves
its surrounding packet and does not silently shorten the query results.

A query does not read DFHack again. Call the existing `fortress.observe` to
acquire a new coherent capture, then start a fresh shelter query without the
old continuation. A citizen who leaves changes current arrival truth; an earlier
True result is not reused. Native source fencing and the existing Query authority
checks still apply before analysis. Neither the query nor its hashes grant
Observe, clock, Plan, ConfigureLogistics or any other effect capability.

## Bounds and evidence boundary

Selection ceilings are 128 citizens, 16 regions and 262,144 summed region tiles.
The native coordinate and observed map-dimension bounds both apply. The existing
bounded JSON shape validator runs before typed decoding. Unknown fields, invalid
IDs, zero generations, duplicate citizen IDs or cuboids and invalid bounds refuse.
Rust also checks cross-field and source invariants that JSON Schema cannot prove.

The optional `max_work` is 1..1,000,000; source inspection and target comparisons
consume it. Request decoding and whole-row output have their existing fixed shape
and byte bounds and share a shrinking cooperative wall deadline with analysis.
Only the bounded named-goal workspace is required. The shelter analysis adds no
full source re-encoding or canonical-world copy; existing transport and Agent
Turn projection costs remain. This analysis can therefore operate under the
native handler's narrowed result allowance.
Larger complete selections still require sufficient negotiated output/work bytes.

This is one sampled observation, not stable or continuous presence. No route,
terrain safety, native burrow definition, restriction assignment or causal
relationship to a previous mutation is proved. IDs/generations are scoped to
the current spatial session; durable cross-restart identity and goal tracking
remain separate work. Production compatibility and admission are unchanged.

## Validation for this increment

Seven Rust query/handler test functions were added, including the checked-in
example, complete-summary pagination, unknown and changed identities, continuation
invalidation, closed decoding, a real registered native handler with an injected
coherent source, explicit observation refresh, schema discovery and source fencing.
With the nine core cases, sixteen Rust test functions are present. They are
source-reviewed but not compiled or executed in this editing environment.

`python3 scripts/test_shelter_status_schema.py` passed the schema metaschema,
the checked-in example, nine valid optional/maximum-size cases and rejection
of 75 malformed structural cases. The maximum structural case names 128 citizens
and 16 regions. These checks execute JSON Schema, not the Rust decoder, native
codec, MCP server, source validation or game. No Rust/native/live qualification
is implied by these results.
