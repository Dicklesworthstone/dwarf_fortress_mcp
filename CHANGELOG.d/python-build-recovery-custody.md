## 2026-10-13 — Preserve original placement evidence through recovery failures

- Require writable private custody before native recovery contact, preserve the
  last persisted receipt's source-generation floor, and retain known uncertain
  evidence when a native lookup is absent.
- Reserve a final positive-receipt frame before retaining uncertainty, without
  increasing the existing 32 KiB journal or 16 KiB frame bounds.
- Preserve concurrent explicit query-only reconciliation, immutable normal
  receipts, original manifests, route-batch routing and publication checks.
- Execute 53 Python tests, including 19 new real-file and loopback/CLI recovery
  cases. Six weakened implementations fail targeted regressions. The current
  predecessor reproduces all four newly covered custody/history gaps.
- No real DFHack SDK, live-fortress, Rust/MCP, complete route-parent or full-suite
  qualification claim. Beads df-dfhack-bridge-plane-c-pic.4/.5 remain broader work.
