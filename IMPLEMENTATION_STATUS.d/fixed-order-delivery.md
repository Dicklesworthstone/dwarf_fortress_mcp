# Fixed original-order delivery allocation

NEW proposals can select items optimally for the original canonical delivery
order without rewriting the request or expanding dependencies. The existing
allocation and review-create commands expose this as --fixed-delivery-order;
retained review checking rejects the option and never changes original items.
No composite room handoff or native effect path is introduced.

29 actual Python tests pass (15 core, 14 workflow), zero skips with warnings denied.
Independent exhaustive assignment/coordinate-route oracles cover 512 obstacle
maps and 250 seeded domains; 32 slots and 65,536 items are exercised. Four weakened
variants are rejected, and four legacy mode packets remain byte-identical for a
shared fixture. Five unchanged dependencies match upstream Git blobs.

Workflow tests inject native acquisition, operations decoding, transport budget
and endpoint parsing. They execute complete runtime/CLI modules and actual
solver/map/projector/review code, not native TCP or a complete room lifecycle.
Rust/MCP, native SDK/game and full-workspace qualification remain unexecuted.
Broader graph/bridge beads and production admission state are unchanged.
