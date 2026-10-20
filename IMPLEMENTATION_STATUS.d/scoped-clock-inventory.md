# Scoped-clock discovery (2026-10-20)

A fresh agent can enumerate an original private clock directory without retaining
individual batch IDs or confirmations. All records are verified before pagination;
original file identity, content, scope/intent linkage and directory membership are
rechecked under one exclusive lock. Historical batch associations grant no authority.

Evidence: 50 tests pass with warnings denied, zero failures/errors/skips; all 13
weakened variants are rejected by assertions. Sixteen new real-file/CLI cases include
256 terminal records, constant descriptor use, stale cursors and custody races.
A completed restored-source run verifies all 14 production/test/runner input hashes.
Earlier incomplete aggregate attempts are not counted as completed evidence.

No DFHack process, native plugin build, Rust/MCP or full-workspace run was performed.
Native formats, execution admission and production admission remain unchanged.
Beads df-dfhack-bridge-plane-c-pic.4/.5 remain in progress.
