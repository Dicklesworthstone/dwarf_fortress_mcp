### Bounded normal-mining designation (development source)

Add isolated native dig/1.16 for a 1..8 by 1..8 visible natural-wall rectangle,
with a complete bounded 3D halo, explicit hidden-neighbor-risk policy, full job
conflict scan, guarded operator enablement, sealed prepare/commit/query/cancel,
pre-write uncertainty and exact dig/priority/scheduling readback. Never remove
jobs, channel, make stairs, excavate immediately, or advance the game. Retain
partial-write uncertainty and block cross-key retries; native retention is not
a durable coordinator.

GCC and Clang each pass 1,319 engine and 4,499 complete-handler assertions under
UBSan/warning denial with explicit SDK/protobuf doubles. Four encoder vectors
match Python; six compiled mutants fail. No real SDK, game, Rust/MCP, durable
host coordinator or production admission is established. See DIG_DESIGNATION.md.
