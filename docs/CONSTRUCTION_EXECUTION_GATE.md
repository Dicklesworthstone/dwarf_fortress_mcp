# Serial construction before the next original furniture delivery

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-franken-storage-mvcc-54h.4`.

## Execution behavior

A feasible ordering of placement *registrations* is not a physical delivery
ordering when previous hauling/construction jobs are still running. The default
`route_furniture_batch.acquire_frontier` now composes its existing mandatory
route/suffix check with current original-prefix construction quiescence. It
uses the SAME complete operations capture already acquired for routing, with no
extra RPC or credential. Every preceding original furnishing must be installed
at its original target and have no held or item-linked work before another step
can receive a ready execution frontier. This deliberately serializes delivery
within a batch, including independent steps; it is not a parallel-job scheduler.

The existing receipt, source, item, route and remaining-order validations still
run first. `construction_frontier.gate_frontier` verifies that envelope and
binds the exact operations hash, tick, ID horizons and every predecessor's key,
receipt digest and plan digest. A changed subset or a different observation
cannot become the current construction predicate for a valid route report.

The complete original plan, review, batch roots and native receipts are unchanged.
The composed policy is `original-route-delivery-and-quiescent-construction-prefix/3`.
Its evidence digest covers both the old route result and the current construction
report, so old route-only confirmation seals are invalid. Computational allowance
fields are excluded from semantic evidence only; their actual work remains charged.

## Outcomes and normal commands

Use the existing `review` / `advance` commands and original batch ID. No flag can
skip the default predecessor gate. The fresh review now reports:

- `ready`: all existing route checks pass and the original construction prefix
  is quiescent (or there are no predecessors);
- `construction_pending`: an earlier construction/hauling or other held job is
  still present, suspended, or incomplete;
- `construction_unestablished`: an original object is missing, changed, not
  verifiably installed, or otherwise fails source/identity requirements;
- `construction_blocked`: removal of an original predecessor is pending.

Existing `route_blocked` and `route_unestablished` results remain unready even
when construction checks pass. The nested `construction_prefix` retains all
current findings, not just the first blocker. A pending review supplies neither
`expected_plan` nor `confirm_review`, and `advance_allowed` is false. Agent Turn
briefing exposes both execution and predecessor-construction status. Offline
`inspect` remains a custody/history report, not a fresh readiness observation.

`advance` calls the same gate before creating a child intent and again after the
existing coordinator's dispatch-intent synchronization but before native commit.
A changed or failed final capture cannot fall back to an earlier positive result.
The pending original key remains pending; the existing query/cancel recovery
rules still apply. There is no redispatch, replacement selection or new key.

A new review may become ready after the game completes its original work. This
change does not advance time, pause/unpause, create a polling loop, or change an
original goal's deadline or stability. The operator must still arrange game
progress separately and supply the existing paused observation frontier. A
bounded batch runner that uses the ordinary review/advance owner also stops at
an unfinished predecessor rather than queueing more original work blindly.

## Bounds, custody and limits

One existing six-million-unit frontier reservation now covers the route analysis,
composition checks, complete operations redecoding and every predecessor. The
composition starts at the route algorithm's consumed work; it does not replenish
or increase the budget. Oversized or exhausted reports refuse before a new child
can be created. The composition input is at most 96 KiB and its output at most
160 KiB; the parent still reserves the complete result within its existing
256-KiB limit before effects. No native wire, protocol runner, registry, private
journal format or production admission is changed.

This is selected construction quiescence at one observation, not global or
continuous quiescence, goal completion, usability, causal attribution or effect
discharge. It provides no atomic map/placement lock. Other controllers, later
changes, source restore and malicious-owner rollback remain outside its claim.
The pure report is not authority: only the existing owner supplies and validates
full original custody, native acquisition and current grants.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_construction_prefix test_construction_frontier test_construction_batch_gate -v
```

46 tests pass: 23 predicate/codec tests, 14 composition tests and nine complete
route-owner-source tests. Twelve weakened implementations fail assertions.
The exact earlier implementation also agrees across 1,566 legacy assessment /
monitor comparisons. All 512 item flag words, 32 selected constructions,
31-predecessor execution reports, malformed/truncated bytes, exact work limits,
current authority loss and final publication refusal are covered.

The owner tests execute the complete production module unchanged except for the
actual checked-in edits. They run real owner authority/budget, acquisition
orchestration, review and advance guards, native Capture/Record and operations
codecs, and the new composition/predicates. Native acquisition, map routing,
private custody and native coordinator effects are explicitly injected ports.
Tests assert no dispatch after either pre-intent or final regression, retention
of the simulated pending key, no second start and no old-seal acceptance. They
are NOT real private-journal, TCP, DFHack SDK/game, Rust/MCP or full-workspace
qualification. Existing broader integration suites were not executed here.
