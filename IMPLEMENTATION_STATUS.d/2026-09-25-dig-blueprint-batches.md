# Sparse blueprint designation through the existing native workflow

Implemented a floor-only `dfmcp.excavation-blueprint/1` compiler and a durable,
explicitly stepped batch driver over the existing dig/1.16 Python client/store.
Exact sparse masks become disjoint bounded normal-mining rectangles. The retained
manifest binds the complete blueprint, source and private child directory. Each
advance re-observes, confirms and attempts one original native commit. Unknown
or refused children block later work; recovery never recommits. A persistent local
stop disables future steps without claiming to cancel miners or pause the game.

Thirty-three actual Python test groups pass. They cover all 4,095 nonempty 4x3
masks, 384 rectangle sizes, multi-level restart, six publication-sync failure
points, torn writes, lost replies, no duplicate commit, source/policy/custody
changes, default single-designation behavior, a full 128-receipt inventory and
actual CLI/TCP execution against joined protocol doubles. Actual JSON measured
65,504 bytes for the complete 128-step inventory and 82,780 bytes for a maximum-
field review, within the 131,072-byte output limit. Exact source hashes and the
executed command are in `docs/evidence/dig-blueprint-batch.json`.

This is development source, not production admission. No new native protocol,
Rust/MCP capability, automatic unpause, global controller lease, game checkpoint,
atomic multi-region transaction, or rollback is added. Source compilation/live
qualification of the Rust/native system remains absent in this environment.
Beads `df-dfhack-bridge-plane-c-pic.4` and `df-action-coordinator-exec-ero.4` remain
open for broader integration and qualification.
