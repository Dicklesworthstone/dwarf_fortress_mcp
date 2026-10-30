# Reviewed finite clock intervals for a complete furnishing batch

`dfmcp_adapter::construction_plan::clock` connects an existing original-batch
construction monitor to the unchanged run/1.13 coordinator. An agent can review
and commit one finite interval while every original furniture target remains
eligible, reconcile the original native run, and then request a separate shared
construction sample. The owner never starts an automatic simulation loop.

The operator integration is documented in
[whole-batch construction clock control through MCP](FURNITURE_CONSTRUCTION_CLOCK_MCP.md).
The [construction monitor](FURNITURE_COMPLETION_RUST.md) and
[durable run coordinator](BOUNDED_SIMULATION_COORDINATOR.md) retain independent
responsibilities. The run journal is unchanged. The monitor retains its existing
header and earlier frame encodings and adds a distinct readiness frame; older
readers refuse histories containing that new frame instead of treating it as a
credited goal sample.

## Preserve the whole original project

The clock retains the complete immutable `MonitorDefinition`: the original
furnishing plan, its complete mapping to Placed receipts, original placement
journal identity and head, original parent/child file identities, and the fixed
construction goal and timing policy. It also retains the operations/1.4
`Manifest` established by an accepted ordinary completion sample. Every clock key
binds all of that identity and the independent clock journal incarnation.

A smaller successful furniture subset cannot replace the original goal. Changed
receipts, deadline, sampling policy, original custody or operations incarnation
cannot adopt previously retained runs. The construction monitor must have at
least one accepted sample before advancement is possible; readiness does not
bootstrap observation authority.

The run endpoint and software pair must match the original furnishing source.
Run, operations and furniture generation numbers are independently pinned and
need not equal one another. These selectors do not prove shared process identity,
named-fortress lineage for run/1.13, construction causality, global controller
exclusion or cross-plugin fencing.

## Fresh readiness at both durable boundaries

`prepare_guarded` synchronizes the run Intent before calling the host's guard.
`commit_guarded` consumes the live review seal and synchronizes DispatchStarted
before calling its guard. Each guard must independently verify the complete
original monitor and placement custody and acquire one fresh `LinkedSample`.
The fixed construction reader queries every original placement receipt before
and after one complete operations capture on the same connection.

The guard returns `ClockSample { progress, sample }`. Its progress must come from
the unchanged retained monitor. The clock verifies the entire original receipt
bracket using a pure readiness assessment. This assessment applies the original
source, identity and temporal checks without consuming a second acquisition or
adding positive stability credit.
Every target must be either `pending` with viable construction or
`condition_met`; a `suspended` condition, missing jobs at an incomplete stage,
changed identity, removal, source regression and unverified item conditions refuse
advancement.
The complete operations capture must be paused and at exactly the selected
paused run tick. The original monitor must remain unfinished, idle and within
its real observation allowance and absolute deadline.

Readiness cannot satisfy the sampled goal, even when treating the same capture
as an ordinary credited sample would do so. Positive construction proof still
requires an explicit ordinary sample. Readiness, a successful unpause and a
historical native pause receipt remain separate from construction completion.

A host using the clock between completion samples records read intent with
`MonitorStore::begin_read`, acquires the whole receipt-bracketed capture, and
retains it with `MonitorStore::publish_readiness` before returning verified
progress and the sample to the clock guard. The readiness frame stores the full
canonical sample and replays it under the same source, authority, custody,
deadline and semantic-work checks as a goal sample.

Each retained readiness consumes one of the original `max_observations`
completed acquisitions. It never increments `streak`, establishes
`first_tick`, advances `counted_tick`, or independently satisfies the goal.
The original observation count therefore includes both ordinary goal samples
and readiness captures; only ordinary samples earn positive stability credit.
Preparation and commit normally consume two readiness acquisitions per run.

Fully qualifying, paused all-target readiness preserves an existing streak.
Pending or unknown conditions, unpaused observations, excessive gaps and changed
bytes at one tick clear it. Source, identity, stage and allocation regressions,
removal, deadline expiry and acquisition-budget exhaustion retain their normal
absorbing outcomes. Every retained capture advances durable evidence floors,
even though it does not advance proof credit.

A successfully retained readiness closes its read intent. Leaving every
successful read interrupted would reset the streak between every pair of runs
and make multi-sample completion impossible. An acquisition that fails before
acknowledgement instead leaves an unfinished read and the normal continuity
reset. Hosts may still call `withdraw_unpublished()` when an enclosing failure
requires withdrawal. Reopening an unfinished monitor withdraws its live streak;
terminal historical achievement and durable evidence floors remain retained.

