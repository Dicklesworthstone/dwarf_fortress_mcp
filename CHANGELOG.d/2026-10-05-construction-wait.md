## Bounded foreground receipt-plan waiting

Implement `track_construction_plan.py wait` over the existing durable goal,
query-only acquisition and journal. Multiple whole-plan samples share one budget;
terminal outcomes, slice limits, stalled ticks and resource allowances stop work.
No background polling, goal renewal, game-time control or failed-read retry.
Authority and custody remain checked through delay and final publication.

20 executor tests and 12 actual TCP/private-journal/CLI tests passed, including
paged 32-target acquisition, durable completion, lost-reply recovery and legacy
commands. Python development evidence only; Rust/MCP, real native DFHack/live-game
and full qualification were not run. No admission or native format change.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
