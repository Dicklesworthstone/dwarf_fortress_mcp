# E09 — Evidence Export for Memory Curation

Mapping specification and templates that turn dfmcp artifacts into
`ee remember --batch --stdin` input. Curated, provenance-carrying entries
only — no automatic ingestion of raw logs.

This document is the contract behind bead `dfmcp-ee-evidence-curation-h7y`.
It is consumed by WP-11 / WP-12 once those work packages land durable
evidence bundles; the laboratory slice in `crates/dfmcp-lab` is the source
today.

## Scope (today)

- Lab transcript events (`dfmcp_lab::LabEvent`).
- Adapter receipts: `PrepareReceipt`, `CommitReceipt`, `CheckpointReceipt`,
  `RestoreReceipt`, `CancelReceipt`, `ActionReceipt`.
- Session outcomes (`OperationOutcome::{Succeeded, Failed, Cancelled,
  Indeterminate}`) emitted by the laboratory slice.

## Scope (deferred to WP-11 / WP-12)

- Durable FrankenSQLite-backed evidence bundles.
- FrankenFS checkpoint manifests.
- FrankenSearch ranked attention evidence.

## Cap (one-way valve)

dfmcp never reads back from ee into its canonical state. ee is a curation
sink and a retrieval aid for the agent loop. The flow is strictly:

```text
dfmcp artifact ─► curated ee batch ─► ee store ─► agent retrieval
       ▲                                                 │
       └─────────────────────────────────────────────────┘
                         (no ingestion)
```

In particular:

- No MCP tool is added that writes to ee.
- No canonical dfmcp state field is populated from an ee read.
- No side effect on dfmcp state is permitted as a side effect of an
  `ee remember` call.
- The `fortress.*` tool surface (WP-13) does not include any
  `fortress_*_memory_*` operation; if a future need arises it must be
  filed as a new bead with explicit acceptance criteria and ledger row.

## Mandatory provenance block

Every batch entry MUST carry a provenance block so ee retrieval can
explain *why* a memory exists, not just *what* it asserts. The template
lint (see `architecture/ee_batch_item.v1.json`) rejects any entry where
these fields are missing or empty.

| Field | Source | Type | Meaning |
|---|---|---|---|
| `fortress_id` | `StateAnchor.fortress_id.get()` | u64 | Fortress the event belongs to. |
| `anchor_epoch` | `StateAnchor.cursor.epoch` | u64 | Observation epoch at the event. |
| `anchor_sequence` | `StateAnchor.cursor.sequence` | u64 | Observation sequence within the epoch. |
| `game_tick` | `StateAnchor.tick.0` | u64 | Game tick at the event. |
| `state_hash` | `StateAnchor.state_hash.to_hex()` | hex string | Canonical state hash at the event. |
| `evidence_bundle_digest` | bundle digest or `Digest32::ZERO.to_hex()` if no bundle | hex string | Content-addressed fingerprint of the bundle that sourced this entry. |
| `artifact_path` | repo-relative path of the producing artifact | string | Where a reviewer can re-derive the entry. |
| `schema_version` | `dfmcp-core` crate version (semver) | string | Which schema version produced the entry. |
| `bridge_protocol_version` | `AdapterIdentity.bridge_protocol_version` | string | Which adapter/bridge produced the entry. |

The `state_hash` is what makes a memory "honest": every entry claims an
exact canonical state. `ee search --explain` returns this field so a
retrieving agent can decide whether the entry is still applicable.

## Mapping dfmcp artifacts → ee kinds

The canonical ee kinds are: `rule`, `fact`, `decision`, `failure`,
`command`, `convention`, `anti-pattern`, `risk`, `playbook-step`. We map
dfmcp events to them as follows. The level (`episodic`, `semantic`,
`procedural`, `working`) defaults to `episodic` and is overridden only
when the entry carries a *durable rule* (then `procedural`) or a
*retrievable fact* (then `semantic`).

### Lab transcript events

