# Schema registry

| Schema ID | Current version | Compatibility | Canonical encoding | Location |
|---|---:|---|---|---|
| `dfmcp.protocol` | `0.1.0` | negotiate major; preserve unknown optional fields | JSON for MCP; canonical semantic digest | `schemas/dfmcp.schema.json` |
| `dfmcp.tool.open_session.input` | `0.1.0` | additive optional fields | JSON | `schemas/open_session.input.schema.json` |
| `dfmcp.tool.observe.input` | `0.1.0` | additive optional fields | JSON | `schemas/observe.input.schema.json` |
| `dfmcp.tool.query.input` | `0.1.0` | additive optional fields | JSON | `schemas/query.input.schema.json` |
| `dfmcp.tool.plan.input` | `0.1.0` | action-registry negotiated | JSON | `schemas/plan.input.schema.json` |
| `dfmcp.tool.commit.input` | `0.1.0` | frozen mutation identity | JSON | `schemas/commit.input.schema.json` |
| `dfmcp.tool.wait.input` | `0.1.0` | additive optional fields | JSON | `schemas/wait.input.schema.json` |
| `dfmcp.tool.cancel.input` | `0.1.0` | additive optional fields | JSON | `schemas/cancel.input.schema.json` |
| `dfmcp.tool.checkpoint.input` | `0.1.0` | additive optional fields | JSON | `schemas/checkpoint.input.schema.json` |
| `dfmcp.tool.restore.input` | `0.1.0` | frozen checkpoint identity | JSON | `schemas/restore.input.schema.json` |
| `dfmcp.tool.explain.input` | `0.1.0` | additive optional fields | JSON | `schemas/explain.input.schema.json` |
| `dfmcp.tool.doctor.input` | `0.1.0` | additive optional fields | JSON | `schemas/doctor.input.schema.json` |
| `dfmcp.bridge` | `v1` | protobuf package/version negotiation | deterministic protobuf where signed | `proto/dfmcp.proto` |
| `dfmcp.ledger.frame` | planned `1` | migration registry | length-delimited canonical frame | phase-two design |
| `dfmcp.replay.bundle` | planned `1` | reader supports prior majors | sealed manifest + content-addressed blobs | phase-five design |

## Evolution rules

1. Required fields are not added inside an existing schema major.
2. Unknown optional fields are preserved by durable envelopes when feasible and ignored only when
   their declared criticality is false.
3. Unknown enum values at a mutation boundary fail compatibility closed.
4. Identity, digest, idempotency, authority, and risk fields cannot be silently defaulted.
5. Canonical-digest coverage is declared per schema and tested with field-mutation vectors.
6. Retired schema IDs remain reserved.
