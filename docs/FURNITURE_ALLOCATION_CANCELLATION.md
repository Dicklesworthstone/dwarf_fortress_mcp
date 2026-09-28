# Foreground-owned furniture allocation

Beads: `df-cx-authority-budget-threading-lmu` and
`df-dfhack-bridge-plane-c-pic.3` / `.4` / `.5`.

The trusted inventory connection already checks the owning request during I/O.
The following CPU-bound allocation previously accepted only a copied
`OperationContext`: cancellation or an operator change after inventory release
could not interrupt its scans and global matching. The caller eventually
refused publication, but abandoned work could keep the joined worker and the
single furniture session lock occupied until completion or the wall limit.

## Additional owner check

`furniture_supply::plan_with_check` accepts the ordinary planner arguments plus
`&mut dyn FnMut() -> Result<()>`. `Handoff::allocate_with_check` carries the same
check through native-type catalog validation, source encoding, the nested supply
planner, selected-item evidence assembly and final publication. The callback is
an additional restriction; the ordinary context's Query authorization, exact
anchor and budgets remain required.

The callback runs before source work, before/after major phases, at most every
256 charged work units during scanning/matching, and immediately before returning
a whole result. Error codes are propagated unchanged. A callback cannot turn a
shortage into a partial executable plan, authorize an effect, substitute an item,
renew the deadline, or increase the shared work ceiling. Time spent checking the
owner belongs to the same wall allowance.

The original `plan` and `allocate` APIs delegate with a permissive additional
check. They still enforce their existing explicit contexts. Successful reports,
work-unit counts, selection tie breaks, canonical handoff bytes and digest domains
are unchanged. Effect-owning callers should use the checked variants and keep
their existing authorization checks around durable publication.

This is cooperative interruption, not a real-time guarantee: bounded synchronous
hashing, encoding and sorting cannot be preempted inside an individual primitive.
No threads, tasks, timer, dependency, native RPC method or journal format is added.

## MCP integration

The actual `fortress.open_session(furniture_request=...)` path calls
`Handoff::allocate_with_check` after verified inventory acquisition and before
furniture bootstrap or creation of the placement journal/batch parent. Its
callback invokes `runtime::boundary` with the original joined `RequestControl`,
exact operator configuration and the existing write/Plan opt-in requirement.

This preserves live checks for request abandonment, parent/worker cancellation,
effective inherited I/O and spawn restrictions, ownership of the blocking pool,
configuration identity and current operator opt-in throughout CPU-bound planning.
The callback grants no placement authority. The existing checks before native
bootstrap, durable custody, disclosure and final session publication remain.
The eleven tool names, argument schemas, original recovery identities and
one-attempt placement rule do not change.

## Regression scope and evidence status

Three work-allowance methods cover initial checks, the exact 256-unit boundary,
revocation, exhausted ceilings, expired wall time and callback execution time.
Six pipeline methods compare legacy and checked results and reject cancellation,
authority revocation or budget errors at every reachable callback in both a
complete global assignment and a material-starved request. They also check
missing/expired Query authority, exact shared work ceilings and owner refusal
before unpublished-source access.

Two additional MCP Rust methods exercise the actual `runtime::owned` joined
worker and `RequestControl::check` with complete and infeasible 512-item captures.
They apply inherited capability restrictions at source entry, interior work and
final publication checkpoints and require complete outcome refusal. These are
runtime-owner unit tests, not native TCP, operator-environment or stdio process
tests.

**All eleven new Rust methods remain uncompiled and unexecuted here.** The
editing container has no Rust toolchain, and local GitHub checkout failed because
DNS resolution was unavailable. GitHub API source reads, object writes and ref
updates succeeded. No new Rust, MCP process, native DFHack, live-game or
full-repository qualification follows from this source increment. The earlier
handoff and completion integration execution gaps remain open.

Run the `furniture_handoff` and `furniture_supply` adapter filters, then existing
placement/batch/construction regressions, `build_placement_server` MCP Rust tests,
and the handoff/completion/batch/single-placement process suites on the exact
pinned toolchain before asserting runtime behavior. The machine-readable scope
is `architecture/furniture_allocation_cancellation_v1.json`.
