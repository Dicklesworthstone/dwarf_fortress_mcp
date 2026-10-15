# Intended room-connection engine (2026-10-15)

Owning beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10).
Source and actual Python execution: 11 unittest methods pass, including all 128
3x3 obstacle masks against exhaustive simple paths and all 512 3x3 masks against
an independent exact-partition oracle. Caller guard interruption is tested at
every boundary; exact work exhaustion refuses without changing input masks.

The engine chooses each next link by new excavation, path length and deterministic
coordinate ties. It does not claim a globally minimal corridor network. It is
geometry only: no native terrain, route, safe shelter, room assignment, action,
bridge method, mutation authority or production admission is established.
At this increment it is not yet called by RoomPlan. No Rust/live/full-workspace
qualification was executed.
