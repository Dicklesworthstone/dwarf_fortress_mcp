# Testing Strategy

## Principle

The project’s hardest failures occur between ordinary success paths: after dispatch but before
receipt, during cancellation, across save/restore, under stale state, and when version assumptions
are almost—but not quite—correct. Tests therefore center on state machines, fault schedules, and
negative evidence.

## Test pyramid

### Pure unit tests

- digest/canonical encoding;
- IDs and scopes;
- field presence;
- predicates;
- delta application;
- plan normalization/sealing;
- capability/risk;
- action/obligation/lease transitions.

### Generated/property tests

- insertion-order independence;
- snapshot/delta reconstruction;
- revision and generation sequences;
- random plan DAGs;
- query bounds;
- canonical round trips.

The project may implement a minimal internal generator before adding a property-testing
dependency.

### Model exploration

Enumerate transitions and injected effect outcomes for:

- action;
- obligation;
- cancellation;
- lease/fencing;
- session shutdown;
- idempotency/recovery.

Safety properties are asserted after every state.

### Differential tests

Reference implementation versus:

- live/full DFHack rescan;
- FrankenSQLite;
- FrankenFS;
- FrankenSearch;
- FrankenMarkdown;
- FrankenGraphDB.

### Golden compatibility tests

Content-addressed fixture fortresses and bridge frames per DF/DFHack/mod manifest.

### Live disposable-fort tests

A fresh copy of a test fortress is checkpointed, mutated, verified, and discarded. Guarded tests
never target an operator’s real save.

### Long-horizon scenarios

Script objectives and external events across seasons, crashes, bridge restarts, index rebuilds,
agent handoffs, and compaction.

## Deterministic effects

Tests inject:

- monotonic/wall/game clocks;
- scheduling/yields;
- bridge frames and delays;
- storage outcomes;
- filesystem outcomes;
- random/planner seeds;
- model responses/manifests where applicable.

No test sleeps to wait for correctness.

## Required fault points

For every effect:

```text
before request
after durable intent
during send
after send before peer receipt
after peer receipt before response
after response before durable receipt
after durable receipt before observation
during observation
after proof before response
during cancellation
during shutdown/recovery
```

Faults include error, timeout, cancellation, duplication, reordering where permitted, corruption,
partial write, process death, and budget exhaustion.

## Core properties

- no verified action without postcondition evidence;
- no duplicate effect under same key;
- conflicting content under same key fails;
- indeterminate never blind-retries;
- stale fence cannot mutate;
- session cannot close with silently orphaned work;
- checkpoint seal precedes guarded effect when required;
- delta gap never silently bridges;
- unknown required field never verifies;
- replay reproduces decision or localizes first divergence.

## Negative-evidence ledger

Each campaign records:

- hypothesis;
- versions and manifest;
- seed/schedule;
- fixture digest;
- result;
- earliest divergence if any;
- coverage limitations;
- artifact location.

Release claims cite these records.

## CI tiers

### Pull request

- format/lint/test;
- schemas and docs;
- deterministic unit/generated seed set;
- no live game.

### Main/nightly

- expanded schedules;
- replay corpus;
- reference differential adapters;
- performance smoke;
- compatibility fixtures.

### Release

- exhaustive configured schedules;
- crash/storage/filesystem campaigns;
- live certified-version matrix;
- long-horizon scenarios;
- security corpus;
- benchmark and negative-evidence publication.

## Flake policy

A nondeterministic test is a correctness defect. Quarantine requires an issue, preserved seed/log,
and scope; it cannot count toward an acceptance gate.
