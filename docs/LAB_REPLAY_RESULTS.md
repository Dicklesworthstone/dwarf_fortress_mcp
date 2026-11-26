# Semantic result verification in laboratory replay

Owner: `df-replay-equality-campaign-test021-c7c`.

New laboratory recordings use `dfmcp.replay.bundle/3`. Replay compares the
recorded outcome, canonical world anchor, request frontier and each tool's
declared result commitment. A changed query answer can therefore fail replay
even when it changes no world state and returns the same success code.

Export `df://session/{id}/replay`, assemble any digest-bound resource pages, and
run `dwarf-fortress-mcp replay bundle.json`. The command stops at the earliest
divergence and exits nonzero. A result mismatch names `result_commitment` and
includes the expected and observed profile/digest. The complete transcript and
all commitment shapes are checked before a disposable session is opened.

## What the result profile covers

The initial `dfmcp.replay.tool-payload/1` profile commits to the root tool payload.
Object keys are sorted explicitly and array ordering is retained. This covers
actual returned data, field presence, projections, coverage and absence claims,
query continuations expressed as ordinary objects, plan review/forecast details,
structured errors and output truncation decisions.

| Tool or result | Comparison |
| --- | --- |
| Observe with ordinary original-goal projections | Semantic payload |
| Summary, entities, observation, terrain, path, search and structured queries | Semantic payload when no excluded result shape is present |
| Plan reviews | Semantic payload |
| Numeric entity Explain | Semantic payload |
| Doctor | Semantic payload with its process-wide session count treated as telemetry |
| Open, Commit, Wait, Cancel, Checkpoint and Restore | Outcome, anchor and request frontier |
| Stateful query modes, process-local query work, opaque returned continuations, withdrawal proof projections and non-entity Explain | Explicit outcome-only result profile |

Agent Turn presentation and process-local Task monitoring are outside this
profile. Three typed unsigned operational measurements are tagged: query
remaining wall allowance, full-output token estimate and Doctor's active session
count. Their absence, null or malformed values remain distinguishable. Declared
own-session identity fields and entity resource URI owner prefixes are normalized
without changing foreign identities or similarly named user data. There is no
recursive removal of fields named `session_id`, `resource_uri` or budget names.
The presentation-only `full` and `compact_turn` output tiers compare as an
intact tool payload. Omission metadata referring only to Agent Turn fields is
excluded; other output tiers, omitted tool fields and coverage remain committed.

Unsupported, malformed or over-bound result shapes use
`dfmcp.replay.outcome-anchor/1`, with a null digest and an explicit fixed reason.
Changing a recorded entry's profile does not disable verification: replay
recomputes and compares the complete commitment, including its profile and reason.
The successful report's `verification` object gives `semantic_result_calls`,
`outcome_only_calls`, `task_drain_events` and every excluded entry with its reason.
A green result must be interpreted with those coverage fields.

## Compatibility and bounds

Version-one and version-two archives retain their original schemas, calls-hash
domains and comparison behavior. A replayed legacy archive re-exports in that
same generation; missing historical result hashes are never synthesized as
evidence. Reports label legacy verification `outcome_and_anchor`. Version 3
retains the existing Task drain event witnesses and effect checks without
recreating Task identities or granting mutation authority.

The new calls hash uses `dfmcp-replay-bundle-calls/3`, then a NUL byte and the
compact canonical entry array. Result hashes use `dfmcp-replay-tool-payload/1`,
then a NUL byte and canonical JSON containing the tool name and normalized
result. Only the small commitment is retained, not another copy of the payload.
Per-result work is bounded to 1 MiB, 65,536 JSON nodes and depth 64. Transcript
bounds remain 4,096 entries and 32 MiB; shared, durable, truncated or incompletely
ordered histories remain nonreplayable.

`scripts/lab_replay_roundtrip.py` records an entity query and excavation waits,
verifies the original export, then independently changes the query limit and a
wait duration. It recomputes the calls checksum so each failure must come from
execution comparison, localizing the changed read to `result_commitment` and the
changed wait to `anchor_after`. `--output bundle.json` saves the untouched verified
original. Execution evidence and remaining full-MCP limits are recorded in
`IMPLEMENTATION_STATUS.md`; this profile does not complete the broader fourteen
domain replay campaign or establish live-game determinism.
