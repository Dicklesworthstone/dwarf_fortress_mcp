# Typed sampled floor goals

`dfmcp_adapter::excavation_goal` supplies the Rust evaluator for progress-journal
integration. It reuses `LiveMapObservation` and the existing map/1.5 codec; no
native method, dependency, transport or game effect is introduced.

A goal names one exact fortress and a 1..8 by 1..8 rectangle at one z level. All
target cells must be visible FLOOR, dry, and without a dig designation. Hidden or
unallocated cells remain unknown. Unsupported shapes, walls, wet floors and
remaining designations cannot satisfy the goal. Occupancy, temperature, material,
walkability and structural safety are deliberately not part of this predicate.

The state machine enforces a fixed inclusive deadline, required matching sample
count and game-tick span. Equal-tick reads do not increase stability. Contradictory
or unknown captures, failed/unfinished reads and excessive sample gaps reset the
streak. Generation, software, fortress, dimensions or clock regression invalidate
the goal without replacing its last accepted original-source sample. Terminal
outcomes cannot be extended, cancelled or used to restore mutation permission.
The native map generation is not the native dig generation.

Types keep transition fields private. This module neither trusts a serialized
success flag nor changes a dig journal. Satisfaction is historical sampled terrain
evidence, not a causal mining receipt, continuous stability, safety, present-world
truth or permission to retry a designation. The existing Python monitor remains
the standalone acquisition/persistence implementation.

Ten Rust tests are registered, including all 576 shape/liquid/designation
combinations. They have NOT been compiled or executed: Rust, Cargo and rustfmt
are unavailable in this environment. Uploaded implementation/test identities
match the locally reviewed files. No Rust, native SDK, live game, power-loss,
MCP execution or full qualification is established by this increment.

```sh
cargo test --locked --offline -p dfmcp-adapter excavation_goal
```
