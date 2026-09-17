# Bounded terrain-mask monitoring

The `terrain_count` condition closes a gap between spatial previews and long-running
monitoring: agents can track an entire declared excavation footprint without one
field condition per tile or an unsound count over only the tiles that happened to
be returned. This is read-only development source, not an admitted mutation family.

## Exact requested domain

A condition names 1..64 inclusive, pairwise-disjoint `min`/`max` cuboids, covering at
most 16,384 coordinates in total. Coordinates are ordered and within 0..32767.
Overlapping or repeated cuboids are refused, not counted twice. Cuboid order is
canonicalized for evaluation evidence; retained watch definitions keep their original
request identity, so changing a definition still conflicts with an existing key.

For example, this condition asks whether all nine requested coordinates currently
have observed floor shape:

```json
{
  "op": "terrain_count",
  "areas": [{"min": [10, 10, 5], "max": [12, 12, 5]}],
  "predicate": {
    "op": "field", "field": "shape", "comparison": "eq",
    "value": {"type": "text", "value": "floor"}
  },
  "comparison": "eq",
  "value": 9
}
```

Use this under the existing `watch.condition`, `watch.failure_condition`, or
`condition_evaluation.condition` envelope. Boolean combinations use the existing
`all`, `any`, and `not` conditions. Per-tile predicates share the existing
`watch_count_predicate` schema and joint 64-node/depth-eight condition budget.
They can inspect shape, raw designation, liquid, occupancy, traffic, walkability,
temperature and visibility fields; unregistered fields remain unknown.

## Evidence rather than absence by omission

The evaluator visits every requested coordinate, not a filtered entity list.
Canonical tile identity, position, visibility, native field path, observation tick,
consistent known presence and one coherent source digest must agree. Capture region
and map dimensions are checked from the same source. The fixed supported source
paths are map/1.5, spatial/1.6.map and spatial/1.8.map; an unrecognized or unproved
capture yields unknown rather than being guessed compatible.

Hidden, unallocated, absent, outside-capture, outside-map and inconsistent-source
coordinates remain unestablished. Hidden cells contribute no attributes, even if an
inconsistent record contains attribute values. Unknown survives negation. A predicate
is evaluated only on an eligible visible tile using the existing three-valued Boolean
rules; a known false conjunct can still decide a visible tile's predicate.

If `m` tiles match and `u` are unestablished, the possible count is `[m, m+u]`.
All six comparisons reuse the existing interval evaluator. A condition is true only
when every possible count satisfies it, false only when every count fails it, and
unknown otherwise. Thus one missing coordinate prevents `eq requested_tiles` from
establishing complete success. Some lower bounds remain provable with partial data;
that never changes the reported visibility or unknown counts.

Evidence records include the mask and predicate digests, snapshot hash, native source
digest, requested/visible tile counts, lower/upper matching counts, known nonmatches,
unknown reasons and at most two coordinate-only unknown examples. They do not prove
native job completion, mutation causation, structural safety or continuous truth
between observations. A floor already present before a watch can satisfy a floor
predicate; the result is not proof that an agent's mining command caused it.

## Lifecycle and budgets

The existing watch state machine remains authoritative: repeated anchors do not
manufacture samples; unknown resets a success streak; deadlines, failure precedence,
epoch invalidation and terminal immutability apply. Foreground batch evaluation shares
one million coordinate/entity/predicate work units and the caller's wall-time budget.
Output refusal occurs before watch root publication and any durable checkpoint.
No timer, bridge refresh, game effect or new top-level MCP tool is added here.

`condition_evaluation` inspects the same predicates without registering or sampling a
watch. Existing exact-record and historical-series condition queries use the same
engine without sampling current watches. On configured spatial/1.8 paired journals,
the new condition is serialized as part of the existing bounded watch definition.
Older binaries reject the unknown variant rather than reinterpret it; no journal
migration or format change is performed. Restart requires the existing fresh-sample
and archive-binding checks, not downtime continuity.

## Validation

Ten Rust tests are registered for coverage/provenance, overlap and bounds, predicate
serialization, pure inspection, watch stability and publication refusal. One includes
810 exhaustive three-coordinate/comparison cases. These Rust tests have **not been
compiled or executed** in this environment: `cargo`, `rustc` and `rustfmt` are absent.

`python3 scripts/check_terrain_watch_reference.py` passed **12,482** independent
reference cases: 810 possible-count comparisons, 11,664 enumerated cuboid pairs and
8 mask boundary cases. The new schema JSON was parsed. This is neither JSON Schema
runtime conformance nor execution of Rust, MCP, filesystem recovery or live DFHack.
Executed reference source SHA-256:
`a4016547abb646afbcf1387803036fa12c8f37a036ec7352baf62a0e0abfc8ce`.

No compatibility registry, production runner, native wire format, dependency or
mutation authority is changed. Full Rust/native/live qualification remains required.
