# Foreground excavation-run sessions

`dfmcp_adapter::excavation_run::session` connects the existing 1.18 evidence,
private-file backend, native RPC client and durable coordinator into a bounded
session lifecycle. This is development source, not a new native protocol or
production admission. Beads: `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-action-coordinator-exec-ero.4`; their broader acceptance remains open.

## Reviewed control and fixed recovery modes

An `ExcavationSession` has one owner and a fixed Offline, Recover or Control mode.
Current Query authorization is required even for cached history. Observe, Plan
and guarded ControlClock grants are separate; injected grants cannot upgrade a
recovery session to Control. Only Control can acquire terrain, create a local
review or start a new run. Recover can query retained native runs and persist
verified evidence. Offline never connects to the game. Native cancellation needs
Control mode and current Clock authority; revoking the separate unpause opt-in
does not revoke an already-authorized safety stop.

A local review is not a native preparation and does not unpause. It binds the
exact selected observation, limits, key and retained inventory projection. It is
consumed before entering the effect shell. The coordinator then re-observes,
checks native key absence and persists intent, preparation and dispatch before
its single commit. Failed calls, reopened journals and native Prepared receipts
never reconstruct dispatch permission. Duplicate commits are historical lookups.
An unresolved journal entry blocks every new key. A failed final inventory read
also keeps the attempted key visible without promoting cached history to verified.

Wait performs one native receipt query, not a polling loop or another unpause.
Native record absence remains unknown. SourceLost is terminal historical evidence
but remains an unresolved obligation. A sampled floor trigger, verified historical
pause and excavation causality are three different claims. Only the first two
have explicit native evidence; current pause, continuous stability, safety and
mining causality are never inferred.

Cancellation of a local review discards only that review. Effect cancellation
uses the original exact retained key/plan and the existing durable native stop
path. Ordinary release refuses unresolved work. Explicit release for recovery
can discard a fenced local session without reopening broken storage; it does not
pause, cancel native work, erase a journal or claim quiescence.

## Concrete effect shell and custody

`session::native::PrivateExcavationBackend` takes a trusted operator directory,
fortress identity and one fixed new-goal region. These are not agent-selected
filesystem paths. The unchanged four-name excavation-run RPC environment remains
in force. Initial creation is explicitly selected and exclusive; old empty,
corrupt and unrelated files are not initialized or repaired.

Each complete operation uses the existing private directory/file custody checks.
Before start or cancellation, the backend opens the journal, compares the exact
expected inventory under its exclusive lock, and retains that same owner through
native dispatch and durable evidence publication. This prevents a journal change
between presentation and dispatch from selecting a different plan. Observe uses
the connection's exact bootstrap capture without acquiring a redundant sample.

The backend does not retain a filesystem lock between separate session requests.
A session detects removed/regressed/changed retained plans and fences itself.
This is not an external anti-rollback root or a global cross-controller clock
lease. Another consistent history installed before a new process starts is not
proved fresh. Native local run ownership and unresolved-journal fences remain
unchanged, and external DFHack/UI/controllers are not globally excluded.

## Bounds and runtime ownership

The session reserves two 8 MiB inventory-work allowances and a complete 32 KiB
response before admitting child work. The request ceiling is 64 MiB, 60 seconds
and 1200 game ticks. One original wall deadline includes caller queue time,
private-file verification, connect, native calls and final inventory verification.
Budgets shrink at nested boundaries. Synchronous filesystem calls remain
cooperative, not forcibly interruptible. No thread, polling loop or timer is
created by this layer.

The caller supplies `ExcavationSessionGuard`, checking current runtime ownership,
cancellation and operator start permission. The concrete source repeats the guard
at native methods, including immediately after dispatch synchronization. Its
cancellation handle must be signalled on parent cancellation/abandonment; the
native socket already checks that handle during bounded I/O. Dropping a request
cannot be represented as proof that a possible effect was cancelled.

Inventory digests cover ordered retained semantic records and the source binding.
They are **projection identities**, not physical journal-frame hashes, canonical
world roots, signatures or authorization tokens. Observations retain their
separate native capture witnesses. Cross-profile Python blueprint monitors are
not imported, sampled or joined to this native source.

## Verification status

Fourteen Rust tests are registered around the actual existing coordinator and
native fixtures with an injected storage/source shell. They cover reviewed
one-shot execution, lost replies, reopened recovery, fixed-mode isolation,
confirmation, revoked unpause permission, cancellation, missing native records,
source loss, final-read failure, rollback, owner/request replay, deadlines,
output reservation, expired authority and local review cancellation.

These Rust tests are **uncompiled and unexecuted** in this editing environment:
Cargo, rustc, rustfmt and the locked dependency graph are unavailable. No real
DFHack SDK/plugin, live fortress, physical power-loss, whole-workspace or production
qualification is established. Run on the declared locked nightly workspace:

```sh
cargo test --locked --offline -p dfmcp-adapter excavation_run::session
```