| `LabEvent` variant | ee kind | ee level | Rationale |
|---|---|---|---|
| `Observed(StateAnchor)` | `fact` | `semantic` | Authoritative observation; reusable across sessions. |
| `Prepared(PlanId)` | `decision` | `episodic` | Plan sealing event; useful only for the session that produced it. |
| `Committed(PlanId)` | `fact` | `episodic` | Mutation actually applied; evidence of effect. |
| `ActionPolled(ActionId, CommitState)` | `fact` | `episodic` | Polling transcript; intermediate state. |
| `CancelRequested(ActionId, CancelMode)` | `decision` | `episodic` | Cancellation request; precedes `CancelFinalized`. |
| `CancelFinalized(ActionId, CommitState)` | `fact` | `episodic` | Terminal cancellation outcome. |
| `Checkpointed(CheckpointId)` | `fact` | `semantic` | Recovery point; reusable across restores. |
| `Restored(CheckpointId)` | `decision` | `episodic` | Restore chosen over an alternative; typically paired with a `failure` entry. |
| `SnapshotInjected(StateAnchor)` | `decision` | `episodic` | Lab-only fixture injection. Always tag `source.lab`. |
| `TickAdvanced(GameTick)` | `fact` | `episodic` | Clock advance; rarely useful alone; pair with the next event. |

### Adapter receipts

| Receipt | ee kind | Required additional fields |
|---|---|---|
| `PrepareReceipt` | `decision` | `plan_id`, `plan_digest`, `expiry_tick` |
| `CommitReceipt` | `fact` | `plan_id`, `plan_digest`, `paused_after`, `actions[]` |
| `CheckpointReceipt` | `fact` | `checkpoint_id`, `label`, `state_hash` |
| `RestoreReceipt` | `decision` | `checkpoint_id`, `prior_anchor`, `restored_anchor` |
| `CancelReceipt` | `fact` | `action_id`, `final_state` |
| `ActionReceipt` | `fact` | `action_id`, `plan_id`, `step_id`, `state`, `stable_observations` |

### Operation outcomes

| Outcome | ee kind | Notes |
|---|---|---|
| `OperationOutcome::Succeeded(value)` | `fact` | `value` is summarized into the entry `content`; full value never inlined (token budget). |
| `OperationOutcome::Failed(error)` | `failure` | `error.code`, `error.message`, `error.retryable`. |
| `OperationOutcome::Cancelled { final_anchor, reason }` | `decision` | If `reason` references a failed obligation, also emit a paired `failure` entry. |
| `OperationOutcome::Indeterminate { last_anchor, reason }` | `risk` | Negative evidence may *reject* but never certify. |

### Promotion discipline

A curated memory is promotable to `procedural` level only when:

1. it appears in ≥3 sessions (verified via the `fortress_id` +
   `schema_version` provenance block), AND
2. it never coincided with a `failure` entry whose `state_hash` is
   recoverable to the same anchor.

Otherwise it stays `episodic`. Promotion is human-supervised; ee does not
auto-promote.

## Rejection (template lint)

Entries are rejected at template-lint time when:

- any mandatory provenance field is missing or empty;
- `evidence_bundle_digest == "0".repeat(64)` AND the entry's `ee_kind` is
  in `{decision, failure, risk}` AND the entry is **not** tagged
  `source.lab` (those require durable backing unless the source is the
  pre-WP-11/12 lab slice);
- the entry has no `content` field (empty/whitespace only);
- the entry has no `tags` (at minimum one tag from the controlled
  vocabulary below);
- the entry references an `artifact_path` that does not exist in the
  working tree at lint time (warning only until WP-11 lands the durable
  bundle registry).

### Controlled tag vocabulary

Tags are a `BTreeSet<String>` so retrieval filters stay stable across
workspaces. The seed vocabulary:

```
dfmcp.transport    dfmcp.intent       dfmcp.world        dfmcp.lab
dfmcp.adapter      dfmcp.bridge       dfmcp.evidence     dfmcp.failure
dfmcp.cancel       dfmcp.checkpoint   dfmcp.restore      dfmcp.pause
source.lab         source.dwarf       source.bridge
mcp.2026-07-28     mcp.legacy-rejected
```

