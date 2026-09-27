# Receipt-linked whole-plan construction in Rust

`dfmcp_adapter::construction_plan` checks later construction state for every
original furniture placement in one complete plan. It closes the gap between
historical stage-zero registration (`all_placed`) and a later sampled condition
in which all requested buildings have reached their expected maximum stage.
It is a development adapter; it grants no placement or game-control authority.

## One goal, one shared capture, every original receipt

A `Goal` retains 1..32 canonical furniture/1.19 Placed receipts with unique
operation keys, building IDs, job IDs, exact items and target positions. All
receipts must share their original fortress, source generation and dimensions.
Its fixed absolute deadline, cadence, sample count, stability span, maximum gap
and observation allowance are part of the goal identity. Reopening cannot renew
those limits or shrink the target set.

The goal (`DFMCPG01`) and linked sample (`DFMCPS01`) preserve the existing Python
wire bytes and digest. The checked-in 32-target fixtures come from the independent
Python codecs. The Rust adapter decodes the complete operations capture, including
cross-roster references, counts and native enum number/key consistency.

`rpc::acquire_trusted` uses one foreground TCP connection with exactly four
existing native method bindings. It queries every original receipt, acquires and
releases one complete paged operations/1.4 observation, and queries every original
receipt again. Both receipt brackets must match the original canonical records
exactly. The original endpoint and placement-time software remain pinned; the
operations generation is independently pinned and need not equal the furniture
generation. Acquisition returns a linked sample; validated advancement is the
boundary that accepts it as construction evidence.

All members use that same capture and one global stability streak. The condition
requires the original building footprint and maximum stage, exact installed
singleton item/material, and no construction/removal job or conflicting item-job
link. Original identity changes and regressing source clocks, ID horizons or
building stages invalidate the monitor. Removal fails it. Suspended jobs,
unverified items and incomplete construction keep it pending. Separate successful
observations of different targets cannot accumulate into whole-plan success.

Repeated paused ticks cannot advance stability. Changed bytes at the same tick,
large gaps and interrupted reads reset the streak. The fixed deadline and total
observation allowance produce absorbing expiry; satisfaction, failure,
invalidation and cancellation are also terminal.

## Retaining the complete original batch

`origin::Origin` binds the full Rust `BatchDefinition`, exact ordered mapping to
every original Placed receipt, original placement journal ID/head/frame count,
and both original file and directory identities. Import requires a completely
audited `all_placed` batch. A later permanent parent stop is outside this immutable
binding. The full receipt goal and this origin form `MonitorDefinition`.

These Rust composite formats are deliberately separate from the Python
directory-based furnishing monitor:

| Object | Magic |
| --- | --- |
| Complete Rust batch origin | `DFMFRO01` |
| Origin-bound monitor definition | `DFMFRG01` |
| Private monitor journal | `DFMFRJ01` |

`store::MonitorStore` retains the definition, durable read intents, complete
linked samples and local cancellation in a hash chain. Replay recomputes every
transition from original bytes under one deadline, byte budget and semantic work
allowance. It never restores permission to publish a previously interrupted read.
The original source is checked before read intent and around sample publication,
including after synchronization. A failed or ambiguous append fences advancement.

Cancellation needs current Query authority and intact monitor custody; it does
not require the original placement files. This keeps local monitoring stoppable
when those files disappear. The caller must separately report that original
construction evidence is unverified. Offline custody never appends. Terminal
inspection continues to verify current custody rather than treating a cached
terminal string as fresh evidence.

The Linux private opener retains no-follow, exact owner/mode, file/directory
identity and complete-byte checks. Its monitor-specific maximum is 128 MiB;
the original placement journal remains limited to 16 MiB with unchanged bytes.

## Bounds and authority

One foreground acquisition permits at most 327 RPC calls, a 16 MiB operations
capture, 20 MiB network bytes and 2 MiB total notifications. Complete entity
accounting includes the world root, jobs, buildings and items. Every I/O boundary
rechecks cancellation, current Query authority and the trusted operator callback;
the observed game tick raises the authority clock before release and trailing
receipt queries. No reconnect, polling loop or effect method is available here.

The journal permits at most 1,030 frames and 512 accepted observations. A shared
20,000,000-unit semantic budget covers the whole replay, not each target or frame.
Complete result admission and separate original-source read reservations belong
to the host. Incomplete evidence never permits replaying a placement.

## Validation

The focused suites cover Python byte parity, full 32-target goals, exhaustive
item flag cases, original identity failures, shared stability, interrupted reads,
complete native receipt brackets, pagination/digest/release faults, authority
expiry, local cancellation, full replay and Linux file substitution.

```sh
cargo test --offline --locked -p dfmcp-adapter --lib construction_plan -- --test-threads=1
cargo test --offline --locked -p dfmcp-adapter --lib build_placement -- --test-threads=1
cargo test --offline --locked -p dfmcp-adapter --lib furniture_batch -- --test-threads=1
```

Executed validation results are recorded in `IMPLEMENTATION_STATUS.md`. Focused
adapter execution is not full workspace qualification, native DFHack SDK
qualification, a live fortress campaign or production admission. Sampled
satisfaction does not prove current usability, continuous stability, causal
attribution or discharge of any placement effect.
