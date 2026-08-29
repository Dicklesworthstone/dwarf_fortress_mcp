# Performance and Economic Budgets

All figures are targets until a versioned benchmark report exists.

## Reference dimensions

Every benchmark records:

- source commit;
- DF/DFHack/bridge manifests;
- fortress fixture digest and profile;
- hardware/OS;
- runtime configuration;
- enabled Franken adapters;
- number of sessions/obligations;
- cold/warm state;
- token estimator/model manifest.

## Default operation budget

```text
wall time            2,000 ms
game ticks           10,000
entities              2,000
decoded bytes         4 MiB
output tokens         1,500
action steps             64
graph depth                4
continuation pages        32
```

Tools can negotiate lower or policy-approved higher bounds. No unlimited sentinel exists.

## Latency targets

| Operation | p50 | p99 |
|---|---:|---:|
| same-anchor heartbeat | 2 ms | 10 ms |
| relevant delta, warm | 8 ms | 25 ms |
| indexed entity query | 15 ms | 50 ms |
| static ≤64-step plan | 25 ms | 100 ms |
| ledger action transition | 2 ms | 15 ms |
| local bridge health | 5 ms | 30 ms |

Bridge/game-thread reads are reported separately so server work is not hidden.

## Token targets

| Projection | Default target |
|---|---:|
| heartbeat | ≤150 |
| normal delta | ≤500 |
| situation summary | ≤1,500 |
| plan overview | ≤2,000 plus drill-down |
| action progress | ≤400 |
| doctor summary | ≤2,000 plus bundle |

The renderer prioritizes continuity, safety, action state, and uncertainty. It returns continuation
rather than malformed truncation.

## Memory targets

Initial local server, excluding optional search/vector indexes:

- idle ≤150 MiB RSS;
- mature-fort canonical hot set ≤500 MiB;
- per idle session ≤1 MiB;
- no unbounded transcript or continuation retention.

These targets may change after real field-volume measurement.

## Storage targets

- periodic compacted snapshots;
- deltas/events compressed and retention-managed;
- map chunks content-deduplicated;
- terminal idempotency/evidence retained per audit policy;
- derived indexes separately budgeted and rebuildable;
- checkpoint amplification reported as physical and logical bytes.

## Bridge budgets

- max frame negotiated with hard server ceiling;
- union read batching;
- no more than policy-defined game-thread milliseconds per frame;
- obligation polling event-driven where possible;
- large map scans chunked and resumable;
- writes never automatically retried by transport.

## Backpressure

When overloaded:

1. coalesce compatible reads;
2. delay low-priority eventual indexes;
3. lower observation frequency;
4. return continuations;
5. deny new plans above budget;
6. pause under explicit emergency clock policy.

Mutation receipts, cancellation, and terminal evidence remain high priority.

## Benchmark success

A fast result is invalid if it:

- omits required facts without marking;
- changes ordering/digest;
- weakens freshness;
- skips postcondition proof;
- drops evidence;
- increases duplicate/indeterminate risk;
- shifts unbounded work into background.
