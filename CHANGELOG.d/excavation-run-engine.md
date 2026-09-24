# Goal-driven excavation clock engine

Add a native-owned, bounded floor-condition run that stops on sampled satisfaction,
unknown/wet target terrain, read failure, clock limits, cancellation or source loss.
Keep safety pause/readback independent of terrain acquisition and retain ownership
until pause is verified. Reuse the unchanged bounded-run engine and preserve all
existing native profiles. Both GCC and Clang pass 7030 assertions and reject four
compiled mutants. No real SDK/live-game/Rust/MCP or production admission claim.
