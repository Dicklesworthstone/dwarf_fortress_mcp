# Evidence-bound semantic live routing

Owner: `df-dfhack-bridge-plane-c-pic.4`.

`dfmcp-adapter::live_routing` describes which separately versioned development
family could implement each semantic step. Its routes are advisory. A successful
`route_plan` or `PlanRoute::fully_routable()` means every step has a family route;
it does not establish execution readiness, native admission, source authority,
capabilities, preparation, or goal completion.

The semantic plan remains the source of its action, canonical anchor, predicates,
idempotency key, and temporal obligation. No generic cross-family execution
coordinator is introduced. Existing guarded family sessions remain responsible
for native preparation and confirmation, capability checks, durable journals,
revalidation and recovery.

## Canonical identities remain canonical

`LiveRequest::WorkDetail` carries `Vec<EntityId>`, the requested `labor`, and the
requested assignment state. Callers must serialize these IDs as canonical IDs.
They must not pass their numeric representation to the native workforce service.
V1 citizen projection IDs encode native ID plus one; spatial/1.8 citizens occupy
a separate namespace above the native integer range. Native unit zero is valid
and maps to canonical V1 ID one. Guessing from an ID's size or casting it changes
which citizen the action addresses.

`LiveRoutingEvidence::citizens_v1` requires the exact typed projection, its
validated source capsule and independently issued `PredicateEvidence` for that
canonical snapshot. `LiveRoutingEvidence::spatial_v1_8` requires the currently
published citizen-inclusive spatial state and its exact scoped snapshot. These
constructors bind the identity schema, fortress, bridge generation, source
digest, and anchor. They do not issue source grants or admit a bridge profile.
The observing shell must establish compatibility and issue the fact/domain scope
independently; importing a profile string, canonical hash or observation-shaped
bytes is insufficient. There is no MCP constructor accepting such authority from
client JSON.

Resolution checks the observed entity kind and canonical generation, decodes with
the existing schema helper, verifies the exact native identity fact and its source
field/digest/tick, and retains the canonical ID and generation beside the native ID.
Untrusted, asserted, replayed, laboratory-only, missing-source or missing-domain
evidence cannot resolve a live identity.

## Single-labor workforce resolution

`resolve_workforce_step` accepts the original sealed `PreparedPlan`, selected
`StepId`, `LiveRoutingEvidence`, and strict `WorkforceCapture`. It validates the
plan and its original preconditions and requires the native capture to name the
same fortress, bridge generation and observed paused tick. The selected native
citizens must exactly equal the resolved canonical selection; additional,
missing, ineligible or substituted citizens are refused.

The requested labor must match the native capture's exact labor-key schema. A
supported detail is selected-only and contains exactly that one labor. There must
be exactly one such detail. A multi-labor detail or ambiguous choice is refused.
Disabling additionally refuses another detail that could continue granting the
labor to a selected citizen. The complete detail capture supplies this check;
non-selected-only details are conservatively treated as potential grants.

The result, `ResolvedWorkforceRoute`, retains the original semantic digest, step,
anchor, source digest and canonical identities alongside a read-only
`AssignmentPlan` candidate. Its key is the original semantic step key. It confers
no authority to prepare or execute the assignment.

Native workforce `Applied` proves membership and native readback under that
family's broader contract. `ResolvedWorkforceRoute::verify_labor_effect` separately
requires the requested labor value on every selected citizen and requires every
other labor column to remain unchanged. A native Applied result that also changes
carpentry while enabling mining fails this semantic check. Removing a membership
while mining remains enabled also fails. Unknown and other non-Applied phases
cannot prove a labor change. This immediate readback check does not establish
current state at a later anchor or complete any original temporal obligation.

## Furniture constraints remain explicit

Both `LiveRequest::Furniture` and `LiveResolution::FurnitureItem` retain the full
`MaterialSelector`: required tokens, forbidden tokens, nearest-item preference,
and reservation count. Token sets are bounded and disjoint. The current
build/1.19 capture contains numeric material IDs; it does not supply an admitted
material-token dictionary, a complete nearest-item comparison or resource
reservation evidence. `resolve_furniture_step` therefore explicitly refuses every
non-default selector. An advisory family route still reports the retained
constraints so the caller can see exactly what remains unresolved.

For a default selector, furniture resolution requires an exact spatial/1.8 item
projection and a matching eligible `BuildCapture`. Source generation, fortress,
paused tick, map dimensions, furniture kind and target must agree. The selected
canonical item's native identity, type, subtype, numeric material IDs, raw
position, on-ground flag and unassigned job flag must all have eligible exact
source evidence matching the native capture. This creates a `BuildPlan` candidate
with the original semantic key and source binding. It does not prove path
accessibility, nearest-item selection, placement, construction completion or
production admission.

Advisory coordinates are checked against the real native family bounds before a
route is offered. Furniture x/y need the same-level halo; z zero is permitted.
Dig requests need a three-dimensional halo, so boundary z levels are refused.
Dig tiling validates even directly constructed cuboids before arithmetic and
refuses requests requiring more than 64 native rectangles without iterating the
oversized area.

## Verification scope

Routing regressions exercise real V1 and spatial/1.8 projection constructors,
strict workforce capture/effect codecs and the checked-in build/1.19 golden
capture. They cover native zero and maximum V1 unit IDs, spatial namespaces,
source-grant refusal, exact capture identity and selection, ambiguous and broad
labor details, disable overlap, broader native Applied readback, preserved
material policies, original plan integrity and preconditions, and native geometry
boundaries. These are synthetic source and semantic contract checks. They provide
no real-DF capture, compatibility qualification or native admission evidence.
