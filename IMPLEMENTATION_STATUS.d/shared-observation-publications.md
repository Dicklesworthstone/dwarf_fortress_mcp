# Live observation publication and retained replay

The operations and spatial development profiles now retain immutable world
graphs, native observations and generation maps by shared ownership. This
removes whole-state copies when a live MCP session takes the journal's current
state, when append prepares a rollback candidate, and when the same observation
arrives again. A changed observation constructs its own graph and identity map.
Appending borrows the candidate's native observation for the record encoder.
A historical snapshot read consumes its private replay state after verification.

The regression exercises bootstrap, heartbeat, advancement and reset in all four
journal profiles. It checks graph and source sharing and verifies the old anchor
and complete graph stay intact when another owner advances or resets. Runtime
measurements and full qualification remain pending.
