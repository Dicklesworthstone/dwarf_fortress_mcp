# From a room intention to its remaining excavation

`scripts/room_terrain.py` reduces one coherent map/1.5 capture against the complete
original `RoomPlan`. It handles rooms that are partly excavated: existing dry,
undesignated floors are recorded as observed geometry, and only the remaining
visible wall targets enter a proposed excavation blueprint. Neither a terrain
sample nor a remaining-mask digest authorizes a game effect.

This is an isolated development path, not a production MCP tool. The original
room recipe, native map/dig protocols, journals and mutation admission are unchanged.
Owning beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open.

## Complete selection and explicit deficits

The original room intent is regenerated before trusting its compiled outputs.
The selection contains every proposed floor (rooms, doorways and corridors) and
all required bedroom boundary walls except each intended doorway. One bounding
capture includes a full one-tile three-dimensional pad; dimensions are not
clamped, split, translated or silently shortened to fit the map profile.

A target is either an observed dry, undesignated floor or a remaining dry,
undesignated, unoccupied wall shape. Hidden/missing evidence, liquid or a magma
flag, existing dig designations, occupied walls, and unsupported shapes block
the complete excavation proposal. A required bedroom wall must independently
have a visible dry, undesignated, unoccupied wall shape. An existing opening is
a deficit, not permission to fill it or change the room recipe.

For every remaining wall target, the unique one-tile 3D halo must be visible,
dry and undesignated. Known liquids, hidden/missing halo cells and existing dig
work block the proposal. This includes vertical and diagonal neighbors. This is
a conservative survey policy, not the native dig eligibility predicate: aquifers,
wall material, structural support, water pressure, protected native state and
unit paths are not established. Fresh native review can still refuse every part.

Unused capture holes are not invented targets or deficits. When no wall targets
remain, there is no remaining-excavation halo to check; this does not turn missing
context into safety evidence. Floor occupancy is counted separately. A floor
with a building or unit can satisfy a shape predicate without granting permission
to place a new furnishing there.

## Exact remaining blueprint, never a successful subset

The reducer retains the full original room plan, constraints and completion
blueprint. It reports every remaining wall coordinate and exact target, required
wall and halo category counts. Diagnostic rows are bounded to 128; all additional
failed checks remain counted. A coordinate can fail more than one domain check,
so the blocker count is a check count, not a unique-coordinate count.

Only an unblocked nonempty residual emits `remaining_blueprint`, using the existing
`dfmcp.excavation-blueprint/1` schema. A complete shortage of coverage, wall deficit,
hazard or capacity failure emits no executable subset. The proposal excludes
all existing floor cells, required walls and unselected holes, including between
levels. No native job, receipt, cancellation or uncertain effect is queried.

Two deterministic exact covers are compared: a global z/y/x width-first cover,
and the same cover clipped to each original disjoint recipe part. The smaller
part count wins, then canonical bytes break ties. This avoids unnecessarily
rejecting ten-bedroom recipes whose original decomposition fits the limit.
It is not a minimum-rectangle algorithm. The independently counted 8x8 partition
matches the existing normal-mining geometry rule; it grants no prepare token.

The existing 32-part, 512-target, 1,024-cell unpadded bounding capture and 128-native-
step limits still apply to the resulting excavation artifact. A fragmented mask
that exceeds either partition limit is refused in full, not truncated or split
into secretly independent plans. The survey's padded capture has at most 4,096
cells with each axis at most 128. Raw survey evidence is at most 82,495 bytes;
the entire result has a 128 KiB reservation including its eventual envelope.

## What success means

`excavation_proposed` means the remaining mask passed only these explicit sampled
checks. `terrain_shapes_satisfied_at_sample` means the full floor and required-wall
shape predicates held in this single capture. Neither means native excavation
eligibility, current terrain, continuous preservation, structural safety,
pathfinding, room assignment or construction completion.

The retained source includes the exact map profile/manifest, folder, site,
dimensions, padded region, tick, pause observation, raw SHA-256 and observation
witness. `survey_digest` binds the full result to the original room intent and
these source facts. Supplied bytes or exported digests alone do not independently
attest native acquisition. Map and dig generations remain separate namespaces.

**Never use a residual to replace an unresolved dig batch or to mint retry keys.**
Reconcile all original effects through their existing original-key query/cancel
workflow first. This survey has no effect inventory and cannot clear uncertainty.
Even successful execution and monitoring of the residual alone cannot discharge
the complete room project: the original floor blueprint remains the completion
goal, and required walls, furnishings and native room assignments are separate.

## Pure API and executed scope

```python
from room_provisioning import RoomPlan
from room_terrain import selection, survey

room_plan = RoomPlan.decode(original_plan_bytes)
region = selection(room_plan).region
# Obtain exactly this region with the existing read-only map client.
result = survey(room_plan, capture_bytes, map_manifest)
```

The 16 focused Python tests execute the unchanged room compiler, furniture request
contracts and strict map decoder. They enumerate all 4,096 4x3 residual masks,
all 250 bedroom count/size combinations (including whole-recipe refusals), and
all 64 dining count/column combinations. Additional cases cover partial and fully
excavated rooms, shared bedroom walls, multi-level holes, vertical/diagonal hazards,
capacity exhaustion, input/source substitutions, map edges, deterministic identity,
complete bounded deficits, and interruption at every guarded boundary of a small
survey. Source and log hashes are recorded in `docs/evidence/room-terrain-python.json`.

These are supplied native-format byte fixtures, not real DFHack or live gameplay.
This first reducer qualification does not execute sockets, the native dig client,
Rust/MCP, the DFHack SDK, a game save, or the full repository suite.
