## Coherent terrain-aware blueprint previews

- Expose `blueprint_layout` through the existing read-only spatial query handler,
  with all five bounded templates and a closed published JSON Schema.
- Report exact disjoint geometry, native footprint and unique 3D halo coverage,
  visible liquids/occupancy/designations and separately measured moat crossings.
- Preserve hidden/unallocated/outside-capture/outside-map distinctions. Geometry
  or complete visibility never becomes safety, excavation eligibility or success.
- Bound cell visits and whole-row output; bind continuations to session, anchor,
  source, policy, request and analysis while allowing page-width changes.
- Add analyzer, routing, authority, pagination and actual-handler regression tests.
  Rust tests remain unexecuted here; Python JSON Schema checks passed. No native
  writes, archive whitelist expansion or compatibility admission is introduced.
