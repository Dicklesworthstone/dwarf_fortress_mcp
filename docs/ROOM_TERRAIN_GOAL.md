# Whole-original-room terrain completion

The room survey's residual blueprint is not the complete room objective. Even
the original floor-only blueprint omits the bedroom walls that must remain.
`scripts/room_terrain_goal.py` retains the complete original RoomPlan and checks
all intended floors and required bedroom boundary walls in one coherent map/1.5
capture. All requirements share one advancing-game-tick stability streak.

Floors must be visible, floor-shaped, dry (including the magma flag), and free
of digging designations. Required walls must additionally be wall-shaped and
unoccupied. Shared walls are counted once. Dining halls gain no invented wall
requirements. Occupied floors are counted but do not establish furniture
placement eligibility. Unselected padded capture cells are not completion
requirements or assumed safe terrain.

Hidden/missing requirements yield unknown; mismatches yield pending. Interrupted
reads, failures and excessive sample gaps reset the whole streak. Repeated or
paused game ticks cannot add matching samples. Source/manifest/dimension changes
or clock regression invalidate the goal. Its deadline is inclusive. Terminal
states cannot be advanced. All diagnostics retain exact aggregate counts and
at most 64 complete deficit rows in deterministic floor-then-wall z/y/x order.
The primary diagnostic reason precedence is missing/hidden, liquid, designation,
shape, occupied wall; counts are mutually exclusive, not a list of every fault.

The closed `dfmcp.room-terrain-goal/1` value preserves the complete room plan,
furnishing constraints and timing policy. Decoding regenerates the original
intent rather than trusting stored geometry. Native capture bytes are decoded
again before transitions; caller-supplied derived tile fields cannot establish
success. The domain-separated goal digest binds the full goal, not just its
excavation subset. Digests are not external attestations or mutation permits.

Fourteen executable Python core tests passed on 2026-10-05 using independent
native-format byte fixtures, including every guarded boundary of a small goal.
They cover wall loss despite finished floors, alternating partial successes,
shared walls, multi-level holes, bounds, source substitution, raw evidence,
interruption, inclusive deadlines and preserved original intent. This increment
is the pure reducer; durable CLI acquisition is a separate integration increment.

No native digging, furniture placement/completion, room assignment, structural
safety, pathfinding, continuous preservation or production admission is proved.
Rust/MCP, real DFHack SDK/ABI, live fortress and full-repository qualification
were not executed. Beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
