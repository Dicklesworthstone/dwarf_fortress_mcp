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
| `dfmcp-world-snapshot-v1` | `1` | existing bytes unchanged; strict bounded decoder | framed ordered canonical state; SHA-256 | `crates/dfmcp-world/src/model.rs`, `canonical_decode.rs` |
| `dfmcp-state-delta-v1` | `1` | existing bytes unchanged; exact-base application | framed ordered changes; strict canonical round-trip | `crates/dfmcp-world/src/delta.rs`, `canonical_decode/delta.rs` |
| `dfmcp-profiled-snapshot-v1` | `1` | reject unknown version/profile; preserve opaque optional extensions | source/profile/provenance plus projected canonical state; envelope SHA-256 | `crates/dfmcp-world/src/completeness.rs` |
| `dfmcp-profiled-capsule-v1` | `1` | exact profiled base; unchanged profile/provenance/extensions; same epoch | bound source and projected anchors, envelope identities, publication tick, and v1 delta | `crates/dfmcp-world/src/completeness.rs` |
| `dfmcp.ledger.frame` | planned `1` | migration registry | length-delimited canonical frame | phase-two design |
| `dfmcp.replay.bundle` | planned `1` | reader supports prior majors | sealed manifest + content-addressed blobs | phase-five design |

## Canonical observation envelopes

The four canonical observation rows use their literal encoding domain tags as schema identifiers.
Adding the profiled envelopes and strict delta reader does not change existing snapshot/delta v1
bytes. The snapshot and delta readers bound input to 256 MiB; profile envelopes have a 16 MiB
encoding and decoding bound, at most 64 opaque optional extensions totaling 128 KiB of names and
payloads, a 256-byte source schema name, and the existing 64-level value nesting bound.

The five profile identities are `control-minimum`, `operations`, `spatial`, `historical`, and
`research-full`. Their precise inclusion policy is documented in `docs/WORLD_MODEL.md`. Envelope
identity covers the declared source schema and manifest, profile, source anchor, projected state,
and extensions. Profiled deltas require an exact base and reconstructed target identity; generic
delta decoding still requires separate exact-base application for transition validation.

Profile inclusion does not prove complete observed domains or action authority. Reduced
projections carry a distinct projected hash and original source anchor; independently retained
source verification is required to authenticate a source claim. A nonzero manifest digest does not
admit a live source. Optional extension bytes remain covered by identity even when their meaning
is unknown to a reader.

## Evolution rules

1. Required fields are not added inside an existing schema major.
2. Unknown optional fields are preserved by durable envelopes when feasible and ignored only when
   their declared criticality is false.
3. Unknown enum values at a mutation boundary fail compatibility closed.
4. Identity, digest, idempotency, authority, and risk fields cannot be silently defaulted.
5. Canonical-digest coverage is declared per schema and tested with field-mutation vectors.
6. Retired schema IDs remain reserved.
