# World-State MVCC, Witnesses, and Semantic Transactions

This document defines the concurrency model for observation, query, planning, and external game effects. It is normative where it uses **MUST** or **MUST NOT**.

## 1. The mismatch the model must resolve

Dwarf Fortress mutates independently of the MCP server. The server cannot lock the simulation while an agent reasons for seconds or minutes, and it cannot roll back arbitrary game effects as if the game were a database. Nevertheless, agents need stable reads, retry safety, concurrent planning, historical explanation, and exact knowledge of when their assumptions became stale.

The solution is to separate two domains:

- **authoritative observed history**, which is fully versioned and transactionally published by the server;
- **external game effects**, which use prepare, final precondition checking, fenced dispatch, observation, and proof.

Serializable ledger state does not imply transactional control of Dwarf Fortress. The API never makes that false claim.

## 2. State anchor

A state anchor is the minimum complete identity of a readable world version:

```text
StateAnchorV2 {
    fortress_lineage: FortressLineageId,
    observation_epoch: u64,
    snapshot_sequence: u64,
    game_tick: Option<u64>,
    bridge_generation: u64,
    adapter_epoch: u64,
    schema_epoch: u64,
    policy_epoch: u64,
    world_root: Digest32,
}
```

`observation_epoch` changes after restore, fortress reload, incompatible bridge restart, or any event that makes prior cursors ambiguous. Sequence numbers are monotone only within an epoch. `world_root` seals canonical semantic content, not process-local representation.

A query may ask for a specific anchor, a bounded staleness interval, or the newest fully published anchor. It may not read “whatever each subsystem currently has.”

## 3. Observation capsule

The canonical append unit is an `ObservationCapsule`:

```text
ObservationCapsule {
    basis: StateAnchorV2,
    successor: StateAnchorV2,
    source_receipt: BridgeReadReceipt,
    entity_deltas: ordered list,
    relation_deltas: ordered list,
    spatial_deltas: ordered list,
    event_deltas: ordered list,
    completeness_changes: ordered list,
    provenance: ordered list,
    diagnostics: ordered list,
}
```

Capsules are immutable and domain-separated. Applying the ordered capsules from an anchor MUST reproduce the successor root exactly. A full snapshot is an anchor plus a materialized projection; it is not a second source of truth.

## 4. Publication protocol

Observation publication is three-phase:

1. **Reserve.** Allocate the successor sequence and unpublished generation identity; pin the basis root and bridge receipt.
2. **Materialize.** Validate identity generations, normalize deltas, create version rows and derived invalidation notices, compute child hashes, and persist all children.
3. **Publish.** Atomically expose the successor root and high-water mark.

Readers can see either the basis or successor, never an intermediate mixture. If materialization fails, the reservation is aborted or tombstoned. Sequence reuse is forbidden when it could confuse replay.

## 5. Semantic version chains

Entity and relation versions use stable external identity plus generation. A DF numeric ID without generation is never sufficient because IDs may be reused across reloads or lifecycles. Each logical record has:

- external identity and generation;
- `valid_from` anchor sequence;
- optional `valid_to` sequence;
- semantic revision;
- field-presence map;
- provenance references;
- canonical value digest.

Spatial state is versioned by chunk and tile-mask delta. Large unchanged chunks are structurally shared. Historical reads compose the nearest retained anchor with forward deltas.

## 6. Read witnesses

A plan’s read set is a set of semantic witnesses. The principal forms are:

### 6.1 Positive entity witness

The plan relied on entity `E`, generation `g`, revision `r`, with specified fields and presence states. Any generation change conflicts. A revision change conflicts only if it touches a witnessed field or a declared invariant domain.

### 6.2 Relation witness

The plan relied on an edge key, adjacency row, or bounded relation predicate. The witness names direction, relation family, endpoint generations, and a digest or interval over the relevant adjacency.

### 6.3 Spatial witness

The plan relied on exact tiles, a cuboid, a path corridor, or a chunk aggregate. The witness includes chunk generation, revision, and a compact mask. Coarse chunk conflicts may be refined to masks under budget.

### 6.4 Aggregate witness

The plan relied on a quantity such as available logs, beds, drink, power, labor, or reachable stock. An aggregate witness names its contributor domain and aggregation policy, not only the final number. This prevents an unchanged total from hiding a changed composition that violates constraints.

### 6.5 Negative-domain witness

The plan relied on absence: no hostile, no occupant, no conflicting designation, no forbidden item, no active job, or no blocking relation. The witness names the bounded domain whose completeness was sufficient to establish absence. Unknown or partially observed domains cannot yield a definitive negative witness.

### 6.6 Epoch witness

Every plan implicitly witnesses adapter, schema, policy, action-registry, capability, and compatibility epochs. A change invalidates plans unless the registry explicitly proves compatibility.

## 7. Write witnesses

A write witness describes the semantic conflict footprint of a plan step:

- entity fields or state families;
- relation keys or adjacency domains;
- tile masks and construction/designation domains;
- resource reservation domains;
- clock, checkpoint, restore, and global configuration domains;
- obligation ownership and lease domains.

Write footprints are conservative. An action-family implementation may refine them after compilation, but it cannot omit a possible effect to improve concurrency.

## 8. Hierarchical conflict index

