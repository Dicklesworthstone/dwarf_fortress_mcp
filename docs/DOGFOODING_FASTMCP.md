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
| 2026-08-29 | `6481d49a6f9282f8161015323283fb7764dcf2f7` (main: fixes #59, #60) | v0.0.1 first integration | #59, #60 | modern-only profile compiles end to end (transport + facade + tasks); #61 (feature-less server build) open upstream, does not affect the admitted profile |
| 2026-08-30 | `12d3469df8081ffdb663019ee4936324fedc98d5` (tags/v0.8.0) | v0.0.1 v0.8.0 preflight | v0.8.0 release | Restructured sse.rs (legacy SSE removed upstream), non-legacy warning fixes (3a82c30), modern-only profile verified |

## Conformance status

As of 2026-08-31, the v0.8.0 pin (`12d3469`) **fails the modern handshake
golden tests in CI**: `test_negative_era_refusal_and_marker_validations`
passes, but `test_modern_handshake_full_lifecycle_and_plan_commit` hangs
indefinitely after a successful `server/discover` because `tools/list`
and subsequent modern requests are not dispatched (see Open defects
below). The previously claimed "PASS (all golden tests green)" row was
premature; this row supersedes it.

| Date | Suite/revision | Scope exercised | Result | Evidence artifact |
|---|---|---|---|---|
| 2026-08-31 | `12d3469df8081ffdb663019ee4936324fedc98d5` (fastmcp_rust v0.8.0) | Modern handshake (negative cases only) | PASS on negative fixtures; full lifecycle hangs at `tools/list` after `server/discover` (DRAFT bug) | `crates/dwarf-fortress-mcp/tests/modern_handshake_golden.rs` |

## Open defects under the v0.8.0 pin (`12d3469`)

Findings from the 2026-08-31 dogfooding pass; not yet filed as upstream
issues (paste the byte captures below into the upstream tracker once the
project is online).

| Finding | Minimal repro | Expected | Actual | Filing status |
|---|---|---|---|---|
| `server/discover` silently fails when `_meta` carries only `protocolVersion` + `clientCapabilities` (no `clientInfo`). | Send one `server/discover` request with `_meta = {protocolVersion: "2026-07-28", clientCapabilities: {}}`. | JSON-RPC response (either discover result or an error envelope). | No response is ever written to stdout; the server keeps reading stdin until the pipe closes. | DRAFT — pending upload |
| After a successful modern `server/discover`, a follow-up `tools/list` (same `_meta`) is not dispatched. | Send `server/discover` then `tools/list` with full modern `_meta` (incl. `clientInfo`). | Two JSON-RPC responses. | Only the discover response is written; `tools/list` hangs forever. | DRAFT — pending upload |

Workaround (annotated at the call site): the modern golden test now
supplies `io.modelcontextprotocol/clientInfo` in `_meta`. This is a
test-side adaptation, not a server-side mask; both fixes belong upstream.