Successful commit retains its readiness only as historical pre-dispatch evidence
for this continuity handling. It retains no preparation seal or further dispatch
permission. Readiness never contributes a positive completion sample.

## Review, one attempt and recovery

Only the current owner that prepared an interval holds its process-local review
seal. Commit consumes it before entering the dispatch path. Reopen,
reconciliation, failed publication and explicit abandonment cannot restore that
permission. Every final preparation publication failure clears the review,
including deadline expiry before the final journal inspection starts; the
historical Prepared record remains available for original-key retirement.
After either guard, the run journal rechecks its own complete bytes and current
authority before calling the native method.

A synchronized DispatchStarted marker permanently prevents another unpause
attempt for that key, including after an original-custody failure before the
native send, a lost response, or failed receipt synchronization. Reconciliation
queries the original key and digest once. Missing evidence remains unresolved;
it does not prove that nothing happened. SourceLost remains unresolved without
claiming a safety pause. Pending preparations and unresolved runs both prevent
new completion sampling through `ClockView::can_sample_goal()`.

Recovery requires the exact original `{journal_id, head}`. The acknowledged head
must occur in the verified complete retained chain. Descendants can recover a
lost response; rollback and unrelated history are refused. Existing empty
journals also need that reference. Only a journal initialized in the current
process can first enter without an external recovery pin.

Cancellation before dispatch retires the local intent. After dispatch it can
request the original native safety pause; only this safety action can repeat.
It requires writable Control mode and current original/native Query and
ControlClock authority. It needs neither Plan/Observe authority, a future
simulation allowance, nor an unexpired construction deadline. Recovery and
Offline modes cannot acquire new control. A historical paused receipt never
establishes the game's current pause state.

## Authority, floors and bounds

Both retained opening grants and current request grants must authorize every
operation. New advancement requires named-fortress Query, Observe, Plan and
fortress-wide ControlClock, plus separately issued NIL-source Query, Plan and
ControlClock for the native clock. Scoped construction grants are never rewritten
into native clock authority. Safety cancellation preserves the narrower paired
Query/ControlClock requirement. Pure `authorize_disclosure` applies both original
and current Query authorities before exposing a cached recovery handoff.

Native run observations, decoded construction captures and supplied session ticks
raise a conservative consumed floor. That floor survives later identity,
authority, runtime or publication failure. Returning to an older context or
providing replacement grants cannot revive expired opening authority. Durable
run records reconstruct their floors on reopen; unjournaled read floors remain
process-local and are not advertised as durable history.

| Bound | Limit |
| --- | --- |
| Original furniture targets | 1..32 |
| One requested interval | 1..1,200 game ticks and 1..60,000 wall milliseconds |
| Goal horizon | Original maximum sample gap and absolute deadline |
| Game-tick authority | Minimum of opening and current paired context allowances |
| Retained run intents | `min(64, original max_observations)`; no eviction |
| Clock inspection allowance | 8 MiB |
| Complete core operation allowance | 192 MiB |
| Core readiness assessment reservation | 96 MiB, within the operation allowance |

The host separately prepays original custody reads and native receipt-bracket
acquisition. One shrinking wall deadline covers each complete foreground
operation; pages, targets and nested checks do not receive fresh allowances.
Native limits are callback stop triggers, not exact-tick or hard real-time
guarantees. Overshoot remains visible and raises subsequent authority floors.

## Validation and remaining limits

The focused tests exercise actual Rust source with independent native-record
fixtures and injected storage/source boundaries. They cover complete target
eligibility, original identity, independent generations, finite horizons, current
and retained grants, exact recovery ancestry, lost replies, failed sync,
source loss, cancellation and stability withdrawal. Use:

```sh
cargo test --offline --locked -p dfmcp-adapter --lib construction_plan -- --test-threads=1
cargo test --offline --locked -p dfmcp-adapter --lib bounded_run -- --test-threads=1
```

Exact executed counts and qualification limitations are recorded in
`IMPLEMENTATION_STATUS.md`. Component execution does not qualify a real DFHack
SDK, live fortress, FastMCP transport or production deployment. This adds no
native wire generation, dependency, compatibility entry or production runner.
It does not supply game checkpoint/restore, worker pathfinding, hauling,
continuous monitoring or proof that a completed building is presently usable.
