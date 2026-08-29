# Test-family registry

Every family produces positive assertions and a negative-evidence record listing fault hypotheses,
seeds/schedules, and artifacts.

| ID | Family | Minimum phase-zero/production evidence |
|---|---|---|
| TEST-001 | canonical encoding | insertion-order, platform, and round-trip equality |
| TEST-002 | identity/ABA | deletion, ID reuse, generation mismatch, stale reference |
| TEST-003 | presence/provenance | unknown/omitted/unsupported/null/stale distinction |
| TEST-004 | snapshot/delta algebra | full→delta equivalence, gap/fork/duplicate/continuation rejection |
| TEST-005 | graph integrity | endpoint, revision, edge, aggregate consistency |
| TEST-006 | query semantics | predicate truth tables, budgets, deterministic ordering |
| TEST-007 | intent normalization | equivalent-input equal digest; malformed input rejection |
| TEST-008 | risk/capability lattice | no authority amplification; risk monotonicity |
| TEST-009 | plan sealing | every covered-field mutation invalidates preparation |
| TEST-010 | idempotency | duplicate same-content success; conflicting-content rejection |
| TEST-011 | action transitions | exhaustive legal/illegal transition table |
| TEST-012 | obligations | completion, blocked, timeout, failure, cancellation, stable observations |
| TEST-013 | checkpoint ordering | no guarded dispatch before durable proof |
| TEST-014 | bridge decoder | fuzz lengths, nesting, variants, truncation, corruption |
| TEST-015 | bridge commit reconciliation | dropped request/receipt, duplicate, partial batch, lookup |
| TEST-016 | crash consistency | kill at every durable transition and recover honestly |
| TEST-017 | cancellation | request at every await/effect boundary; no owned-work leaks |
| TEST-018 | concurrency/leases | stale fences, overlap, commutative actions, starvation |
| TEST-019 | compatibility | supported matrix, unknown probes, read-only degradation |
| TEST-020 | prompt/taint security | hostile names, descriptions, imported knowledge, tool arguments |
| TEST-021 | deterministic replay | equal outputs across schedules/platforms for recorded transcript |
| TEST-022 | performance budgets | token/latency/memory envelopes with regression gates |

## Release gate minimum

A production release requires all deterministic unit/property/model tests, bridge contract tests for
each supported compatibility tuple, crash and cancellation matrices, a bounded schedule-exploration
campaign, replay of the standing incident corpus, and a signed negative-evidence ledger. Passing a
single happy-path game session is not sufficient evidence.
