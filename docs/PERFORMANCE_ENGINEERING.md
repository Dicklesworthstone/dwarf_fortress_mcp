# Performance Engineering Doctrine

The goal is world-class performance through representation and algorithms, not memory unsafety or semantic shortcuts.

## 1. Optimization order

1. establish the semantic oracle;
2. measure the complete request path;
3. identify the dominant resource;
4. form one falsifiable hypothesis;
5. add a runtime-selectable experimental arm to the same binary;
6. prove output equivalence before timing;
7. run A/A to measure noise;
8. run A/B distributions under a pinned workload;
9. retain receipts and flamegraph/counter artifacts;
10. promote only if reliability and tail behavior do not regress.

## 2. Primary levers

The preferred levers are:

- immutable structural sharing;
- compact typed IDs and dense projection ordinals;
- sorted small vectors for micro-adjacency;
- temperature-tiered storage;
- delta publication instead of full rebuild;
- factorized query intermediates;
- incremental standing analyses;
- deterministic flat combining for brief commit points;
- sharded ownership with no global hot lock;
- cache-line-conscious layouts in safe Rust;
- request coalescing for identical reads;
- scatter/gather bridge batches;
- bounded zero-copy views over immutable generations;
- work avoidance through interest sets and progressive refinement;
- ATP multi-donor transfer for large artifacts.

## 3. What is measured

Every benchmark reports:

- workload manifest and semantic mode;
- source anchor and input root;
- warm/cold/cache state;
- p50, p95, p99, p99.9, maximum;
- throughput and concurrency;
- peak/resident memory;
- bytes read/written and amplification;
- allocations where measurable;
- bridge round trips;
- observed algorithm operations;
- cancellation and deadline behavior;
- output and decision-witness digests.

Averages alone are not accepted.

## 4. Workload classes

- idle fortress heartbeat;
- active medium fortress observation;
- large mature fortress with dense history;
- map-heavy excavation planning;
- production dependency diagnosis;
- multi-agent disjoint planning;
- adversarial overlapping plans;
- checkpoint under active simulation;
- bridge reconnect and catch-up;
- long-horizon obligation monitoring;
- historical explanation;
- remote evidence/checkpoint transfer.

## 5. Budgets as API

Every operation accepts budgets for CPU steps, wall/virtual time, memory, output bytes/tokens, bridge calls, graph expansions, witness refinements, search candidates, and retries. Budget exhaustion returns a typed partial/continuation result. It never silently switches to an unbounded algorithm.

## 6. Deterministic operation counters

Wall time is machine-dependent. Planning-relevant kernels additionally record operation counters tied to declared complexity. A counter regression can fail qualification even when a faster machine hides it. Counters are versioned and cannot be changed merely to reset a baseline.

## 7. Memory safety and unsafe code

Safe Rust is the baseline. The project first exhausts algorithmic, layout, batching, and ownership improvements. A proposed unsafe island must demonstrate a material residual wall after safe optimization, preserve a bit-identical safe path, and remain outside the default trusted workspace until separately admitted.
