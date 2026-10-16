# Room placement and corridor constraints from one finite site mask

`site_mask` is an optional closed object on both the automatic room-layout
request and the lowered room-provisioning request. It contains `region`
(`origin`, `size`) and an x-fastest `cells` string with exactly one character
per cell. The region is one level, at most 1,024 cells, and at most 128 per axis.
For automatic layout it must equal the complete requested search region.

A dot allows an intended floor or preserved wall; `F` allows only floor; `W`
allows only preserved wall; `X` allows neither. Cells outside the mask are
unavailable for both uses. These are planning constraints, NOT observed facts,
safety certificates, or permission to modify the game.

The existing solver prunes translations against both floor and required-wall
masks before trying complete candidates. The existing RoomPlan compiler repeats
these checks for every original floor, doorway, corridor and required bedroom
wall. Connecting corridors treat floor-forbidden cells as protected and find
another bounded route or refuse the entire project. No rectangle cover or
32-exclusion approximation is necessary for a fragmented 1,024-cell mask.

The complete mask is retained in normalized layout and RoomPlan intent. Saved
artifacts regenerate the same constrained search/geometry. Existing resource,
part, tile, output, work and cancellation bounds are unchanged. Mask omission
preserves old bytes and work counters; unknown mask fields or partial masks
refuse. Old readers reject the new optional field rather than ignoring it.

The existing layout and provisioning commands accept the field without another
entry point. The lowered room artifact preserves it through ordinary canonical
import. An excavation export remains only the chosen target geometry: original
RoomPlan and any observation evidence must be retained separately. Native review,
hazards, authority, exact effect custody and completion are not supplied by this
mask. No protocol, native method, dependency or production admission changed.

Executed: 12 actual Python compiler/router/solver tests, including 256 obstacle
masks versus an independent complete-placement oracle, wall/floor distinctions,
corridor detours and whole barriers, canonical import refusal, all guard
interruptions in a small search, exact work/node limits, schema checks and the
eight-area/32-furnishing/1,024-cell case. Separately, 314 legacy template outcomes
were compared against the Git-blob-verified pre-change compiler: 310 successful
artifacts were byte-identical and four refusals unchanged. Ten unmasked layout
artifacts, including work counters and trace digests, were also byte-identical.

Run: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest test_room_site_mask -v`.
These are Python component tests, not native, Rust/MCP or full-repository
qualification. Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (remain in progress).
