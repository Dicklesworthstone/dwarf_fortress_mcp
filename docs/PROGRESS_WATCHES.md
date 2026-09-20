# Exact-archive progress predicates

`work_order_progress::watches` evaluates bounded temporal predicates over the
existing progress/1.12 archive. It neither dispatches native work nor supplies
creation reconciliation or completed-goods evidence. This first increment is the
pure evaluator; durable registration/cancellation and MCP routing are separate
integration work. Existing native protocols and admission are unchanged.

A definition seals an ASCII key, native order ID, goal, absolute game-tick deadline,
cadence, required stable samples, archive identity and exact origin record. The
origin must select a present recognized finite wooden-furniture order. Supported
goals are validated, validated-and-active with nonzero remaining, and validated
remaining-at-most a finite threshold. Unknown configuration never satisfies a
predicate. The origin is a baseline and is not itself a stability sample.

Consume EVERY later archived record in order. A false observation interrupts
stability even between cadence points. Positive samples must be separated by the
specified number of game ticks; repeated reads in one tick cannot satisfy a
multi-sample goal. A sample exactly at the deadline may satisfy the goal; otherwise
that record expires it. No sample after the deadline can satisfy. Deadlines never
renew through replay. Cadence is 1..10000 ticks, stability 1..16, and registration
horizon at most 120000 ticks. Impossible minimum sampling schedules are refused.

An unfinished watch terminates explicitly when the selected order disappears,
its recognized configuration changes, its remaining counter increases, or the
archive enters another comparison segment. Missing means outcome unknown, not
completed. A new segment includes server reopening, selection/source changes and
clock/horizon resets; no downtime continuity or cross-segment identity is inferred.
Within one segment, replay, omitted records and contradictory metadata fail closed.
Terminal evidence is immutable. Local cancellation must be ordered after all
observations through its exact pending frontier; it cannot rewrite satisfaction.

The result contains the exact origin/frontier and the bounded set of positive
sample record references. `satisfied_observation` means ONLY that the declared
predicate held at those sampled observations. It is not a continuous-time proof,
an inventory-production count, a canonical-world obligation discharge, current
freshness, or permission to alter the game.

Eleven Rust regression groups are registered, including a 192-schedule truth
oracle inside the Rust tests. They have NOT been compiled or executed because
Rust, Cargo and rustfmt are unavailable. The independent executable reference
`python scripts/check_progress_watch_reference.py` passes 2018 truth schedules,
10 cadence/deadline/discontinuity controls and 144 predicate arithmetic cases.
This is a Python model, not Rust, filesystem, native DFHack or MCP execution.
Source hashes and scoped results are in docs/evidence/progress-watch-reference.json.