A new tag may be added in a PR that updates this vocabulary; raw
free-form tags break the retrieval contract.

## Worked example — laboratory transcript → batch

Given a transcript of three `LabEvent`s after a successful unpause:

```text
[0] Observed(StateAnchor { fortress_id: 7, epoch: 3, sequence: 18422, tick: 9120031, state_hash: abcd… })
[1] Prepared(PlanId(0x9f4a…))
[2] Committed(PlanId(0x9f4a…))
```

the curated batch is three JSONL lines, each on its own line
(see `examples/ee_batch/lab_unpause.jsonl`):

```jsonl
{"content":"Observed fortress=7 anchor=3:18422 tick=9120031 state_hash=abcd…","level":"semantic","kind":"fact","tags":["dfmcp.lab","source.lab"],"_meta":{"fortress_id":7,"anchor_epoch":3,"anchor_sequence":18422,"game_tick":9120031,"state_hash":"abcd…","evidence_bundle_digest":"0000000000000000000000000000000000000000000000000000000000000000","artifact_path":"crates/dfmcp-lab/src/lib.rs","schema_version":"0.0.1","bridge_protocol_version":"dfmcp-bridge-v1-lab"}}
{"content":"Prepared plan=0x9f4a… at anchor=3:18422","level":"episodic","kind":"decision","tags":["dfmcp.intent","dfmcp.lab","source.lab"],"_meta":{"fortress_id":7,"anchor_epoch":3,"anchor_sequence":18422,"game_tick":9120031,"state_hash":"abcd…","evidence_bundle_digest":"0000000000000000000000000000000000000000000000000000000000000000","artifact_path":"crates/dfmcp-intent/src/plan.rs","schema_version":"0.0.1","bridge_protocol_version":"dfmcp-bridge-v1-lab"}}
{"content":"Committed plan=0x9f4a… verified actions=1","level":"episodic","kind":"fact","tags":["dfmcp.intent","dfmcp.lab","source.lab"],"_meta":{"fortress_id":7,"anchor_epoch":3,"anchor_sequence":18422,"game_tick":9120031,"state_hash":"abcd…","evidence_bundle_digest":"0000000000000000000000000000000000000000000000000000000000000000","artifact_path":"crates/dfmcp-lab/src/lib.rs","schema_version":"0.0.1","bridge_protocol_version":"dfmcp-bridge-v1-lab"}}
```

The exact JSON object schema is `architecture/ee_batch_item.v1.json`.

## Promotion to procedural — worked

A `playbook-step` example, promoted from repeated successful patterns:

```jsonl
{"content":"After any plan sealed with requires_checkpoint=true, call fortress_checkpoint before fortress_commit; commit fails with CheckpointRequired otherwise.","level":"procedural","kind":"playbook-step","tags":["dfmcp.intent","dfmcp.checkpoint"],"_meta":{"fortress_id":7,"anchor_epoch":3,"anchor_sequence":18422,"game_tick":9120031,"state_hash":"abcd…","evidence_bundle_digest":"<sha256 of bundle dir>","artifact_path":"docs/INTENT_MODEL.md","schema_version":"0.0.1","bridge_protocol_version":"dfmcp-bridge-v1-lab"}}
```

The artifact path is the design doc that recorded the rule; the bundle
digest is the qualified evidence bundle (waits for WP-11/12). Until
then, `evidence_bundle_digest` MUST be the literal 64-zero string and
the entry MUST be tagged `source.lab`.

## Reject example — empty content

This entry MUST be rejected by the lint:

```jsonl
{"content":"","kind":"fact","tags":[]}
```

Reason: missing provenance block, empty content, empty tag set.

## Tests & evidence

This bead delivers the contract. Tooling lives in WP-11/12 (durable
ledger) plus a follow-up E10 (`dfmcp-ee-team-memory-policy-uum`, gated on
WP-15 leases) for the multi-agent sharing policy. The template lint and
the round-trip test land with WP-11/12 — the contract is the precondition.

## Owner / next

The one-way valve is final: ee never grants capability, never mutates
dfmcp state, never appears in the `fortress.*` tool surface.