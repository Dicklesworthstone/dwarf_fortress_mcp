# Civilian protection and stable-clear recovery

WP-PLN-03 implementation increment, 2026-10-11.
Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm`.

## Functional changes

The existing `EmergencyLockdown -> Recovering` proposal now keeps civilian
burrow membership and the restriction standing order active. Previously it
immediately proposed releasing civilians, before recovery had established any
clearance. The explicit `transition_to(Peace)` API remains an operator-requested
release proposal; it is not an automatically justified threat decision.

Autonomous callers can use `CivilianAlertFsm::plan_observation` instead. It
accepts a shell-declared threat assessment and civilian roster bound to one
complete StateAnchor and source-scope digest. It proposes existing semantic
burrow and standing-order actions and reports the target defensive level,
clear-window start, clear sample count, source scope and release readiness.

Threat Present immediately proposes protection. Unknown threat state cannot end
an existing or previously proposed lockdown. Clear starts recovery while keeping
protection active; a release proposal requires complete civilian coverage plus
both the configured minimum clear-sample count and minimum game-time duration.
Defaults are two samples over at least 100 game ticks, with at most 100 ticks
between consecutive samples. These are explicit reference-policy parameters,
not calibrated native Dwarf Fortress thresholds.

Each counted observation must advance both tick and cursor consecutively within
the same epoch, source scope and civilian roster. Duplicate reads cannot add
stability, and paused-world reads cannot accumulate elapsed game time. Cursor
gaps, epoch/source/roster changes, overlong sampling gaps, unknown threats and
partial coverage reset the window. Invalid, stale, contradictory or foreign
fortress input clears stability without replacing the last accepted observation
floor. Newly observed civilians receive protection proposals even when the
observed alert level itself has not changed.

Entering an observed lockdown cannot reuse a preexisting peacetime clear window.
Planning never confirms a defensive level: `confirm_observed_level` remains the
explicit shell seam for independently observed action results. Confirming Peace
retires the emergency latch but not the accepted observation floor. This monitor
is process-local: after restart it must be initialized from the observed defense
state and build fresh stability, never restore a guessed clear streak.

## Authority and coverage boundary

These are reference-model proposals, not a native threat detector or a new MCP
mutation path. The shell is responsible for canonical observation eligibility,
complete threat-domain coverage before supplying Clear, exact civilian-roster
coverage, safe-burrow suitability and mappings to native semantics. A source
scope digest is a label, not proof or authority. Empty entity results alone do
not establish Clear. Partial rosters can protect their known subset but cannot
justify automatic global release. An actually complete empty roster can propose
only the standing-order change; no empty-unit action is generated.

All action proposals still require the ordinary capability, plan, witness,
lease where applicable, idempotency, commit-revalidation and observation/proof
path. An emitted protection action is not a claim that civilians reached safety.
No military squad mobilization or bridge-raising effect is invented. The frozen
eleven-tool surface, live protocol maps and empty admission registry remain
unchanged. Direct explicit transition callers retain their own policy duties.

## Bounds and evidence

At most 512 unique nonzero civilian or squad IDs are accepted; ordering is
canonical. Each decision contains at most two action proposals. Recovery policy
requires 2..1024 samples and positive bounded durations no greater than one
million game ticks. The monitor stores only its latest observation/roster and
constant-size recovery state, with no I/O, timers or background work.

Nine Rust module tests cover the original cycle, guarded recovery, replay,
unknown and partial coverage, gaps, changed epochs/scopes/rosters, paused ticks,
invalid/stale/conflicting inputs, new civilians, peacetime-clearance invalidation,
and independent limits. They were source-reviewed but not run: this editing
environment has no Rust toolchain, Cargo dependencies or DFHack runtime.

The retained independent Python model checker exercises 295,245 synthetic
transitions against a clear-suffix oracle. This validates the bounded model's
window bookkeeping, not the Rust implementation, transport, native game,
workspace qualification or admission. Run it with:

```text
python3 scripts/check_alert_recovery_model.py
python3 scripts/check_labor_allocation_model.py
```

The companion workforce checker covers 14,040 exhaustive assignment/model
comparisons. Neither Python checker replaces the Rust regression suite.
