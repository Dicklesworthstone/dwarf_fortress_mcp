# fastmcp_rust Integration

**Status:** adopted (ADR-013). The MCP presentation plane of this project is the owned
[`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling, pinned to an exact
upstream revision and constrained to **modern-only MCP 2026-07-28**. This document is the
integration contract; the ADR records the decision, and
`docs/DOGFOODING_FASTMCP.md` records the upstream evidence loop.

## 1. Why the transport is borrowed and nothing else is

MCP framing — JSON-RPC dispatch, stdio byte framing, Streamable HTTP, session lifecycle, era
negotiation, pagination, cancellation routing — is commodity plumbing with a large adversarial
surface and near-zero relation to this project's thesis. The thesis is the semantic substrate:
multi-version world state, witnessed plans, bounded obligations, evidence-backed completion.
`fastmcp_rust` is the same author's asupersync-native MCP framework with the same safety
discipline (`unsafe_code = "forbid"`, edition 2024, dated nightly) and the same cancellation
doctrine (request/drain/compensate/finalize). Borrowing the plane and owning the semantics is the
highest-leverage split available.

What is **never** borrowed: canonical state, plan sealing, capability/lease logic, idempotency,
obligation semantics, evidence, doctor. No `fastmcp` type crosses the intent, world, or adapter
seams. `dfmcp-mcp` is the only crate permitted to depend on `fastmcp-rust`, and it must remain
replaceable — if the sibling ever violates the counterexamples in ADR-013, the seam reverts to a
hand-rolled transport without touching any other crate.

## 2. The admitted profile

| Constraint | Value | Enforced by |
|---|---|---|
| Crate | `fastmcp-rust` (facade) | `architecture/dependency_allowlist.toml` `[mcp_transport]` |
| Version | exact git revision pin (`pin` key) | `scripts/check_dependency_policy.py` |
| Runtime graph | `default-features = false` (no `legacy-2024-11-05`) | checker + `scripts/validate_repo.py` |
| Features | `tasks` only | checker: required/forbidden feature sets |
| Forbidden features | `legacy-2024-11-05`, `websocket-experimental`, `apps`, `proxy*`, `enterprise-auth`, `oauth-client-credentials`, `builtin-auth-server`, `redis-tasks`, `jwt-resource-auth`, `testing-lab` | checker |
| MCP protocol | 2026-07-28 (`modern::PROTOCOL_VERSION`) only | profile definition; conformance gate WP-21 |
| Callers | `crates/dfmcp-mcp` exclusively | validator manifest screen |

`prost` appears in `Cargo.lock` solely as an asupersync-internal transitive (OTLP evidence
mapping) and is admitted as a documented lock exception; the dfmcp trust domain never references
it.

## 3. Layering

```text
MCP client (Claude Code, Codex, Inspector, …)
    │  MCP 2026-07-28 (JSON-RPC over stdio; Streamable HTTP later)
    ▼
fastmcp_rust: framing, session lifecycle, cancellation routing, pagination
    ▼
dfmcp-mcp: the frozen 11-tool fortress.* waist (thin, replaceable)
    │  dfmcp/0 semantics — never negotiated away by the transport
    ▼
dfmcp-intent / dfmcp-world / dfmcp-core: sealed plans, predicates, anchors, authority
    ▼
dfmcp-adapter seam: GameAdapter trait (no fastmcp types)
    ▼
dfmcp-lab (now) → DFHack bridge (later phases)
```

`dfmcp/0` semantic negotiation remains authoritative above MCP negotiation: a client that speaks
MCP but refuses `dfmcp/0` gets a session that can observe and plan nothing.

## 4. Tool mapping

Logical registry names keep their dots (`fortress.open_session`); MCP wire names render the dot
as an underscore because the wire name is an identifier, not an identity. Schemas in `schemas/`
and all registries use the logical names.

| Logical tool | MCP name | Laboratory status (WP-13 gate 1) |
|---|---|---|
| `fortress.open_session` | `fortress_open_session` | implemented over `MemoryAdapter` |
| `fortress.observe` | `fortress_observe` | implemented (summary projection) |
| `fortress.query` | `fortress_query` | `summary` mode only; DfQL is WP-04 |
| `fortress.plan` | `fortress_plan` | implemented (pause/resume registry family) |
| `fortress.commit` | `fortress_commit` | implemented (digest-matched prepare→commit) |
| `fortress.wait` | `fortress_wait` | implemented (`poll_action`) |
| `fortress.cancel` | `fortress_cancel` | implemented (request/drain/finalize) |
| `fortress.checkpoint` | `fortress_checkpoint` | implemented |
| `fortress.restore` | `fortress_restore` | implemented (epoch invalidation) |
| `fortress.explain` | `fortress_explain` | transcript-tail evidence |
| `fortress.doctor` | `fortress_doctor` | implemented (health/compatibility) |

Run it:

```bash
cargo run --locked -p dwarf-fortress-mcp -- serve
```

## 5. Gates

1. **Gate 1 (done): stdio laboratory slice.** All eleven tools over `MemoryAdapter`; process-local
   session state; digest-checked plan commit; epoch-safe restore.
2. **Gate 2: session-scoped authority.** Per-session state and capability grants replace the
   process-local lab; `open_session` arguments negotiate budgets and grants; multi-client safety.
3. **Gate 3: obligations as MCP Tasks.** The bounded-obligation engine backs an
   application-owned Tasks store (`ServerBuilder::final_tasks`); `tasks/get|update|cancel`
   project obligation state; cancellation stays request/drain/compensate/finalize.
4. **Gate 4: Streamable HTTP.** Same dfmcp semantics over the facade's HTTP transport, still
   modern-only, localhost-first.
5. **Gate 5 (WP-21): conformance evidence.** The MCP 2026-07-28 conformance suite runs in CI on
   the pinned revision; results recorded in `docs/DOGFOODING_FASTMCP.md`.

## 6. Security posture

- Transport identity grants nothing: capability scopes come from `dfmcp-core` grants negotiated in
  `open_session`, never from the connection.
- No auth/JOSE/OAuth features are enabled; the admitted profile is stdio/localhost-first. Remote
  deployment is out of scope until a transport-boundary admission design exists.
- Untrusted-content rules are unchanged: game text, mod text, and MCP arguments are data; they
  cannot become commands or authority through the transport layer.

## 7. Determinism

The transport is replay-irrelevant: recorded inputs, lab transcripts, and evidence references
fully determine replay regardless of framing. Modern-era request routing may interleave, but every
dfmcp semantic remains anchored, idempotent, and transcript-logged.

## 8. Rollback

Delete `crates/dfmcp-mcp`, drop the `fastmcp-rust` workspace dependency, and revert the
`[mcp_transport]`/`[admitted]` allowlist sections. The scaffold compiles without the transport,
exactly as it did before adoption (ADR-013).
