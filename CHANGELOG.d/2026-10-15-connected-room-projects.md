# Connect whole room projects through the existing compiler

RoomPlan now accepts the explicit connect_areas Boolean and routes complete
same-level area circulation while preserving required bedroom walls, furnishings
and excluded regions. Exact added corridor cells enter the existing excavation
blueprint and all original-plan/mask digests; imports regenerate the whole result.
Omitted/false requests retain legacy bytes. Existing limits are not enlarged.

All 27 actual Python component tests pass, including 314 legacy template outcomes,
whole-project detours/refusals, exhaustive tiny-map oracles and cancellation.
See docs/CONNECTED_ROOM_PROJECTS.md. No native effect or admission is added.
