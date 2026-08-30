# Dogfooding fastmcp_rust (MCP 2026-07-28)

This project is a deliberate conformance hammer for the owned
[`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling at the MCP
**2026-07-28** spec (ADR-013). The fortress control plane exercises modern-era sessions,
tools, cancellation, and Tasks harder than almost any workload: long-running bounded
obligations, digest-sealed plans, epoch invalidation, and evidence-carrying responses.

The deal is symmetric:

1. dfmcp consumes fastmcp_rust through a pinned revision and refuses to mask protocol defects;
2. every defect found is filed upstream and tracked until the pin takes the fix.

## Filing upstream defects

Repository: `Dicklesworthstone/fastmcp_rust` (gh issues). Before filing:

1. **Minimize against the lab.** Reduce to the smallest `dfmcp-mcp` (or raw `fastmcp-rust`)
   reproduction on the pinned revision. The `MemoryAdapter` session makes semantics irrelevant —
   if a repro needs fortress semantics, it is probably two bugs.
2. **Classify.**
   - *spec violation*: contradicts MCP 2026-07-28 text — cite the spec section;
   - *protocol bug*: valid spec, wrong fastmcp behavior (cancellation, framing, session state);
   - *ergonomics*: correct but obstructive for substrate-style integrators.
3. **Capture evidence.** Exact pinned `rev`, JSON-RPC byte capture (or stdio transcript), expected
   vs actual, and the minimal repro. A doctor bundle is not required; a byte capture usually is.
4. **File.** Title `[2026-07-28][area] summary`; body with repro, capture, expectation, and the
   failing spec sentence. Link the dfmcp commit that exercises it.

## Workaround policy (do not mask bugs)

A dfmcp-side workaround for a fastmcp protocol defect is **prohibited** unless it is annotated at
the site with `// DOGFOOD-WORKAROUND:` plus the upstream issue URL, and listed in the pin-history
table below. Silent workarounds destroy the dogfooding signal and are treated as bugs in this
repo. Workarounds are removed when the pin advances past the fix.

## Taking fixes upstream → pin bumps

Upstream fixes land here by **bumping the pinned revision** in
`architecture/dependency_allowlist.toml` (`[mcp_transport].pin`) and the workspace
`Cargo.toml`. Each bump must:

- state the upstream issues it closes (or the upstream commit range);
- re-run the conformance suite and the workspace gates (`scripts/verify.sh`);
- append a row to the pin-history table;
- never bundle unrelated dfmcp changes (bisectability of upstream regressions matters).

Pins move only forward within a dogfooding cycle; rolling back a pin requires a note explaining
which upstream regression forced it.

## Pin history

| Date | Pin (`fastmcp_rust` rev) | dfmcp consume revision | Upstream issues closed by the bump | Notes |
|---|---|---|---|---|
| 2026-08-29 | `bd41e69070f5604d6dbb24185dcabaef591a01e1` (main @ adoption) | v0.0.1 initial integration | — | modern-only profile (`default-features = false`, `tasks`); first stdio laboratory slice |

## Conformance status

MCP 2026-07-28 conformance evidence for the pinned revision is a WP-21 gate: it is produced by
running the upstream conformance suite against this integration and recorded here. Absence of a
row below means *no conformance claim*, consistent with `IMPLEMENTATION_STATUS.md`.

| Date | Suite/revision | Scope exercised | Result | Evidence artifact |
|---|---|---|---|---|
| — | — | — | — | pending WP-21 |
