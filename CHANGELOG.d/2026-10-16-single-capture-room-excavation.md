## One-source room layout to original-room excavation

Compose the existing layout solver, full terrain survey and original-room dig
handoff from one complete padded map/1.5 payload. Search excludes wall targets
with unsupported surrounding context while retaining them as possible room
walls. Exact source-linked subregions preserve native tile bytes and unknown
presence; all room/furniture constraints and the exact remaining mask survive.

25 component tests passed, including actual legacy handoff decoding and complete
eight-area/32-furnishing plans. No native protocol, game mutation, effect-retry
permission or production admission changes. See docs/ROOM_LAYOUT_EXCAVATION.md.
