# Observed-room circulation engine (2026-10-15)

Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/07/10; remain open).
Source present: scripts/room_circulation.py computes clear and optimistic
reachability within the fixed original floor mask. All area entries and all
furnishing approaches participate; connected projects use one canonical root.
Unknown or unit-occupied candidate cells cannot establish a clear route.
No path outside the mask, implicit stairs, native navigation or game authority.

Executed: all 12 Python core tests pass, including all 2,187 ternary 3x3 maps
against independent set closure, complete group truth, detours, selected-item
barriers, guard interruption at every checkpoint, exact work limits, hash-seed
equality and 512 cells/8 areas/32 furnishings. Published implementation and test
blob hashes match executed local bytes. Native projection and durable readiness
integration are the next increment; no Rust, DFHack, CLI/TCP, journal or full
repository qualification is claimed by these core results.
