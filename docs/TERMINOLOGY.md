# Terminology

**Action** — one registered semantic mutation step.

**Anchor** — fortress ID, observation cursor, game tick, and canonical state hash naming a state.

**Bridge** — out-of-process DFHack-side service connecting canonical protocol to game APIs.

**Canonical state** — versioned normalized semantic truth used for planning and proof.

**Capability** — explicit authority over an operation class and bounded scope.

**Checkpoint** — verified content-addressed recovery point with save and ledger evidence.

**Commit** — the protocol phase that attempts authorized effects from an exact prepared plan.

**Compensation** — a new action intended to mitigate/reverse effects; not rollback.

**Compatibility manifest** — certified semantics for an exact DF/DFHack/bridge/platform/mod tuple.

**Continuation** — opaque authenticated token for the rest of a bounded result at one target
anchor.

**Cursor** — epoch and monotonically increasing sequence in an observation stream.

**Delta** — ordered changes that transform one exact canonical anchor into another.

**DfQL** — bounded structured semantic query language.

**Effect** — interaction with storage, filesystem, bridge, clock, transport, or other external
state.

**Evidence** — content-digested support for a fact, decision, receipt, transition, or finding.

**Fencing token** — monotonically increasing value that invalidates stale lease holders.

**Fortress lineage** — identity and checkpoint/restore ancestry of a controlled fortress.

**Generation** — anti-ABA component distinguishing reused native entity IDs.

**Heartbeat** — observation response indicating no relevant semantic change at current anchor.

**Idempotency key** — stable identity ensuring same mutation is resumed/returned, not duplicated.

**Indeterminate** — system cannot safely establish whether an effect occurred.

**Interest set** — entities, fields, areas, events, and operation state a session wants observed.

**Lease** — time-bounded ownership/reservation of a semantic write scope.

**Obligation** — owned long-running responsibility to reach/fail/cancel a semantic condition.

**Plan** — immutable, sealed action DAG compiled from intent and source anchor.

**Postcondition** — normalized predicate required to prove action success.

**Prepare receipt** — adapter evidence that exact plan/action was revalidated and staged.

**Projection** — capability- and budget-specific presentation of canonical state.

**Provenance** — source, tick, schema, digest, derivation, confidence, and taint of a fact.

**Rebase** — compile a new plan from same intent against a new anchor.

**Reconciliation** — determine outcome of an indeterminate effect from journals and observations.

**Region** — structured ownership scope for tasks and obligations.

**Revision** — monotonic content version within an entity generation or chunk.

**Risk tier** — read-only, reversible, guarded, or irreversible minimum handling class.

**Semantic probe** — version-specific test of meaning rather than only structure.

**Terminal candidate** — obligation condition observed but not yet stable enough to discharge.

**Taint** — marker that content is untrusted data and cannot provide authority.

**Verified** — semantic postconditions proven from authoritative normalized observation.
