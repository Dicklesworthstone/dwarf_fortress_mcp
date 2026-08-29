# Invariant registry

**Status:** frozen for the phase-zero contract.
**Normative source:** Part III of the comprehensive plan.

Each invariant must be enforced by at least two independent mechanisms and eventually by all
applicable layers: type construction, pure transition checks, effect-boundary revalidation, and
evidence-producing tests.

| ID | Invariant | Primary enforcement | Required evidence |
|---|---|---|---|
| INV-001 | Every authoritative world view has one fortress ID, epoch, sequence, game tick, and canonical state hash. | `StateAnchor`; snapshot constructors | anchor round-trip and corruption tests |
| INV-002 | State hashes cover canonical semantic bytes, never incidental ordering or transport framing. | canonical encoder | permutation/metamorphic tests |
| INV-003 | Entity identity includes a type domain and anti-ABA generation. | `EntityId` and adapter identity map | delete/recreate replay tests |
| INV-004 | Names, labels, titles, and coordinates are never primary keys. | schema and query types | rename/move tests |
| INV-005 | Unsupported, unknown, omitted, redacted, stale, null, and absent are distinguishable. | presence algebra | projection and compatibility tests |
| INV-006 | Externally meaningful facts retain source, observation tick, compatibility state, and source digest. | provenance records | evidence-chain verification |
| INV-007 | Indexes, summaries, scores, embeddings, and notes cannot overwrite canonical facts. | storage namespaces | projection corruption tests |
| INV-008 | A delta applies only to its exact base cursor and state hash. | delta validator | gap, fork, replay, and duplicate tests |
| INV-009 | Restore or non-resumable reset starts a new epoch and requires a full snapshot. | session state machine | restore/reset model tests |
| INV-010 | Partial reads are explicitly truncated and use integrity-protected continuations. | observation envelope | truncation boundary tests |
| INV-011 | Any content change advances the corresponding revision. | canonical update API | mutation property tests |
| INV-012 | Canonical edges never dangle unless explicitly typed external. | graph validator | endpoint deletion tests |
| INV-013 | Event identity is stable across polling overlap. | event key registry | overlapping-window dedupe tests |
| INV-014 | Every observation has entity, byte, time, depth, and token limits. | `WorkBudget` | adversarial budget tests |
| INV-015 | Any covered change to a prepared plan changes its digest and invalidates preparation. | sealed plan digest | field-flip mutation tests |
| INV-016 | State, authority, leases, and checkpoint policy are revalidated immediately before effect. | coordinator + adapter | stale-anchor race tests |
| INV-017 | Every mutation step has a stable idempotency key; conflicting reuse fails. | ledger uniqueness | duplicate/conflict crash tests |
| INV-018 | Adapter acceptance is not semantic completion. | action state machine | delayed-postcondition tests |
| INV-019 | Every mutation step has registered semantic postconditions. | action registry/compiler | registry completeness check |
| INV-020 | Work that may finish later owns a bounded obligation. | plan verifier | missing-bound rejection tests |
| INV-021 | Unknown effect outcome becomes `indeterminate`; automatic duplicate retry is forbidden. | outcome algebra | dropped-receipt reconciliation tests |
| INV-022 | Context may raise but never lower registry minimum risk. | risk classifier | monotonicity property tests |
| INV-023 | Required durable checkpoint proof precedes the first guarded effect. | coordinator | crash/order trace tests |
| INV-024 | Compensation is a new authorized action, not fictional rollback. | compensation compiler | stale/denied compensation tests |
| INV-025 | Terminal records are immutable except append-only evidence or explicit supersession. | ledger schema | tamper and migration tests |
| INV-026 | Every task, bridge call, timer, lease, action, and obligation has structured ownership. | asupersync region tree | leak/orphan checks |
| INV-027 | Cancellation requests, stops, drains, reconciles/compensates, then finalizes. | cancellation state machine | cancellation-at-every-await tests |
| INV-028 | Every leased mutation carries the current fencing token. | lease validator | stale-holder tests |
| INV-029 | Overlapping write scopes require serialization or a registered commutativity rule. | conflict detector | schedule exploration |
| INV-030 | Delegation only narrows capability, scope, expiry, risk, uses, and budget. | grant constructor | lattice property tests |
| INV-031 | Pause/advance requires an explicit clock lease. | capability + lease checks | concurrent clock tests |
| INV-032 | Durable intent and idempotency identity precede bridge dispatch. | write-ahead ledger | crash-point tests |
| INV-033 | Bridge receipts are durable before being represented as durable. | receipt transaction | power-loss tests |
| INV-034 | Recovery never infers success from outbound-request presence alone. | recovery state machine | request/receipt gap tests |
| INV-035 | Durable frames, snapshots, deltas, continuations, manifests, and bundles are checksummed. | frame formats | bit-flip tests |
| INV-036 | Failed schema migration leaves the old ledger readable or seals recovery. | migration protocol | kill-at-step tests |
| INV-037 | Compaction preserves current anchors, idempotency, terminals, active obligations, and audit policy. | compaction verifier | pre/post proof comparison |
| INV-038 | Default surfaces expose no arbitrary Lua, shell, command strings, memory writes, or paths. | MCP schema + bridge allowlist | protocol fuzzing |
| INV-039 | Tainted game/imported text cannot grant authority or select executable actions. | typed parser + taint markers | prompt-injection corpus |
| INV-040 | Unknown required fields, enum variants, offsets, or semantic probes fail closed for mutation. | compatibility gate | future-variant tests |
| INV-041 | Untrusted lengths, nesting, coordinates, strings, and collections are bounded before allocation. | decoders | fuzz/resource tests |
| INV-042 | Protocol, schema, adapter, DF, and DFHack negotiation precedes session operations. | handshake state machine | out-of-order tests |
| INV-043 | Unsupported mutations degrade to read-only when observation remains safe. | compatibility modes | degraded-mode tests |
| INV-044 | Human summaries cannot substitute for digests and source references. | evidence schema | summary-tamper tests |
| INV-045 | Core clocks, randomness, storage, filesystem, bridge I/O, and scheduling are injected. | effect traits | deterministic lab replay |
| INV-046 | Externally visible unordered collections have canonical order. | encoders | randomized insertion tests |
| INV-047 | Equal canonical inputs and effect transcripts yield equal anchors, plans, scores, errors, and transitions. | replay engine | cross-run equality tests |
| INV-048 | Tests advance virtual/injected time; no correctness test sleeps. | lint and lab clock | source scan + suite policy |
| INV-049 | Each effect boundary is tested for failure, timeout, cancellation, duplication, delay, and corruption where applicable. | fault matrix | matrix coverage report |
| INV-050 | Release gates preserve a negative-evidence ledger with hypotheses, seeds, and artifacts. | release tooling | signed gate bundle |

## Change rule

An invariant may be strengthened in place. Weakening or removing one requires a versioned design
decision, migration impact analysis, adversarial review, and a major protocol revision.
