# Fortress-bound order-condition runs (native foundation)

`bridge/common/order_run.h` adds the native engine foundation for an isolated
`order-run/1.14` profile. It composes the unchanged bounded-run clock owner with
one selected finite wooden-furniture manager order and a closed stop predicate:
`approved`, `active`, or `remaining_at_most`. These are observed flag/counter
conditions, **not evidence of produced goods or reconciliation of creation**.

The intended control loop is capture the selected order while paused, seal the
complete source/order/predicate/limit input, prepare, commit once, then allow the
native owner to pause when its condition or a safety limit is observed. At this
increment the engine is callable through injected native callbacks only; no new
RPC, Rust or MCP entry is claimed. Existing read, pause and run/1.13 bytes remain
unchanged. The new profile is not admitted and no bead is closed.

## Source and stop semantics

A capture contains generation, dispatch sequence, game tick, pause state, exact
world-folder/site identity, selected ID, allocation horizon, recognized recipe,
original total, remaining counter and current status bits. A recognized recipe
means the native adapter has checked every field of the existing finite wood
creation template. Caller data or raw job enums cannot assert recognition.
The native shell must validate UTF-8 folder bytes before capture publication.

Every setter receives the complete generation/folder/site identity and must
independently recheck it before touching the clock. A changed source retires
ownership as `source_lost` without pausing the replacement fortress, including a
folder/site change at an unchanged generation. This identifies a source, not a
global lease: external UI, plugins and other controllers are not fenced.

Preparation requires a paused, fully observed recognized order whose predicate
is not already true. Commit revalidates the exact original capture; an order
that becomes approved between preparation and commit cannot silently unpause.
Replay never renews preparation or repeats an unpause. The existing limits of
1..1,200 game ticks, 1..60,000 milliseconds, 60-second preparation lifetime and
256 retained operations apply. No record is evicted to regain capacity.

A condition requests 1..16 positive samples at a minimum 1..1,200 game-tick
spacing, with the product bounded by the run tick allowance. Preparation is not
a positive sample. Repeated callbacks at the same game tick cannot count twice.
Any sampled negative resets the streak even before the next eligible count.
This proves only sampled positives, not continuous truth between callbacks.

Complete queue validation precedes absence claims. Target absence, loss of
recognized configuration, changed recipe/total, increased remaining counter or
regressed allocation horizon requests a safety pause with a distinct anomaly
trigger; none means goal success. Source identity and clock discontinuity,
external pause, tick and wall limits take precedence over a new predicate claim.

Predicate evidence and safety-stop evidence are separate. A matching sample is
retained before requesting the safety pause. The predicate can be observed while
the operation remains `stopping` because pause readback failed. Stop ownership
is retained until fresh pause readback or explicit source loss. Retrying safety
pause is allowed; repeating unpause is not. Terminal evidence is immutable and
historical, not a claim that the game is currently paused.

A native callback opportunity is required to enforce either limit. There is no
hard real-time or exact-tick guarantee. Missing/invalid native state triggers
safety stopping rather than guessed progress. A complete target capture may be
needed before this profile can verify a stop after a read failure.

## Executed evidence

Run `python3 scripts/test_order_run_engine.py`. The actual C++ engine and its
unchanged clock dependency are compiled under C++17, warning denial and UBSan.
Both GCC and Clang pass 10,422 assertions, including all 256 eight-sample truth
schedules against an independent streak oracle, duplicate/early samples,
negative streak resets, all three predicates, missing/changed orders, counter
and horizon resets, source replacement, stale preparations, ambiguous setters,
failed safety pauses, cancellation, shutdown and retention exhaustion.

The dependency bytes were verified against Git blob
`b8f54a21bee65670389e34ed2f69e2b0d1adc4fc`. This is deterministic callback-double
evidence, not a real DFHack SDK/ABI, plugin-manager, protobuf, live-game, Rust,
MCP, complete repository qualification, global clock lease or admission result.
