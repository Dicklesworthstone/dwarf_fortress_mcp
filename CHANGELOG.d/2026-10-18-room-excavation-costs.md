## Cost-aware whole-room placement

Add explicit binary target costs to the existing complete-layout optimizer.
Rank whole accepted projects by cost, footprint, entry distance and stable ties
without relaxing walls, exclusions or original furnishings. Old requests retain
exact bytes and budgets. See `docs/ROOM_LAYOUT_EXCAVATION_COSTS.md`.
