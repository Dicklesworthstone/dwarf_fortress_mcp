# Receipt-linked construction: executed condition/replay core

An additive development capability now validates complete unchanged operations/1.4
captures against an exact furniture/1.19 Placed receipt. It binds the original
building footprint/type/stage and exact item/material identity, refuses missing or
changed identities, and implements bounded sampled stability with fixed deadlines,
source/horizon/clock fences and interrupted-read resets. It never changes original
placement obligations or authorizes another effect.

Sixteen actual Python test functions pass, including all 512 item flag words,
truncated records, full-roster integrity and restart-state transitions. Four
weakened implementations fail regression assertions. See
`docs/RECEIPT_CONSTRUCTION.md` and `docs/evidence/construction-receipt-core.json`.
This first increment is the executable pure core only; native transport and durable
monitor integration are not established yet. No Rust/MCP, native plugin, live game,
full qualification or production admission is claimed. Broader owning beads remain
open.