Witnesses are indexed at multiple levels. Each level has a domain-separated digest and no-false-negative requirement:

```text
L0 fortress/global
L1 semantic domain
L2 region, entity kind, or relation family
L3 chunk, entity, or adjacency row
L4 tile mask, field, edge key, or resource interval
```

Coarse overlap means “possibly conflicts.” The coordinator may request finer proof. Missing detail, budget exhaustion, corrupt witness data, or unsupported refinement yields a conservative conflict.

## 9. Refinement economics

Refinement is selected by expected value of information:

```text
VOI = P(disjoint | coarse overlap) × avoided_replan_cost
      - refinement_cpu
      - refinement_latency
      - additional_memory
```

The policy is deterministic for a pinned policy epoch. Hard ceilings bound work. The baseline path never depends on statistical correctness: choosing not to refine can only reject safe concurrency.

## 10. Plan transaction states

```text
Draft
→ Compiled(anchor, intent_digest)
→ Prepared(plan_digest, witnesses, leases, expiry)
→ CommitCandidate(revalidation_anchor)
→ DispatchReserved(effect_ids)
→ Dispatched
→ Observed | Indeterminate
→ ProvenSucceeded | ProvenFailed | Compensated | AbandonedIndeterminate
```

A plan can return to `Compiled` by deterministic rebase. It cannot reuse a prepare token after witness, lease, epoch, or expiry changes.

## 11. Revalidation

Commit revalidation proceeds in this order:

1. authenticate request and plan digest;
2. validate capability scope and risk tier;
3. validate lease incarnation and fencing token;
4. validate all epochs;
5. choose one fully published revalidation anchor;
6. compare read and write witnesses;
7. refine possible conflicts under budget;
8. detect dangerous dependency structures;
9. re-evaluate hard preconditions;
10. reserve idempotency/effect records;
11. issue a short-lived dispatch ticket.

The bridge performs another bounded check of bridge-visible preconditions on the game thread. A mismatch returns `PreconditionChanged`, not a partial success.

## 12. Dependency graph and serializability

Committed and in-flight plan transactions form a dependency graph with write-read, read-write, and write-write edges. The coordinator prevents forbidden cycles and dangerous structures. The graph is bounded by retention and active-plan windows.

The system distinguishes:

- conflicts among server plans;
- changes caused by ordinary game simulation;
- changes caused by human or mod actions;
- ambiguous changes whose origin is unknown.

All can invalidate assumptions. Only server-plan conflicts participate in deterministic plan ordering; external changes force revalidation or reconciliation.

## 13. Semantic rebase and merge

Rebase never edits serialized plan bytes. It recompiles intent against a new anchor while preserving intent identity and declared constraints. It produces a new plan identity and a `RebaseCertificate`:

```text
old_plan
old_anchor
new_anchor
changed_witnesses
reused_steps
recompiled_steps
dropped_steps
new_constraints
canonical_decision_path
new_plan_digest
```

For concurrent compatible plans, merge follows the ladder:

1. exact replay of both intents;
2. stable-key structural composition;
3. registered commutative action composition;
4. explicit ordering with proof that constraints survive;
5. reject and replan.

A merge certificate contains the canonical normal form. Unknown commutativity is conflict, not permission.

## 14. External effect journal

For each effect the ledger persists, in order where possible:

- effect identity and idempotency key;
- plan and step digest;
- dispatch ticket and expiry;
- bridge generation and operation token;
- request bytes digest;
- durable “dispatch attempted” marker;
- bridge acceptance receipt;
- operation-lookup observations;
- world observations attributed to the effect;
- terminal predicate evidence;
- compensation or reconciliation actions.

A process crash after “dispatch attempted” but before an acceptance receipt creates `UnknownDispatchOutcome`. Recovery queries by operation identity. It does not resend unless the action family proves retry safety.

## 15. Branches and counterfactuals

A branch is:

```text
BranchManifest {
    branch_id,
    basis_anchor,
    owner,
    hypothetical_capsules,
    policy_epoch,
    root_digest,
}
```

Branches structurally share immutable state. They support alternative layouts, production chains, burrow policy, and military plans. Results are labeled hypothetical. To act, the branch emits a semantic intent that is compiled against live state.

## 16. Garbage collection and retention

Retention operates on reachability from:

- current root;
- pinned readers;
- active/prepared plans;
- obligations and evidence;
- checkpoints;
- branches;
- audit retention policy;
- replica/ATP transfer journals.

Versions are reclaimed only after no retained root can reach them. Reclamation produces a receipt with cutoffs and surviving roots. A time-travel query beyond retention returns a precise `HistoryUnavailable` interval.

## 17. Required tests

The minimum campaign includes:

- snapshot plus deltas equals full snapshot;
- ID reuse and ABA adversaries;
- negative-read phantom insertion;
- disjoint tile-mask concurrent plans;
- aggregate unchanged but contributors changed;
- schema/adapter epoch invalidation;
- rebase determinism;
- merge normal-form determinism;
- dangerous-structure rejection;
- crash at every publication and effect-journal boundary;
- restore invalidates old cursors, plans, and views;
- refinement exhaustion never misses a true conflict;
- branch isolation and live-intent recompilation;
- long reader under compaction and retention pressure.
