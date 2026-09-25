# Sparse, multi-level excavation goal evidence

The read-only model in `scripts/excavation_blueprint.py` evaluates an exact terrain
mask against one unchanged map/1.5 native capture. It does not designate terrain,
advance time, authorize a mutation, or discharge a native effect obligation.
Owner: `df-action-coordinator-exec-ero.4` (bounded temporal verification increment).

## Blueprint contract

```json
{
  "schema": "dfmcp.excavation-blueprint/1",
  "parts": [
    {"region": {"origin": [10, 20, 30], "size": [3, 3, 1]}, "shape": "floor"},
    {"region": {"origin": [13, 21, 30], "size": [1, 1, 1]}, "shape": "stair_up"},
    {"region": {"origin": [13, 21, 31], "size": [1, 1, 1]}, "shape": "stair_down"}
  ]
}
```

Parts are disjoint cuboids in zero-based map tile coordinates. All selected cells
must have the exact requested normalized shape, zero liquid depth and no dig
designation in the same sample. Shapes are `empty`, `wall`, `floor`, `ramp`,
`ramp_top`, `stair_up`, `stair_down`, and `stair_up_down`, using the existing
map/1.5 tags documented in `LIVE_MAP.md`. Unsupported/other is not a target.

Bounds: 1..32 parts, at most 512 target cells, and at most 1,024 cells in their
single enclosing capture. Existing coordinate and 128-tile side bounds apply.
An oversized enclosure is rejected, not split into incoherent reads. Input JSON
is at most 16 KiB with depth at most eight, duplicate and extra fields rejected.
Even same-shape overlapping parts are rejected. The semantic digest is invariant
under part ordering and disjoint rectangle splitting; it is not an authorization
signature. Source and fortress identity must be bound separately.

Unselected holes in the enclosure are acquired but never evaluated as targets.
Hidden/missing selected cells remain unknown, with no fabricated attributes.
`diagnose` returns exact counts and 0..64 whole remaining-cell rows in native
x-fast/y/z order, with an explicit omission count. Shape, liquid and designation
mismatch counts may overlap; matched/mismatched/hidden/missing counts partition
all targets. These are observed deficits, not inferred causal blockers.

`BlueprintGoal` and `advance` implement whole-mask sampled stability. Every part
must qualify simultaneously. Strictly advancing game ticks count as new samples;
failed/unfinished reads, unknowns, contradictions and excessive gaps reset the
streak. The fixed deadline is inclusive. Source/software/generation, fortress,
dimension changes or a regressed clock invalidate the goal. Terminal progress is
immutable. Both evaluation and diagnostics re-decode native bytes, rather than
trusting caller-supplied derived fields in a Capture object.

This first increment is the pure library and tests. The existing floor-goal CLI
and journal format are unchanged; durable blueprint command integration is not
claimed by this increment. The intended separate goal format is
`dfmcp.excavation-blueprint-goal/1`, not a reinterpretation of old floor journals.

## Evidence and limits

`PYTHONPATH=scripts python3 -m unittest test_excavation_blueprint -v` passes 12
actual Python tests: 4,608 exact shape/liquid/designation combinations, 500
floor-transition traces against the unchanged legacy evaluator, sparse multilevel
indexing, overlap/bounds, simultaneous targets, unknowns, source invalidation,
fixed deadlines, interrupted stability and native-byte revalidation. Test captures
are explicit native-layout doubles, not DFHack or live-fortress evidence.

Matching stairs do not prove connectivity; matching walls do not prove continuous
preservation. No material, construction provenance, structural support, occupancy,
temperature, route safety, continuous history or mining causality is established.
No native protocol, dependency, MCP route, production map or admission is changed.
