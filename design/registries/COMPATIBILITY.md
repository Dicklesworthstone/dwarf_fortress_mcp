# Compatibility registry

Compatibility is established by negotiation plus semantic probes, never by version-string optimism.

## Compatibility tuple

A session records:

- Dwarf Fortress distribution and exact version/build identity;
- DFHack exact version/commit and ABI/protocol identifiers;
- bridge plugin protocol and schema digest;
- MCP negotiated protocol version and transport;
- `dwarf_fortress_mcp` build and registry digests;
- operating system/architecture where behavior is relevant;
- successful, failed, and unavailable semantic probes.

## Modes

| Mode | Observation | Query | Mutation | Entry rule |
|---|---|---|---|---|
| `verified_read_write` | yes | yes | allowlisted verified actions | all required probes pass |
| `verified_read_only` | yes | yes | denied | read probes pass; mutation probes unavailable/fail |
| `diagnostic` | bounded raw/metadata only | limited | denied | identity known but semantics incomplete |
| `unsupported` | health/version only | no | denied | no safe interpretation |

## Probe classes

- identity uniqueness and generation behavior;
- pause/tick monotonicity;
- map coordinate/bounds semantics;
- unit/job/building/work-order field presence and enum mappings;
- event overlap identity;
- prepare-without-effect guarantee;
- mutation idempotency lookup;
- save/checkpoint quiescence and restore visibility;
- game-thread execution constraints.

## Support record lifecycle

A tuple moves from experimental to verified only after golden fixtures, live probes, mutation
round-trips in disposable saves, fault tests, and replay artifacts pass. A DF/DFHack change starts as
unknown. Read-only support may be admitted separately. Known regressions can quarantine individual
action families without disabling safe observation.
