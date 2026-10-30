# Implementation Status

This file is the authoritative antidote to accidental overclaiming. Prospective architecture prose
describes the target system; this file describes what the checked-in source and exact evidence
actually establish.

## Current phase

**Phase 0D-R0 with implemented but unadmitted read profiles through citizen-inclusive spatial/1.8,
optional archive-bound durable foreground watches, and an explicitly unadmitted pause-control/1.7
development slice. No live tuple is currently admitted.**

The repository contains:

- a substantial authenticated protocol-1.0 read-only DFHack stack;
- canonical live citizen observations and an agent-oriented MCP server;
- exact compatibility, anti-rollback, artifact, and process-admission machinery;
- an implemented protocol-1.1 retained-announcement extension;
- separate unadmitted jobs-only 1.2, operations/1.3, paged operations/1.4, map/1.5 and spatial/1.6 development profiles;
- a citizen-inclusive spatial/1.8 source path that captures strict citizens, jobs, buildings, items and bounded terrain during one native suspension, then publishes one combined anchor;
- optional paired spatial/1.8 observation and watch journals preserving monitoring definitions, outcomes, cancellation and release across restart without claiming downtime continuity;
- a pause-control/1.7 bridge/client/development MCP runtime supporting prepare, durably coordinated commit, receipt-verified reconciliation and bounded foreground recovery for `Pause { paused }` only;
- a private hash-chained pause-effect coordinator journal whose source records `CommitStarted` before dispatch and terminal evidence before acknowledgement;
- offline read-only pause-effect discovery without a bridge connection or mutation authority;
- an explicitly unadmitted furniture/1.19 native, Rust and eleven-tool MCP development path for exact-item bed/chair/table placement, durable single-attempt coordination and original-key recovery;
- a protocol-bound V2 production ticket and runtime dispatcher whose map still contains only protocol 1.0.

The checked-in compatibility registry remains empty. Source presence does not imply qualification,
admission, or support. No protocol beyond 1.0 appears in the production runner map.

## Evidence hierarchy

1. **source present** — code, contracts, and tests are checked in;
2. **static/Python checked** — static or design checks passed for one exact source generation;
3. **Rust-qualified** — latest-nightly formatting, warning-denied Clippy, debug/release tests and rustdoc passed;
4. **native-qualified** — exact DFHack plugin built and passed native qualification;
5. **live-qualified** — required disposable-fort campaign passed;
6. **registry-admitted** — reviewed receipt promoted to compatibility registry;
7. **floor-accepted** — deployment host advanced monotonic floor;
8. **artifact-qualified** — exact server executable qualified;
9. **runtime-admitted** — protocol-bound single-use ticket consumed by the exact runner.

Higher rungs apply only to the exact source, binary, protocol, platform and inputs they name.

## Present now

### Exact construction-monitor recovery ancestry (2026-10-30)

The whole-furniture monitor now exposes an exact external recovery reference and
verifies acknowledged ancestry against its complete semantically replayed history.
Valid descendants recover lost acknowledgements; rollback, divergent history,
changed file identity and malformed descendants fail without rewriting custody.
The existing bounded verification reservation covers this read-only check, and
the private journal format remains unchanged.

The monitor also retains its first accepted source independently of the latest
diagnostic progress. A later source-changing observation can invalidate the
monitor and replace that diagnostic source; it cannot replace the original
operations association needed to reconstruct an existing construction clock.
This core prerequisite permits independent clock recovery without treating a
replacement source as original evidence. Public recovery integration follows
separately.

After preserving both incoming construction-readiness increments, all 113
actual-source adapter tests pass: 91 construction-plan and 22 bounded-run tests.
The seven new recovery/source groups include later invalidation from either an
ordinary goal sample or retained readiness. All 276 adapter Rust source hashes
remain unchanged during both final runs. Changed Rust is formatted and introduces
no new warning. One unrelated existing workforce-test import warning remains.
Full FastMCP/native/live qualification and compatibility admission remain unverified.

### Durable MCP furnishing run/sample cycles (2026-10-30)

The whole-batch MCP clock now uses the durable readiness path introduced below.
Preparation and commit each synchronize a monitor read intent, acquire the full
original receipt-bracketed capture, and acknowledge it as readiness before the
native effect. Guards prepay all original-custody reads, monitor replay,
publication and decode work before synchronizing the clock's durable boundary.
Each acknowledged capture consumes the original acquisition allowance and
retains source, stage, allocation and tick floors. Clock reassessment uses a
pure probe, so it cannot charge that acquisition twice or manufacture a credited
completion result.

Whole 2-target and 32-target scenarios now traverse real reviewed intervals
between ordinary completion samples. Fully valid readiness preserves an
existing shared streak across those intervals; pending, unknown, interrupted
or contradictory evidence resets it or retains the appropriate terminal
outcome. Ordinary goal samples remain the only source of positive proof credit.
Original budgets of two or three acquisitions stop preparation or commit at
the relevant boundary before unpause. Terminal negative readiness survives
reopen rather than disappearing as an unjournaled preview.

Opening a combined session now prepays and rechecks original placement and
batch custody, unchanged monitor history and exact clock recovery after its
last runtime callback, under the same absolute deadline. Clock response and
recovery metadata retains the greatest consumed tick even when a later custody
failure prevents the owner from accepting the capture. Final preparation
publication still withdraws every live review seal on failure.

**Evidence: 106 adapter tests and 60 MCP/AgentTurn component tests pass (166
total).** The adapter view retains all production bodies and selected tests,
disabling only 120 unrelated test-module declarations for the memory limit.
The MCP view retains the complete public handler/native bodies and selected
tests, with nonexecuted FastMCP/asupersync ambient stubs; four worker-runtime
tests and the allocation-owner runtime module are excluded. All 278 adapter
source/fixture inputs and all 761 MCP/dependency inputs match their tested
snapshots. This proves component behavior and type compatibility against the
stubs, not execution of the real ambient runtime.

The required repository gates were attempted. `verify.sh` reaches the existing
source-bundle documentation gate and fails because it omits the exact clean
commit. Development-mode `qualify_local.sh` reaches 76 existing validation
findings across 62 files; every named file is unchanged from the incoming
`9c93fda` baseline. No release qualification receipt or native/live admission
is claimed. The compatibility registry and production runner map are unchanged.
The broader mutation-dispatch bead remains in progress. Operator instructions:
[whole-batch clock through MCP](docs/FURNITURE_CONSTRUCTION_CLOCK_MCP.md).

### Durable construction readiness without positive proof credit (2026-10-30)

The whole-furnishing monitor now has a separate readiness publication path for
clock preparation and commit. It retains the complete canonical receipt-bracketed
sample in a distinct journal frame and replays all original source, identity,
stage, allocation, deadline and shared-condition checks. A completed readiness
acquisition consumes the original observation allowance and advances durable
evidence floors, but never adds a stability sample, establishes a first proof
tick, advances a counted proof tick or independently satisfies the goal.

Paused readiness in which every original condition still holds can preserve an
existing candidate streak. Pending, unknown, unpaused, same-tick changed or
excessive-gap evidence resets it. Failure, invalidation and expiry remain
absorbing. This supplies the missing acknowledgement between explicit finite
runs: leaving each successful readiness read interrupted would erase progress
between every ordinary completion sample. Interrupted or failed acquisitions
still retain the existing continuity reset, and restart still withdraws live
unfinished continuity while preserving recorded facts.

The shared evaluator also exposes a pure readiness probe for clock integration:
assessing an already retained readiness can avoid a second acquisition charge or
a manufactured positive goal result. The clock host adopts that path separately. A final preparation publication failure, including wall
expiry before the final replay, now also withdraws the local review seal while
preserving original Prepared history and original-key retirement.

**Evidence: 104 focused adapter tests pass (82 construction/readiness and 22
bounded-run), including all 14 local TCP fault tests.** The isolated offline
component view retains every production body and all selected test bodies;
120 unrelated test-module declarations are disabled for the memory limit. The
278 source/fixture inputs (276 Rust files) match the tested snapshot exactly.
This is not full workspace, ambient runtime, native/live or admission
qualification. Incoming main already exposes whole-batch clock routing through
MCP; adoption of this new durable readiness publication path in its guards is
a separate follow-up. This checkpoint supplies the tested core contract. See
[the construction clock contract](docs/FURNITURE_CONSTRUCTION_CLOCK.md).
The broader live mutation-dispatch bead remains in progress.

### Whole-furnishing-batch clock control through MCP (2026-10-30)

The furniture development server now exposes the complete original construction
clock through its existing eleven tools. `plan(clock=...)` prepares an explicitly
finite interval; `commit(scope="clock", monitor_id, review_seal)` separately
confirms it; `wait(scope="clock")` reconciles one original outcome;
`cancel(scope="clock")` requests original safety cancellation; and query mode
`construction_clock` verifies bounded history against the exact supplied head.
Game ticks default to zero and require independent operator opt-in and current
clock authority. Placement keys and witnesses cannot be mixed into clock requests.

Activation requires an existing complete original batch and construction monitor
with at least one accepted ordinary sample. The operator closes that session and
reopens with a fourth distinct private run file and independent source/recovery
configuration. Clock custody is opened eagerly; no sample lazily initializes it.
Fresh original receipt brackets and source/monitor checks guard both durable
native boundaries. Pending, unresolved, missing or unverified configured clock
history blocks fresh completion. Contradictory readiness, rejected publication
and unacknowledged worker results withdraw unfinished monitor stability without
adding positive samples or renewing its temporal policy.

The complete Agent Turn includes original batch/monitor/run identities, separate
review instructions, exact recovery, bounded history and safety-cancel arguments.
Original and native Query authority apply even to cached handoffs. Every operation
prepays complete work, narrows one wall deadline, and retains consumed clocks
through later rejection. Final exact clock replay occurs after the last worker
runtime callback at opening, clock actions, generic handoff and completion
publication. The retained native connection also rechecks its credential digest
before each call. Outer joined-worker acknowledgement protects the next request;
it does not prove client/network delivery.

All 54 focused actual-source handler/Agent Turn tests pass, including 16 new
whole-batch clock integration groups and three strict helper groups. Together
with the 93 adapter tests for the core increment, 147 focused tests pass. The executable component
view retains the generic handlers, complete clock helper, projections, private
journals and actual core crates; it excludes FastMCP transport, public async and
native effect shells. A separate compile-only view retains the complete public
async/native wrapper bodies using nonexecuted ambient runtime stubs. Its type
check passes. All 414 handler compiler inputs and 280 async-view inputs remain
unchanged during final checks; changed Rust files pass formatting. These results
do not execute the excluded runtime or native game paths.

Repository-wide `verify.sh` passes its first four checks and
remains blocked by unchanged source-bundle documentation requiring an exact clean
git commit. `qualify_local.sh` stops at static contracts; after correcting the
three findings introduced by new test helpers, the standalone validator reports
76 remaining findings across 62 files, all byte-identical to the starting main
revision. The locked/offline full MCP build cannot resolve the uncached pinned
`fastmcp-core` Git revision `180a7c88890705217bb8e202d19555adabf24187`.
Full production FastMCP/ambient runtime, native/live qualification and compatibility
admission remain unverified. The complete combined restart still
needs the original parent, placement and monitor files; no furniture clock-only
recovery mode is added. After successful opening, original-file loss does not
strand reconciliation or safety cancellation through intact clock custody.
Retain the clock configuration and exact recovery pin across restarts: omitting
it cannot establish that no earlier run exists. See the
[operator workflow](docs/FURNITURE_CONSTRUCTION_CLOCK_MCP.md). Broader mixed-action
execution and verified game checkpoint/restore remain incomplete.

### Complete furnishing-batch finite clock core (2026-10-30)

`construction_plan::clock` binds an independently pinned run/1.13 journal to the
complete immutable original furniture monitor, every original Placed receipt,
original batch/placement custody and the operations source established by a real
accepted completion sample. A fresh complete receipt bracket after durable Intent
and DispatchStarted must agree with the paused clock; every target must remain
eligible. Readiness contributes no positive goal sample. Runs have separate live
review seals, one native unpause attempt, fixed deadline/cadence and tick/wall/work
bounds, and at most `min(64, max_observations)` retained original intents.

Exact acknowledged ancestral recovery rejects rollback and unrelated histories.
Retained and current named-fortress/native grants are checked independently;
consumed ticks survive later rejection. Safety cancellation remains possible
under paired Query/ControlClock after Plan/Observe or future budgets expire.
Pending or unresolved clock work blocks fresh completion. Pure monitor withdrawal
clears unfinished stability after contradictory readiness, reopening or failed
publication without renewing the goal; the next read durably records interruption
using existing journal frames. Terminal historical achievement remains retained.

All 93 focused actual-source adapter tests pass: 71 construction-plan tests
(22 new clock groups, five new continuity groups and 44 existing tests) plus 22
bounded-run regressions. Loopback TCP fixtures ran with explicit local-network
permission. All 273 adapter Rust source hashes remained unchanged during the
final runs. The pinned nightly toolchain reports one unrelated existing test
import warning; new source is warning-free. These minimal-dependency component
runs do not establish a complete workspace build.
The implementation changes no native protocol, dependency, production runner or
compatibility entry. Full FastMCP/runtime, native/live and release qualification
remain unverified. Independent source generations do not prove a common process,
clock fortress identity, global exclusion or construction causality. See
[the core contract](docs/FURNITURE_CONSTRUCTION_CLOCK.md). The broader live
mutation-dispatch bead remains in progress; generic mixed-action execution and
verified game checkpoint/restore remain incomplete.

### Independent whole-blueprint clock recovery (2026-10-30)

The existing mining MCP server now opens `clock-offline` and `clock-recovery`
from the exact retained monitor and run histories without opening the native
designation journal or blueprint parent, taking a paused bootstrap observation,
acquiring terrain or issuing a mining lease. Both modes omit `region`, forbid
positive game-time allowances, and retain no Plan, Observe or Designate grant.
Offline inspection contacts no native source. Query-only writable recovery
performs one original-outcome reconciliation per explicit request; independently
retained named-fortress and NIL-source Clock authority permits the original
safety cancellation. Neither mode can prepare a new interval or restore a
recovered review. Later opt-in cannot promote a fixed Query-only owner.

After intact startup, damage to the monitor file cannot strand cancellation
through the intact original clock journal. Clock damage still refuses native
operations; explicit recovery release preserves unverified references without
claiming pause, settlement, quiescence or excavation proof. Every response
retains the highest consumed authority tick. Opening reserves its full work
before either file, checks both histories after the final runtime boundary,
and preserves normal recovery-reference shapes through late failure and the
outer runtime handoff. Agent Turn coverage distinguishes these two histories
from the absent original designation inventory.

Nineteen new routing/configuration/lifecycle regression groups join the earlier
publication tests. All 100 focused actual-source MCP/Agent Turn tests pass,
including lost dispatch, missing native/parent custody, post-open monitor
damage, independent grant expiry, fixed Query-only recovery, late clock damage
and failed cancellation publication. The query schema passes 59 valid and 205
invalid cases. Full unchanged opening/dispatch/close and native I/O bodies also
pass a separate Rust type/ownership check with actual core/world/intent/adapter
and serde; only FastMCP registration and ambient asupersync APIs are nonexecuted
stubs. This is component evidence, not full runtime or live qualification.

Required `verify.sh` still stops at the existing source-bundle documentation
failure. `qualify_local.sh` still stops at 76 static findings in 62 unchanged
files. The full production MCP build remains unverified after shared memory
pressure killed its earlier dependency build. No native/live campaign, release
qualification, compatibility admission or production-map change is claimed.
See [the recovery workflow](docs/DIG_BLUEPRINT_CLOCK_RECOVERY.md).

### Clock publication custody and consumed authority floors (2026-10-30)

The mining MCP clock handler now prepays a final exact journal replay before
performing any operation. After rendering the result and completing the final
runtime checkpoint, it rechecks the retained clock identity and head before
publishing success. A late custody change withdraws the response, drops the
process-local review seal, and retains the exact recovery reference. Failed
commit/cancel responses explicitly preserve possible-effect uncertainty.

Recovery handoffs retain the maximum consumed authority tick across the native
dig owner, whole-blueprint monitor and clock owner, including a fresh map read
whose subsequent original-custody check fails. Such a failure cannot reduce the
next authorization floor or turn an unknown dispatch into permission to retry.

Three new handler regression groups cover final-checkpoint history replacement,
post-capture custody rejection with a learned tick, and acknowledged versus
unacknowledged worker results. Focused actual-source component execution passed
81 MCP/Agent Turn tests (75 mining handlers/helpers and 6 envelope tests), and
the current query schema passed 59 valid and 205 invalid cases. The component
harness excludes FastMCP attributes/stdio, asynchronous owner entry points and
native socket shells. The full production MCP check was killed by shared memory
pressure before reaching project type-checking; no full Rust, native, live,
release or admission qualification is claimed. Independent clock-only restart
is a separate remaining feature at this checkpoint.

### Whole-blueprint finite clock control through MCP (2026-10-30)

The existing eleven-tool mining server now joins the complete original
excavation monitor to separately reviewed finite run/1.13 intervals. Operator
configuration pins a fourth private journal and independent run source; the
session's game-tick allowance defaults to zero. New advancement requires
explicit clock opt-in, current disposable-fortress checkpoint policy, scoped
original Query/Observe/Plan and separately retained global/native clock grants.
No designation grant is rewritten into clock authority.

Plan prepares one closed monitor/tick/wall request; commit confirms its live
review seal; clock-scoped wait reconciles one original run; ordinary monitor
wait acquires a separate completion sample. Pending, unresolved and SourceLost
clock history block fresh goal publication. Fresh sparse readiness must match
the paused run tick, and the unchanged original deadline, cadence, sample
capacity and whole-project inventory constrain every interval. The full
original mask is retained through every step; no automatic run loop is added.

Clock custody is acquired before sampling, including exclusive initialization
when the monitor already exists at session opening. An existing journal cannot
masquerade as new custody. The canonical intended header identity is retained
before creation; existing-history failures preserve the operator's acknowledged
pin until verified ancestry permits a descendant. Opening work splits one
prepaid byte allowance and shrinks one wall deadline. Recovered preparation,
failed responses and unacknowledged owned-worker results discard clock review
permission without erasing the pending obligation.

Readiness contributes no positive completion samples. Known contradictions,
changed same-tick captures and broken sampling windows withdraw unfinished
stability; the next real sample durably resets it before source contact.
Clock and original histories are rechecked after durable dispatch intent and
again around completion publication. Safety cancellation retains current
Query/Clock requirements without Plan, Observe, future tick budget or a renewed
monitor deadline. Historical pause receipts never prove the current pause state.

Clock queries expose exact recovery, bounded head-pinned pages and actionable
review/reconciliation/cancellation steps. Normal close refuses unsettled clock
work; explicit recovery release preserves unverified references. Combined
restart still needs original native, parent and monitor files. The existing
standalone run recovery profile can inspect the clock journal independently.
Independent source pins still establish neither shared process identity,
named native-clock fortress lineage, causality nor global controller exclusion.

All 194 focused tests pass on the final reconstructed source: 100 blueprint
adapter groups, 22 existing bounded-run regressions and 72 actual-source MCP
handler/configuration groups. The latter include 16 new clock integration
groups and three clock parser/authority groups. All 390 adapter and 412 MCP
compiler/pipeline input hashes remain unchanged during their runs. MCP
compilation is warning-free; the adapter retains one preexisting unrelated
workforce-test import warning. Query Schema checks accept all 59 valid cases
and reject all 205 invalid cases.

The actual public async open/dispatch/close bodies and concrete clock/map IO
also pass a separate compile-only check, with all 278 inputs unchanged. Only
the ambient RequestControl/owned span is replaced with fail-closed nonexecuted
type stubs. The executable handler view excludes FastMCP macros/stdio and three
Cx-specific runtime tests. This checks signatures, lifetimes and borrowing;
it does not establish real Asupersync scheduling, MCP transport, DFHack SDK,
live-game or release qualification. All earlier pre-reset counts were rerun
against the recovered source before this increment was committed.

See [the complete operator workflow](docs/DIG_BLUEPRINT_CLOCK_MCP.md). The native
wire, production runner map, compatibility registry and dependencies remain
unchanged. Generic mixed-action execution and verified checkpoint/restore
remain open; this increment does not claim live or production qualification.

### Reviewed finite whole-blueprint clock core (2026-10-30)

The adapter now binds one independently pinned run/1.13 journal to the complete
original excavation monitor, every original Designated receipt and the unchanged
sparse mask and temporal policy. Each interval requires a fresh complete map
readiness check, separate process-local review and explicit confirmation. Native
bounds, original deadline/cadence, opening/current game allowance and retained
run capacity all constrain advancement. No automatic run chain is introduced.

The existing coordinator now supports external revalidation after durable
Intent and DispatchStarted, followed by final verification of its own bytes and
current authority before the native call. Same-size journal corruption during
the map guard cannot unpause. Lost replies and failed synchronization preserve
uncertainty; exact acknowledged recovery heads accept descendants but reject
rollback. The shared canonical header identity can be pinned before creation,
preserving exact recovery after a lost initialization acknowledgment. Reopened preparation cannot recover a dispatch permit. Consumed native,
map and supplied authority floors cannot be reset by stale contexts or new grants.
Safety cancellation remains possible without future tick budget or Plan/Observe.

All 122 focused adapter tests pass: 100 blueprint cases (19 new clock groups and
81 prior regressions, including the concurrently added continuity fixes), plus all 22 existing bounded-run tests. All 390 compiler
and source input hashes remained unchanged. The one preexisting unrelated
workforce-test import warning remains; no new warning was introduced. These are
actual-source component tests, not full Rust/FastMCP, native/live or release
qualification. No native protocol, dependency, production runner or admission
registry changed. MCP composition follows separately. See
`docs/DIG_BLUEPRINT_CLOCK.md` for the contracts and source-identity limitations.

### Preserve blueprint monitor continuity across failed MCP responses (2026-10-30)

The existing dig-control MCP server now withdraws unfinished monitor stability
after failed request preflight, runtime or authority checks, failed publication
and final response overflow or timeout. The next permitted sample persists the
continuity reset before reading again. Original monitor identity, deadline,
attempt allowance, historical achievement and native recovery remain intact.

Each native session retains an explicit owned-worker acknowledgement. Only a
successful join and final outer runtime checkpoint set it. If the next request
finds the preceding worker result unacknowledged, it abandons stale local native
preparation permission and withdraws unfinished monitor continuity before
creating the new request context. This acknowledges runtime completion, not
network delivery to an MCP client. Early and final failures preserve updated
recovery handoffs. A source-invalidating acquisition keeps the last accepted
target page explicitly historical, without labeling it a fresh sample.

Two new handler regression groups cover the acknowledgement latch and early
budget/runtime failures without source contact; a separate runtime test covers
acknowledgement after joining. The existing source-drift case now checks the
historical freshness labels. Query-schema validation passes 49 valid and 175
invalid cases. New Rust test execution remains unverified: complete adapter
test compilation ended in SIGKILL, and the isolated follow-up also encountered
filesystem exhaustion, including compiler metadata writes after scoped cleanup.
The exact full FastMCP dependency set is unavailable. No full MCP transport,
ambient-runtime, native/live or admission qualification is claimed.

Changed Rust files pass the pinned rustfmt check. The contract matches all 13
operator environment names, 11 frozen tools and five opening modes. `verify.sh`
passes its first four gates, then stops at the existing source-bundle
documentation failure. The final local qualification attempt cannot create its
receipt because the filesystem is full; no qualification receipt is claimed.

This increment preserves the independently contributed monitor-only recovery
implementation. It adds only acknowledgement, continuity withdrawal and evidence
presentation corrections. See `docs/DIG_BLUEPRINT_MONITOR.md`.

### Preserve whole-blueprint stability across contradictory or unpublished observations (2026-10-30)

The existing monitor now clears its shared matching streak when a complete
capture changes at the same game tick, including changes in unselected padding.
An equal-tick correction, repeated capture or restarted owner cannot seed a new
streak. A later advancing-tick observation must establish a new matching window;
the original deadline, attempt allowance and terminal historical evidence remain
unchanged.

The store also exposes local withdrawal of unfinished progress after an enclosing
runtime or response publication fails. Withdrawal performs no I/O and grants no
authority. The next permitted attempt synchronizes a continuity reset before
source contact. A held abandoned attempt can still retain already consumed raw
evidence and its newer clock floor, but cannot publish that evidence as progress;
interruption or Query-only cancellation settles its retained ownership.

Four new regression groups cover same-tick contradictions and corrections,
restart, local withdrawal, durable reset before another attempt, preserved first
historical achievement and abandoned-read evidence retention. The changed source
has been independently reviewed and formatted. The attempted final 51-case
actual-source monitor run was blocked when compilation of the complete adapter
test target ended with SIGKILL under shared resource pressure. No new test
execution is claimed at this checkpoint.
This is a source checkpoint, not full Rust/FastMCP/native/live qualification or
production admission. MCP failure-boundary integration follows separately; the concurrent independent
monitor restart implementation is preserved. See `docs/DIG_BLUEPRINT_MONITOR.md`.

### Independent whole-blueprint monitor recovery through MCP (2026-10-30)

Explicit `monitor-offline` and `monitor-recovery` opening modes now reopen an
existing private monitor without opening the original native journal, blueprint
parent or game connection. The exact configured monitor identity, acknowledged
history head, original binding and map source remain mandatory. These modes
omit `region` and retain only Query authority. Offline recovery is read-only;
writable recovery additionally permits durable local monitor cancellation.

The independent owner supports historical progress pages, restricted schema
discovery and history inspection through doctor. It refuses every native action,
new monitor registration and live sample request. Neither recovered satisfaction
nor a verified monitor history proves current terrain or verifies the original
designation inventory. Original grants and the retained clock floor still fence
every request. Normal close verifies history; explicit recovery release preserves
an unverified exact handoff if the retained history or configuration is damaged.
Ordinary mining opening still requires its original blueprint custody.

All 128 focused tests pass: 77 adapter cases and 51 MCP cases, including nine new
standalone recovery groups. Fixtures drop the combined owners, remove native
journal bytes and damage the blueprint parent before reopening the intact
monitor. Tests also cover Query-only authority, forbidden routes, exact source
and recovery pins, historical satisfaction, cancellation after response failure,
clock/grant expiry and damaged-history release. All 388 adapter and 404 pipeline
input hashes remain unchanged. MCP compilation is warning-free; the adapter
retains its preexisting unrelated workforce-test import warning. Actual async
open/dispatch/close wiring received independent source review, but FastMCP and
the Cx-owned async entrypoints remain outside the executable source-view harness.
Full server, native, live and release qualification remain unverified.

### Whole-blueprint excavation progress through MCP (2026-10-30)

The existing eleven-tool dig-control server now registers a monitor for one
complete original Designated blueprint, samples it with `wait(monitor_id=...)`,
queries exact history and bounded target diagnostics, and stops it with
`cancel(scope="monitor")`. Operator-selected source pins and private custody
stay outside tool input. Creation is lazy at explicit registration, allowing
long-running designation projects to restart before a monitor exists. Recover
mode can register and sample using Query/Observe without Plan or Designate.

One bounded map/1.5 capture covers the original whole sparse mask. Durable raw
evidence precedes newly learned tick/grant checks; current source, original
receipt inventory and history are rechecked before fresh progress escapes.
A shared fixed stability window prevents independently timed regions or repeated
paused ticks from completing the project. Complete source/history/final-audit/
response allowances are prepaid before map contact and one absolute wall limit
covers connection, wire work and rendering. No game time advances automatically.

Agent Turns expose sampled progress, deficits, exact recovery and usable next
queries/sampling requests. Current-result proof is withdrawn from historical
query, terminal replay, errors, overflow, close and restart. Source selectors
explicitly do not prove shared process identity, mining causality, continuous
stability or structural safety. Local query/cancellation can inspect the monitor
within an existing session even after native/parent damage. Combined MCP restart
still requires those original files; the independent recovery modes described
above now reopen the monitor alone. Offline mode never appends or contacts the game.

All 119 focused tests pass on the final feature source: 77 adapter cases and
42 MCP handler/configuration cases, including 47 new adapter and 11 new MCP
monitor groups. All 388 adapter and 402 pipeline compiler/source input hashes
remain unchanged during their runs. MCP compilation is warning-free; the adapter
retains its one preexisting unrelated workforce-test import warning. The actual
production socket deadline wrapper compiles unchanged; FastMCP macros/stdio,
public async wrappers and the Cx-owned connection/session shell remain outside
the focused source-view harness. The query schema separately passes 49 valid and
175 invalid cases. This is component evidence, not full server/native/live or
release qualification. Existing production admission and native protocols remain
unchanged. See `docs/DIG_BLUEPRINT_MONITOR.md` for the complete operator workflow.

### Durable whole-blueprint excavation monitor core (2026-10-30)

The Rust adapter now binds the complete original sparse blueprint and every
terminal native Designated receipt to an independently pinned map/1.5 source.
One complete enclosing capture evaluates only the original selected cells.
Visible dry floors with cleared designations must hold together through one
fixed shared sampled stability window. Equal ticks cannot add samples; unknown,
unfinished, interrupted or over-gap observations break the streak. Native
Unknown cannot be discharged by terrain. Source/clock drift invalidates the
monitor; deadline and attempt allowance cannot renew on restart.

The private append-only history synchronizes read intent before map contact,
raw captures before learned-tick authority checks, and separate publication
before exposing progress. Exact recovery heads reject rollback while accepting
complete descendants after lost replies. Reopen preserves terminal historical
achievement, resets unfinished stability and fences unfinished durable reads.
Local cancellation requires current scoped Query and dispatches no native work.
The bounded foreground wire wrapper reuses the existing typed map protocol,
shrinking one deadline and checking scoped authority at every I/O boundary.

All 77 focused actual-source adapter tests pass: 47 new monitor model, durable
history, actual map-wire and private-file cases plus 30 existing blueprint
regressions. All 388 compiler/source input hashes stayed unchanged during the
run. Only the preexisting unrelated workforce-test import warning remains.
Tests cover complete sparse multi-level masks, original receipt substitution,
shared stability, clock/grant expiry, ambiguous writes, rollback, cancellation
and exact loopback protocol bytes. The temporary workspace compiles original
source with a minimal dependency manifest. This is component evidence, not
full Rust/FastMCP/native/live qualification or admission. No native protocol,
dependency, production runner or existing recovery-only profile is changed.
MCP composition follows separately; see `docs/DIG_BLUEPRINT_MONITOR.md`.

### Sparse mining projects through the eleven MCP tools (2026-10-30)

The existing dig-control development server now composes the retained Rust
blueprint parent with actual one-attempt native mining. A closed original floor
blueprint publishes its full shape, source, journal and deterministic keys.
Every next region requires a fresh observation and its own paired review;
parent identity, stop state, native prefix/head, exact plan and current policy
are rechecked at the effect boundaries, including after the dispatch marker.
Unknown, refused or cancelled predecessors block advancement. Gaps and levels
remain exact; no blanket bounding-box designation or multi-child commit occurs.

Complete original-prefix inventory is paged under both native and parent head
pins. Canonical layout export, exact offline/reconcile/control reopening and a
permanent parent stop are available through existing tools. Current native
Query/Cancel recovery remains usable when the parent is stopped or damaged.
Error, overflow, close and post-action runtime failure preserve historical
project/native identities without restoring a review or dispatch permit.
Agent Turns expose next/pending original work and structured review/recovery
steps. Historical `all_designated` is explicitly not excavation completion.

Control reopen verifies one fresh exact-source clock/sequence before issuing a
new lease, retaining both monotonic floors while discarding the observation
connection and selection. Offline/reconcile opening remains source-free.
Known initialization authority/checkpoint/budget failures are refused before
creating the native journal. Unpredictable cross-file failures retain evidence
without repair; parent/native publication is not an atomic transaction.

All 79 focused tests on the final feature source pass: 31 MCP handler/configuration
cases, 30 adapter model/store cases and 18 mining-session regressions. Coverage
includes sparse three-region execution, all 128 original records, lost replies,
post-marker parent corruption, durable stop, reopen after 2,000 ticks and native
sequence rewind. Final 393 MCP and 381 adapter compiler/source hashes remain
unchanged. The earlier core checkpoint also passed 27 adjacent regressions.
MCP compilation is warning-free; the adapter retains one preexisting unrelated
workforce-test import warning. JSON Schema checks pass 39 valid/145 invalid cases.
Changed Rust files pass rustfmt.

Required verification passes its first four checks, then reaches the unchanged
source-bundle wording gate. Development static qualification reports the same
76 baseline findings across 6,346 checks, with all 62 implicated files unchanged.
Full locked/offline metadata cannot resolve the unavailable exact FastMCP git
checkout in this environment. The dirty development failure receipt is not a
release qualification result.

The focused temporary workspaces compile original handler/adapter bodies but
exclude FastMCP macro/stdio/public async wrappers and ambient runtime ownership.
This is component evidence, not full Rust qualification, native/live evidence
or compatibility admission. The native wire, production protocol map and sealed
recovery-only server are unchanged. Generic mixed-action execution, multi-region
original-goal excavation proof and verified game checkpoint/restore remain open.
See `docs/DIG_BLUEPRINT_MCP.md`; the broad live-dispatch bead stays in progress.

### Durable sparse mining blueprint core (2026-10-29)

The Rust adapter now retains one complete sparse floor blueprint, its exact
native dig/1.16 source and journal, and a paused bootstrap capture. Geometry
matches the existing Python compiler: at most 32 nonoverlapping parts, 512
selected cells, a 1024-cell bounding capture and 128 deterministic native
rectangles. Gaps remain unselected. Every future halo and shared scheduling
block is checked before parent publication. Only a complete original prefix of
durably terminal Designated receipts unlocks another original child key.

The private parent has immutable original bytes and one permanent stop marker.
Exact initialization/reopening, scoped current authority, source and clock
floors, exclusive filesystem custody, complete readback and monotonic recovery
pins preserve failed or uncertain publication without repair. A lost stop reply
can adopt a complete newer stop; an acknowledged stop cannot reopen as unstopped.
Stopping future advancement never cancels native work or proves excavation.
A new bounded complete-inventory accessor verifies the native journal once,
retains all original records and never restores an old dispatch permit.

All 75 focused actual-source tests pass: 30 new model/store tests, 18 existing
mining-session tests, three private-file regressions and 24 furniture-batch
regressions. Tests include all 4095 nonempty 4x3 masks, independent Python digest
vectors, all 128 original child receipts, corrupt/torn/changed custody and actual
Linux private files. The final 381 compiler/source inputs remain unchanged.
The sole warning is an unchanged unrelated workforce-test import. The temporary
workspace uses original core/world/intent/adapter source with minimal manifests;
this is component evidence, not full Rust qualification, FastMCP compilation,
a native DFHack build, live-fortress evidence or compatibility admission.
MCP project orchestration follows in the increment above. Existing original
single-region semantics, native wire and production admission are unchanged.

### Finite advancement of original mining goals (2026-10-30)

The original mining runtime now composes optional run/1.13 clock ownership with
the exact original Mine plan, archive, paired review and terminal Designated
receipt. An independent operator source pin, credential and clock gate issue
native clock authority without converting named-fortress terrain grants. Exact
private `clock.journal` identity is required on reopening; a clock-only control
opening cannot restore designation permission. The native clock still identifies
an incarnation, endpoint and software rather than a named fortress lineage.

Through the existing eleven tools, `plan(kind="run")` prepares a finite interval,
`commit(scope="run")` confirms its distinct review and `cancel(scope="run")`
retires undispatched work or requests the native safety pause. Wait reconciles
retained clock work before a fresh paused original-goal sample; it never starts
or chains runs. Tick permission defaults to zero and is capped at 1200, further
bounded by the immutable original deadline and cadence. Up to 64 retained runs
support the original 64-sample stability bound without pruning history.

Independent exact-target readiness accepts only dry visible original Mine walls
or cleared floors without building occupancy. Pending walls remain unknown in
the canonical floor proof; readiness does not establish reachability, structural
safety or completion. This exception is limited to the exact original default
Mine floor goal with no unrelated terminal or failure condition. Fresh paused
evidence, original custody and clock authority are rechecked after the durable
marker, immediately before native prepare/unpause. Unknown dispatch never
permits another effect. Current proof is withheld across unsettled clocks,
failed observations and restart; later samples must respect retained clock floors.

Sessions explicitly opening with positive ticks use a 3 GiB logical byte-work
default when the caller omits `max_bytes`; ordinary sessions retain 2 GiB.
Explicit limits remain authoritative. Known complete work is checked before
clock contact, and newly retained history is charged again before preparation
or unpause. Longer history may require an explicit allowance up to 64 GiB.
Safety cancellation needs current Query/ControlClock authority without renewed
Plan, Observe, Designate, deadline or future-tick permission. Permanent native
clock loss retains the exact indeterminate handoff and recommends local release;
it never promises a pause or recommends a wait that cannot resolve the record.

Evidence: all 102 current-source tests pass without compiler warnings: 28 joined
and original runtime cases, 19 native-shell/clock cases, 16 clock-owner cases,
23 presentation cases, ten isolated facade boundary cases and six Agent Turn
cases. Both loopback-dependent native fixtures pass with the required execution
permissions. All 285 recorded compiler/source inputs remain unchanged across
the final run. The harness uses original core/world/intent/adapter source;
runtime views change only child-module paths, while facade extraction excludes
ambient runtime/session I/O and FastMCP macros/stdio. Full-file independent review
also corrected the facade dispatch method and stale server instructions. This
does not establish full-server compilation, Rust qualification, a native DFHack
build, a live fortress campaign or compatibility admission.

The current welfare/schema suite separately passes 14 cases with 421 unchanged
inputs. Required verification passes its first four checks and stops at the
unchanged source-bundle exact-clean-commit assertion. Dirty development
qualification reports 76 static findings across 6291 checks; all 62 implicated
files are byte-identical to base `6770fc5`, and none is a changed feature file.
The failed development receipt is not release qualification. No compatibility
entry or production runner protocol was added. Operator setup and exact recovery
are documented in `docs/SEMANTIC_DIG_CLOCK.md`.

### Historical citizen welfare comparison (2026-10-29)

The existing spatial/citizen server exposes `historical_citizen_welfare` through
`fortress.query`, including archive-only reopening. Two exact retained record
digests identify the endpoints. A complete strict-roster scan reports stress
categories and job availability before filtering or pagination; same-generation
rows compare category movement, elevated-category crossings, availability and
observed status flags. New, absent and replaced-generation identities stay
separate. Each row carries runnable inspections of its exact archived citizen
generation. The bounded cursor binds the current session/authority anchor,
archive identity/head, both endpoints, selection and work allowance.

All evidence remains historical and source-qualified. Category 3 may be the
native no-soul fallback; the comparison does not measure raw stress, physical
health, needs, causes or continuous trends. Leaving the strict roster does not
prove death or migration. Missing or mismatched native fields remain
unestablished. Queries neither acquire native state nor append journals,
advance durable watches, grant mutation authority or confer admission.

All 14 focused tests pass: nine model tests, four actual archive/live-handler
fixtures and one schema catalog case, with 421 compiler input hashes unchanged
across the final run. They cover
exact-record inspection, pagination, authority/anchor rejection, complete
summaries and unchanged journal/watch state. The minimal harness compiles the
repository's core, world, intent and adapter source plus spatial handlers and
Agent Turn code. Its source view omits only FastMCP imports/attributes/stdio
and the unrelated process-admission loader; handler and model bodies are
unchanged. External dependencies retain the exact repository serde/JSON pins.
This is component evidence, not a qualified FastMCP executable, native build,
live campaign or release. Operator examples and the query contract are in
`docs/HISTORICAL_CITIZEN_WELFARE.md`.

### Mining review windows and interrupted proof (2026-10-28)

The original-mining runtime now permits human review between foreground requests
on its retained preparation socket. A newer trusted request in the same session
receives a bounded wall window, capped at 60 seconds; callbacks within one request
only narrow the deadline. The original socket, token, preparation permit, prepaid
byte allowance and game-time limits remain unchanged. Foreign/regressed requests,
cancellation, active expiry, malformed replies and source/authority errors fence
the owner permanently. Existing retirement/checkpoint distinctions are preserved.

Every failed wait withdraws cached current goal proof, including failures before
polling or in facade context/publication checks. The next permitted wait durably
records the interruption within its own budget before another sample can continue
unfinished stability. Historical first achievement and exact original custody
remain intact. Bounded retained references suppress cached proof, preparation and
enablement flags, including after overflow; exact recovery identities remain.

Evidence: 66 focused current-source cases pass without warnings: 16 runtime,
12 native, 15 presentation, six Agent Turn and 17 adopted adapter tests. The
recorded production-source hashes remain unchanged across the final run. The
temporary runtime view adds only four absolute child-module paths; runtime,
authority and test bodies are unchanged, as is the admission-prefix getter.
The earlier full locked MCP library check was SIGKILLed while checking pinned
asupersync 0.5.0 before MCP typechecking in the memory-constrained shared runtime.
FastMCP registration/stdio, ambient runtime acquisition, full Rust qualification,
native/live evidence and admission remain unverified. The operator guide and
emitted commit guidance now describe the per-request wall windows explicitly.
Required verification passes its first four checks, then reaches the unchanged
source-bundle documentation assertion. Integrated static qualification reports
76 existing findings across 6199 checks; all 62 implicated files match the prior
commit and none of the follow-up files introduces a finding. The failed dirty
development receipt does not establish release qualification.

### Documented MCP names registered explicitly (2026-10-28)

The spatial/citizen and original-mining servers now explicitly register all eleven
dotted `fortress.*` tool names. The pinned FastMCP macro's default is the Rust
function name; explicit names align registration with emitted calls and the
frozen public surface. All handler bodies remain byte-identical after excluding
tool attributes. The historical-job fixture cleanup also reran all 15 focused
model/archive-handler/schema tests successfully; twelve unused-import warnings
belong only to the temporary admission-prefix harness. Full macro/stdio execution
remains unverified under the existing pinned-dependency build limitation.

### Original mining workflow through MCP (2026-10-28)

A dedicated dfmcp-live-semantic-dig-dev-server now exposes the original mining
adapter through the frozen eleven tools. It acquires a complete paused canonical
terrain basis, pages exact observation-bound tiles, compiles a closed single-Mine
request, retains the full original archive and paired native review, confirms one
designation, and acquires independently qualified original-goal evidence. The
first post-designation wait initializes private proof history; subsequent waits
sample terrain. Wait never advances game time or starts background polling.

The concrete operator shell pins exact dig/1.16 and spatial/1.8 incarnations,
software, named fortress, dimensions and scope independently. Current credentials,
runtime I/O/cancellation, checkpoint policy, protected whole-block writes and
exclusive host-local leases are checked at bounded native boundaries. The initial
bootstrap capture's clock is retained privately before custody opens, without
publishing an unqualified canonical anchor. Exact private file creation/reopening
prevents missing custody from becoming a new goal. Sources, credentials, paths,
checkpoint exceptions and raw native keys cannot enter through tool arguments.

Offline and reconcile reopening preserve exact original history without restoring
a native dispatch permit. Current goal truth, historical first achievement and a
historical Designated receipt remain separate claims. Unknown native outcomes do
not permit redispatch or endless recovery recommendations. A confirmed recovered
preparation offers explicit retirement only with current native authority;
retirement requires no fresh Plan grant, lease or checkpoint exception. Error,
overflow and session-close packets retain bounded recovery identities, withdraw
cached goal/control claims and never inherit production-admission provenance.

The default wall allowance is the existing 60-second cap, with explicit narrowing
still supported. Trusted foreground requests now receive separate wall windows
on the same retained native connection as described above. A failed active request
cannot renew a fenced connection or recover a commit permit by reconnecting.
Known planning work is
fully prepaid before source acquisition or original archive publication, including
all three evidence acquisitions, both native stages and fixed archive/guard/output
costs. A byte-budget refusal leaves the original observation and custody unchanged.

The initial workflow baseline, before the follow-ups above, passed 113 distinct
focused tests without warnings:

- four actual-world partial-terrain regressions;
- 17 public actual-adapter execution/goal/history cases;
- 15 joined original-runtime lifecycle and failure cases;
- nine native source/configuration/private-custody and cost-bound cases;
- 13 actual Agent Turn presentation cases and seven verbatim facade cases;
- 34 adjacent dig journal/session, seven control-policy and seven spatial cases.

The combined 91-case component run passed with 413 actual source/fixture/harness
inputs unchanged. After final prepayment/default-window corrections, all 35
affected runtime/facade/presentation cases pass with another unchanged 413-input
snapshot. Unchanged native/adjacent/adapter/world cases retain their prior passing
evidence. Every ordinary joined workflow uses the actual facade-retained default
allowance (2 GiB minus 256 MiB minus 128 KiB), 8192 observed nodes, zero game ticks
and 430 MiB for initial evidence and each refresh. The actual native cost helper
fits that envelope. A larger 520 MiB source refuses before source/archive/native
preparation and successfully reuses the same basis with sufficient already
negotiated allowance. Full FastMCP macros/handlers/stdio, a real DFHack build,
live-fortress execution and compatibility/runtime admission remain unverified.

Required verification and static development qualification were run again after
preserving concurrent historical-job-progress commit 577e4a02f831560e6bd8a9d233dd8699c1b37956.
None of its 15 changed paths overlaps the mining compile inputs; all 413 mining
input hashes are unchanged. Verify passes its first four checks and reaches the
same source-bundle documentation assertion. Static qualification reports 77
findings across 6198 checks: all 76 starting findings plus a forbidden expect in
the incoming, unchanged job_progress_tests.rs. Mining introduces no new finding.
All 2476 repository file hashes, the path set and HEAD remain stable during the
runs. The failed dirty development receipt has no Rust-toolchain or release
qualification claim. This evidence paragraph records those completed runs;
production code and the operator guide are unchanged afterward.

The operator guide is docs/SEMANTIC_DIG_DEVELOPMENT.md. This remains an isolated
unadmitted development profile with one original rectangle/goal per custody
bundle. Multi-region and general action-DAG execution, automatic mining clock
advancement, checkpoint/restore, mining reachability and structural-safety proof
remain outside this implemented slice.

### Historical job progress across exact retained observations (2026-10-28)

The existing spatial/1.8 `fortress.query` surface now accepts
`historical_job_progress` in journal-backed live and archive-only sessions. It
compares exact original job generations and independently sourced suspension,
worker assignment, holder/build-stage and attached-material indicators at two
selected records. Disappearance, replacement and unestablished evidence remain
explicit; unchanged endpoints do not establish a stall or completion.

The full-roster summary precedes filtering and whole-row pagination. Every row
includes record/digest/generation-pinned inspection queries. Current authority,
exact archive custody, source incarnation, reset boundaries, complete response
bounds and session-bound continuations are rechecked. Reads perform no native
capture, journal write, watch progression or game effect.

Evidence: all 15 focused actual-source model/archive-handler/schema tests pass
(10 model, four handlers and one schema regression). The temporary harness
omits FastMCP tool attributes/stdio startup and uses the unchanged admission
provenance declaration/getter; it does not test transport registration, admission
ticket loading or a running game. The twelve unused-import warnings belong only
to that temporary admission prefix. No compatibility admission is added.
See `docs/HISTORICAL_JOB_PROGRESS.md` for the query and precise proof limits.

### Original mining execution and durable terrain proof (2026-10-28)

The adapter now connects one original canonical Mine rectangle (1..8 by 1..8,
one z-level) to exact dig/1.16 preparation, one confirmed dispatch, original-key
reconciliation and fresh spatial/1.8 proof. The canonical terrain projection is
independently scoped and retains explicit unknown cells. Its new routing
constructor takes a source policy and validates the exact projection's eligible
domains; laboratory evidence, broad entity/edge coverage and detached hashes
cannot be substituted. Spatial and dig generations remain independent.

An immutable private association binds the full original object and anchor,
original action key, exact native plan/witness and initial spatial source identity.
The original predicates, terminal/failure conditions, deadline, cadence and
stable-sample obligation remain unchanged. Source/authority/custody are checked
after native intent and at the post-dispatch-marker pre-RPC boundary. Native
journal bytes are verified again after external policy work. Recovery cannot
restore a lost live preparation permit, and a missing query response or permanent
native Unknown cannot authorize a replacement dispatch.

A terminal Designated receipt records historical designation only. Goal polling
links that exact retained receipt and original rectangle to one fresh complete
paused spatial capture, with exact local native-journal and association checks
around acquisition. It does not issue a new dig-query RPC bracket. Only observed
dry floors with cleared dig designations can prove the original excavation.
Hidden, unallocated, unsupported or wet cells remain unproven. All original
postconditions and terminal/failure predicates participate in the original
cadence/deadline/stability evaluation.

Private goal history retains Started/Captured/Published/Interrupted boundaries,
source/cursor/tick floors and first achievement within 64 MiB and 1024 events.
Reopening independently reauthorizes history, clears current proof and interrupts
unfinished stability without changing the original plan or deadline. Denied
policy cannot bank a stable sample or erase a consumed private cursor. Source
incarnation drift, replaced custody and corruption fence the owner. Exact-create
and exact-reopen native journal APIs prevent accidental custody recreation.

Focused actual adapter validation passes 17 integration tests without warnings;
all 396 recorded actual-source, fixture and manifest inputs remain unchanged
across that run. Cases include single dispatch and duplicate acknowledgement,
lost replies, recovered-preparation refusal, post-marker policy revocation and
persistent native-journal tampering before RPC, unknown/unsafe terrain, current
completion and temporal stability, exact historical recovery, denied policy,
source drift, corrupt history and immutable deadlines. The four prerequisite
world regressions also pass. The concrete operator shell and eleven-tool MCP
entry point are separate integration work at this checkpoint. No full FastMCP,
native DFHack build, live fortress, compatibility or runtime admission is claimed.

### Partial canonical terrain for original mining (2026-10-28)

Canonical terrain can now retain explicitly unknown cells inside otherwise
observed chunks. Tile lookup reports those cells as unavailable, three-valued
terrain predicates cannot certify them or their negation, and reference mutation
refuses to turn them into invented terrain. The existing spatial index requires
fully observed chunks and rejects partial chunks instead of assigning safe
properties. Canonical snapshot bytes preserve this distinction without changing
the format or existing complete-chunk encodings.

All four focused actual-world integration tests pass without warnings, covering
roundtrip identity, predicate evidence, mutation refusal and spatial-index safety.
This is the prerequisite for a new independently scoped original-mining terrain
projection. The canonical-plan/native mining handoff and dedicated MCP runtime
are being implemented separately; this checkpoint does not claim them complete
or establish native, live, full-server or compatibility qualification.

### Original construction and finite clock runs through MCP (2026-10-28)

The original furniture workflow now exposes separately reviewed bounded game-time
advancement through the existing eleven tools. Opening accepts an optional
max_game_ticks allowance with a zero default and 1200-tick ceiling. A closed
plan(kind=run) request binds the original paired seal and finite game/wall limits;
commit(scope=run) requires its distinct clock review and explicit confirmation.
Placement confirmation and authority remain separate. Safety cancel(scope=run)
retires an undispatched review or requests its native owner's pause, while
cancel(scope=session) releases only local resources.

Agent Turns propose a cadence-sized interval only when current qualified
receipt-linked construction remains viable and unfinished, and the original
deadline and negotiated allowance permit it. Known pending construction may
have false completion predicates without blocking useful advancement. Unknown,
blocked, failed, mismatched, expired or fulfilled goals cannot receive a new run
offer. A stale undispatched review offers explicit retirement. Wait first
reconciles retained clock work and withholds paused construction sampling while
that clock may still be running; it never starts, extends or chains a run.

The runtime, tool facade and bounded response path retain the exact original
and clock journal identities through partial errors, lost replies, close and
response overflow. Clock uncertainty is independent of placement uncertainty.
Cached goal truth is suppressed after a possible clock effect and until a fresh
qualified construction sample reaches the retained clock floor. Historical clock
receipts prove neither current pause nor original goal completion. Released
handles offer no further work, and development packets cannot inherit ambient
production-admission provenance. The operator guide documents the complete setup,
request sequence, exact recovery handoff and independent clock-only reopening.

Final focused validation passes 122 distinct tests without compiler warnings:

- 74 actual-source component cases: 16 clock-owner, ten joined construction/
  clock/recovery, eight original runtime, 15 native/configuration/custody,
  16 Agent Turn presentation and nine verbatim facade boundary cases;
- 26 public adapter/coordinator/goal/archive integration cases;
- 22 existing bounded-run core/journal/RPC cases.

The joined cases execute viable pending construction through finite advancement,
original completion and stability with the ordinary 2 GiB allowance after facade
reservations. The full 64-sample goal completes through 64 explicitly confirmed
runs and fits the new 1024-event bound. Exact offline recovery preserves original
achievement and all 64 runs with an explicit 64 GiB logical allowance; 32 GiB
refuses safely without native contact or history changes. Lost replies, native
edge refusals, immutable original deadlines, clock-floor rejection and
Query/ControlClock-only safety cancellation are covered. A denied policy retains
its consumed private canonical cursor while withholding public proof, allowing
both a later safety pause and a fresh higher-cursor sample.

All 454 source/manifests/harness inputs remained stable across the 74-case run.
The static gate then identified one newly introduced expect in a test helper;
that sole changed test input now propagates Result, and all 16 affected owner
cases pass again without warnings. The other inputs remained identical. The
component target compiles actual runtime/native source and the complete facade
Action dispatcher, but excludes FastMCP macro expansion, handler registration,
transport and the complete stdio executable. The six private goal-history unit
cases previously blocked by build storage are not included in this executed
count. No native DFHack build, live campaign or compatibility admission is claimed.

Required repository checks ran against frozen implementation and guide files.
verify.sh passes its first four checks and stops at the unchanged source-bundle
assertion about the exact clean git commit. The final static dirty-tree
qualification has 76 failures across 6,014 checks; every failure line is identical
to the prior baseline, in the same order, with zero new findings. All 63 files
implicated by those findings and the source-bundle assertion match the original
baseline commit. The failed development receipt records dirty=true and
head_equivalent=false, with no cargo/rustc qualification evidence. All 2,447
repository file hashes and HEAD stayed stable during that run. Final evidence
annotations do not turn it into a clean release receipt.

The [operator guide](docs/SEMANTIC_FURNITURE_DEVELOPMENT.md) contains the complete
workflow. Automatic chaining, a multi-step furnishing dispatcher, a checkpoint/
restore provider and named-fortress identity in the native clock remain open
functional gaps. The compatibility registry and production protocol map are
unchanged; the broader semantic dispatcher bead remains in progress.

### Original furniture bounded clock owner and native composition (2026-10-28)

An optional separately authorized run/1.13 owner can now advance the original
furniture construction goal through finite, explicitly reviewed intervals. Each
clock target binds the exact original PreparedPlan and archive object, placement
journal, paired review and verified Placed receipt, including the original
building, item and job identity. The original deadline, cadence, stability and
failure predicates remain unchanged. There is no automatic chaining, replacement
goal or retry of an uncertain unpause.

Fresh independently qualified construction evidence must be receipt-linked and
viable. Known unfinished construction is eligible when its condition is pending;
condition_met is eligible while the original temporal obligation remains
unfinished. Unknown truth, a failed predicate, suspended or missing work,
unverified items, removal and changed identity refuse a new interval. Original
archive custody and fresh construction policy are checked again after the clock
journal's durable marker, immediately before the native effect boundary.

Canonical fortress authority and native source authority remain independently
issued. Run/1.13 identifies a source incarnation, not named-fortress lineage;
matching endpoint/software pins and ticks do not turn that source into lineage
proof. The optional operator configuration retains an independent clock generation,
credential and enablement gate. Exact create/reopen discovery checks the fixed
private clock.journal before furniture custody can be initialized. Existing
bundles without a clock remain usable but cannot silently acquire a new clock
journal on reopening. Recovered placement cannot dispatch again; independently
authorized clock control can continue the retained original construction goal.

Wait reconciles unsettled clock work before acquiring paused construction proof.
A potentially running clock suppresses current goal truth, and retained clock
ticks become a minimum for subsequent construction evidence. Source/canonical
floors consumed by a denied policy stay private and monotonic without publishing
an unqualified current anchor. A later safety pause or fresh sample can continue
from that exact floor. Safety cancellation needs Query and ControlClock; it does
not require Plan, Observe, Construct or an extended original deadline.

At most 64 original-bound runs are retained, with one unsettled run. Furniture
history now permits 1024 events within the unchanged 64 MiB file bound, covering
fresh review/dispatch samples throughout a 64-sample stability goal. The logical
byte-work ceiling is 64 GiB for complete history reauthorization; the ordinary
2 GiB default and physical file/frame/native limits remain unchanged. Runtime
opening prepays 576 MiB, including optional clock opening and inspection, before
custody creation. No allocation of the full logical allowance is implied.

Final core/native validation passes all 49 actual-source cases: 16 clock-owner,
ten joined construction/clock/recovery, eight original runtime and 15 concrete
native/configuration/custody cases. The current 26 public adapter integration and
22 existing bounded-run cases also pass, giving 97 relevant core/native tests.
The immediate same-tick observation correctly earns no temporal stability credit;
full 64-run completion and exact recovery, plus denied-policy safety cancellation
and fresh retry, now pass. The MCP section above records the complete 122-test
integration evidence and qualification limits. No native/live or admission claim
is made, and the compatibility registry and production map remain unchanged.

### Original furniture goals through the MCP loop (2026-10-28)

The dedicated `dfmcp-live-semantic-furniture-dev-server` carries one original
single-tile bed, chair or table goal through the frozen eleven tools. A complete
paused spatial observation supplies canonical item pages. Closed plan input
retains the original Build, postconditions, action key, absolute deadline,
cadence and stability while separately sealing the exact item in the paired
native review. Explicit confirmation performs one placement; subsequent waits
reconcile the original key or acquire receipt-bracketed construction samples.

The concrete native shell binds independently pinned spatial/1.8, furniture/1.19
and operations/1.4 sources. Paths, loopback endpoint, credentials, source pins,
protected regions and host-local construction leases remain operator inputs.
Exact private create/reopen APIs preserve journal and immutable association
custody. A required checkpoint is never removed from a sealed plan; the explicit
disposable-fortress exception is decided before fresh initialization and plan
compilation. Fresh opening prepays all 512 MiB of logical work before custody
creation, in addition to facade inspection and response reservations.

Original publication retains its intended archive identity before storage I/O,
so a lost acknowledgement cannot substitute another goal. Error, close and
overflow packets preserve recovery references and uncertainty without granting
authority or making cached evidence current. Offline reopening performs no native
calls. Restarted sessions cannot restore a placement permit. A recovered native
preparation can be queried and explicitly retired; fenced histories recommend
preserving the handoff and releasing the session. Closed packets retain closure
state and offer no continuation on the released handle. Unknown completion stays
unknown; historical placement, current construction and first achievement remain
distinct claims.

The explicit read-only archive authority-floor API permits authorization at a
retained later tick with the same exact original cursor/hash, while returning
the original anchor unchanged. Strict archive APIs and publication retain their
behavior. Recovery checks Query at current authority time, rejects clock rollback
and changed identity, and cannot revive a grant that expired after planning.

Final focused evidence passes 63 distinct tests: 32 actual-source component cases
(eight runtime, eight concrete native composition, nine Agent Turn presentation
and seven verbatim facade boundary cases), 26 public adapter/coordinator/goal/
archive cases and five concrete spatial wire-codec cases. All passed with no
compiler warnings. The component target compiles the real native/runtime sources
and complete facade Action dispatch. It excludes FastMCP macro expansion, handler
registration, transport and the complete stdio executable. Six private
goal-history unit cases were front-end checked during an earlier attempt but
could not complete code generation/linking because build storage was exhausted;
they are not included in the executed count.

Required static verification was attempted on the frozen Rust source.
`verify.sh` stops at the existing source-bundle documentation phrase check.
Static dirty-tree qualification fails with 76 existing findings across 5,964
checks: 67 forbidden-call/macro findings and nine room-example schema errors.
The failure lines match the prior run, and all 62 files they name match the
prior committed head. None names the new furniture implementation. The receipt
records failed, dirty and not head-equivalent; these runs do not establish full
Rust or release qualification.

The [operator guide](docs/SEMANTIC_FURNITURE_DEVELOPMENT.md) documents exact setup,
tool inputs, current versus historical claims, bounds and restart handoffs.
Wait never advances game time or retries placement automatically. Construction
clock composition, multi-step furnishing, a checkpoint provider, native/live
campaigns, compatibility admission and production runner admission remain open.
The broader mutation/dispatcher bead remains in progress.

### Original construction goals and durable proof (2026-10-27)

The original furniture owner now has process-local and durable goal monitors.
They reuse the strict receipt-linked construction assessor and fixed native
query/capture/query transport, without substituting a new goal or deadline.
Fresh evidence must name the original fortress, software and furniture source,
a separately pinned operations generation, and the original building, item and
construction job. Completion requires the installed original item, maximum
construction stage, and no outstanding construction/removal or item-job links.
The selected canonical building is the identity derived from the original action
key. Its construction fact is explicitly a certified derivation, requiring
independent policy; the projection grants no global absence or terrain coverage.

The monitor preserves original postconditions, plan and obligation terminal
predicates, failure predicate, game deadline, cadence and sampled stability.
Duplicate game ticks cannot accumulate stability. Unknown facts remain unknown;
a later contradiction clears current completion while historical first
achievement remains separately visible. A monitor is exclusive and cannot be
cloned to discard an observed contradiction. Source incarnation, allocation,
stage, retired-identity, tick and consumed-cursor floors survive policy denial
and monitor replacement.

Durable history is limited to 64 MiB and 512 events, with all possible current
poll records prepaid before source contact. It synchronizes Started before the
read, complete LinkedSample evidence before qualification, and Published before
acknowledging proof. Recovery verifies exact original native custody and issues
historical policy twice independently; stored fingerprints cannot reconstruct
policy. Restart exposes historical achievement only, clears the current result
and breaks unfinished streaks while keeping the original cadence/deadline.
Interrupted or ambiguous acquisitions and publication failures fail closed.

The actual adapter source compiles and all 28 focused tests pass: 23 joined
coordinator, construction and history integration cases plus five concrete
spatial wire-codec cases. This does not establish a live DFHack campaign,
automatic construction clock advancement, multi-step furnishing, production
qualification or compatibility admission. The broader mutation bead remains in
progress.

### Original canonical furniture placement and recovery (2026-10-27)

`SemanticFurnitureSession` now owns one original canonical `Build` for a
single-tile bed, chair or table, using the existing closed furniture resolver
and native coordinator. Its paired review binds the lossless original archive
object as well as semantic digest, stable action key, source witness, exact
native plan and original native journal. The immutable association store must
be synchronized before native Intent. Complete original spatial/1.8 evidence
is refreshed after Intent and at the final pre-writer boundary after
DispatchStarted; independent caller policy and exact file custody are checked
again after refresh. Furniture and spatial generation counters remain separate.

Preparation and historical placement are distinct from the original
construction goal. The placement result deliberately reports that the original
goal is unproven and retains its original predicates and temporal obligation.
Read-only reattachment, original-key query, preparation retirement and lost-reply
recovery cannot create a dispatch permit. Repeated calls return exact retained
state without repeating placement. Unsupported selectors, dependencies,
compensation, unprotected preconditions and mandatory checkpoints are refused
without weakening the original plan.

Production adapter source compiles. New focused tests exercise lifecycle,
durable-before-effect ordering, stale evidence, revoked authority, replaced
custody, insufficient budgets, source generation separation and exact restart.
All 20 focused tests pass: 15 coordinator/native-guard integration cases on the
actual production adapter and five concrete spatial wire-codec cases.
The broader live mutation bead remains in progress; source/component evidence
does not establish native or live qualification, compatibility admission, a
multi-step dispatcher, or original construction completion.

### Explicit labor clock control through the MCP loop (2026-10-27)

The frozen eleven-tool semantic workforce server now exposes the bounded clock
owner. `open_session` keeps a zero game-time default and accepts an explicit
`max_game_ticks` allowance up to 1,200. Closed `plan(kind="run")` binds one finite
interval to the original paired labor review; `commit(scope="run")` confirms
the separate clock review; `cancel(scope="run")` uses independent clock
authority for the native owner's safety pause. `wait` reconciles the retained
run before a paused labor sample and never starts or chains intervals.

Agent Turns recommend only a bounded interval within the original cadence,
remaining deadline and negotiated allowance, after verified assignment and
fresh true unfinished labor predicates. Current control authority, clock source
identity and original custody remain explicit. Uncertain effects and the newest
clock journal identity survive failed calls, bounded overflow and session close
without turning cached evidence into current proof or permission. Clock-only
recovery cannot regain labor authority; no original deadline or unfinished
stability streak is renewed by a run or restart.

Focused validation passes 146 distinct actual-source component tests: the
137-test combined runtime/presentation/Agent Turn target and nine verbatim
workforce facade boundary cases. The complete facade Action dispatch compiles
against the real runtime. The target includes 49 workforce runtime cases and
25 workforce presentation cases, plus existing work-order and shared regressions.
The ordinary core28, workforce-integration47 and bounded-run22 suites also pass.
These checks exclude FastMCP macro expansion/transport, the complete executable,
DFHack SDK compilation, live gameplay and compatibility admission. The required
repository verifier passes its first four contract checks, then stops at the
unchanged source-bundle documentation phrase check; both that guide and checker
are byte-identical to the initial main revision. No full qualification is claimed.

The complete operator configuration, closed tool requests, exact recovery and
restart-cadence example are in
[`docs/SEMANTIC_WORKFORCE_MCP.md`](docs/SEMANTIC_WORKFORCE_MCP.md).

### Original labor goals with reviewed clock advancement (2026-10-27)

The original workforce runtime now composes its retained `SetLabor` goal with
the existing bounded-run/1.13 coordinator. Each finite run has a distinct review,
explicit confirmation and durable dispatch marker. Exact original plan, citizen
identity, verified assignment, absolute deadline and current same-tick labor
evidence are rechecked after that marker before native unpause. Lost replies,
source loss and restart preserve the original run without another unpause.

Clock authority is independent of labor authority. Run/1.13 supplies source
incarnation identity rather than named-fortress lineage; the shell explicitly
issues separate source-scoped authority under the clock gate while retaining
independent named-fortress checks. Optional exact private clock custody reopens
without recreating missing files or adopting substituted identities. Offline
recovery needs no native credentials. Clock-only recovery cannot regain labor
control, and safety cancellation does not require fresh labor truth or Plan.

The owner retains at most 64 runs, one unsettled at a time. `wait` reconciles
clock uncertainty before a paused goal read; new goal projections older than
retained clock evidence are rejected before proof publication. A pre-run sample
is suppressed as a current goal while a run may still be active. Historical
assignment, clock receipts, original sampled goal proof and first achievement
remain separate. No run chains automatically or changes the original deadline.

Executed validation passes all 49 actual-source workforce runtime tests: 17
clock-owner, six native clock/configuration/custody, nine joined clock/goal,
13 original runtime and four existing native-shell cases. The joined cases
include 64 original samples through 63 finite runs, exact offline restart,
clock-only control, lost replies, post-marker custody/authority changes and
stop-tick rejection before goal publication. Reopening correctly resets an
unfinished stability streak and still requires later cadence-qualified samples.
The existing core suite also passes 28 tests, ordinary semantic-workforce
integration passes 47, and actual bounded-run codec/RPC/journal tests pass 22.
The actual-source component target compiles the real runtime and adapters with
injected native/storage edges; it does not compile full FastMCP transport/macros.
No full-server, native, live or admission qualification is claimed. Explicit MCP
run dispatch follows in the next checkpoint. See
[`docs/SEMANTIC_WORKFORCE_CLOCK.md`](docs/SEMANTIC_WORKFORCE_CLOCK.md).

### Original native labor goals through MCP (2026-10-27)

The dedicated `dfmcp-live-semantic-workforce-dev-server` exposes the original
labor-goal runtime through the frozen eleven tools. A caller observes canonical
citizens, selects 1–32 identities, prepares one exact labor enable/disable goal,
reviews its paired semantic/native seal, confirms one assignment and obtains
fresh original-goal evidence through bounded foreground waits. Citizen discovery
uses complete acquisition of at most 128 citizens and observation-bound pages of
at most 32 identities. Clients cannot supply native IDs, detail indices, effect
keys, source endpoints, credentials, paths or evidence grants.

Agent Turns retain the complete original goal and exact recovery identities.
Historical native Applied, current labor predicates, temporal fulfillment and
first achievement remain distinct. A permanent Unknown outcome cannot become
semantic completion merely because current labor values are true. Current
operator authority and an exact retained observation are required before commit
is recommended. Failed calls merge the owner's newest pure recovery references
before rendering, so a published original archive or newly initialized history
cannot disappear behind an older cached response. Those references remain
unverified, cannot authorize control and survive close and bounded overflow.

Capacity testing now executes a complete 32-citizen assignment from a 128-citizen
roster. It found that this plan uses 557 aggregate archive nodes, so the narrow
archive profile now permits 1,024 nodes while retaining 64 KiB, one step, depth 16
and the existing native predicate limits. The measured original archive is 4,824
bytes. The supported 64-sample stability history also survives offline restart:
its 192 retained events exceed the default 2 GiB logical replay allowance, so
`open_session` accepts an explicit allowance up to 32 GiB. The ordinary default
remains 2 GiB. A real 64-sample case verifies safe default-budget refusal and
successful 8 GiB recovery with no native contact or history mutation. These are
logical work ceilings, not eager allocations; native cardinality, history file
and event limits, original deadlines, and the 60-second wall ceiling stay fixed.

Final executed validation passes 114 distinct current-feature tests: 24 ordinary
adapter workforce-control tests, 47 public semantic-workforce integration tests
and 43 combined actual-source tests. The combined target contains 13 joined
runtime cases, four native/configuration/custody cases, 13 presentation cases,
seven verbatim facade-boundary cases and six shared Agent Turn cases. It checks
single dispatch, lost replies, post-marker archive/authority failure, immutable
modes, exact restart, unfinished-stability reset, later false current truth,
32-citizen capacity, 64-sample recovery and complete failure handoffs. Actual
Action dispatch, closed parsers, the live callback constructor signature and
`NativeRuntime: Send` compile. The final run has no warnings and no source drift
across 352 inventoried source/dependency files. Its task log is
`workforce-43-tests.log`, SHA-256
`ae8d2f3a3785b9c52bb32aa0ac4d21245ec045e6ad7fd268197b253c3e3281d5`.

This is actual-source component evidence using complete real adapter dependencies;
it does not compile FastMCP macros, full handler composition/registration, the
executable or stdio. The unchanged full MCP graph was not blindly retried after
the recorded upstream compiler SIGKILL: the shared memory ceiling remains 8 GiB.
No full-workspace, DFHack native or live-game qualification is claimed. The empty
registry and production runner map remain unchanged. Native labor control still
requires one existing exact single-labor selected-only work detail, supplies no
job/tool/path/productivity guarantee, and does not fence other controllers.
See `docs/SEMANTIC_WORKFORCE_MCP.md` for setup, all tool transitions, bounded
recovery and the limits of incomplete original-file publication.

### Original native labor-goal execution core (2026-10-27)

A new `semantic_workforce_runtime` joins the existing native workforce coordinator,
original-plan archive, canonical citizen identity and durable original-goal monitor.
Its closed request compiles one `SetLabor` goal for 1–32 canonical citizen IDs,
one exact labor key and desired value, with an unchanged absolute deadline,
cadence and stability requirement. The native profile cannot execute an inverse
compensation, so this compiler explicitly seals a noncompensating original plan
before publishing or reviewing it. It does not drop a caller-supplied compensation.

The runtime archives the original complete citizen capsule as immutable historical
identity and independently reauthorizes it on exact recovery. Native and citizen
incarnations remain separately pinned. A paired review binds original semantics
to the exact native work-detail assignment; one explicit confirmed dispatch is
followed by fresh selected-citizen labor observations. Historical native Applied,
current original-goal proof and first achievement remain separate. A true goal
cannot repair an Unknown native action. No clock advancement or background polling
is added.

Fixed private custody distinguishes initialization from reopening. External journal,
plan archive and citizen identity digests are required on recovery; omitted original
files or a mismatched history-presence flag cannot hide retained work. Offline opens
use no native connection or credential. Read-only recovery cannot regain labor
authority. Original plan and identity custody, current operator authority and the
live runtime cancellation/I/O hook are checked at the native effect boundary after
the durable dispatch marker. Partial publication retains identity for exact retry;
corrupt or uncertain custody is never repaired by replacing it.

At this core checkpoint, the generic owner and concrete resource shell are exported
through the Rust API; dedicated MCP facade registration follows in the next increment.
Executed verification passes 71 ordinary adapter tests and 14 actual-source
core tests. The latter include ten joined lifecycle/fault cases and four concrete
configuration/custody/checkpoint cases. Exact custody openers also execute real
private-file checks. The component harness compiles the actual runtime, native shell,
archive codec, original goal monitor and complete real adapter dependencies. It does
not compile FastMCP macros, the complete handler composition or stdio. The existing
full MCP dependency graph remains unqualified after the previously recorded upstream
compiler SIGKILL; its unchanged memory limit was checked and no blind retry was made.
No native DFHack, live-game, full-workspace or production admission is claimed.

The concrete source acquires at most 128 citizens completely and selects at most 32
for a goal. Work must map to one existing exact single-labor selected-only work
detail. Other labor columns must retain their captured values for semantic action
verification. This does not create details, allocate jobs/tools, guarantee productivity,
fence other controllers or prove continuous truth. Existing native wire, workforce
journal, plan formats, the empty registry and production map remain unchanged.
See `docs/SEMANTIC_WORKFORCE_HANDOFF.md` for the new runtime API and retained limits.

### Reviewed native goal advancement through MCP (2026-10-27)

The semantic work-order development server now exposes the bounded execution
core through the existing eleven tools. `open_session` negotiates an explicit
0–1200 game-tick allowance (zero by default). After verified insertion,
`plan` accepts the closed `kind: "run"` request and derives the native order
from the original Created receipt. `commit(scope="run")` confirms the separate
clock review; `wait` reconciles that same run before attempting fresh paused
original-goal proof, and `cancel(scope="run")` retires preparation or requests
the native owner's safety pause. No tool chains another run automatically.

Production and clock authority remain independent. Clock-only Control recovery
cannot regain insertion authority. Every native source boundary rechecks an
injected live runtime cancellation/I/O checkpoint; in the concrete MCP facade
this is the owned Asupersync context, including after a durable dispatch marker
and before unpause. Closed request parsers cannot choose an order ID, predicate,
key, endpoint or replacement deadline. A native stop receipt remains separate
from the exact original goal, sampled temporal proof and physical quiescence.

Clock-aware Agent Turns expose the active review and exact recovery identities.
Recommendations for an already-true goal preserve the original polling cadence
so multiple stable samples can fit before its unchanged deadline. The validated
opening identity is retained as an explicitly unverified reference before the
first inspection; failure, close and bounded overflow cannot discard a newly
created clock journal ID. Current custody must still be verified before control.
The opening inspection reserves 32 MiB and the complete response 128 KiB.

Final executed verification passes 155 distinct tests: 28 ordinary adapter
order-run tests, 58 public semantic work-order integration tests and 69 combined
actual-source tests. The combined target includes 17 Runtime tests (seven joined
clock lifecycle/fault cases), 16 clock-owner tests, five concrete native/config
checkpoint tests, 19 presentation tests, six shared Agent Turn tests and six
verbatim facade-boundary tests. The actual Action dispatch implementation and
closed parsers compile against concrete NativeRuntime; its Send and checked
opening constructor signatures also compile. Complete adapter library checking,
scoped formatting and diff checks pass. All operator documentation JSON examples
parse. The combined test log is `semantic-facade-final.log` in task evidence.

The focused harness uses the actual Runtime, native resource shell, presentation
and complete real adapter dependencies. It does not compile FastMCP macros,
complete handler composition or stdio. The full unchanged locked MCP build still
stops in upstream asupersync with SIGKILL before reaching dfmcp-mcp, as recorded
below. No full-server, full-workspace, DFHack native or live-game qualification is
claimed. Native stop bounds are callback triggers and can overshoot; other
controllers are not globally fenced. Inventory quotas, material/workshop
feasibility and generic multi-goal execution remain outside this native slice.
Existing wire/storage formats, the empty registry and production map are unchanged.
See `docs/SEMANTIC_WORK_ORDER_MCP.md` for the complete configuration, two review
boundaries, bounded runs and exact original/clock recovery workflow.

### Original-order bounded simulation execution core (2026-10-27)

The original native work-order runtime now composes the existing order-run/1.14
coordinator with its exact original plan, archive, creation journal and verified
Created receipt. Separately sealed finite runs retain the original native order,
recipe, amount and absolute deadline. One unsettled run and at most 32 retained
runs are permitted; a durable dispatch marker prevents another unpause after a
lost reply or restart. Native stop receipts remain historical clock evidence,
separate from fresh original-goal samples and physical quiescence.

Incomplete orders stop on observed zero remaining. A completed, validated,
inactive order can receive another explicit finite run for temporal stability
or independent job quiescence, stopping early if it unexpectedly reactivates.
Neither trigger substitutes for original-goal proof. Reconciliation precedes
fresh paused goal acquisition; no run automatically starts another. Original
archive and insertion custody are rechecked after the clock dispatch marker,
immediately before the native effect.

Clock configuration, credential, independent generation and authority are
optional and separate from production authority. Exact external journal identity
and fixed private custody are required on reopen. Omitting configuration cannot
hide an existing clock journal. Offline opens require no native credential or
contact. Immutable creation/clock modes cannot regain authority from later
environment changes. Existing native wire and journal formats are unchanged.

At this core checkpoint, the execution API, concrete resource shell and Agent
Turn presentation were implemented; public MCP run dispatch was pending. The
newer entry above records its completed integration. Executed validation: the complete real adapter library check,
28 order-run unit tests, 58 public semantic work-order integration tests and 59
actual-source runtime/presentation tests pass (145 distinct tests). Six joined
runtime cases exercise insertion, confirmed runs, pending reconciliation,
subsequent stable samples, lost replies, recovery, independent authority and
post-marker custody/gate failure. Concrete NativeRuntime ownership is Send.
The combined target uses the actual Runtime, NativeShell, clock, Agent Turn and
presentation with the full real adapter dependency graph and unchanged admission
accessor closure; it does not compile FastMCP macros or the complete facade.

The unchanged locked full MCP build was attempted again and stopped in upstream
asupersync 0.5.0 with SIGKILL before compiling dfmcp-mcp; the shared cgroup OOM
counter increased during that attempt. No dependency pin or feature was changed.
Full MCP/executable, native DFHack and live-game qualification remain unverified.
The empty compatibility registry and production runner map remain unchanged.

### Native original-goal MCP execution (2026-10-27)

The new `dfmcp-live-semantic-work-orders-dev-server` connects the existing
canonical citizen source, semantic work-order handoff, native creation journal,
original-plan archive and durable goal monitor through the frozen eleven-tool
MCP interface. One closed request compiles a finite wooden bed/door/table/chair
order, prepares its paired original/native review, permits one explicit confirmed
insertion and performs bounded foreground reconciliation and original-goal
sampling. The original absolute deadline, cadence, stable samples and action key
survive exact restart; first achievement, current truth, insertion certainty and
physical quiescence remain distinct.

The operator supplies immutable exact source pins, separate credentials and a
private custody directory outside MCP. Independent citizen, creation, progress
and operations generations are accepted only within their own namespaces.
The fixed development evidence policy exposes only the required canonical
identity/pause, linked order counters/status and complete global job-count
sources. This explicit development acceptance is not compatibility admission.
Offline recovery performs no bridge contact; reconcile mode has Query/Observe
only. Exact creation-versus-reopen storage modes cannot replace missing recovery
custody with a fresh journal.

Complete Agent Turn packets expose the retained goal and recovery seals, require
current operator authority before suggesting commit, preserve uncertain effects
and mark cached state unverified after failure. A complete newly returned view
is retained before response-publication cancellation checks, preserving recovery
identities even when the outer call fails. Every response is reserved before
work and bounded to 128 KiB without JSON truncation. Session close releases
resources after cancellation or fencing without deleting evidence or cancelling
game effects.

The final combined focused target passes 28 tests: ten real-runtime groups,
nine new presentation regressions, six shared Agent Turn cases and three facade
boundary tests. Runtime tests execute the actual compiler, semantic coordinator,
wire clients, durable monitor and private-file modes with injected native
sources. Presentation tests use the actual Agent Turn implementation and
unchanged production-provenance accessor closure. The three facade tests execute
verbatim extracted production budget, session-identity and error helpers; they
do not compile the MCP macros or handler composition. The real `NativeRuntime`
also satisfies the compile-time `Send` assertion required by owned session
storage. The concrete TCP/private-file resource shell is compiled by the runtime
harness. Full MCP compilation and
transport remain unverified: the real locked/offline full-library and dedicated
binary check fetched the exact FastMCP pin successfully, then upstream
`asupersync` compilation was terminated by SIGKILL before reaching `dfmcp-mcp`.
This produced no Rust source diagnostic for the new server and supplies no
server-build evidence. Scoped source formatting and diff checks passed. These
component results do not establish full-workspace, native or live qualification.

The implemented goal is completion of the exact retained native order, with
qualified global job quiescence; item quantities, inventory quotas, material
and workshop feasibility, generic multi-goal execution and automatic clock
control remain outside this slice. Operator source acceptance, the empty
compatibility registry and the production protocol runner map remain separate.
See `docs/SEMANTIC_WORK_ORDER_MCP.md` for the complete operator/client workflow.

### Independent native profile incarnations (2026-10-26)

Semantic labor, finite work-order and exact-item furniture routing now accepts
independently generated canonical-read and mutation-plugin incarnations. The
plugins initialize their clocks separately; comparing those numbers previously
rejected otherwise matching native captures. Resolution still requires the same
fortress, exact paused tick and qualified selected identities or item facts.
The original canonical manifest/generation/digest/anchor and the native
generation/witness/plan/journal remain independently bound.

The concrete citizen evidence owner separately pins its first matching workforce
incarnation. Failed reads, invalidation and canonical-only refresh retain this
pin. Replacing either profile after review or preparation cannot authorize the
original effect, including when the replacement number aliases the other
profile's original number. Existing exact reattachment, indeterminate-effect
reconciliation and Query-only recovery retain the original semantic/native
association without another insertion or assignment.

Executed focused validation: 20 routing/resolution tests, all 58 public
semantic work-order lifecycle tests and all 46 public semantic workforce
lifecycle tests passed (124 total, zero failures). Default lifecycle fixtures
now use deliberately distinct profile generations. New cases cover independent
labor/order/furniture resolution, original-source reload, native reload after
preparation, retained pins after failed acquisition, lost replies and read-only
reopen without redispatch. The harness compiles the actual 27-root adapter
dependency closure and original public integration targets; unrelated bounded
run tests were filtered from the routing target. No wire, journal or plan format
changed. These synthetic-native and real-storage component results do not
establish full adapter/workspace, MCP transport, DFHack or live qualification.
See `docs/LIVE_SEMANTIC_ROUTING.md` and
`docs/SEMANTIC_WORKFORCE_HANDOFF.md`.

### Citizen welfare inspection and stress attention (2026-10-26)

The existing spatial/1.8 `fortress.query` now accepts `mode: "welfare"` and the
structured `citizen_welfare` query over the complete strict-citizen roster from
one coherent capture. It reports observed stress categories, source-qualified
current job assignments and availability under preserve-social and
interrupt-social policies. Filtering and whole-row pagination rank lower stress-category
numbers first, with unestablished stress disclosed separately. Each row carries
an exact entity-generation inspection request.

The same query works against selected historical records and offline archives.
Its inspection drilldowns retain the exact record number and digest, so later
live observations cannot redirect them to a different citizen generation or
source. Historical responses retain their historical scope and do not inherit
current situation attention. Reads use existing captured data and perform no
additional native capture or game mutation.

The separately versioned `dfmcp.spatial-situation/2` policy adds attention for
living strict citizens observed in DFHack stress categories 0 through 2.
The finding is an inspection priority, not a diagnosis or evidence of danger.
Welfare coverage explicitly leaves needs, physical health, hunger, thirst and
injuries unknown. Category 3 may include DFHack's no-soul fallback. The strict
roster does not cover every resident, visitor, dead or insane unit; an empty
filtered page cannot establish that the population is well.

Current Query authority, source health/fencing and observation-journal custody
remain enforced. Request shape, complete-roster/entity scans, work, wall time,
response bytes and whole-row pages are bounded. Continuations bind the session,
source anchor/digest, normalized filters and work allowance. Inspecting welfare
confers no labor permission, reservations or effect authority.

Executed focused validation: all 189 actual-source MCP semantic-module tests
passed, with zero failures or ignored tests. This includes all nine new welfare
tests, all twelve pure situation tests and four workforce schema/pagination
tests. The welfare cases execute source qualification, uncertainty disclosure,
complete-roster filtering, authority and work/output bounds, continuation
identity, coherent job-worker links and exact historical inspection pinning.
Independent source/integration review, request-schema checks and scoped
formatting/diff checks also passed.

The harness uses the actual GameAdapter API, eight original native-read source
modules and the complete laboratory MemoryAdapter. It excludes unrelated adapter
transport/bootstrap/mutation modules and the full MCP server, facade and stdio
runtime. Corresponding runtime-test children, including the new current,
historical and offline facade integration cases, remain unexecuted. DFHack and
live-game execution were not performed. These focused results do not establish
full-workspace, native or live qualification; the compatibility registry and
production runner map remain unchanged. See `docs/CITIZEN_WELFARE.md`.

### Explicit future food and drink reserves (2026-10-26)

The existing laboratory production request accepts an absolute
`reserve_until_tick` through the separate `consumption_reserve_v1` compiler.
It budgets consumption through the later of that tick and the actual work
horizon, including currently satisfied stock minima, shared finite PLANT,
queued work, workforce contention and explicitly authorized workshop setup.
Action obligations keep their work-based deadlines. Original current-stock
predicates remain unchanged; predicted reserves are not continuous maintenance
proof or future observed stock.

When the exact qualified original minima are already True, a typed
laboratory-only reserve constraint admits the generated production work. It
requires a unique bounded future horizon, established stock minima on actual
stock-ledger records, registered recipes and explicitly allowed setup. Generic
preparation and unrelated goal predicates cannot use this exception. The typed
horizon is independently sealed into the bounded prepared-plan summary without
changing the plan archive format. Previously valid False-goal reserve plans
retain their exact summary and seal.

Canonical production sources seal the exact requested tick and compiler.
The distinct `dfmcp.production-continuation/3` preserves both across flat
lineage and exact reopening; expired horizons cannot silently renew. Historical
sources and horizon-free requests retain their prior compiler and bytes. A
True current-stock goal still requires a fresh explicit reserve request after
interrupted reserve work, rather than bypassing the existing continuation gate.

The production forecast now separately samples the exact requested future
frontier on a discarded reference-world fork. A tick before work completion
cannot borrow later output, and a later reserve simulation does not change
action deadlines or the work-completion report. Paused or unavailable future
state remains unavailable. Public planning and reviewed stale-plan replay use
the retained source to select this forecast.

Final focused validation passed all 203 actual `dfmcp-intent` library tests and
all 189 MCP semantic-module tests, with zero failures or ignored tests. Six new
intent groups cover typed admission, qualified stock-goal restrictions,
registered action families, exact seals/archive roundtrip and unchanged
source/authority/risk/budget boundaries. Twenty reserve-specific module cases
cover nine compiler/execution groups, three source-seal/replay groups, four
continuation groups and four forecast groups. They execute real prepare,
commit, advance and poll behavior, including initially satisfied minima, exact
shared ingredient budgets, explicit workshop setup and exact future frontiers.
This is the same 189-test harness described in the welfare entry above, not an
additional disjoint test set. Scoped pinned-nightly formatting, source review
and diff checks passed.

The full MCP server/facade/stdio runtime and its test children, including the
new public-facade regressions, remain unexecuted. The focused harness imports
the actual intent, laboratory and named MCP semantic modules with the exact
GameAdapter API and native-read subset; it does not execute unrelated adapter
transport/bootstrap/mutation modules, DFHack or a live game. The required
repository checks retain their pre-existing source-bundle and static-contract
failures. No full-workspace, native or live qualification is claimed. See
`docs/PRODUCTION_RESERVE_PLANNING.md` and
`docs/PRODUCTION_GOAL_CONTINUATION.md`.

### Finite shared ingredients for laboratory food and reserve plans (2026-10-24)

The separately declared `plant-provisions/1` model composes with shared labor
and charges one qualified PLANT for each completed brewing or cooking batch.
Both families share the same original stock ledger. Input gates precede worker
selection and are rechecked during simultaneous completion; an exhausted input
cannot produce another batch. Partial work and cancellation neither consume
an unfinished batch nor refund already produced supplies. Original orders
retain consumed-input history through canonical recovery. Legacy worlds and
active-history upgrade/downgrade boundaries remain explicit.

Action planning budgets all new food/drink batches against existing active
finite-food demand, binds the original ledger and input quantity, and seals
mandatory consumed-input proof. Caller-supplied predicates cannot turn an
unfinished order into completion. Dispatcher and laboratory proof boundaries
validate current original identity, generations and the complete input domain.

The opt-in `starter_fortress_provisions` scenario composes this model with
shared workers and finite workshop WOOD, adding twenty observed PLANT inputs.
The quota compiler budgets all new FOOD/DRINK batches jointly after debiting
active original food orders, and recomputes ingredient demand through the
consumption-reserve fixed point and explicit workshop setup. Raw PLANT has no
direct metabolism allowance; its recipe demand includes every reserve batch.
Earlier scenarios and historical source interpretation remain unchanged.

Observe-gated opening, observation and per-action reports distinguish current
available inputs, consumed history and remaining demand. Forecast accounting
is explicitly predicted on a discarded fork. Ingredient shortages remain
separate from worker capacity, and low-stock recommendations requiring new
production are suppressed when necessary input is short or unqualified.
Retained need flags cannot invent a plant deficit for an already-met target.
Original goal truth, action completion and physical quiescence remain distinct:
stock may meet the requested goal before all reserve batches finish. Canonical
reopening preserves cancelled consumption history, and an explicit continuation
replans from current stock without refunding or repeating completed output.

Executed development validation: all 197 actual-source intent tests passed,
including 16 new engine groups and four planner tests; all 17 dispatcher tests
passed, including two new current-provisions proof groups; all 83 laboratory
tests passed; and all 191 focused MCP semantic-module tests passed, including
15 new provisions cases. This is 488 passing tests, with 37 new cases across
the two increments. The engine regressions cover shared-input contention,
retained partial work, exact original cancellation identity, source/custody
refusals, partition/recovery invariance, failure atomicity and exact legacy
bytes. Integration cases cover exact shared budgets, pending demand, reserve
and setup costs, early goal truth, input exhaustion/replenishment, canonical
reopening, continuation, current/predicted reports and ingredient-aware alerts.
Production Clippy passed for the changed core libraries and actual-source MCP
semantic modules with the three previously recorded world-crate baseline
allowances and all other warnings and prohibited methods denied. These
focused Rust checks used nightly-2026-08-31.

Required repository verification still stops at the pre-existing source-bundle
documentation check; development local qualification still stops at the same
64 pre-existing static-contract failures. Focused actual-source checks are not
full-workspace, native or live qualification. The focused MCP harness imports
the actual semantic modules but excludes the full FastMCP server, facade and
task-session shell; their changed plumbing was source-reviewed and formatted,
not compiled in this evidence set. This is a bounded reference model;
gathering, agriculture, barrels, native recipe calibration and live admission
remain separate. See `docs/LAB_FINITE_PROVISIONS.md`.

### Material-backed laboratory workshop construction (2026-10-23)

The separately selected `wooden-workshops/1` model composes with the existing
shared workforce and requires one qualified WOOD log for each supported Still,
Kitchen or Carpenters Build. Dispatch removes that log from free inventory and
holds it for the original building. Completion consumes the same allocation;
unfinished cancellation releases it once. Original model, ledger generation,
quantity and allocation state remain in canonical history through recovery.
Unsupported Build kinds and material selectors refuse in this opt-in model.
Existing marker-free worlds retain their original effects and plan semantics.

Joint action planning includes new workshop setup, new furniture demand and
remaining existing carpentry demand before producing a candidate. Per-action
availability guards and mandatory material-completion predicates are sealed
into supported Build steps. Explicit deadlines remain unchanged. Physical-work
and proof boundaries independently validate current material identity/custody.

The additive `starter_fortress_economy` scenario composes finite materials and
shared labor through the existing room and production workflows. The compiler
budgets actual deduplicated setup before furniture feasibility, including
queued carpentry when food/drink requests need new workshops. Current material
and per-action allocation reports are Observe-gated and source-anchored;
forecasts expose a separate predicted report. A cancelled workshop site is
reusable only with validated released custody. Recovery retains that receipt,
and a newly reviewed room continuation allocates its refunded input again.

Executed development validation: 177 actual-source intent unit tests passed,
including 16 new material-lifecycle groups and four resource-planning tests;
15 dispatcher tests passed, including two new current-custody proof groups;
83 laboratory unit tests passed; and 176 focused MCP module tests passed,
including nine new economy integration cases (451 total passing tests).
The two-bedroom integration refuses two logs, completes with exactly three,
matches forecast and execution, and recovers an interrupted workshop at its
original site without duplicating its refund. The dispatcher regressions refuse
replaced building/ledger generations, duplicate ledgers and removed model
declarations even when ordinary completion field predicates remain true.
Focused production Clippy passed for the changed intent, adapter, laboratory
and MCP semantic modules with the three previously recorded world-crate
baseline allowances and all other warnings/prohibited methods denied.

The required repository verification still stops at the pre-existing source
bundle documentation check; development local qualification still stops at
the same 64 pre-existing static-contract failures. The focused MCP harness
does not compile the full FastMCP server, facade or task-session layer; the
opening, observation and briefing wiring was independently source-reviewed
and formatted. These actual-source tests are not full workspace/server,
native or live qualification. The one-log
recipe is laboratory calibration; individual log hauling, native material
costs and unsupported building families remain outside this model. See
`docs/LAB_WORKSHOP_MATERIALS.md`.

### Usable shared-workforce room and production workflows (2026-10-24)

The opt-in `starter_fortress_workforce` scenario exposes the shared service
model through the existing eleven-tool laboratory surface. Existing connected
and provisioned room requests now exercise finite excavation, production and
construction together. Current Observe authority gates the bounded opening,
observation and per-action assignment/blocker reports. The orientation briefing
uses only a current returned report; predicted workforce assignments remain
explicitly forecast evidence from the isolated simulation.

Production reserve solving and sealed obligations use the same complete
workforce horizon, including queued construction. This prevents food/drink
reserves from being sized against an earlier, unrealistically short service
schedule. Temporal plans that read the common workload always require reviewed
source replay after an anchor change; a narrow per-action witness cannot
silently reuse old workload-derived deadlines. Original scenarios and their
optional-field shapes remain unchanged when the model is absent.

**Executed evidence:** all 167 tests in the actual-source focused MCP module
workspace pass. Seven new cases cover additive scenario identity and legacy
canonical bytes, unavailable source evidence, blockers beyond the briefing row
cap, connected-room forecast/execution agreement, authorized construction-labor
removal and restoration without banked service, canonical reconstruction with
carried proof, and a single worker sharing queued construction with consumption-
aware production. The latter verifies the exact reserve/sealed horizon and the
original food/drink quotas through that horizon. Existing room, continuation,
production, witness and forecast tests in the same module workspace also pass.
Focused production Clippy passes with the same three pre-existing world-source
allowances recorded below; scoped pinned-nightly formatting and diff checks
pass. The earlier 157 intent tests remain the core engine/planner evidence.

The focused harness compiles the actual core, world, intent, adapter and
laboratory libraries and actual MCP semantic modules. It excludes the full
FastMCP server/facade/task-session surface. The two new public-facade tests for
Observe redaction and reviewed stale-plan replay are source-reviewed and
formatted, not compiled or executed. The prior complete MCP dependency check
was killed while compiling `asupersync` under the environment memory limit;
this increment does not claim to resolve that full-build limitation. No new
full Rust, native or live qualification or registry admission is asserted.

Required `verify.sh` passes the first four checks, then stops at the existing
source-bundle documentation gate (`exact clean git commit`). The explicitly
marked development qualification run passes repository integrity, receipt
custody and implementation status, then stops at the existing static-contract
failures. It produces no passed full qualification receipt. These baseline
release-gate failures were not expanded into unrelated cleanup work.

Usage, current report fields and scope limits are documented in
`docs/LAB_SHARED_WORKFORCE.md`. This implementation adds finite reference labor;
native hauling, pathfinding, ownership, skill calibration and general building
material consumption remain separate work.

### Finite shared laboratory service for excavation and construction (2026-10-23)

An explicit `shared-workforce/1` model now shares qualified living workers
between mining, construction, brewing, cooking and carpentry. Exact furniture
installation uses the construction family. Deterministic maximum-cardinality
matching preserves earned work priority, requires completed production
workshops, and assigns one activity to each worker per causal interval.
Unstaffed work earns no service. Workshop completion, cancellation, worker
death and changed labor facts take effect at the next selection boundary.

The canonical settings marker and per-task stamps preserve this model through
snapshots and refuse implicit migration or downgrade of active/history-bearing
work. Marker-free worlds retain the previous model. A pure report exposes
current assignments, blockers and remaining service using the same selector.
Bounded planning accounts for queued and peer service, retains explicit
deadlines and seals a model guard into temporal steps. These global queue reads
require reviewed source replay after an anchor change.

**Executed evidence:** all 157 actual-source intent unit tests pass, including
15 new workforce groups, five planning groups and the existing timeline,
carpentry, furniture, proof and planning regressions. The independent workforce
cases include all 512 three-worker eligibility graphs against a brute-force
oracle, shared service across five families, real-item installation/cancellation,
workshop completion, source refusals, worker death, overlap settlement,
partition/reconstruction invariance and atomic budget exhaustion. Planning
cases cover queued service, dependent work, explicit deadlines, authority,
legacy obligations and hard bounds. Intent production Clippy passes with
warnings denied and the three previously recorded world-source lint allowances
(`map_or_identity`, `manual_unwrap_or`, `unnecessary_option_map_or_else`); panic,
unwrap, expect, todo and unimplemented remain denied. The pinned toolchain is
`nightly-2026-08-31`; the isolated workspace compiles actual repository sources
without the FastMCP/server dependency graph. Scoped formatting and diff checks
pass. No full-workspace, native or live qualification is asserted.
Required `verify.sh` passes its first four checks and then stops at the existing
source-bundle documentation gate (`exact clean git commit`). The development
qualification attempt refused a concurrent source change during its initial
snapshot; it produced no passed receipt.
The model remains a laboratory
calibration without movement, hauling, skill-dependent rates or room ownership;
general building material semantics remain unchanged. See
`docs/LAB_SHARED_WORKFORCE.md`.

### Plan, furnish and resume entrance-connected laboratory rooms (2026-10-23)

The existing `fortress.plan` blueprint argument accepts a separately versioned
`connected_room_v1` wrapper around the complete `provisioned_room_v1` request.
It retains an explicit observed FLOOR entrance and a same-level routing region
of at most 1,024 tiles. The compiler routes safe connector excavation around
current qualified building footprints, actual planned workshop sites,
furnishing targets and protected bedroom wall rings. For the template's fixed
access endpoint, it minimizes new wall excavation, then path length, then the
complete `(z, y, x)` path. Only selected missing wall cells become disjoint dig
regions; no enclosing rectangle may over-excavate. Planned interior circulation
must provide a cardinal approach to every furnishing.

Production retains its original action identities. New placements wait for all
required circulation and connector excavation as well as their exact items.
Work-scaled excavation deadlines support large accepted rooms. The complete
terminal goal requires every original terrain region, every exact furnishing,
connector terrain and current `LaboratoryRoomAccess`; a later obstruction can
make current truth False without rewriting historical achievement. Existing
ordinary room source bytes, seals and behavior remain unchanged.

`continue_goal` preserves the original entrance, routing domain, layout and
setup choices in a private `dfmcp.connected-room-continuation/1` source. Its
fixed `connected_room_residual_v1` compiler retains qualified installed items,
procures only missing furnishings and can repair access with excavation alone.
Distinct durable source tags retain the full original request and flat
parent/root lineage through reopen and compaction. Both normal and residual
connected plans require reviewed source replay after every changed anchor,
including excavation-only repairs. Proposal and commit recheck current
authority, original False evidence and same-root work; another currently
satisfied pursuit suppresses duplicate continuation. Historical exact goals,
receipts and first achievement are not reassigned to replacement work.

**Executed evidence:** all 38 focused runtime tests pass on the actual source:
seven connected compiler/execution cases, five connected continuation source
cases, two durable connected-source cases, two occupancy witness cases, two
access-predicate archive integration cases, sixteen existing provisioned-room
cases and four existing room-continuation cases.
They include forecast-versus-execution agreement, a 105-tile hall, retained-item
access-only repair, planned-workshop detours, aliased room/workshop occupancy,
2,187 independently checked routing graphs and 512 exact excavation masks.
The final occupancy correction was rebuilt and all twelve connected cases
passed again. Warning-denied production-module Clippy passes on that final
source. Scoped pinned-nightly formatting and diff checks also pass.

The temporary validation workspace points directly at the actual
core/world/intent/adapter/laboratory library sources and real MCP compiler,
continuation, forecast, progress and witness modules. All five component
libraries compiled successfully. This harness excludes the FastMCP/server
facade. The earlier intent/archive type checks and world/intent/laboratory
production Clippy evidence remain as recorded below; all 119 world unit tests
also pass on this increment's unchanged world source.

The full locked MCP test-target check resolved the original pinned dependencies
without modifying manifests or the lockfile, but `asupersync` compilation was
killed with SIGKILL in the memory-limited environment before any `dfmcp` crate
was reached. The nine new public MCP lifecycle cases and server/objective
wiring remain source-reviewed and formatted, not compiled or executed by that
attempt. Focused component evidence does not establish full MCP,
full-workspace, native or live qualification.

Required `verify.sh` again passes repository integrity, qualification-receipt,
implementation-status and release-custody checks before the existing source
bundle documentation failure (`exact clean git commit`). Ordinary local
qualification refused the in-progress dirty tree; the explicitly marked
development run passes its first three gates and then stops at 64 existing
static-contract failures. Standalone bead coverage cannot run because `br` is
not installed. No new passed full qualification receipt is produced.

See `docs/LAB_CONNECTED_ROOM_ACCESS.md` for the request and limits. This is
bounded structural access in the laboratory reference model. Bedroom wall
protection constrains routing; it is not a terminal claim about all surrounding
walls. Native pathfinding, hauling, ownership and construction workforce remain
separate work, and no live admission or mutation capability is added.

### Current laboratory room-access proof (2026-10-23)

A new bounded `LaboratoryRoomAccess` predicate retains an explicit entrance,
a same-level region of at most 1,024 tiles and up to sixteen furnishing targets.
It proves a current cardinal FLOOR approach to every target while treating
building footprints and all furnishing tiles as blocked. Complete qualified
occupancy, conservative missing-terrain truth, combined work bounds and
laboratory-only evidence authority prevent a stored floor route from being
mistaken for current accessible space. Canonical/archive tags are appended;
old predicate bytes remain unchanged. MCP projections expose the policy and
read witnesses include the complete negative building domain.

**Evidence:** all 119 world unit tests pass on the actual current source,
including ten new access/authority/budget groups with exhaustive small-map and
rectangle-union comparisons. The world library and unit-test target pass
pinned-nightly type checking and warning-denied production Clippy. The intent
library, unit-test target and archive integration-test target also type-check;
intent production Clippy passes. Two new archive cases and two MCP witness
cases cover appended serialization and phantom building insertion. These
checks do not establish latest-nightly full-workspace or native/live qualification.

The contract is documented in `docs/LAB_CONNECTED_ROOM_ACCESS.md`. This is a
reference-model proof; native pathfinding, ownership and live admission are
unchanged.

### Continue interrupted furnished-room projects (2026-10-23)

The existing `continue_goal` blueprint now accepts retained item-backed room
goals. Its separately sealed `provisioned_room_residual_v1` compiler retains
complete installations only at their original requested kinds and tiles with
qualified exact generation/provenance/custody evidence. Only missing slots use
loose inventory or manufacturing, so completed furnishings require no additional
item, stock debit or WOOD. Cancelled placements can supply their released items.
Residual excavation splits around retained installation tiles and remaps every
placement dependency to all unfinished pieces of its original room. The final
goal still requires every original terrain region and furnishing. Existing
sixteen-slot, sixty-four-action and source/evidence bounds remain enforced.

The private `dfmcp.room-continuation/1` envelope preserves the complete original
`provisioned_room_v1` request, including unused setup permissions/sites, plus
flat parent/root lineage and a fixed residual compiler identity. A distinct
appended durable source tag supports pure reconstruction at the archived
snapshot and intent ID. Original normal-room source bytes and seals remain
unchanged. New placement keys prove only the new pursuit; old action receipts,
exact original terminal conditions and first achievement are never rewritten.
A currently satisfied same-root pursuit prevents duplicate continuation of an
old False root and suppresses its continuation affordance.

Proposal and commit require current Observe/Plan authority, verified active
observation continuity, a False parent goal and physical/deferred quiescence
across the complete retained lineage. Every changed anchor requires reviewed
source replay, including when all furnishings are retained and only excavation
remains. Durable reopen uses the existing verified recovery frontier; restore
abandonment remains terminal. No new MCP tool or native mutation is introduced.
See `docs/ROOM_GOAL_CONTINUATION.md` for the complete request and proof contract.

**Evidence:** eighteen new regressions cover six residual compiler/execution
scenarios, four private source/seal cases, two durable source/compaction cases
and six full MCP lifecycle/recovery scenarios. The current adapter production
library, laboratory production library and unit-test target, and actual-source
MCP module test target pass pinned-nightly type checking. This includes the
earlier large-room deadline regression that could not be recompiled in its
original increment. Warning-denied production Clippy passes for the laboratory
and actual-source modules; the intent library check is recorded below.

The actual-source harness imports the real room, production, continuation,
forecast, progress and laboratory-world modules. It does not compile the
FastMCP/server facade: the six new public lifecycle tests and server/objective
wiring are source-reviewed and formatted, but not compiled or executed here.
All 693 inventoried source inputs remained unchanged during the final module
checks. No new runtime execution pass is claimed; the attempted fresh engine
test build was killed before a runnable binary was produced. Full-workspace,
native and live qualification remain unestablished.

Required `verify.sh` passes repository integrity, qualification-receipt,
implementation-status and release-custody checks before the existing source
bundle documentation failure (`exact clean git commit`). Required local
qualification aborts with `No space left on device` before its full gates,
including after one retry following removal of this task's inactive failed
compiler fragments. No full qualification receipt is produced.

### Exact proof for retained installed furniture (2026-10-23)

Completed-installation inspection exposes the exact building, item, kind and
tile only after validating original generations, production provenance and
exclusive reciprocal custody. Reusable completion predicates preserve the
original nineteen-predicate templates and canonical fingerprints for all four
furniture kinds.

Current, forecast and archived goal proof additionally validate mandatory
completed-furniture claims even when a plan has no new placement action for
that furnishing. Only root/nested `All` claims qualify; `Any` and `Not` branches
do not become mandatory. Predicate shape validation and a sixty-four-binding
limit precede one shared bounded attachment census. Existing per-action
completion checks remain in place.

**Evidence:** the actual intent production library and unit-test target pass
pinned-nightly type checking, including five new regression groups.
Warning-denied production Clippy passes. All 123 inventoried core/world/intent
Rust inputs stayed unchanged through the metadata checks. Scoped formatting
and diff checks pass. The fresh intent test build was terminated with SIGKILL
before producing a runnable binary; no new runtime execution is claimed. The
MCP call-site change also passes the subsequent actual-source module checks
recorded in the room continuation section above. No full-workspace or
native/live qualification is established.

### Complete provisioned-room goals (2026-10-23)

The existing `fortress.plan` blueprint input accepts a versioned
`provisioned_room` wrapper around bedroom-cluster or dining-hall geometry.
The compiler selects eligible exact loose items in canonical order, plans only
missing carpentry output, preserves all original excavation regions and emits
exact item-backed placements after production and excavation verification.
Excavation deadlines use the reference model's work-scaled defaults; dependent
placements retain their own completion horizon after prerequisite deadlines.
Future item IDs derive from the original production step key and completion
ordinal. Legacy aggregate stock grants no item identity credit; procurement
uses the current aggregate plus the missing eligible count.

The final original goal contains every excavation region and every installed
furnishing. Procurement stock minima are intermediate conditions because
placement spends that inventory. Already-excavated rooms and released cancelled
placements can be reused, and wholly stocked rooms need no WOOD input evidence.
Optional production setup retains all original permissions and supplied sites,
including unused choices. Sixteen furniture slots, sixty-four combined actions,
bounded source/geometry domains and exact occupancy checks bound the compiler.
Existing occupied rooms, unknown supply, material shortage, overlapping workshop
setup and unsafe excavation refuse without mutation.

New intake canonicalizes `provisioned_room_v1` before source custody. Durable
Blueprint replay requires that exact marker and retains the complete original
layout/setup request. Changed anchors require reviewed source replay under the
new placement rule. Historical direct room blueprints keep their abstract Build
semantics. The feature establishes laboratory excavation and installation;
room ownership, fortress connectivity and native/live mutation remain separate.
See `docs/LAB_ITEM_FURNITURE.md` for complete request examples.

**Evidence:** before the final excavation-deadline correction, the actual-source
room compiler and nine regression groups passed type checking, and the
room-inclusive production module passed Clippy with warnings denied. Cases
cover full-room execution logic, partial existing supply, already-excavated
geometry, legacy stock, cancellation/reuse, exact source replay,
shortage/unknown/occupancy refusals, closed bounds and generation replacement.
The full-room regression also compares discarded-fork forecasting with real
execution and requires the original source world to remain unchanged.

A tenth regression covers an accepted 105-tile hall whose 1,050 ticks of dig
work exceed the inherited 1,000-tick deadline. The new compiler now delegates
excavation obligations to the existing work-scaled default, giving this case
2,200 ticks and propagating the deadline into dependent placements. This narrow
final correction and tenth test are source-reviewed and formatted but were not
recompiled; exhausted storage prevented recreating required adapter metadata.
None of the new room regressions were executed: shared storage filled while
linking the fresh adapter library. The underlying engine has 156 passing
executed tests as recorded below. Three additional server-boundary regressions
cover durable PlanSource round trips, compiler-marker/tag refusal and reviewed
stale replay; they are source-reviewed and formatted but neither compiled nor
executed against the full server. Required repository verification stops at
the existing source-bundle documentation gate (`exact clean git commit`); local
qualification passes its first three gates and then reports 64 existing static
failures, including nine unrelated room-schema examples. This is not MCP-host,
full-workspace, native or live qualification.

### Exact item-backed laboratory furniture (2026-10-23)

A distinct `PlaceFurniture` action binds a closed Bed/Door/Table/Chair kind,
canonical produced item and one target tile. Its scope includes both item and
map area under Guarded Construct authority. Canonical and archive encodings
append a new tag; old Build actions, material selectors and seals retain their
original abstract behavior.

New carpentry items carry explicit loose inventory custody. Placement validates
the original producer and completion ordinal, item generation, WOOD singleton
data, complete reciprocal attachments, eligible stock and unoccupied observed
floor before atomically allocating the exact item and debiting one stock unit.
The existing 500-tick construction duration installs that same item. Cancelling
unfinished placement returns the same item, frees the target and refunds once;
completed placement is not undone. Legacy items without custody remain
ineligible and unchanged. Bounded global scans and transaction shadows fence
contradictions, unknown data, collisions, overflow and work-budget exhaustion.

Physical-work inspection, action verification, current goal truth and archived
first achievement additionally validate exact generation and reciprocal custody,
which ordinary field predicates cannot express. Durable restart retains the
allocation without recreating an item or importing old dispatch authority.
The MCP grammar, step review, leases, physical ownership and forecasts include
placement; stale anchors require reviewed replay. Native semantic routing
explicitly refuses these laboratory item identities. See
`docs/LAB_ITEM_FURNITURE.md` for the executable contract and request form.

**Evidence:** 156 actual-source engine tests pass on the pinned nightly: 132
intent unit tests, ten action-normalization tests and fourteen plan-archive
tests. Eight new engine groups cover all four kinds, conservation, exclusivity,
cancellation/reuse, source and generation refusal, occupancy, budget/overflow
atomicity, wait partition and snapshot recovery. Current adapter production,
laboratory production and unit tests, and actual-source MCP module tests all
pass type checking. Warning-denied production Clippy passes for intent,
laboratory and MCP modules. Scoped formatting and diff checks pass.

Laboratory/MCP runtime tests remain unexecuted: shared persistent and temporary
volumes filled during adapter archive creation, after successful code generation.
The broad adapter test target also exceeded shared memory. Type checking is not
execution evidence. No full-workspace, MCP host, native or live qualification
is claimed.

### Complete furniture production quotas and shared setup (2026-10-23)

The existing production blueprint accepts BED, DOOR, TABLE and CHAIR alongside
DRINK and FOOD. Compilation sizes finite orders from explicit qualified stock,
budgets a shared WOOD supply across all requested outputs and current marked
carpentry orders, and preserves every original stock minimum in the sealed
terminal predicate. Furniture has zero metabolic consumption; mixed food/drink
goals retain reserves through the complete shared production horizon.

Joint staffing uses bounded exact selection across the three actual
workshop/labor capacity families. All furniture outputs reuse one selected
carpenter and workshop, avoiding labor enables that could not increase
throughput. Explicit prerequisites can establish one carpenter workshop and
enable an observed disabled labor under ordinary action authority. Shared
setup actions are deduplicated, and shared workers or workshops serialize
orders through ordinary verified-completion dependencies. The compiler remains
conservative: it selects one canonical workshop per family rather than
distributing work over every existing workshop.

Starter scenarios now establish carpenter capacity, twenty logs and explicit
zero furniture stocks. Original request round trips retain exact quotas and
seals; historical food/drink sources preserve their interpretation of unmarked
abstract furniture work. Discarded-fork forecasts and actual execution must
prove the original stock goals as well as verified actions and physical
quiescence. See `docs/CARPENTRY_PRODUCTION.md` and
`docs/PRODUCTION_RESERVE_PLANNING.md` for complete usage and boundaries.

**Evidence:** all 125 actual-source MCP component tests pass, including 12 new
furniture planner/execution groups, seven joint-staffing groups, exact source
replay, mixed-goal forecasts and existing consumption, workload, continuation,
projection and progression regressions. Production-module Clippy passes with
warnings denied and the repository deny policy. The harness references the
actual repository modules and exact compiled dependencies; no replacement
implementations are used. When the shared disk filled during code generation,
only test/compiler output moved to a task-specific memory filesystem.

This is component evidence. The MCP facade/FastMCP runtime and native/live
DFHack path are not qualified by that harness. `verify.sh` stops at the existing
source-bundle documentation contract. A development `qualify_local.sh` attempt
passes its first three gates, then stops at 64 existing static-contract failures
outside this feature across 5,190 checks. No production admission changes.

### Material-backed laboratory carpentry (2026-10-23)

The reference effect timeline now manufactures beds, doors, tables and chairs
from explicitly observed finite WOOD inventory. Each 50 eligible service ticks
completes one unit, consumes one log, increments its exact aggregate stock and
creates one canonical item. Item identity derives from the original work order
and completion ordinal, retaining identical output across partitioned waits and
snapshot recovery. The closed aliases share one recipe definition.

Carpentry joins brewing and cooking in the bounded shared worker/workshop
allocator. One living citizen cannot supply simultaneous service to several
orders, and one completed carpenter workshop services one order at a time.
Conditions and material availability gate service; blocked time is not banked.
Canonical item growth updates scan charges and remains within the existing
entity bound. Duplicate output identities, stock overflow and budget exhaustion
refuse within the existing atomic advance shadow.

New carpentry orders carry an explicit `carpentry/1` production marker. Missing
or unqualified input/output counts remain unknown, and legacy abstract orders
cannot gain fabricated physical production history after reopen. Produced items
belong to the laboratory world; this does not allocate native items, change
construction semantics, or admit a DFHack protocol. The broader production and
semantic execution beads remain in progress.

**Evidence:** all 124 actual-source intent unit tests pass on the final engine
source, including 12 new carpentry regression groups. These cover all nine
recipe aliases, exact input/output conservation, 512 three-family capacity
configurations, partial work, conditions, cancellation, partition parity,
snapshot recovery, provenance and transactional bounds. The 115 integration
tests also passed before the final one-line completion ledger-scan budget
charge; the final unit suite and focused carpentry group were rerun afterward.
The pinned nightly compiler, doc-test phase, scoped formatting and
warning-denied production-library Clippy pass. Whole test-target Clippy stops
at an existing forbidden `expect_err` in `blueprint_layout_tests.rs`. This does
not establish full-workspace, MCP runtime, native-build or live qualification.

### Recover original native work-order goal achievement durably (2026-10-11)

`SemanticWorkOrderSession` now supports an operator-owned `GoalHistoryStore`
and exclusive `DurableWorkOrderGoalMonitor`. The bounded private history retains
read intent, both creation-queue endpoints, both selected-progress endpoints,
complete operations captures, sampled native creation context and publication
boundaries. Each capture is durable before a later source or policy can fail;
the proof transition stays private until its publication is synced and verified.

Recovery binds the same original plan, semantic review and native journal, then
replays operations through one persistent owner and the original predicates
through the real obligation runtime. Every historical policy must be reissued
independently, exactly, twice at the current maximum authority tick. A stored
fingerprint grants nothing. Historical Unverified insertion remains unlinked
after later reconciliation; a historical Created sample must still match its
exact terminal native receipt.

First achievement and terminal history survive repeated reopen. `latest()` is
absent, unfinished stability resets, and original deadlines/cadence remain.
Fresh goal truth, physical quiescence and insertion certainty remain distinct.
Interrupted reads retain progress/allocation/source floors. Unrecordable source
changes and well-formed incomplete crash prefixes fence new proof; previous
reauthorized achievement remains inspectable. Corrupt/torn frames refuse recovery
without repair. Higher consumed ticks remain session authority floors even when
the monitor is replaced. Full event and byte costs are reserved before contact.

The new `DFMWOH01`/`DFMWOF01`/`DFMWOE01` family is bounded to 16 MiB and 2,048
events, uses existing descriptor-pinned private-file custody, and permits
read-only historical recovery. No insertion, native protocol, grant issuer,
generic MCP dispatch or production admission is added. The broader execution
bead remains in progress. See `docs/SEMANTIC_WORK_ORDER_HANDOFF.md`.

**Evidence:** all 56 actual-adapter work-order integration cases pass, including
16 durable-history cases and both partial-bracket counter regressions. The
history cases exercise repeated reopen, False/Unknown current truth, original
deadline/cadence, exact historical policy, later receipt reconciliation, partial
source floors, lost publication acknowledgment, source/oversize fencing,
authority-preserving monitor replacement and real private-file custody.
Adapter-library and integration-target Clippy pass with warnings denied; all
416 inventoried native/semantic inputs remained unchanged during the successful
run. These are component tests with synthetic native responses and real local
storage, not a DFHack plugin build, live-fort or full-workspace qualification.

### Retain source floors through rejected original-goal brackets (2026-10-11)

Selected-order observations now advance the retained progress floor before
cross-source bracket validation. A newer finite counter remains known even
when that progress endpoint disagrees with the queue or the other endpoint;
later observations cannot increase the counter and erase the contradiction.
Original-goal evaluation uses a private pending transition and exposes it only
after final policy and publication checks. All 40 actual-adapter work-order
integration cases pass, including both partial-bracket regressions. This
increment underpins the optional durable recovery described above.

### Continue interrupted setup after action retirement and another reopen (2026-10-11)

An original production goal can now retain proof that a dependent step was
never dispatched after the owning action commit retires. Previously, recovery
could finish workshop construction, discard the undispatched order's custody,
then permanently block continuation after another restart because the missing
old order was indistinguishable from an uncertain dispatched effect.

Retirement now copies only the latest explicit `not_dispatched` state and exact
atomic anchor into the retained original objective. New journal generations
preserve that transfer without changing legacy replay; compaction writes a
separate bounded custody map and retains every referenced snapshot. The source,
original plan seal, historical achievement and old action identities remain
unchanged. No old action handle or dispatch authority is reconstructed.

Both first-opening carried recovery and later retired-goal recovery check the
exact archived frontier against the original seal, chronology and creator
identity. Unanchored legacy records, missing custody and contradictory current
creators remain unresolved. A failed custody check cannot fall through to a
completed physical entity and permit new work. Authorized goal observations
expose retained step anchors; current Observe/Plan authority, verified active
lineage, False original truth and complete same-root quiescence still gate a
fresh continuation. See `docs/PRODUCTION_GOAL_CONTINUATION.md`.

**Evidence:** all 35 durable store/objective unit tests (eight new custody
cases), all 13 durable-progress integration tests and all-target lab Clippy with
warnings denied pass. All 109 actual-source module tests (five new custody
cases) and warning-denied all-target harness Clippy pass, with 653 unchanged
inventoried inputs through the final helper window. Actual planning, reference
effects, durable retirement, compaction, repeated recovery and successful
continuation execute in those helper tests. Three new MCP facade tests and
existing fixture corrections are source-reviewed/formatted but unexecuted;
unchanged Asupersync still prevents the full MCP target from compiling within
the shared memory budget. This is component evidence, not full-workspace,
native-build or live qualification. The production bead remains in progress.

### Prove original native work-order goals (2026-10-11)

`SemanticWorkOrderSession::poll_original_goal` now observes and proves the
complete original goal after native insertion or exact read-only reattachment.
It uses concrete existing operations/1.3 and selected progress/1.12 read clients
with matching work-orders/1.10 queue endpoints. Captures must describe one
paused fortress frontier; native generations and the owner-allocated canonical
cursor remain separate. Both progress endpoints and the operations/queue
witnesses remain available for independently issued source policy.

The exact original Created receipt binds the finite order to its original
key-derived canonical entity. All original postconditions, plan terminal and
obligation terminal participate in proof, with unchanged failure, deadline,
cadence and stability rules. Current False/Unknown truth is separate from
process-local first achievement. A source reset, changed order, cursor gap or
failed policy cannot manufacture continuity or positive samples. Replacement
invalidates the old monitor; a new session opening cannot import its streak.

Semantic completion additionally requires physical quiescence: the exact
original finite order remains present with qualified zero remaining and
validated/inactive status, alongside a qualified complete empty global job
roster. A missing order, zero counter alone, nonempty roster or Unknown original
insertion cannot prove completion. This conservative job rule remains necessary
because the read family does not expose originating manager-order IDs on jobs.
The monitor performs no game mutation or time advancement and consumes explicit
entity, byte and wall budgets. See `docs/SEMANTIC_WORK_ORDER_HANDOFF.md`.

**Evidence:** all 38 actual-adapter public integration tests pass, including 17
new original-goal cases and the existing 21 insertion/custody cases. Positive
completion runs the real closed RPC clients over strict fixture streams;
refusal, policy, recovery, deadline, continuity, counter and ownership cases also
pass. Adapter-library and focused test Clippy pass with warnings denied on
unchanged compiled native source/test inputs. These are synthetic native
responses, not a DFHack plugin or live-fort qualification. Optional durable
native goal history is implemented above. The generic MCP route, independent
policy issuer and live admission remain separate unfinished boundaries.

### Complete dependent work within one foreground wait (2026-10-11)

The laboratory's `fortress.wait` now interleaves time advancement and retained
action polling. A sufficiently long call can prove excavation, dispatch and
complete construction, then dispatch and complete production. The first round
observes without advancing; later rounds honor original cadence/deadlines and
current scoped effect authority. A deferred pause stops time within the call.
The original sealed plan, identities and obligation runtimes remain unchanged.

Each round publishes an atomic simulated frontier. The session then persists
world, work dispositions and original-goal history before another round can
consume time or dispatch. Intermediate achievement survives later consumption
and durable reopen. Failure reports actual advanced/remaining time, and current
Observe authority gates the partial frontier shown in the Agent Turn. A durable
publication failure fences later progress. Final Task rendering cannot poll or
cause another effect; each action retains its original proof anchor separately
from its current physical work.

One request bounds aggregate dispatches, tracked identities, rounds and polls
and uses one cooperative wall deadline. It does not create new plans, run in the
background or transfer a shared peer's action authority. See
`docs/LAB_FOREGROUND_PROGRESS.md` for exact behavior and limits.

**Evidence:** all 104 actual-source module tests and all-target warning-denied
Clippy pass. Eight new tests use the real StaticPlanner, MemoryAdapter and
obligation runtime, covering dependency chains, exact deadlines/cadence,
deferred pause, rollback, authority, original identity, aggregate budgets and
explicit continuation. Four facade/persistence integration regressions and
existing durable fixture corrections are source-reviewed and formatted but
unexecuted. Full MCP compilation remains blocked before MCP in the unchanged
Asupersync graph; no full-workspace or native/live qualification is claimed.

### Continue retained goals after verified durable recovery (2026-10-11)

A durable laboratory reopen resets observation cursors; it does not abandon the
retained production goal. The previous continuation gate compared current epoch
to the immutable original plan and therefore rejected every reopened pursuit.
Recovery now verifies the exact persisted head and its single reset/rehashed
anchor, then binds each fully verified, nonabandoned objective to that current
process-local observation frontier. Original plan digests, source bytes and
first achievement remain unchanged.

Continuation still requires current authority, False original goal truth and
complete original/same-root physical quiescence. Current evidence must advance
the verified active frontier within its epoch; arbitrary resets, backward ticks
or cursors and same-cursor forks refuse. Checkpoint restore clears continuity
immediately while staging abandonment, and durable abandonment cannot be reopened
as an active goal. Neither recovery nor planning restores old action handles,
preparation lifetimes or dispatch grants.

**Evidence:** all 96 actual-source production/projection/source/continuation/
forecast tests pass, including three new recovery regressions. The real
MemoryAdapter test executes two recovery epochs and new pursuits while retaining
the original goal and seal. Mutation cases cover exact persisted/recovered
identity, clocks, cursor forks, authority, Unknown truth and unresolved work.
All-target warning-denied module Clippy passes; all 644 inventoried inputs remain
unchanged. Two new MCP facade tests cover arbitrary epoch changes and expired
current authority, and the existing repeated-reopen test now explicitly checks
old seals in new epochs. These facade tests remain source-reviewed/formatted,
not executed: the full MCP check was stopped during unchanged Asupersync
compilation at the shared 8 GiB ceiling before reaching MCP. The final native
increment also passes all 96 laboratory tests. This establishes no full-workspace,
plugin or live qualification.

### Original semantic native work-order handoff (2026-10-11)

`SemanticWorkOrderSession` now carries one original unconditional
`CreateWorkOrder` for the four closed wooden-furniture recipes into the existing
work-orders/1.10 coordinator. It preserves the original semantic key, complete
plan and obligation while binding the native candidate to independently
qualified canonical source and a complete paused queue capture. The concrete
complete citizen acquisition owner supplies the observing side without creating
new source-policy or compatibility authority.

A separate private `DFMSOJ01` association store retains the exact semantic/native
identity and journal incarnation before native preparation. Fresh evidence and
custody checks guard preparation and the sole insertion attempt; the final guard
runs after DispatchStarted sync. Cross-goal canonical and native source floors
survive restart, and sorted retained-history validation rejects forks before
merging maxima. Ambiguous persistence or native effects remain unresolved.

Current Query authority permits exact original-plan reattachment, historical
inspection and receipt reconciliation. Unknown cannot become permission to
insert again. Native Created readback is reported separately as
`NativeOrderCreation::Inserted`; original postconditions and goal remain unproved
and the original obligation is unchanged. Cancellation retires only local
undispatched preparation. This is a bounded operator-owned Rust handoff, with
no new MCP tool/dispatch route, wire method, production runner or admitted tuple.

**Evidence:** all 21 new public integration tests pass against the actual adapter
library, covering strict native codecs, complete citizen acquisition, all closed
recipes, lost reply without redispatch, authority and budget refusal, sync
failure, exact restart, cross-goal chronology and real private-file storage.
All 18 existing work-order coordinator regressions, all 43 workforce integration
tests and all 93 actual-source production tests pass. Warning-denied adapter
library Clippy passes; all 644 inventoried source/build inputs stay byte-identical
across these checks. Independent review and scoped formatting pass. Native
responses are synthetic; this does not establish a plugin build, live campaign
or full-workspace qualification. See `docs/SEMANTIC_WORK_ORDER_HANDOFF.md` for the supported API,
original-plan recovery requirements and limits.

### Consumption-aware original production and whole-goal forecasts (2026-10-11)

New laboratory production intake uses fixed `consumption_aware_v1` source
semantics. Joint staffing applies even without setup options, and orders
sharing their only eligible worker receive verified-completion dependencies.
Observed population and metabolism phase determine reserves through the complete
sealed obligation horizon, including setup and existing queued service. The
bounded monotone calculation includes every original quota while preserving its
exact terminal minimum and refusing unknown or overflowing planning evidence.

New production sources retain their compiler field and bind every canonical
request field into the actual plan seal, including unused setup permissions and
sites. Historical sources without the field preserve their prior bytes,
compiler, summary and seal. New `production_continuation` `/2` envelopes choose
the fixed new compiler while preserving the original request; historical `/1`
reconstruction and exact pending-candidate retry preserve the sealed generation.

Discarded-fork forecasts evaluate the original terminal predicate and physical
work separately from action proof. Whole-plan completion requires Verified
actions, True original goal and physically quiescent work together. Unknown goal
truth stays Unknown, and early proof does not hide active work. Separate output
fields retain action timing, goal truth and physical-work state. Prediction
never certifies canonical state, creates authority or dispatches real work.

**Evidence:** all 93 actual-source production/projection/source/continuation/
forecast tests pass, including 28 new cases. These execute actual workspace
StaticPlanner, MemoryAdapter, finite-capacity effects and source qualification.
Focused warning-denied Clippy passes for the actual source modules and their
tests on the pinned 2026-08-31 compiler. Updated MCP facade/Task expectation
regressions are source-reviewed and formatted but have not executed for this
increment. No full-workspace, native-game or live qualification is claimed.
See `docs/PRODUCTION_RESERVE_PLANNING.md` for usage, bounds and assumptions.

### Explicit continuation of retained production goals (2026-10-10)

The existing laboratory `fortress.plan` blueprint argument now accepts the
closed `continue_goal` template naming a retained committed production digest.
It retrieves every original quota and setup permission/site, checks current
Observe/Plan authority and original False truth, proves original and same-root
physical/deferred work quiet, then compiles a new plan for explicit review.
Unknown evidence, active work, abandoned restore epochs and legacy action
sources cannot authorize an inferred replacement request.

Commit repeats the complete-lineage gate before leases, durable admission or
adapter reservations. A reviewed proposal cannot duplicate another session's
newly committed pursuit. Only an unchanged exact seal may exclude its identical
previously admitted candidate for idempotent retry; stale replay creates a new
seal and therefore excludes no unresolved old candidate. Both production source
kinds require reviewed replay after anchor changes.

The new durable `production_continuation` kind stores a flat versioned envelope
with immediate parent, original root and complete production request. Its source
digest, including unused setup permissions, participates in the actual plan
seal. Recovery reproduces the exact archived plan without consulting today's
mutable goal book or restoring dispatch authority. Original first achievement,
action receipts and source remain immutable. Goal, handoff and compact Task
projections preserve lineage and expose an eligible explicit planning request.
Existing source, planner, capability, workload and 64-goal bounds remain active.

**Evidence:** all 65 actual-source production/projection/continuation tests pass,
including ten new continuation regressions using the real planner and adapter.
All 96 laboratory tests pass, including all 27 durable-store tests and the new
continuation retirement, compaction and reopen case. Focused warning-denied
Clippy passes for all laboratory targets, the adapter library, and the actual
production/continuation source modules. Inventoried source inputs remained
unchanged through these final tests and lint checks. Ten MCP integration tests cover shared sessions,
unknown siblings, authority expiry, repeated restarts, restore abandonment and
the exact-seal retry boundary. They are source-reviewed and formatted but have
not executed: full MCP compilation is killed in unchanged Asupersync before
reaching MCP. No full-workspace or native/live qualification is claimed.
See `docs/PRODUCTION_GOAL_CONTINUATION.md` for the usable request and limits.

### Durable native original-goal monitoring (2026-10-10)

The semantic workforce adapter now provides a separate operator-opened
`GoalHistoryStore` and exclusively owned `DurableWorkforceGoalMonitor`.
`begin_durable_goal_monitor` and `poll_original_goal_durable` preserve the exact
original semantic/native review, raw selected-workforce captures, publication
boundaries, interruptions and canonical/native/tick floors across process loss.

Recovery independently reissues the original historical source policy twice
under current Query/Observe authority, reprojects each raw capture and replays
all original predicates through the real temporal runtime. Stored policy
fingerprints are comparison-only. A missing, changed or broader source grant
cannot manufacture a previously unproved historical achievement.

The verified first achievement and original terminal outcome survive recovery;
current evidence and unfinished stability do not. Fresh observations remain
necessary, original deadline/cadence do not renew, and an Unknown native action
remains unresolved even when the goal becomes true. Read intent precedes native
contact; raw capture precedes proof; synced publication precedes returned state.
Ambiguous writes fence the handle. The private append-only store has an 8 MiB /
1,024-event bound, exact journal/review binding, and no silent repair or pruning.
All four possible poll events are reserved before contact, including caller
bounds, so refusal cannot strand newly observed floors behind a small budget.

**Evidence:** all 43 actual adapter integration tests pass on pinned nightly,
including eleven new durable-history cases and real private-file custody tests.
Independent review covered historical authority, raw replay, original temporal
semantics and ambiguous I/O. Final native43 and queue55 runs kept all 379
inventoried semantic inputs byte-identical. This is an adapter feature; broader
semantic MCP dispatch, an independent canonical issuer, real native execution
and live admission remain separate work. No protocol or production map changes.

### Production plans account for existing physical work (2026-10-10)

Original DRINK/FOOD quota compilation now inspects current registered brewing
and cooking orders, including work recovered without process-local handles.
An active producer of the same requested output causes explicit refusal with
its source anchor, identity, status and earned work. Predicted output is never
credited as current stock or used to justify a duplicate conditional order.

Other registered production contributes a conservative queued-service allowance
to newly sealed obligation deadlines. The allowance includes remaining batches
and earned partial work, respects setup/dependency horizons, and is bounded with
the whole plan to one model year. Complete-source scans are capped at 65,536
entities and 128 active registered orders; explanations show at most 16 rows.
Unknown lifecycle/progress evidence refuses planning. The allowance describes
reference-model scheduling under stated assumptions, not guaranteed completion.

Current order absence, progress and setup candidates are range reads absent
from the existing action/predicate witness. Production plans therefore require
explicit source replay and review after any anchor change; equal action bytes
cannot silently approve changed deadlines. Uncontended requests preserve their
existing source and action shapes, and replay recomputes current workload.

**Evidence:** the actual-source production/projection harness passed 55 tests,
including twelve new queue cases and all fifteen prerequisite cases. Regressions
execute the real planner, MemoryAdapter and finite-capacity timeline: a small
order fails under its former isolated deadline and succeeds after allowing for
twenty earlier batches in both production directions. A further MCP regression
covers mandatory reviewed replay; full MCP compilation was killed in unchanged Asupersync before reaching MCP.
No native/live admission or whole-workspace qualification is claimed.

### Executed Rust evidence for the preceding feature generation (2026-10-10)

With the execution service restored, the exact unchanged `57c0cd6` source passed
32 semantic-workforce integration tests, all 21 effects-timeline tests (including
512 exhaustive finite-capacity configurations), all 95 laboratory tests, and
43 actual-source production/projection tests on the pinned 2026-08-31 nightly.
These results execute the previously unverified capacity, prerequisite and fresh
goal-monitor code; their earlier outage-era notes remain historical. They do not
establish formatting, warning-denied Clippy, full MCP or live qualification.

### Production goals compile missing staffing and workshops (2026-10-10)

Original laboratory production sources now accept opt-in `prerequisites`:
permission to enable known-disabled labor and explicit bounded Still/Kitchen
sites. The compiler emits existing SetLabor, Build and CreateWorkOrder actions
in one sealed dependency plan. Each order waits for its required setup proof;
normal capabilities, risks, compensation and checkpoint rules remain effective.
Existing complete workshops and eligible staffing are reused.

Staffing is selected jointly across both roles with bounded top-two candidate
sets. Distinct usable workers take priority, followed by fewer labor enables,
military-status preference and canonical IDs. This protects scarce specialists
and scarce enableable roles. When one worker must serve both orders, the later
order depends on the earlier one's verified completion, so its original sealed
deadline includes the known wait. Existing FOOD-before-DRINK lowering is retained.

Assignment requires explicit known life and an eligible false prior labor flag;
unavailable evidence never becomes permission or exact inverse compensation.
Needed construction requires caller-selected one-level sites of at most 64
tiles, established floor/support/safe surroundings, and known non-overlapping
building geometry. The complete entity scan is bounded to 65,536 records; at
most four setup actions and two orders reach the ordinary bounded parser.

The durable original source retains every quota and unused site permission.
Replay recomputes batches and missing setup from current evidence; all original
stock minima remain the terminal goal. Analysis explains observed/planned
staffing and workshops, added dependencies and required action capabilities.
Requests without prerequisite options retain previous source and action shapes.

**Evidence:** fifteen focused Rust tests are present, including actual
planner/MemoryAdapter setup execution, capability refusal, unknown evidence,
replay and an unequal shared-worker deadline scenario. Two independent source
reviews traced the compiler, planner and adapter integration. An independent
JavaScript oracle matched exhaustive staffing selection in 1,103,762 bounded
configurations. That oracle is not Rust execution. Current compilation, Rust
tests and rustfmt could not run during the execution-service disconnection;
no full-workspace, native or live qualification is claimed.

### Fresh original-goal proof for semantic workforce actions (2026-10-10)

The single-step semantic workforce handoff now has a foreground original-goal
monitor backed by fresh workforce/1.17 observations. It retains the original
canonical/native/historical citizen identities and projects current labor bits
from a newly acquired capture after closing its source. An independently trusted
policy callback must authorize the exact source, anchor and selected domain;
normalization and historical Applied receipts grant no observation authority.

Each sample evaluates all original postconditions, the plan terminal condition
and the original obligation terminal together. The existing temporal runtime
keeps the original deadline, cadence and stability requirement. Gaps, failed
reads and denied publication erase unfinished stability; duplicate ticks cannot
earn extra samples. The immutable first satisfaction anchor and historical
fulfillment stay separate from current false/unknown facts. A current true
failure predicate blocks current proof even after historical fulfillment.

Native action history is a separate result. Fresh true goal predicates cannot
resolve an Unknown action; semantic completion requires both current goal proof
and verified historical single-labor readback. Query/Observe-only recovery can
start a new process-local monitor after exact original-plan reattachment,
without renewing the deadline, importing a stability streak or dispatching work.
Canonical cursor, source-clock and grant-expiry floors survive failed reads.
The monitor cannot be publicly cloned to branch an unfinished proof streak.

**Evidence:** seventeen new public-API integration tests are present, bringing
the isolated target to 32 tests. Source/API review includes the actual native
coordinator, evidence scope, obligation runtime and bounded read reservation.
Current compilation, Rust tests and rustfmt could not execute because the
execution service disconnected. The preceding fifteen passing acquisition
tests apply to their earlier generation, not this changed source.

This is an adapter API over an existing development workforce source. Independent
source authority, broader semantic MCP dispatch, real native evidence and live
admission remain separate work. Durable history is supplied by the later
increment above. No wire method,
production runner or compatibility registry entry is added.

### Finite laboratory production capacity (2026-10-10)

Registered brewing and cooking orders now compete for actual reference-model
worker and workshop slots. Each running order needs one eligible living
labor-enabled unit and one completed matching workshop. A multi-skilled unit
cannot service both families in one interval. A bounded iterative matching
algorithm preserves earlier partial-work/canonical-ID preferences while moving
flexible workers where needed to keep specialists productive.

The causal timeline allocates service after condition evaluation and refreshes
capacity blockers after world events. Waiting orders preserve earned work but
bank no elapsed time. Workshop completion and worker death affect only later
intervals. Matching is capped at 65,536 canonical entities before new matching
allocations, and source scans, traversal and ordering consume the existing
advancement budget. Whole-advance publication remains atomic in the caller's
transaction shadow.

**Evidence:** nine new Rust test functions are present, including 512 exhaustive
actual-timeline skill/workshop configurations, partition comparisons, source
eligibility, capacity changes and budget refusal. Two existing simultaneous-stock
fixtures now provide the two workers and workshops their scenario requires.
An independent JavaScript exhaustive oracle matched the allocation algorithm
against all feasible distinct-worker assignments in 60,016 small configurations.
That oracle does not execute Rust. Compilation, Rust tests and rustfmt for this
increment could not run because the execution service disconnected even for
read-only commands; earlier passing suites do not qualify these changed bytes.

The model covers brewing/cooking capacity only. It does not schedule shared
labor across excavation, construction or military work, and adds no native
protocol, MCP tool, live capability or compatibility admission.

### Original-goal lifecycle in the MCP laboratory (2026-10-10)

The laboratory commit, progress, restart, shared-session and restore paths now
use the durable original-goal store. Source and action admission are saved
together before effects. Current-authorized predicate evaluation precedes the
atomic world/progress publication; in-memory first achievement changes only
after that publication succeeds. Goals remain discoverable after action Done
and compaction, independently of recovered unfinished action monitors.

Recovery reconstructs each exact original plan at its archived sealing snapshot,
checks the full plan digest, and re-evaluates historical achievement against its
original predicate at the pinned proof anchor. Unreproducible or unverified
history remains indeterminate and supplies no dispatch authority. Current truth
may be false even with a verified immutable first achievement. Restore keeps
that historical fact while publishing abandonment with the restored world.
It does not erase the original goal or let the restored world newly satisfy it.

Goals follow the canonical shared fortress while grants, budgets and dispatch
receipts remain session-specific. Current Observe authority gates source,
owner, predicate, anchors and counts as well as truth. A denied view is an
explicit unavailable/unknown coverage entry. Numeric session IDs can repeat
across processes; recovered goals retain historical origin metadata and never
claim ownership by a new process solely from matching numbers. Compact Task
results preserve immutable history separately from current truth.

Admission checks the 64-goal bound before consent, reservations, preparation
or effects. Removing ordinary history requires a currently proven true goal,
verified historical achievement, all original physical effects proven quiet
and no unfinished durable commit. A once-achieved quota that consumption makes
false, unknown predicates, abandoned pursuits and missing work identities do
not yield capacity. Some recovered never-dispatched work stays conservatively
unknown after its old dispatch bookkeeping retires; there is no fabricated
quietness or automatic replacement plan.

**Evidence:** all 95 laboratory tests passed for the underlying durable and
effect implementation. All 28 actual `lab_world.rs`/`observation_projection.rs`
module tests passed through the direct-source harness, including actual
consumption timelines where completed order proof and original quota truth
diverge. MCP lifecycle and compact Task regressions are checked in and reviewed,
but their runtime execution is not claimed: full MCP compilation is blocked by
SIGKILL in unchanged pinned Asupersync before this crate under the shared memory
limit. This does not establish full Rust, native or live qualification.

### Fresh canonical evidence for semantic workforce execution (2026-10-10)

`CitizenWorkforceEvidenceOwner` now implements the observation owner required
by `SemanticWorkforceSession`. Construction independently reacquires the entire
original citizen roster, and every refresh after native capture invalidates
the preceding publication before any validation or I/O. The owner checks the
native capture against the exact original canonical source, then uses a fresh
source to assemble the complete paused V1 capsule. Any changed fortress, bridge
generation, software identity, tick, pause state, roster or source bytes requires
a new original plan instead of refreshing authority for the old plan.

The owner receives an operator-supplied source factory and independently trusted
policy callback. It issues no compatibility or predicate authority itself. The
actual TCP implementation uses the existing authenticated V1 codec, preserves
one absolute connection/handshake deadline, narrows every I/O timeout to the
remaining allowance and configured read/write cap, and fences failed sources.
Acquisition sources close before publication policy is checked twice. Local
validation, pages and policy calls consume the original shrinking byte budget;
only two prepaid routing views may be borrowed before another refresh.

The configured roster ceiling is 1–4,096 citizens (default 256), but each read is
bounded to the exact original roster. The audited local reservation is 64 KiB
per citizen plus fixed work, so the parent coordinator's 16 MiB refresh allowance
can conservatively refuse larger rosters. The V1 source proves citizen identity
and pause state within independently authorized scope; it supplies no labor
state and cannot complete the original labor goal. See
`docs/SEMANTIC_WORKFORCE_HANDOFF.md` for the integration contract.

**Evidence:** all 15 new public-API integration tests passed through the ordinary
compiled production adapter. They exercise actual semantic/native coordinator
handoff, loss of evidence after DispatchStarted synchronization, complete
repagination, canonical identity boundaries, invalidation, independent policy
refusal, deadlines, source closure and the actual V1 wire codec. No private
production implementation is substituted by the test target. The earlier twenty
handoff/store unit tests remain unexecuted after the monolithic adapter test
build exceeded memory; the new passing tests do not stand in for that suite.
Full workspace, native DFHack and live qualification remain unestablished.
No new protocol, MCP mutation route, issuer or production admission is added.

### Durable original-goal storage (2026-10-10)

`DurableLabStore` now retains original objectives independently of unfinished
action commits. `persist_objective_commit` saves the original source, sealed
snapshot, plan and intent identities, owner session and action admission in one
record before effects. Retiring the action commit does not retire its goal.
`persist_progress_with_objectives` publishes the world, action frontier, first
satisfaction and restore abandonment atomically. First proof and abandonment
anchors remain immutable through subsequent progress, compaction and reopen.
Their exact snapshots and the original sealing snapshot remain retained roots.

The store caps history at 64 goals per fortress. New work cannot evict an
unresolved goal or a goal with an unfinished durable commit. Removal additionally
requires the caller to prove current physical quiescence of every original
effect; the storage layer does not infer that evidence from a missing commit.
Identical admission retries cannot recreate retired action work or move history.
Unknown, duplicate and conflicting goal updates are refused before publication;
uncertain writes fence the store until verified reopen. Legacy records remain
readable without inferred objectives or reconstructed historical success.

This is the storage boundary. The caller remains responsible for current
Observe authority, evaluating the original predicate, and verifying its sealed
source when reconstructing history. The MCP lifecycle above supplies that
integration. The format and crash tests use actual snapshots,
journal files and compaction; they establish no native/live admission.

**Evidence:** all 95 `dfmcp-lab` tests passed: 63 library tests (including all
26 durable cases, eleven new), 11 deferred-lifecycle tests, 13 atomic-progress
tests and eight full-action laboratory tests. The new format tests also refuse
30 malformed admission, compacted-goal and progress payloads. There were no
failures or ignored tests. This is focused laboratory execution; full workspace
and MCP runtime qualification remain pending.

An additional regression covers automatic compaction failing after admission was
already synchronized: every later write and apparent no-op is fenced until
verified reopen reconciles the published goal. The caller cannot silently
continue with a goal book that differs from durable storage.

### Original production goal preservation (2026-10-10)

Production plans retain normalized original quota requests instead of replacing
those requests with the work orders calculated at the first anchor. Explicit
replay recompiles the quantity still needed from current eligible inventory and
returns its fresh analysis. The sealed plan goal includes every requested stock
minimum; individual work-order postconditions still prove their own completion.
Finishing four brewing batches does not prove a 60-drink goal when consumption
has left only 53. A quota already satisfied at planning remains part of the goal.

Task views resolve witnessed rebase aliases to the actual committed objective,
require current Observe authority, and expose current original-goal evidence,
action proof and physical work separately. A quiescent finished plan cannot
complete its task with a false or unknown original goal. It instead requires
explicit replanning or reconciliation, with no replacement dispatch or blind
retry. The original-goal lifecycle above now retains historical objective
custody beyond completed-plan retirement.

A distinct bounded `production` journal source preserves original quota JSON
for exact sealed-world reconstruction and later replay. Legacy action records
retain their original action semantics and gain no inferred stock goal. The
parser validates the closed template and tokens, bounds 1–64 submitted quotas
before canonicalization, and refuses duplicate explicit/alias input channels.

**Evidence:** all 15 actual `dfmcp-lab` durable unit tests passed, including two
new source-codec/reopen cases. All 27 tests in the actual `lab_world.rs` and
`observation_projection.rs` modules passed in a temporary harness importing those
files directly with the current workspace crates and pinned serde dependencies.
This validates the production/compiler and projection boundary, not the MCP
transport. Five new MCP facade/replay tests and additional Task regressions are
checked in. Full MCP `cargo check` was SIGKILLed while compiling unchanged
Asupersync 0.5.0 under the shared memory limit, before reaching `dfmcp-mcp`;
MCP runtime execution, full workspace qualification and live admission are not
claimed.

### Durable original-room readiness workflow (2026-10-10)

`scripts/track_room_readiness.py` now makes the existing joint room condition
usable through `start`, `sample`, `wait`, `inspect` and `cancel`. Its separate private
journal retains the complete original room-backed batch, geometry, furnishing
DAG, allocation evidence, every original placement receipt and file identity,
endpoint and fixed game-time policy. Reopening cannot substitute a successful
subset or renew the deadline. A single sample brackets the released complete
operations capture with identical paused map endpoints and all original receipt
queries. Every original floor, required wall and furnishing shares one stability
window; separately timed successes cannot complete the room.

Synchronized read intent precedes native contact. Full raw evidence and complete
output reservation precede publication, followed by file/parent synchronization
and original-source rechecks before acknowledgement. Interrupted reads carry no
effective stability credit. Offline replay recomputes progress without restoring
publication authority; cancellation remains available after original-source loss.
Both top-level and nested readiness claims require original custody in the current
call. Whole original plans, per-target room associations and historical evidence
remain visible after restart. Source, clock and identity changes, torn journals,
ambiguous writes and failed final output checks cannot become verified success.

The workflow is bounded to 32 furnishings, 4,096 map cells, a 128 MiB journal,
1,030 frames and a 256 KiB complete response under one shrinking operation budget.
It uses existing unadmitted furniture/1.19, operations/1.4 and map/1.5 query
profiles. It adds no native mutation, room assignment, canonical evidence issuer,
Rust MCP integration or production admission. Readiness is historical sampled
terrain and furnishing evidence; present usability, continuous preservation and
an atomic cross-profile snapshot remain unproven. See `docs/ROOM_READINESS.md`
and `architecture/room_readiness_monitor_v1.json`.

Foreground `wait` reuses the exact original journal for 1–32 new samples under
one shrinking call budget. It retains original source custody through bounded
delays, reserves the five additional map RPCs before each read intent, and
reserves the full original-room response plus wait envelope before publication.
Terminal goals replay offline. Sample limits, stalled game ticks and insufficient
RPC or wall allowance return bounded partial progress. Failed acquisitions stop
without retry; subsequent explicit calls retain the original deadline and total
observation count. The workflow creates no background worker and does not
advance or unpause the game.

**Evidence:** the existing joint-condition and synthetic TCP acquisition suites
passed all 27 tests. New private-file and CLI subprocess tests exercise full
original-plan custody, restart, interrupted acquisition, source loss, immutable
deadlines, map drift, bounds and complete-result publication. Their recorded
execution is described in `docs/ROOM_READINESS.md`. These are Python/POSIX and
synthetic-peer tests, not native SDK, live-fortress or full-repository qualification.
The durable increment passed 60 focused methods. Foreground waiting then passed
ten additional room tests, all 20 shared-scheduler regressions and two existing
CLI regressions, including the maximum 32-target paged construction monitor.
Both late-publication revocation scenarios also passed through the new wait path.

### Durable semantic workforce handoff (2026-10-07)

`SemanticWorkforceSession` now connects an original sealed, single-step `SetLabor`
plan to the existing workforce session and native effect journal. Observation
returns a review seal covering both plans, exact source identity and the native
journal incarnation. Preparation first syncs an immutable semantic/native
association. The native journal still owns Intent, DispatchStarted, receipts and
at-most-once dispatch. Independent canonical evidence and current original-unit
authority are rechecked after the native capture and immediately before native
prepare or commit, including after durable dispatch intent.

The new private association store binds the full original semantic digest rather
than only the native idempotency key. Recovery requires exact original-plan
reattachment; changed predicates or obligations under the same key refuse.
Missing associations cannot adopt old native records. A lost acknowledgement
continues through original-key reconciliation, and Unknown never grants another
dispatch. Operator-owned durable storage uses the existing descriptor-pinned,
exclusively locked private-file implementation and strict bounded hash chaining.
Ambiguous writes or changed custody fence the owner. Read-only association
recovery does not create or sync that file.

Results retain the original plan, postconditions and optional obligation. A
separate historical action result verifies the requested labor value and unchanged
other columns; a broader native Applied receipt remains explicitly outside the
single-labor semantics. Original goal proof stays pending. Cancellation retains
current canonical unit scope and remains possible after the original plan expires.
The whole call reserves native connection/RPC work, evidence refreshes, journal
work and output before effect I/O under one shrinking wall deadline.

This is a callable Rust coordinator with an injected independently trusted evidence
owner. It adds no evidence issuer, generic dispatcher, live MCP write path or
admission. The existing native owner requires whole-fortress Query, Plan and
ConfigureLabor grants for control; selected-only canonical grants are never widened.
It refuses multi-step plans, cross-family preconditions, compensation
and mandatory checkpoints it cannot satisfy. Other native families and original
postcondition/obligation verification remain unfinished. See
`docs/SEMANTIC_WORKFORCE_HANDOFF.md`.

**Evidence:** the final production source passed
`cargo check -p dfmcp-adapter --lib --offline --locked` with no diagnostics and
unchanged hashes across all 386 scoped files. Twenty new coordinator, recovery,
deadline, scope and durable-store tests are present, including real private-file
append/reopen and every partial or corrupted association publication. Their
focused unit build was OOM-killed before any test ran under the shared 8 GiB
memory limit; the unchanged source snapshot and the cgroup OOM counter confirm
the failure. The sole unused-import warning was removed afterward, and all three
new Rust files pass scoped Rustfmt. The 262 earlier passing intent, lab and
routing tests remain evidence for their own recorded source generations; they
do not qualify this new handoff. Full workspace, native and live qualification
remain pending; the compatibility registry and production protocol map are unchanged.

### Evidence-bound live routing and honest forecasts (2026-10-07)

Advisory live routes now preserve canonical unit IDs, the exact requested labor,
and every furniture material selector. V1 unit IDs decode through their actual
native-plus-one projection; spatial/1.8 uses its actual citizen namespace.
`LiveRoutingEvidence` binds resolution to a validated typed projection, canonical
anchor, source digest, bridge generation and independently issued fact/domain
scope. Missing or untrusted evidence cannot turn route metadata into a native ID.

`resolve_workforce_step` requires the exact native paused capture, selected unit
set and one unambiguous selected-only work detail containing only the requested
labor. Disabling refuses another detail that might continue granting that labor.
Its immediate effect verifier requires native Applied and readback showing the
requested labor value with all other columns unchanged. `resolve_furniture_step`
binds exact native item identity, material IDs, position, flags, target and map
dimensions to spatial evidence. Non-default material tokens, nearest selection
and reservation policies remain explicit and unresolved because the current
native capture cannot prove them. Geometry is bounded before tiling or offering
a native route, including cuboids constructed without their validating constructor.

MCP routing output identifies canonical units as strings, leaves native units
unresolved, includes the retained material selector and explicitly reports
`execution_ready: false`. The new Rust resolvers produce source-bound native
plan candidates with the original step key; they do not execute, preserve full
semantic custody through a native journal, complete original obligations or add
admission. The single-labor coordinator above now supplies that custody handoff
for its explicitly bounded subset; other families remain separate work.

Forecast advancement and poll errors now return `available: false` with their
actual reason instead of an available partial prediction. The final simulated
slice stops at the sealed horizon. Two new MCP source tests compare a dependency
forecast with one long real wait and exercise an unavailable future-input forecast.

**Evidence:** all 17 focused adapter routing tests passed (13 new resolution cases
and four existing routing cases), with zero failures or ignored tests and all
383 scoped source hashes unchanged. Tests use the real projection constructors,
strict workforce codecs and checked-in furniture capture fixture. Changed Rust
passes scoped Rustfmt and independent source review. The final core also passed
163 intent and 82 lab tests. The two new MCP tests remain unexecuted: MCP source
has not compiled after pinned asupersync compilation exceeded available shared
memory. A read-only resource review found about 1.6 GiB compiler headroom against
a previously observed 5.924 GiB compiler RSS lower bound; no repeated build or
qualification success is inferred. Full workspace, native and live qualification
remain pending; the registry and production protocol map are unchanged.

### Causal reference time and bounded progress (2026-10-07)

Reference physical work now advances at actual production, construction,
excavation, metabolism and combat boundaries. Work eligibility is frozen for
each interval, so a completed predecessor or workshop cannot retroactively
award work, consumption releases a stock gate only for later time, and a dead
worker cannot produce afterward. Same-tick production uses ascending entity
order before meals and combat; competing orders retain earned partial work.
The same source world produces identical physical values across successful
wait partitions without claiming identical publication metadata or proof cadence.
Internal events never create obligation observations or stability samples.

`EffectAdvanceLimits` bounds a physical call to 1,000,000 game ticks, 200,000
events and 100,000,000 charged work units, with caller-selected smaller limits.
Cached pending tile sets avoid quadratic excavation scans and update overlapping
designations. Aggregate cached footprints are capped at 1,048,576 tiles before
any region allocation or terrain read. Future source facts, invalid active counters, unobserved excavation
terrain, counter overflow and exhausted budgets refuse. The existing adapter
transaction shadow preserves the complete world, receipts and transcript on a
failed advance. This adds no native timing model or live execution capability.

**Evidence:** the frozen source passed all 163 intent tests and all 82 lab tests
(50 unit, 11 deferred, 13 durable-progress and eight full-action), with zero
failures or ignored tests. The 12 new timeline tests compare long and irregular
waits with one-tick advances and check causality, competing stock gates, workshop
completion, deaths, overlapping excavation, source facts and bounded atomic
refusal. Three new full-action cases prove dependency timing, foreground-only
obligation samples and unchanged world/receipts/transcript on horizon refusal.
Four older future-input fixtures now expect the stronger source-time refusal.
The 109 intent-scope and 391 lab-scope source hashes remained unchanged across
their respective final runs. Changed Rust passes scoped formatting checks.
These are exact development test results; full workspace, native and live
qualification remain pending, and MCP has not compiled or executed.

### MCP physical-work cleanup, clock authority and recovery custody (2026-10-07)

The MCP laboratory now uses actual physical-work inspection alongside immutable
goal receipts. Failed and early Verified work remains in waits, handoffs and
Agent Turns. Plan and single-action cancellation can stop that remaining work
under current original scope authority; `scope="session"` also reaches retained
open work from earlier committed plans. Finalize certificates require terminal
receipts and established quiescence, and session certificates additionally
refuse to hide carried or untracked snapshot work. Refused drains retain explicit
unknown-work progress. Emergency pause resets shared unpause consent and is
reflected in the returned Agent Turn, including partial drain refusals.
`scope="oldest_open_plan"` selects and identifies the oldest unfinished original
plan, so a session with more running work than one call's budget can still drain
its plans individually without weakening aggregate admission.

Tasks expose `proof_status` immediately while their monitor remains Working for
unfinished physical cleanup. Staged cancellation still targets the original
plan after a later commit, preserves original proof receipts and anchors, and
returns separate physical-drain evidence. Bounded terminal summaries preserve
work counts; lost observation is explicitly unknown. This does not add general
asynchronous engine-region drains.

Positive bounded waits require current clock authority, and an unpaused span
must remain inside clock and observation grants through its final tick. Ordinary
stop requires observation and original effect authority, not clock control.
The whole drain shares an aggregate physical-action budget. A rejected or
unauthorized compensation can be explicitly narrowed to `StopFutureSteps`,
keeping actual work drainable without restoring compensation authority.

Spatial reservations publish atomically. Retained action ownership outlives
proof deadlines and lease TTLs while work is active or unknown. Durable commits
also remain retained while terminal goals still own work, and unchanged terminal
states keep their original proof anchors as later worlds are saved. Recovery
observes physical work separately without restoring dispatch authority.

Checkpoint restoration can bring back running entities after invalidating their
action handles. A bounded source-qualified census now exposes those entities as
untracked snapshot work and fences their observed regions. Missing lifecycle or
geometry remains unknown; missing geometry fences spatial admission globally.
The census runs again from the canonical snapshot across restart, including when
the old commit journal has retired. It creates no synthetic goal receipt or
actuator. Such work must be reconciled or observed to become quiescent before
its region can be reused or session quiescence can be certified.

**Evidence and current limits:** the final frozen core passed all 79 lab tests
(50 unit, 11 deferred, 13 durable-progress and five full-action tests), with
zero failures or ignored tests and unchanged exact source hashes across all
five exercised crates. This includes condition-identity and cancellation-mode
narrowing. The frozen production implementation passed all 151 intent tests.
The prior source passed world 180, intent 141, lab 68 and adapter 1,033 tests after
the four deferred fixture corrections. This integration adds 25 MCP tests
(three clock/lease, ten full tool flows, three Tasks, seven census and two
durable terminal-work cases), extends the checkpoint/restart scenario and two
compensation regressions, and corrects one preexisting deferred Task cadence
fixture. All changed Rust files parse under Rustfmt and received independent
source review. **The final MCP source has not compiled or executed.** Protected
local build attempts were stopped or SIGKILLed before tests under the shared
8 GiB memory limit while unrelated compilers consumed the available memory;
later final-source checks still failed before MCP source compilation.
There are no GitHub Actions runs to substitute. Full workspace formatting,
Clippy, debug/release tests, rustdoc, `verify.sh` and `qualify_local.sh` remain
pending; no Rust-qualified, native, live or registry admission claim is made.

### Conditional production is part of actual reference effects (2026-10-07)

Work orders now enforce their sealed `ItemCountBelow`, `MaterialAvailable` and
`CompletedOrder` conjunctions. New orders persist source-qualified names and an
explicit canonical condition list, including an empty list for unconditional
work. Action normalization removes exact duplicate conditions. Completion proof
and physical ownership inspection bind the original name, job and conditions.

Eligible inventory counts release stock and material gates; unavailable,
untrusted, inconsistent, future or malformed observations remain blockers.
Named dependencies require one established order name, complete status and zero
remaining work. Material gates do not imply consumption or reservation.
Same-product thresholds stop production after the first unit that crosses the
threshold. Blocked elapsed time is not banked; earlier partial work is retained.
Forecasts use this same reference transition logic. The causal event timeline
now releases changing gates at their actual boundaries, preserving physical
outcomes across successful wait partitions; foreground proof and deferred
step-dispatch cadence remain separate.

The MCP action grammar and production quota compiler preserve conditions and
show them in sealed step views. Legacy orders missing condition records cannot
prove that their original conditions were empty and stay visibly blocked.
Stronger default proofs can make an old durable plan's digest unreproducible;
the existing indeterminate recovery path retains it rather than retrying it.
The exact JSON shapes and namespaces are in `docs/LAB_SEMANTIC_ACTIONS.md`.

**Evidence:** all 151 actual intent tests pass, including nine condition-effect
regressions and duplicate-conjunction normalization. Source and focused tests
were independently reviewed. The six-case name/condition ownership-mutation
regression passed in the final 79-test lab run. Four new MCP parser/compiler
tests remain unexecuted after the isolated MCP build was stopped under shared
cgroup memory pressure. This increment does not claim full workspace, native
or live qualification.

### Physical reference work and immutable goal outcomes (2026-10-07)

`inspect_effect_work` distinguishes never-dispatched, active, quiescent and
unknown physical work independently of an action's proof state. Dig,
construction and work-order ownership requires the original derived entity,
generation, immutable parameters and eligible lifecycle observations. Missing
or mismatched state after dispatch cannot establish quiescence. Immediate
reference effects own no continuing action work.

`MemoryAdapter::drain_action_work` stops the remaining physical work of an
already terminal action with fresh scoped authority, separate evidence and
transactional rollback. A `Failed` or early `Verified` receipt remains exactly
unchanged. Ordinary cancellation still requests and finalizes nonterminal work;
finalization now proves physical quiescence, rechecks cached cancellation against
current work, and refuses compensation that would launch an unmonitored temporal
effect. Never-dispatched steps do not compensate effects that never happened.

**Evidence:** the isolated core increment passes 78 actual lab tests, including
ten new physical-work regressions, and all four deferred adapter tests. The
deferred fixtures now count later eligible observations rather than registration
as a positive sample. The prior source also passed world 180, intent 141 and
adapter 1,033 tests after that fixture correction. These are executable targeted
results, not full workspace Rust qualification: MCP compilation, formatting,
warning-denied Clippy and the repository qualification scripts remain pending.
MCP cancellation certificates, task views and retained spatial ownership still
need the new physical-work API in the next increment. Asynchronous region
drains and native/live admission are not established by this reference change.

### Immediate laboratory postconditions and transactional refusal (2026-10-07)

Non-temporal laboratory actions must establish every sealed postcondition at
the actual effect boundary. False or unknown postconditions return
`AdapterRejected` instead of leaving an action indefinitely pending without a
proof obligation or deadline. The same rule applies when a prepared successor
first dispatches during polling. Existing commit and deferred-poll transaction
shadows restore the world, receipts, dispatch flags and transcript on refusal,
preserving a clean retry. Explicit temporal actions continue through their
canonical proof monitors.

Two new regressions cover initial and deferred actions with unavailable,
asserted and known-false postconditions. Existing temporal workflows retain
their explicit or planner-synthesized obligations. The deferred post-effect
shape-error regression uses a field exceeding the actual predicate limit,
ensuring it exercises the rollback path.

**Evidence: source and regression review only.** The execution service remains
unavailable, so this final increment has not compiled or executed. Final
formatting, targeted tests and the repository qualification scripts remain
pending under `df-predicate-evidence-authority-r7m`.

### Canonical adapter cadence and atomic deferred polling (2026-10-07)

Both reference adapters now own a canonical `ObligationRuntime` for each
dispatched temporal action. Registration occurs at the actual effect boundary,
including deferred dispatch, and binds the conjunction of the sealed terminal
condition and all postconditions. The registration observation sets the cadence
floor. Different hashes or cursors at the same game tick cannot manufacture
additional stability; an off-cadence contradiction or loss of eligible evidence
resets the unfinished streak. Failure precedence, fixed deadlines, an eligible
exact-deadline sample, observation lineage and immutable terminal receipts all
use the shared engine.

Laboratory time advancement validates the original hash and canonical lookup
identities before creating its transaction shadow. It also rejects pre-existing
future-dated known facts from registered reference producers, so moving the
clock alone cannot grant them input or proof authority, even while paused.
Arbitrary-source prediction metadata remains inspectable and untrusted, while
genuine new reference writes retain the destination tick. A deferred poll now
publishes its effect, monitor, receipt, dispatch flag and transcript together;
post-effect verification errors and cursor exhaustion restore the prior adapter.

Ten new tests cover both adapters' cadence, same-tick polls, contradictions,
deadlines, failure priority, terminal preservation and lineage refusal, plus
clock-source integrity and deferred-poll rollback. Authority recovery fixtures
now explicitly establish a fresh valid observation before expecting success.
A repository-wide source/test/demo caller audit also completed the explicit
laboratory planner migrations.

**Evidence: final sources and regression bodies were independently reviewed and
the uploaded GitHub blobs were read back byte-for-byte. This cadence/atomicity
increment has not compiled or executed.** The execution service remained
unavailable after the recorded disconnect, including a subsequent read-only
probe. Final formatting, tests, `scripts/verify.sh` and
`scripts/qualify_local.sh` remain pending; no full qualification or admission
is asserted. The complete source implementation remains tracked as in progress
under `df-predicate-evidence-authority-r7m` until final executable verification.

### Source-authorized execution and recovery (2026-10-07)

Static planning, deferred-step admission and the canonical obligation/recovery
APIs now accept explicit `PredicateEvidence`. Their raw-snapshot defaults grant
no observation authority; known laboratory callers select the registered
reference scope. Preconditions, already-satisfied goals, terminal predicates and
failure predicates use the same three-valued source/coverage boundary.
Obligation batches validate and evaluate all active predicates before publishing
any transition, preserving atomic refusal and existing cadence/deadline rules.

Reference effects consume only eligible registered laboratory facts for work
counters, lifecycle and hostile selectors, worker/workshop eligibility, resource
quantities, population membership, construction, excavation and combat.
Assertions, replay, arbitrary derivations, changed digests and future or
unavailable inputs cannot be converted into reference-derived progress at the
consumption boundary. Unknown modeled lifecycle cannot silently remove work.
An explicit authorized same-value write re-establishes reference provenance
once; a repeated already-established write remains a no-op.

Both reference adapters apply the evidence boundary during preparation, actual
dispatch, postconditions, deferred admission and polling. MCP carried recovery
uses one evidence context and publishes its cloned frontier only after every
proof succeeds; errors interrupt unfinished stability. Objectives report
established true, established false and unknown separately. Only established
true is achieved; invalid or untrusted evidence remains unknown with its reason.
The demo and legitimate reference fixtures explicitly select laboratory scope.

The increment adds eight simulator, eight intent integration, eight adapter
authority and two objective-projection regressions. **Validation is limited: the
actual intent, adapter and laboratory production libraries compiled before the
large adapter unit-test target was killed by shared-memory pressure.** No
successful execution-suite result was received. The execution service then
disconnected. Recovered source was reapplied from checked replacement bodies
and independently reviewed through GitHub; some formatting differs from the
unavailable workspace, so compilation evidence is not asserted for these exact
final bytes. New tests remain enabled for execution on a functioning host.

The shared world primitive's earlier 178-test result is recorded separately
below. Full final-source tests, formatting and workspace qualification remain
pending. The adapter cadence increment now replaces its duplicate stability counters
with the canonical runtime, as described above. This source work is tracked by
`df-predicate-evidence-authority-r7m` and grants no live/native admission.

### Source-qualified world predicate evidence (2026-10-07)

The pure world layer now separates readable predicate values from evidence that
may support an execution decision. `PredicateEvidence` borrows one immutable,
hash-valid snapshot and binds exact source and domain grants to its full anchor.
Its default scope grants no observation authority. Observed and certified
derived grants require exact producer identifiers and nonzero digests; replay
and agent assertions cannot acquire a grant. The explicit laboratory scope
admits only the two registered reference derivations under their existing
zero-digest convention, with consistent known presence and nonfuture timestamps.

Positive entity/relation observation and complete-domain absence are separate
rights. Missing records and uncovered terrain remain unknown; relations also
require observed endpoints. Unknown leaves remain unknown through negation and
boolean composition. Canonical kind aliases have identical evidence meaning,
ambiguous duplicate kind grants are rejected, and lookup keys must match
canonical record identities. Source/policy and predicate bounds are checked
before evaluation. Profile metadata and canonical hashes alone confer no source
authenticity, completeness, or capability authority.

**Evidence: the initial world evidence implementation passed all 178 actual
locked, offline Cargo world tests, zero failed and zero ignored**, including
13 new source, digest, freshness, presence, anchor, coverage and malformed-input
tests. Review then corrected canonical kind alias handling and added two
regressions. That follow-up passed formatting, but its full 180-test run was
interrupted by an execution-service disconnection; no passing result was
received. The available compiler is `rustc 1.100.0-nightly (908501772 2026-08-30)`.
Sources recovered through GitHub after the disconnection are reviewed source,
not a claim that the final published generation passed the interrupted run.

This increment provides the shared evidence primitive. Its planning, reference
effect, deferred execution, obligation and MCP recovery integration is documented
above under `df-predicate-evidence-authority-r7m`. The observing
shell remains responsible for independent source admission and issuing grants;
the explicit laboratory constructor is not an imported-data or live-source
admission path. No native, live, registry or full-workspace qualification is
claimed.

### MCP profile queries and faithful observation rendering (2026-10-07)

The public laboratory query accepts
`{"mode":"observation","completeness_profile":"spatial","section":"chunks","limit":25}`.
All five canonical profiles are usable, with bounded pages for entities,
relations, map chunks and retained events. Responses identify the original
source anchor separately from the projected anchor and envelope digest.
Continuations preserve the source hash, profile, section and next returned
record; the existing historical router resolves that exact retained source.

The JSON rendering includes explicit field presence, including omission for
binary length summaries nested in records, events and overlays. Unavailable
retained values are withheld from entity views, briefings and change evidence;
known null remains distinct from absent or unrecorded fields. Query coverage
names its exact record-membership domain. Excluded sections and unresolved
filters cannot certify absence. Successful historical coverage follows the
queried source even while the Agent Turn orients to the current session head.

Output budgets now remove whole presentation records, preserve semantic lists,
coordinates, predicates and known null members, retain coverage, downgrade
omitted domains to partial, and repair continuation offsets to the records
actually returned. Empty, excluded and exhausted sections have no continuation.
The public query description and laboratory documentation explain the new mode.

**Evidence: the actual projection, budget, laboratory query and world-change
source modules compiled and passed all 29 unit tests, zero failures or ignored
tests**, linked against real world/intent and pinned serde dependencies without
replacement module bodies. Source hashes were stable across compilation and
execution. These 29 tests include 22 newly added MCP regressions; the ordinary
world/intent Cargo suites separately passed 290 tests. Relevant changed source
passed rustfmt. Independent review covered the historical router and facade.

This module harness does not compile or execute the complete MCP server,
facade macros, supervisor or stdio transport. A fresh full-library preflight
found about 0.73 GB of recoverable memory headroom, below the unchanged
Asupersync compiler's previously observed roughly 2.9 GB RSS before termination,
so another full build was not started. Full workspace qualification and live
admission remain unestablished. The completeness-profile bead's pure profile,
presence, round-trip and exact-base acceptance is implemented and tested.

### Typed completeness profiles and observation capsules (2026-10-07)

The world now implements all five typed completeness profiles through immutable
`ProfiledSnapshot` and `ProfiledObservationCapsule` envelopes. A projection binds
its original source anchor separately from its recomputed projected hash, with
the profile, source schema, nonzero manifest digest and opaque optional
extensions covered by the envelope identity. Excluded entity and relation
fields retain explicit `Omitted(profile)` presence; included facts retain their
presence and provenance. Research-full preserves the source canonical bytes.

Profiled capsule construction, decoding and application require the exact base,
unchanged profile/provenance/extensions, contiguous same-epoch observations and
the reconstructed target envelope identity. A strict canonical delta reader
covers all seven existing change types. Existing unprofiled snapshot and delta
v1 bytes remain unchanged. Profile envelopes are bounded to 16 MiB before
encoding and during decoding, with bounded extensions, schema names and value
nesting. Independent source verification remains distinct from envelope
integrity, observed-domain completeness and action authority.

**Evidence: the actual locked, offline Cargo world/intent suites passed 290
distinct tests, with zero failures and zero ignored tests**, including eight new
profile/codec tests. These cover all profiles and presence states, exact-base
and provenance mismatch refusal, optional-field/extension preservation, all
delta variants, malformed frames, identity forgery and encoding/decoding
bounds. The focused eight-test rerun is included in that total. The recorded
compiler is `rustc 1.100.0-nightly (908501772 2026-08-30)`; tested source hashes
were unchanged. This is targeted Rust test evidence, not full workspace,
native, live, or registry qualification. MCP presentation integration evidence
is documented separately above.

### Presence-safe temporal execution (2026-10-07)

The world exposes a shared `Fact::known_value()` read boundary and public
three-valued predicate results. Unavailable or internally inconsistent presence
does not expose a compatibility value as evidence. Ordered comparisons between
incomparable types remain unknown through negation and boolean composition,
including deferred-step admission and obligation terminal/failure evaluation.

The reference simulator now requires known, correctly typed work counters and
job tokens, plus positive building durations. Unknown remaining work cannot
become zero or completed construction. Unavailable workshop/worker evidence
keeps production blocked. Population-dependent consumption and combat reject
explicit unavailable life, squad or burrow inputs before deriving outcomes.
Older reference fixtures retain their documented missing optional defaults.
These checks apply to laboratory reference semantics; they grant no live effect
or compatibility authority.

**Evidence: the actual locked, offline Cargo suites for `dfmcp-world` and
`dfmcp-intent` passed 282 distinct tests, zero failures and zero ignored tests**,
including ten new regressions and 70 unavailable temporal-input vectors. The
available compiler was `rustc 1.100.0-nightly (908501772 2026-08-30)`; source
hashes were stable throughout the run. Focused reruns are included in that
distinct total. Relevant changed source also passed the same toolchain's
formatter. This is executable test evidence for these crates, not a full
workspace/latest-nightly qualification or native/live admission.

Source-authority certification and complete-domain witnesses remain separate
from presence and predicate truth. The canonical completeness-profile and MCP
projection work is tracked by `df-capsule-completeness-profiles-k8p`.

### Faithful restart monitoring and shared durable ownership (2026-10-07)

`RecoveredObligation` is an observation-only wrapper around the canonical
obligation runtime. It retains the original sealed specification, absolute
deadline and registration tick, while resetting unfinished stability after a
restart or observation interruption. The archived recovery frontier is a cadence
floor, not another positive sample. Fresh observations obey the normal cadence,
failure precedence and exact-deadline rules; terminal proof anchors are immutable.

The MCP recovery source uses those monitors under the acting session's current
Observe authority. Legacy terminal records without atomic world anchors and
plans or proof specifications that cannot be reconstructed remain indeterminate
and retain their durable provenance. Recovery never dispatches or authorizes a
retry. Original terminal evidence remains distinguishable from fresh proof.
New frontiers also explicitly anchor deferred steps as not dispatched without
finishing their plans. Missing or unanchored legacy nondispatch/abandonment at a
world head that differs from the sealed basis remains indeterminate; absence
of a journal step alone cannot prove that an old peer saved no effect.

Shared laboratory worlds now own unfinished durable plans, carried monitors,
pending restore abandonment and persistence faults. A peer therefore cannot
save the common world while omitting another member's outstanding progress or
bypass its failed save. Joining an existing shared world reuses it without a
second recovery or epoch bump. Failed joins remove their new member and session
without dropping the common world's outstanding progress. A read/write admission
fence serializes private reopen against complete calls and publication, including
already-resolved older session handles, and rejects mixed private/shared durable
writers. Checkpoint restore publishes the restored world and old-plan abandonment
together, retaining pending retirement for retry when publication fails.

**Evidence: the ordinary Cargo `dfmcp-intent` suite passed all 118 tests,
including nine new recovery cases**, with locked offline dependencies and the
recorded nightly toolchain. The new cases cover cadence and duplicate reads,
exact versus late deadline proof, an already-expired frontier, failure-predicate
precedence, off-cadence contradiction, repeated restart, observation interruption,
lineage regression and immutable terminal evidence. The 26 exact-source
durable-store tests described below also passed, for 144 distinct passing cases
in this increment's validation. The separately run nine-case filter is not
counted twice.

The MCP integration was reapplied through GitHub from the recorded reviewed
changes after the execution service disconnected, then reviewed as a new source
generation. It is not asserted byte-identical to the unavailable workspace copy.
The intent module and exports remain unchanged from `be2a9ff`. The MCP server
and regression cases have not compiled or executed; final-source formatting and
aggregate qualification checks also remain pending while the executor is offline.
The unchanged pinned Asupersync dependency had no usable cached build and could
not compile within the available shared memory and disk headroom before that
outage. These changes do not establish full Rust qualification or any native,
live or production admission. Tracking
remains under the laboratory storage beads `df-franken-storage-mvcc-54h.1` and
`df-franken-storage-mvcc-54h.2`, with current authority also covered by
`df-cx-authority-budget-threading-lmu`.

### Atomic laboratory world and execution recovery (2026-10-07)

The durable laboratory store now publishes the world snapshot, all submitted
step-state transitions and completed-plan retirement in one bounded, hash-chained
progress record. A torn write exposes the preceding complete frontier, rather
than a verified step paired with the world before its effect. The MCP persistence
source calls this atomic API instead of writing steps and the world separately.

Each new step transition retains the exact snapshot anchor that established it.
Compaction preserves those anchors and their snapshot objects together with the
original sealed-plan basis. Legacy step records remain readable without being
silently assigned proof anchors. Progress input is validated and canonicalized
before publication; unchanged progress is idempotent. Journal capacity is checked
before append, and compaction cannot collect an object awaiting publication.
Uncertain writes, file-sync failures and post-rename directory-sync failures fence
the open store until it is reopened, preventing an unsafe retry on an uncertain
journal or an obsolete file handle.

**Evidence: 26 passing tests against the exact durable-store source** (13 module
unit tests and 13 integration tests), compiled with real cached `dfmcp_core` and
`dfmcp_world` dependencies on nightly 1.100.0 (`908501772`). Coverage includes every
byte prefix of mixed world/progress/retirement publication, pause effects, large
frontiers, invalid input, proof-anchor preservation, corrupt objects, byte limits
and injected write/sync failures. Resource pressure killed the ordinary Cargo
test build in the unchanged adapter, so the exact module was compiled and run
in isolation. This is not a full laboratory or MCP qualification claim. The MCP
crash regressions are source present and remain unexecuted; the separate
shared-session and carried-obligation increment is described above.

Tracked as laboratory progress under `df-franken-storage-mvcc-54h.1` and
`df-franken-storage-mvcc-54h.2`; this does not complete the owned FrankenSQLite or
FrankenFS integrations, or change native, live or production admission.

### Real modern MCP Tasks for original laboratory plans (2026-10-07)

**Evidence for this increment: source/API review and formatting only for the
MCP integration. The seven-case stdio test harness was independently
metadata-type-checked, but the server was not compiled or executed in this
session.** Every full and metadata-only MCP build attempt was killed while
compiling the unchanged pinned Asupersync dependency, before reaching this
crate, as other workloads' shared-memory files exhausted the shared 8-GiB
cgroup. Low-memory compiler settings and cleanup of this run's disposable
artifacts did not resolve that environment limit. The process tests remain
enabled and must be run on a host with sufficient headroom. This paragraph is
not a Rust-qualification or passing-process-test claim.

The default laboratory `serve` entry now registers an application-owned
`FinalTaskRuntime` with `ServerBuilder::final_tasks` and runs its sanctioned
`AuthorizedTaskServiceRunner` beside stdio in the caller-owned Asupersync
lifetime. `fortress_commit(as_task=true)` retains the exact originating session
and sealed plan digest before dispatch; the supervisor enters the ordinary
authorized, idempotent commit path. The eleven tool names and the modern-only
2026-07-28 dependency profile stay unchanged.

`tasks/get` exposes the real task and its terminal result or failure evidence.
Session Agent Turns, handoffs and bounded Tasks resources discover opaque task
handles. Foreground `fortress_wait` calls advance and verify the laboratory;
task reads never advance game time or dispatch deferred actions. Completion
requires all original plan actions to verify, even after later plans are
committed. Current observation authority is checked on task reads, and current
action authority is checked again at commit and cancellation boundaries.

`tasks/cancel` drains the original plan, retaining request-phase progress and a
finalization certificate. Cancellation inspects retained receipts without an
eligibility poll, so cancelling a prepared successor cannot start its effect.
The bounded laboratory drain is proved before cancellation intent reaches the
upstream store; a separate client poll between its retained request and final
phases is not guaranteed.
Verified plans refuse cancellation; failed deadlines retain proof; unresolved
effects remain explicit and block blind retry. A transport-owned drain marks
the exported call log non-replayable until that event has a replay representation.

This first integration is limited to one active task monitor and 256 retained
task records per process. Task identities are not restart-persistent, and this
does not add live or production qualification. The implementation and focused
regressions are tracked by `df-fastmcp-conformance-5pj.3` and
`df-tasks-projection-tests-yy4`; see `docs/LAB_MCP_TASKS.md` for the wire workflow
and explicit remaining scope.

### Bounded deferred execution and immutable terminal evidence (2026-10-07)

The laboratory and in-memory dispatcher now share the same deferred-step
decision: failed, cancelled or compensated prerequisites prevent new dispatch;
an undispatched obligation fails at its own deadline even while prerequisites
are pending; a newly ready step rechecks its sealed preconditions. Missing or
indeterminate prerequisite evidence never unlocks an effect. The laboratory
also checks current scoped action authority and the action budget immediately
before a poll dispatches a deferred step. Observe authority alone can continue
verifying work already dispatched, but cannot start its successor.

Terminal laboratory receipts keep their original anchor and evidence on later
polls. Both execution paths reject completion first observed after an
obligation's deadline; a sufficient proof at the exact deadline remains eligible.
These changes implement part of `df-cx-authority-budget-threading-lmu` and the
bounded lifecycle required by `df-fastmcp-conformance-5pj.3`.

Focused deterministic Rust tests cover authority expiry/scope, changed
preconditions, prerequisite failure and cancellation, dependency chains,
deadline endpoints and terminal receipt stability. All 17 added regressions
passed. The lab/adapter/intent all-targets, all-features suite passed 1,172 tests;
the final additional lifecycle test passed in an 11-test rerun, for 1,173
distinct passing cases across those three crates. This is laboratory
development functionality; no native bridge or production admission changes.

### Every semantic action family in the deterministic laboratory (2026-10-06)

`dfmcp_intent::effects` now defines the reference meaning of every non-extension
action (dig, build, labor, work orders, stockpiles, squads, burrows, standing
orders, pause): created-entity identities derived from step idempotency keys,
default postconditions (including the new `Predicate::RegionTerrain` map
predicate), bounded default obligations along the dependency critical path,
exact inverse compensations, and deterministic game-time progress. The planner
seals these defaults. The lab `MemoryAdapter` and the in-process
`MutationDispatcher` (new `reconcile` against later observations) execute them,
and the process-local MCP laboratory exposes them through the existing eleven
tools: `scenario="starter_fortress"`, structured `entities`/`terrain` queries,
`fortress.plan(actions=...)` and `fortress.wait(max_game_ticks=...)`. See
`docs/LAB_SEMANTIC_ACTIONS.md`.

Later the same day the laboratory gained multi-step Agent Turn tracking, plan-scoped
cancellation with drain progress, a `df://session/{id}/handoff` resume packet (all
session views now resolve on the real `serve` binary), and **shared fortresses**:
several agent sessions on one world/clock with exclusive spatial leases at commit
and intent replay of plans made stale by another member. `fortress.plan(blueprint=...)`
now accepts an objective (bedroom cluster, dining hall, workshop hub, stockpile vault)
that the blueprint planner decomposes, after a bounded hazard preflight, into dig
steps plus furnishing steps (a bed per bedroom; table and chair in a dining hall)
that depend on their room's excavation.

The workspace runs **3,085 tests, 0 failed, 0 ignored**; the previously ignored
end-to-end pipeline test now dispatches a real excavation and proves it by
reconciliation. This is laboratory semantics and development execution only:
no DFHack mutation, native, live-game, registry or production evidence. The
reference progress rates are calibration, not Dwarf Fortress claims.

### Laboratory agent loop: changes, history, replay, economy, objectives (2026-10-06)

- **What changed, every turn.** Each lab session retains its last 32 exact
  canonical world versions plus every older version a live root still names
  (pending plan anchor, durable in-flight plans, checkpoints), collecting the
  rest with tombstones so a refusal says "collected" or "never observed"; every Agent Turn's `changes` reports observed world
  changes since the agent's previous anchor (entities created/removed/changed
  with field before/after, terrain per level with bounding box and transitions,
  clock and pause). `fortress.query` reads any retained version exactly
  (`"at"`) and diffs it against now (`{"mode":"changes","since":...}`).
- **Deterministic replay.** Every lab call is recorded; `df://session/{id}/replay`
  exports `dfmcp.replay.bundle/1`; `dwarf-fortress-mcp replay` re-executes it
  and names the earliest divergent call and field.
- **Economy.** The starter fortress has drink/food stocks consumed over game
  time, explicit thirsty/hungry needs, producing work orders, and
  `world_alerts` raised as attention and top recommendations with an exact
  remedy plan (laboratory calibration only).
- **Objectives.** Committed intents are tracked and their terminal conditions
  re-evaluated against observation (`objective_status`), separate from action
  dispatch state.
- **Live routing.** `dfmcp_adapter::live_routing` maps each sealed step onto the
  live development families (control/1.7, dig/1.16 rectangles, build/1.19
  furniture, work-orders/1.10, workforce/1.17) or refuses with a reason;
  `fortress.plan` returns it. Routing is pure and grants nothing; it is not
  live execution or evidence.

- **Threats.** `besieged_fortress` schedules a hostile raider fought in rounds
  by squad members or killing exposed dwarves; alerts carry a
  capability-gated squad remedy.
- **Policy evaluation.** `dwarf-fortress-mcp evaluate <scenario> <policy>
  <ticks>` scores `idle` and `follow_recommendations` deterministically through
  the eleven tools (survivors, deprivation, threats, objectives, cost).
- **Witness revalidation.** A plan made stale only by unrelated concurrent work
  (unchanged read witness) commits by identical-action intent replay with a
  deterministic certificate; otherwise the explicit replay names the first
  changed read. World changes are disclosed only to sessions holding observe.

All of this is laboratory semantics and development execution; none of it is
native, live-game, registry or production evidence.

### Laboratory control-plane depth: queries, attention, publication, faults (2026-10-06)

- **Queries.** `fortress.query` adds closed `where` filters (field/op/value,
  all/any/not; unknown facts never match), lexical `search`, and `path`
  walkability routes over observed terrain via `MapRegion` (an unreachable
  answer is certified only when the bounded search touched neither the region
  edge nor an unobserved tile). `fortress.explain` returns a Merkle inclusion
  proof verified against the world root.
- **Attention.** Every Agent Turn ranks attention under a lexicographic total
  order (severity, urgency, category, id), keeps at most six items, collects
  every applicable control condition, and certifies the selection
  (`coverage.attention_selection`, `dfmcp_world::select_top_k`).
- **Production realism.** Brewing/cooking orders progress only with a completed
  Still/Kitchen and a living worker with the BREW/COOK labor; stalled orders say
  why (`blocked_by`) and fail their obligation at the deadline. Production
  objectives compile against observed stock and refuse with every blocker named;
  supply alerts remedy the actual blocker (assign the labor) before production.
  The civilian alert FSM locks civilians down during a raid.
- **Publication and faults.** `dfmcp_world::CapsulePublisher` implements PUB-OBS
  root-last publication over the in-memory ledger (readers see only complete
  roots; recovery re-derives the root by replay). `dfmcp_lab::faults` replaces
  the chaos RNG loop with seed-pinned canonical `FaultSchedule`s driving crash/
  drop/reorder at publication boundaries and crash/torn-write at journal appends,
  with hash-chained injection transcripts.
- **Replay.** `schemas/replay.bundle.schema.json` pins `dfmcp.replay.bundle/1`;
  a checked-in golden bundle must replay with zero divergence, replayed sessions
  re-export byte-identical bundles, and every lab scenario replays a broad tool
  mix with zero divergence.
- **Adversarial and noninterference tests.** A threat corpus maps every plan
  §14.2 adversary class to a test (tainted text, closed vocabularies, forged or
  replayed digests, forged sessions, hostile sizes, checkpoint-id traversal); a
  session without observe/query learns no world fact through any channel.
- **Measured, not asserted.** `dwarf-fortress-mcp bench [iterations]` scores
  SLO-001..015 through the eleven-tool facade. Release build, 500 iterations,
  this container: p99 heartbeat 0.34 ms, delta 0.39 ms, entities query 0.33 ms,
  10-step plan 4.1 ms (SLO-001..004 pass); briefing median 1,444 tokens
  (SLO-007 pass); RSS 11 MiB (SLO-011 pass); 500 publication fault schedules
  with no divergence and 99 journal crash points recovered (SLO-008/010
  partial: below the 10,000/1,000 sample targets); mutating responses carry
  identity (SLO-014 pass). **Failing:** SLO-005 heartbeat median 473 tokens
  (target 150) and SLO-006 delta median 667 tokens (target 500) once the bench
  fortress is in a supply crisis. SLO-009/013/015 need the live corpus.
- **Semantic profiles.** Pulse turns omit the briefing, affordance catalogue
  and references (named in `coverage.omitted_by_profile`) and drop repeated
  anchors and boilerplate; briefings elide default-valued affordance fields.
- **Anchor v2.** `dfmcp_core::StateAnchorV2` is the complete version tuple with
  strict canonical bytes and a total continuity classification, exposed on
  `df://session/{id}/anchor`; v1 remains the wire anchor.
- **Concurrency exploration.** `dpor_tests` enumerates every interleaving of
  two agents on one shared fortress, groups them into Mazurkiewicz trace
  classes (Foata normal form; cross-agent plans/queries commute, commits and
  waits do not), re-runs each class representative identically and checks a
  second linearization agrees. Overlapping digs never both commit (20
  interleavings, 14 classes); disjoint digs always do (70/24); adjacent digs
  that read each other through the hazard halo are refused as stale rather
  than committed on an outdated read, so commit-time revalidation is
  serializable and no separate SSI gate is needed while commits are serialized.
- **Objectives.** Default plan summaries name each step and its arguments;
  objectives record when they were first achieved and report
  `no_longer_holds` when a later plan undoes them.
- **Fixes found by these tests.** Path endpoints overflowed `i32` arithmetic;
  plan-scope `compensate_reversible` applied the inverse of deferred steps that
  never dispatched.

Laboratory semantics and development execution only; none of it is native,
live-game, registry or production evidence.

### Crash-durable laboratory fortresses (2026-10-06)

`fortress_open_session(durable=true)` makes a laboratory fortress survive
process loss when the operator sets `DFMCP_LAB_STATE_DIR` (absolute, `0700`;
MCP callers cannot choose paths). `dfmcp_lab::durable::DurableLabStore` stores
each world state and checkpoint as its exact canonical snapshot bytes
(content-addressed, strictly decoded by the new
`WorldSnapshot::from_canonical_bytes`) named by an exclusively locked SHA-256
hash-chained journal; objects are synced before the record naming them; only a
torn final record is ever discarded, any other damage refuses the store.
Reopening resumes the last persisted world in a new observation epoch, adopts
every durable checkpoint, and fences older sessions of that fortress. Work that
lives in the world (designations, construction, orders) continues; action
handles are not carried, but obligations are: each unfinished commit is
journaled (request + sealed world) before dispatch, deterministically recompiled
on resume, accepted only if it reproduces its sealed digest, and its dispatched
steps are re-proven against observation until verified or failed. Proven in-process and by
`scripts/lab_durable_restart.py` across a real SIGKILL of the `serve` binary.
This is laboratory durability, not power-loss qualification of any live effect
journal.

### Whole-workspace Rust test execution (2026-10-06)

For the first time in recent history the complete workspace test graph built
and ran: **3,054 tests passed, 0 failed, 2 ignored** (`cargo test --workspace`,
debug profile with `CARGO_PROFILE_DEV_DEBUG=0`; full debuginfo is what drove the
earlier asupersync compiler OOMs). This executes the previously uncompiled
furniture-request, allocation-owner and completion MCP source paths' unit tests,
but not their process-level scenarios, warning-denied Clippy, rustdoc, native
DFHack, live-game or production qualification. Repairs made to get there:
stale integration tests, nine MCP servers whose `cx.io()` gate refused all work
under the native runtime, closed-query field leaks, a cross-module test session
race, and three-way merge branch validation.

### Requested furniture slots through MCP: source present, execution blocked

The furniture server now accepts `open_session(furniture_request=...)` as an
alternative to manually chosen item IDs. It validates the entire request and
operator target scope before native contact, acquires one paged operations/1.4
inventory, and creates an exact allocation-backed batch only when all slots can
be assigned. A complete shortage returns without opening a session or creating
placement custody. Explicit plan and single-selection paths remain available.

The closed `allocation` query exposes original request or item evidence, and
every normal batch handoff retains a compact allocation identity. Fresh
observe/prepare/commit checks enforce the original constraints. Reopening never
reallocates; the retained source tick initializes the new lease before the
first child exists, fixing late-game empty-batch restart. Operator Plan
revocation is rechecked before new custody and final publication. Shortage and
monitor recovery disclosures also recheck authority after serialization.

**Nine new MCP process methods are present but have not executed.** Independent
Python syntax/import and native-peer fixture checks ran, including the global
scarcity assignment, distinct protocol generations, and a 95,618-byte,
2,032-candidate capture producing 32 exact items. These are fixture checks,
not Rust behavior evidence. New code was reconstructed from reviewed successful
patch context after the executor disconnected; final formatting, byte
comparison, compilation and process tests remain outstanding.

The prior completion MCP integration's 13 process methods also remain
unexecuted. Four further attempts on 2026-09-28 died inside Asupersync before
workspace compilation; the final attempt still failed after compiler-only
trait-solver and borrow-checker memory reductions. The subsequent backend error
was `409 environment_offline`. There is no new devserver executable or full
qualification claim. See `docs/FURNITURE_HANDOFF_MCP.md` and
`docs/evidence/furniture-handoff-mcp-source.json`.

### Source-bound inventory allocation through placement: Rust source present

The new `furniture_handoff` adapter derives an exact 1..32-item furniture plan
from one complete operations/1.4 capture and the existing global allocator.
A closed Python-compatible request retains material, subtype, same-level
distance, excluded items and dependency constraints. Shortages produce no
partial executable assignment.

An immutable handoff seals the normalized request, original source/capture
digests, endpoint, software, ID horizons and selected-item evidence. New
`DFMFBD02` batch definitions retain it; legacy `DFMFBD01` bytes and native
placement journals are unchanged. Every fresh selection and retained child
audit checks the original item attributes and request constraints. Independent
operations and furniture generations remain separate.

The trusted read path binds only operations/1.4 Handshake/ReadObservation,
requires complete paging, strict decoding and verified release, and preserves
the original source clock under one foreground authority and budget.

**33 new Rust test groups are implemented but uncompiled and unexecuted.**
Independent Python request and fixture checks ran before the execution outage;
they do not establish the Rust feature's behavior. Four further MCP build
attempts on 2026-09-28 were killed in the pinned Asupersync dependency. The
executor then became disconnected (409 environment_offline), preventing the
planned adapter-only build and required final verification commands.

The reviewed source was reconstructed from successful patch context and
preserved directly through GitHub after the outage. Final formatting and
byte-for-byte comparison with the inaccessible local files remain pending.
The earlier 123 completion adapter passes are historical evidence and do not
qualify these changed adapter files. See `docs/FURNITURE_HANDOFF_RUST.md`
and `architecture/furniture_handoff_v1.json`.

### Whole-original-plan completion MCP integration: source present, build blocked

The furniture development server source now connects the tested Rust completion
adapter to the existing eleven tools: `query` modes `completion_start` and
`completion`, `observe` selection `completion`, and `cancel` scope `completion`.
It retains the full original plan and every target assessment in the Agent Turn,
records read intent before native acquisition, and checks original custody around
sample publication. No new placement permission or effect method is introduced.

Separate operator modes `completion-recover` and `completion-offline` open only
the retained monitor after original-file loss. They compare its sealed original
paths, fortress and endpoint to configuration, permit only local Query work, and
withhold verified construction claims. Local monitor verification and terminal
state are separate from original-source verification, so durable cancellation
removes only the monitor obligation while unknown original custody still cannot
prove absence of placement work.

The complete **13-scenario process suite is implemented but has not executed**.
It covers actual placements followed by shared construction observations, normal
Query-only recovery, abrupt process termination, receipt/paging/source faults,
original-file loss before and after restart, and a full 32-target capture. Syntax
and independent fixture checks passed; source reviews found and corrected the
terminal-obligation projection and a reservation that necessarily refused the
32-target case. These checks do not establish runtime behavior.

**MCP compilation and process validation remain blocked.** Three test-build
attempts and one metadata-only check were killed in pinned Asupersync 0.5.0
before reaching the changed MCP source. A measured idle shared-memory floor of
6,475,874,304 bytes persisted with zero Cargo/rustc processes under an 8 GiB limit.
Serial compilation, reduced compiler metadata and allocator tuning did not
overcome that limit. There are no passing MCP tests or executable results for
this integration, and the adapter's passing tests must not be treated as those
results. See `docs/FURNITURE_COMPLETION_MCP.md`,
`architecture/furniture_completion_mcp_v1.json` and
`docs/evidence/furniture-completion-mcp-source.json`.

### Receipt-linked whole-original-plan construction in Rust

`dfmcp_adapter::construction_plan` now validates later construction conditions
for every original furniture receipt against one shared operations capture. It
preserves Python-compatible goal/sample bytes, pins both native receipt brackets
to the original source, and requires one global stability streak across all
targets. Identity changes, source regressions, interrupted reads, paused ticks,
deadlines and observation limits have explicit outcomes.

A separate private monitor retains the full original Rust batch and both
original file/directory identities. Durable read intent precedes native contact;
sample publication rechecks complete original custody after synchronization.
Replay recomputes all transitions under one shared work allowance and restores
no publication permit. Query-authorized local cancellation remains possible
without the original placement files, while callers must withhold verified
construction claims. Existing placement bytes and effect permissions are unchanged.

All **44 new adapter tests passed** on the exact pinned nightly: 14 condition and
codec groups, 16 origin/store groups and 14 actual TCP acquisition groups. The
32-target goal/sample fixtures also match the independent Python codecs byte for
byte. All **79 existing placement, batch and paged-capture regressions also passed**
against the same compiled test executable, for **123 passing checks**. This is
focused Rust development execution, not full workspace, native
DFHack SDK, live-game or production qualification. Sampled satisfaction does not
prove current usability, continuous stability or discharge of a placement effect.
The owning bridge beads remain open. See `docs/FURNITURE_COMPLETION_RUST.md` and
`architecture/furniture_completion_rust_v1.json`; exact execution scope and source
hashes are in `docs/evidence/furniture-completion-rust.json`.

### Complete furnishing batches through the existing MCP control loop

The isolated furniture/1.19 server now imports the existing complete exact-item
plan under operator-selected private batch custody. The frozen eleven tools
expose the full original plan, every deterministic child key and outcome, the
next unblocked selection, and permanent local stopping. Each placement requires
a fresh native observation, original-key preparation and a review binding the
full batch, prepared journal head, native plan, session, lease and policy.
Only a synchronized terminal Placed prefix unlocks the next step.

Reopening the two original files performs no native bootstrap and restores no
dispatch permission. Original-key Query recovery remains available after a lost
reply or parent-file substitution. Parent loss at the final post-dispatch-intent
guard prevents the writer; parent loss after the writer preserves unresolved
identity until the original outcome is recovered. A lost parent cannot be
recreated by supplying another plan. Restarted child recovery can explicitly
omit the unavailable parent under the existing recover/offline configuration.

Complete-plan and future-row output reservation precedes creation/preparation.
Established batch sessions retain the complete parent inventory under current
Query disclosure authority, while the Agent Turn names its identity and location
without duplicating the plan. A permanent stop does not cancel construction or
erase pending effects. Native wire, child-journal format, dependency pins, empty
compatibility registry and production runner map remain unchanged.

All **32 furniture MCP Rust tests and two shared runtime-entry tests passed**,
including seven new batch boundary tests. All **nine new batch process scenarios
and seven existing single-placement process scenarios passed** against the final
compiled executable. The 32-step case uses maximum-width names, native source
strings and near-limit IDs/sequences, compares every capture, key, plan digest
and receipt against the independent Python codec, and produces a complete
36,407-byte Agent Turn. Source and binary hashes remained unchanged across the
final execution. The first process run caught premature native bootstrap on a
supplied replacement plan; the repaired path now rejects it before native contact.

This is executed development functionality with independent native peers and
private files, not full workspace, real DFHack SDK, live-game or production
qualification. `all_placed` establishes historical stage-zero registrations and
does not prove construction completion or current usability. The broader owning
bridge beads `.4` and `.5` remain open. See `docs/FURNITURE_BATCH_MCP.md` and
`docs/evidence/furniture-batch-mcp.json` for exact execution scope.

### Durable complete furnishing-plan core in Rust

`dfmcp_adapter::furniture_batch` accepts the existing closed
`dfmcp.furniture-plan/1` artifact, preserving every exact item, target and
dependency with Python-compatible normalization and digest. A complete immutable
definition binds the original native source and one original placement journal.
Deterministic child keys and a full prefix audit prevent skipped, substituted or
foreign work from advancing a plan. Only a native Placed receipt synchronized in
the terminal coordinator state unlocks another step; source clocks and native
building/job ID horizons must remain monotonic across steps.

A separate private parent store synchronizes the complete definition and at most
one permanent stop. It reuses descriptor-pinned Linux custody while preserving
the original single-placement format. Control creation needs current Query and
Plan authority; Query-authorized stop and original evidence inspection do not
create placement permission. Offline custody never writes, failed publication
fences advancement, and reopening never restores a dispatch permit.

All **24 new adapter tests and 45 existing placement regressions passed** on the
exact pinned nightly. Coverage includes all 4,096 four-node dependency graphs,
every prefix of 32 placements, original native golden bytes, source and selection
substitution, every truncated parent/stop boundary, interrupted synchronization,
and real Linux file/parent custody. This is focused Rust development execution,
not full workspace, real DFHack SDK, live-game or production qualification.
The broader owning bridge beads `.4` and `.5` remain open. See
`docs/FURNITURE_BATCH_RUST.md` and `docs/evidence/furniture-batch-rust.json`.

### Complete inventory-driven furnishing plans through MCP

The typed `furniture_allocation` query connects the Rust allocator to the existing
eleven-tool operations/1.3, paged operations/1.4 and citizen-inclusive spatial/1.8
development runtimes. Agents can request a full constrained layout and receive
the existing `dfmcp.furniture-plan/1` artifact with distinct selected items,
canonical handles, normalized request, Python-compatible digests and the exact
published source. A shortage includes the complete competing-slot witness and
no partial plan. Allocation neither rereads the bridge nor reserves or places
items; the exported plan enters the existing fresh-review furnishing workflow.

Whole-result budgeting reserves the Agent Turn and active work before rendering.
Historical/archive schemas exclude this current-supply query. All nine new
MCP Rust tests and five actual modern-stdio/TCP process scenarios pass. The
32-slot process case uses a 2,000-item capture over six verified native pages;
its complete Agent Turn is 18,630 bytes. An 8,192-byte output allowance refuses
the complete layout, then accepts a smaller query from the same capture.
Existing operations, paging and spatial-production query regressions also pass.

This is executed development Rust and MCP process evidence on the repository's
exact pinned nightly and dependency lockfile, not full warning-denied workspace,
real DFHack SDK, live-game or production qualification. The registry and production
runner map are unchanged. The broader owning bridge beads `.3` and `.4` remain
open. Usage and exact execution scope are in `docs/FURNITURE_ALLOCATION_MCP.md`
and `docs/evidence/furniture-allocation-mcp.json`.

### Global furniture selection from coherent native inventory

`dfmcp_adapter::furniture_allocation` solves a complete 1..32-target furnishing
request against up to 65,536 exact candidate items. It preserves kind, material,
subtype, same-level distance, explicit exclusions and dependency constraints.
Global minimum-distance allocation with exact lexical item-ID tie breaking avoids
greedy material starvation. A complete Hall-deficiency witness explains a
shortage; no partial executable assignment is emitted.

`dfmcp_adapter::furniture_supply` derives that supply from a sealed published
operations view under current whole-projection Query authority. It excludes
non-ground, attached, contained, building-held, nonsingleton and unestablished
candidate states, checks fortress and anchor identity, and retains the enclosing
source digest and original canonical entity generations. One work/wall allowance
covers the full inventory scan and allocation. No extra native read, reservation
or game effect occurs. See `docs/FURNITURE_ALLOCATION.md`.

All **32 focused adapter Rust tests passed**, with zero ignored, on the exact
repository-pinned nightly. Fifteen solver tests include all 4,096 small bipartite
graphs and 200 unpruned inventories against independent exhaustive oracles, the
65,536-item/32-slot bound and interruption at every guard boundary. Seventeen
projection tests cover every observed flag word, actual attachments, source and
entity reuse, enclosing spatial identity, authority and shared budgets. This is
executed Rust development evidence, not full workspace, real DFHack SDK,
live-game or production qualification. The broader owning bridge beads `.3` and
`.4` remain open. Exact inputs and execution scope are recorded in
`docs/evidence/furniture-allocation-rust.json`.

### Complete original furnishing plans linked to construction goals

`scripts/track_furniture_batch.py` connects an executed exact furnishing batch
to the existing shared-capture construction monitor. It imports the locked
original batch only when every retained DAG step has its registered canonical
Placed receipt. The full normalized plan, batch/source identity, endpoint,
ordered step mapping and original child evidence become part of a new immutable
goal and separate journal. Selecting only the successful furnishings cannot
satisfy the complete original request.

The start/sample/inspect/cancel workflow retains one fixed deadline and global
stability streak over every original target. Original batch custody is rechecked
for successful sampling and inspection, including terminal inspection. Missing
or changed original evidence cannot produce a successful completion report;
an active monitor can still be cancelled with its source explicitly unverified.
The original placement journals and selected-receipt monitor formats remain
unchanged. See `docs/FURNITURE_COMPLETION.md` for the operator workflow.

The combined checker passed 183 Python tests, including 40 new core, custody and
complete batch-to-monitor tests. The boundary workflow executed 32 reviewed
placements and monitored all original steps from a 16,345-byte dependency plan;
its complete response measured 53,320 of 65,536 bytes. Four independently weakened
implementations were rejected by assertions, including original-source loss
during rendering and after synchronization. Exact input hashes and scope are in
`docs/evidence/furniture-completion.json` and
`docs/evidence/furniture-completion-mutations.json`.

This executed increment provides Python development functionality, not Rust/MCP
integration, real DFHack/live-game qualification or production admission.
Satisfaction remains a historical sampled condition; it grants no game effect,
retry permission, effect discharge, current-usability or continuous-stability
claim. Both owning bridge beads remain open for their broader scope.

### Executable whole-plan receipt-linked construction monitoring

`scripts/construction_plan.py` defines one bounded construction goal for 1..32
original furniture/1.19 Placed receipts. Canonical ordering and unique native
building/job/item identities, placement keys and target positions bind one
fortress and furniture-plugin generation. The shared sample codec carries every
original receipt before and after one complete operations/1.4 capture.

One global stability streak requires every selected building and original item
to satisfy their receipt-linked condition in the same advancing-tick samples.
Independently timed successes cannot complete the whole plan. Repeated paused
captures, interrupted reads, changed captures at one tick, false/unknown targets
and source/identity regressions preserve conservative sampled evidence.

`scripts/construction_plan_rpc.py` acquires these samples through one foreground
connection: every original receipt is queried before and after one complete
operations capture, whole-capture digest verification and release acknowledgment.
Every receipt query must report retention sufficient for the whole selected set.
One shrinking deadline and budget cover all targets and pages; only the existing
four query-only method bindings are used. The single-receipt transport retains
its original profile while sharing the unchanged paged-capture verification.

`scripts/track_construction_plan.py` now provides the complete bounded
start/sample/inspect/cancel workflow. It imports one private JSON receipt bundle
into a separate `DFMPJR01` append-only journal, retaining the canonical receipt
set, fixed policy/endpoint and every complete shared sample. Read intent
synchronizes before native contact; complete rendered results, file/parent
synchronization and old/new-byte verification precede acknowledgment. Restart
never restores publication permission or renews the goal. Terminal replay is
offline and immutable, and original placement journals remain untouched.

The combined checker passed **108 actual Python test functions**: 59 new
whole-plan tests (17 core, 14 TCP, 15 private-store and 13 CLI) and all 49 existing
single-receipt tests. All **five weakened whole-plan implementations were
rejected** by regression assertions. The suites exercise fragmented 32-target
multi-page acquisition, every original receipt boundary, source/retention
contradictions, lost final replies, release failure, authority revocation,
subprocess restart, shared stability, interruption, locking, corruption,
publication/output failures, immutable deadlines and unchanged placement custody.
`docs/evidence/construction-plan-monitor.json` binds the exact result and all
20 input source hashes, rechecked unchanged after the run.

`docs/RECEIPT_CONSTRUCTION_PLAN.md` documents the workflow;
`architecture/construction_plan_monitor_v1.json` freezes its bounds and custody.
This is executed Python/TCP/POSIX development evidence, not Rust/MCP integration,
a real DFHack SDK build, live-fortress behavior, physical power-loss safety, full
qualification or production admission. Satisfaction remains a historical sampled
condition and never discharges or retries original placement effects. Canonical
Agent Turn anchors remain null. Both owning bridge beads remain open for their
broader scope.

### Executed furniture placement through the eleven-tool MCP surface

`dfmcp-build-placement-dev-server` now connects the native/Rust furniture path to
the frozen eleven tools. An agent can open operator-selected custody, observe an
exact item and target, prepare and review its sealed plan, commit once, query a
lost outcome, retire a preparation with Query authority, or inspect the original
journal offline. `docs/BUILD_PLACEMENT_MCP.md` contains the configuration and
complete workflow. Typed native building/job/item insertion details make placed
receipts useful for subsequent observation selection without decoding binary data.

Current host spatial leases cover both the exact item's ground position and the
target context; protected regions and checkpoint policy gate preparation and
commit. Required checkpoint policy refuses by default. Only explicit operator
disposable-fortress configuration allows this development placement path. Review
seals bind the full plan, original session/source/journal and host policy. Reopened
history never restores commit permission. Native-global uncertainty and capacity
are separate from local inventory and are checked before creating new intent.

The actual executable built on the pinned nightly and **all seven real
stdio/TCP/private-file process tests passed**. They cover modern discovery and all
eleven tool names, reviewed one-write placement and local replay, lost-reply
restart recovery, Query-only retirement after placement revocation, policy
refusals, offline immutable uncertainty, closed-query errors with visible pending
work, and a native uncertainty fence appearing before or during preflight without
stranding new local intent. The native TCP peer is an explicit joined test double
using the independent native engine corpus.

All **25 focused MCP Rust tests passed**, with zero ignored: eighteen handler and
policy groups and seven configuration/runtime groups. Two existing runtime-entry
tests also passed. The tests exercised authority expiry at an advancing native
tick, cached-evidence redaction, original-connection dispatch, review mismatch,
protected item/target and expired leases, bounded response refusal, strict query
parsing, absorbing uncertainty, and refused fallback work without effective I/O,
spawning authority or an owned blocking pool.
Exact test cases, compiler settings, binary identity and 37 source hashes are
recorded in `docs/evidence/build-placement-rust-mcp.json`.

Native I/O uses inherited Asupersync blocking work and joined results. Effective
I/O and spawning authority plus an owned blocking pool are required; the optional
asynchronous I/O facade is not mistaken for that authority. Restricted contexts
cannot create fallback workers. Current Query authorization also gates cached
failure evidence; revoked or expired reads cannot recover it through an error or
forced-release response. A native indeterminate outcome stays visibly
`indeterminate` even when its coordinator record is immutable.

Existing MCP source and fixture compilation blockers were minimally repaired to
enable executable checks. This is focused Rust/process development evidence, not
full workspace qualification. Native SDK, live fortress, verified game checkpoint,
receipt-linked construction-completion obligations and global controller fencing
remain unestablished. Canonical Agent Turn anchors stay null. The production runner
map and empty compatibility registry remain unchanged.

### Executed Rust furniture placement, durable custody and recovery

`docs/BUILD_PLACEMENT_RUST.md` documents the typed furniture/1.19 codec, fixed
native TCP client, private Rust journal and original-connection session owner.
The complete native capture, expected-after state and independent insertion proof
are checked before accepting a placed receipt. Query and replay cannot recreate
commit permission; every native attempt follows synchronized intent, preparation
and dispatch records. Immutable indeterminate history remains unresolved and
blocks new keys. Recovery can retire a preparation with Query authority after
placement grants are revoked; offline replay performs no native I/O or storage
synchronization.

All **45 new Rust tests passed**, with zero ignored: ten canonical codec groups,
sixteen real TCP/authority/deadline groups and nineteen coordinator/session/Linux
storage groups. They use all eight unchanged independent native fixtures and
exercise real private-file locking, corruption, publication failures, lost
replies, original-key recovery and non-restored dispatch permission. The adapter
production check also passed. The compiler is the repository-pinned
`nightly-2026-08-31`, rustc `1.100.0-nightly (908501772 2026-08-30)`.

The native-global unresolved and retention summary now survives the source/session
boundary. New-key preparation checks the latest summary after its preflight query
and before persisting intent. A known native fence or full retention leaves no
stranded local obligation. Four additional actual TCP tests verify these refusals,
a fence appearing during preflight, and Query-only retirement of an owned key
despite an unrelated native uncertainty fence.

Provisioning the compiler exposed and fixed four preexisting test-fixture
compilation blockers. An earlier baseline adapter run executed 718 tests: 715
passed and three existing announcement/projection fixture assertions failed.
Those unrelated assertions were preserved. Earlier uncompiled notes below
describe their original implementation increments; this executed scope does not
turn the entire repository into a qualified release. This does not establish a
real DFHack SDK build, live fortress campaign, game checkpoint, global controller
fence or production admission. The agent-facing MCP integration is documented in
the preceding section.

### Native exact-item furniture placement through the durable developer client

`docs/BUILD_PLACEMENT_NATIVE.md` documents the new isolated furniture/1.19
DFHack plugin and six fixed methods. Native reads capture the selected item,
3x3 terrain, fortress identity, clock and building/job horizons under suspension.
The writer revalidates after private allocation and invokes
`Buildings::constructWithItems` at most once. Complete expected-after state,
exact native building/job/Hauled-item/reverse links and final readback are required
for a Placed receipt. Partial writes, false returns and lost observations retain
immutable uncertainty and fence new keys. Hidden payload, zone associations,
unrepresented secondary item flags and wrong-profile authority are refused.

The native handler passed 53 groups / 27,699 assertions with GCC 13.3,
warning denial and nonrecovering UBSan against explicit SDK/protobuf doubles.
Five actual-handler capture/plan/token/prepared/placed outputs pass strict Python
canonical decoding and commitment checks; five weakened native implementations
fail their regression assertions. The updated native engine passed
23 groups / 1,133 assertions, all eight independent existing vectors and five
compiled mutation regressions. The complete Python codec/journal/RPC/CLI path
passed 52 tests. Ordinary native temperature/weight-cache flags are now eligible
while remaining part of full witnessed state. Native fixture bytes are unchanged.

This is executed C++/Python development evidence. Clang, Rust/Cargo, a real
DFHack SDK and a live game were unavailable. It does not establish actual plugin
ABI/protobuf-runtime behavior, a live mutation campaign, completed construction,
physical power-loss safety, global controller fencing, full qualification or
production admission. The production map still contains only protocol 1.0.

The final native write now repeats credential authentication after private
allocation and complete revalidation. Removal, malformed replacement and valid
credential rotation all prevent the writer. Retained uncertainty remains immutable
and fences new keys even after a replacement credential authenticates successfully.

### Executable furniture placement client and durable recovery

`docs/BUILD_PLACEMENT_CLIENT.md` describes the separate Python furniture/1.19
review/start/inventory/inspect/query/cancel workflow over the existing exact-item
bed/chair/table engine contract. Strict canonical capture and receipt validation
checks complete expected-after terrain/item state and the native insertion proof.
A fresh same-connection preparation can dispatch once only after private intent,
preparation and dispatch publications have synchronized. Reopened or queried
preparations cannot recover dispatch permission. Lost outcomes remain pending;
indeterminate native records are immutable and fence every new key in the same
private directory. Offline inspection/discovery needs no bridge or credentials.

All 51 codec, POSIX journal, loopback RPC and CLI tests pass. Tests execute actual
Python with independent native fixtures and explicit TCP peers; they do not
execute a real DFHack SDK/plugin, Rust/MCP, live game or physical power-loss
campaign. The native furniture handler is a separate implementation increment.
Placed means historical construction-job registration, never finished or usable
furniture. No production admission, checkpoint, global controller lease or other
mutation family is established. Beads `df-dfhack-bridge-plane-c-pic.4` and `.5`
remain open for their broader scope.

### Executable sampled excavation goals and restart-safe monitoring

`docs/EXCAVATION_PROGRESS.md` describes `scripts/track_excavation.py`: a standalone,
read-only Python developer workflow over the unchanged map/1.5 source. It tracks
an explicit visible FLOOR/zero-liquid/no-designation condition for one bounded
rectangle. Distinct advancing-tick samples, a fixed stability span/deadline and
maximum sample gap prevent paused repeats or interrupted observations from
manufacturing sampled goal satisfaction. Hidden/missing cells remain unknown;
source identity, map dimensions and clock regressions invalidate the goal.

Private append-only goal journals retain complete native sample bytes, source
manifests and the declared goal. Read-start intent is synced before connection;
interrupted reads reset the next streak on replay. File and parent-directory sync
precede acknowledgement. Offline inspection is read-only; cancellation stops only
this monitor. Complete bounded JSON includes an authority-free Agent Turn spine.
Terminal history is immutable and can be inspected without credentials or native
access. Corrupt/torn history is preserved and refused, never repaired or reset.

All 35 actual Python/loopback/POSIX/subprocess test groups pass, including the
unchanged 455-byte native fixture, 576 shape/liquid/designation combinations,
restart/failure/custody cases and complete output. Four deliberately weakened
implementations fail regression assertions. This is executed Python evidence,
not a Rust/MCP integration, real DFHack campaign, power-loss proof or full
qualification. A satisfied floor goal is historical sampled evidence, not mining
causality, continuous stability, safety or a cleared native-effect obligation.
Existing dig journals, dependencies, native protocols and admission are unchanged.

### Policy-guarded development mining control through MCP

`docs/DIG_CONTROL_POLICY.md` and `docs/DIG_CONTROL_MCP.md` describe the separate
`dfmcp-dig-control-dev-server`. It composes the existing native connection owner,
Rust journal and verified receipt path with actual core spatial-lease checks,
protected shared blocks and a policy-bound review seal. Observe/prepare/commit
retain the original native connection; commit has no reconnect factory or retry.
Query reconciliation and native preparation retirement retain unresolved work.
The existing Query-only recovery server remains unchanged.

New designation requires current Query/Observe/Plan/Guarded Designate authority,
a live exclusive host lease covering every shared scheduling block, protected-region
exclusion, the exact native plan digest and the consumed review seal. Policy and
runtime checks repeat after dispatch-state synchronization. The checkpoint default
is Required and refuses because this profile has no verified game-checkpoint
provider. Only explicit trusted operator selection of a disposable fortress allows
the development no-checkpoint exception; no checkpoint or rollback is fabricated.

Every response uses the shared Agent Turn, reserves complete output/final custody
reads, and keeps the pending identity visible independent of history pagination.
Async handlers use inherited runtime-owned blocking tasks and joined results.
Session release preserves all durable history and does not claim native quiescence.
Lease scope is host/journal-local, not a global controller fence. Actual game
checkpoint/restore, structural safety and excavation completion remain absent.

Three core lease, seven policy, seventeen handler and five runtime tests are
registered across the two increments; **all 32 are UNCOMPILED AND UNEXECUTED**.
The executed JSON Schema checker passes 28 valid and 102 invalid cases. It does
not execute Rust parsing, MCP routing, native I/O, storage or runtime behavior.
No real SDK, live fortress, power-loss, full qualification, dependency change,
native-wire change or production admission is established by this source.

### Owned mining sessions and query-only MCP recovery

`docs/DIG_SESSION.md` describes the session owner retaining the original native
connection across observation, preparation and commit. Commit accepts no factory;
reopen, query and replayed preparation cannot restore dispatch permission. A
verified constant-sized inventory exposes the sole pending key independent of
history pagination, and recovery work shares one shrinking allowance.

`docs/DIG_RECOVERY_MCP.md` documents the registered
`dfmcp-dig-recovery-dev-server`: existing Rust-journal bootstrap, discovery,
retained-plan/tile inspection and one-shot native query reconciliation through
the eleven-tool MCP waist. Offline is the default. Only Query grants exist; a
separate source wrapper rejects terrain acquisition and every native mutation.
No journal creation, Python-capsule migration, native cancellation or production
admission is added. Session release preserves all original obligations.

Async handlers use inherited Asupersync-owned blocking tasks and joined results,
with cancellation/configuration checks and no fallback threads. Complete Agent
Turns retain pending work even beyond page one, distinguish historical proof from
current terrain and reserve output plus final custody verification before work.
Cursor state is published only after a complete verified response is renderable.

Eighteen adapter session tests and 22 MCP/runtime tests are registered across the
two increments; **all 40 are UNCOMPILED AND UNEXECUTED**. The executed published
query-schema checker passes 24 valid and 87 invalid cases, not Rust parsing, MCP
routing/rendering, I/O or runtime execution. Native SDK, live game, full Rust
qualification and power-loss durability remain unverified. The subsequent separate
policy-guarded development control route is described above; the recovery profile
remains Query-only and cannot opt into mutation.

### Durable Rust mining coordinator and private-file recovery

`docs/DIG_RUST_COORDINATOR.md` and `architecture/dig_journal_v1.json` describe
source-bound dig/1.16 intent, dispatch, cancellation and terminal-proof journaling.
The coordinator syncs before native preparation/commit and before acknowledging
terminal evidence. Reopened, replayed and queried preparations never regain
commit permission. Unsettled work fences new keys across all selected regions in
that journal. Current authority and a mandatory supervising-runtime guard are
rechecked at native edges, including after dispatch synchronization.

`journal::private_file::open_private_dig` supplies Linux x86_64/aarch64 backing:
exact 0600 single-link files under owned 0700 directories, descriptor-pinned
no-follow opens, exclusive file locking, append-only extent checks and file plus
parent-directory synchronization. Offline replay is read-only down to storage
methods and cannot create, sync, truncate or repair files. Partial/corrupt history
fails closed. Typed get/list discovery binds current session and exact journal head.

Sixteen coordinator groups and sixteen Linux storage/runner tests are registered,
but **all 32 are UNCOMPILED AND UNEXECUTED**. The independently executed Python
framing reference rejects 7,752 single-byte corruptions and 7,748 incomplete
prefixes and checks 100 state/phase pairs; it does not execute Rust or filesystem
custody. No real SDK, live game, power-loss, full qualification or admission is
established. The subsequent owned session and query-only recovery MCP integration
are described above, along with the separate policy-guarded development control
route. Actual game-checkpoint/restore and global controller fencing remain absent.
Native protocol and Python recovery are unchanged.

### Typed Rust mining adapter for the existing dig/1.16 development profile

`docs/DIG_RUST.md` and `architecture/dig_rust_v1_16.json` describe the new
`dfmcp_adapter::dig_designation` capture/plan/effect codec and fixed six-method RPC
source. Captures retain complete bounded halo evidence with payload-free hidden
cells. Sealed plans use unchanged native digest/token domains. Designated receipts
require the exact predicted terrain, priority and shared-block scheduling witness.

The client pins one region and native software/incarnation, checks current Query,
Observe, Plan and Guarded Designate scopes, and includes whole affected map blocks
in write authorization. Only its own fresh preparation can enter one commit
attempt. Replayed/query evidence does not grant dispatch; lost or invalid replies
fence the stream without automatic reconnect. Absolute deadlines and connection
byte reservations cannot be renewed by later calls.

Twenty-two Rust groups are registered: ten codec and twelve RPC scenarios. **All
are UNCOMPILED AND UNEXECUTED** because Rust, Cargo and rustfmt are unavailable.
Independent Python reconstruction matches all four existing native fixture Git
blobs; this is not execution of Rust or the new RPC client. No real SDK, live game,
full qualification or production admission is established. The native wire and
existing Python recovery remain unchanged. The subsequent durable coordinator
and private-file source are described above, followed by owned session, recovery
MCP and policy-guarded development control integration. None is production admission.

### Selected work-order approval and progress monitoring

`docs/WORK_ORDER_PROGRESS.md` and `docs/WORK_ORDER_PROGRESS_MCP.md` describe a
separate read-only protocol-1.12 native reader, bounded Rust codec/RPC/session and
explicitly unadmitted eleven-tool development server. Complete queue validation
establishes presence for 1..32 selected IDs; flags and counters expose current
approval/activity without promoting disappearance or zero remaining to production
completion. Exact-witness waits perform one foreground read; replayed sequences
are rejected and identity/clock/horizon changes cannot manufacture progress.
Failed refresh clears cached selection. The separately published single-order
1.11 reader remains unchanged. No existing wire generation, creation
journal, production map, dependency or compatibility admission is changed.

Both GCC and Clang passed 1,358 actual-handler assertions and rejected three
separately compiled mutants using explicit SDK/protobuf doubles and UBSan.
Independent Python checked 4,800 recipe/counter/status cases, the native fixture,
malformed records and response-size models. Twenty-four Rust regression groups
are registered but UNCOMPILED AND UNEXECUTED; Rust/Cargo/rustfmt were unavailable.
No real DFHack SDK, MCP execution, live-game or full repository qualification is
claimed. Monitoring is session-local, not durable completion or downtime evidence.

### Native pause preparation fencing

`docs/NATIVE_PAUSE_FENCING.md` documents local dispatch-sequence and pause-state
revalidation, a 60-second monotonic preparation lifetime, map-incarnation resets,
and exception-safe receipt handling. Old preparations cannot override newer native
setter attempts, including no-ops and ambiguous failures. Token/receipt formats,
RPC methods, durable coordination and production admission are unchanged.

Ten grouped actual-handler C++ scenarios passed with explicit DFHack/protobuf doubles
on GCC and Clang with UBSan, and optimized GCC. A removed-gate mutant fails; the
upstream handler reproduces the stale-unpause defect. This is not Rust, real plugin,
protobuf-runtime, live-game or full-repository qualification. It does not implement
bounded simulation advancement or fence external controllers.


### Operational situation and attention in the live spatial loop

`docs/SPATIAL_SITUATION.md` describes the fixed `dfmcp.spatial-situation/1` derivation integrated into
spatial/1.8 session opening, observations and queries. Seven ordered rules summarize observed citizen,
job, building, item and visible-terrain signs without inferring causal blockers, global safety,
starvation or threat exposure. Native field provenance, coherent source digest, tick, type and
consistent known presence are required; unknown and redacted inputs remain explicitly unestablished.

Open/observe/wait and query `mode="situation"` include all signal counts and at most two detailed
attention groups with generation-checked, exact-anchor inspection requests. Ordinary tactical queries
retain one compact priority signal, omitted/unknown group counts and a situation-detail request.
Observe/wait add at most four endpoint-count changes, with explicit omissions and no cross-epoch,
continuous-history, acknowledged-client-cursor or game-effect-success claim. Situation queries acquire
no extra native capture and do not sample watches. Fenced sources and expired Query grants suppress
current findings; historical and archive-only responses do not inherit live operational attention.

Watch result-budget reservation now uses the actual final renderer's watch-specific metadata shape,
including metadata added by the post-refresh callback, rather than unrelated baseline-comparison
metadata. Full output is still checked before watch root publication or durable checkpoint append.
No native protocol, top-level tool, dependency, mutation authority or production admission changes.

Fifteen Rust scenarios are registered: nine projection tests and six actual live/archive handler
tests, including compact/detail navigation, unknown/provenance and hidden-data cases, stale links,
authority expiry, unchanged watch evidence, 8,192-byte pagination and budget-refused registration.
**None has been compiled or executed here**: Rust, Cargo and rustfmt are unavailable. Six independent
Python reference watch-packet shapes fit an 8,192-byte budget, measuring 6,553–6,590 bytes. That sizing
reference does not execute Rust serialization, rules, watch transitions, MCP or native/live DFHack.
No Rust, runtime or whole-repository qualification is established by this increment.

### Restart-safe historical endpoint comparisons

`docs/HISTORICAL_CHANGES.md` describes the new `historical_changes` query in both journal-backed
live and archive-only spatial/1.8 sessions. It compares complete bounded entity selections at two
exact record numbers/digests without creating a process-local baseline. After restart, retained
record references remain usable; continuations remain session-bound and must be restarted.

The operation shares the existing baseline selection and comparison implementation. It separates
selected-set entry, departure and semantic changes by entity ID/generation, discloses provenance-only
refreshes, and preserves unknown/absent/omitted/redacted/source distinctions. ID reuse is a separate
departure and arrival, not an update to the old identity. Cross-epoch, reversed, forked or missing
endpoints fail instead of manufacturing a change history. Before/after rows are emitted only after
both complete selections have been acquired and counted.

Both record replays, both selections, comparison and rendering share one cooperative wall-time
allowance. Full historical Agent Turn metadata, both record witnesses, pagination and current live
watches are reserved before replay. Limits are 256 selected rows/256 KiB per endpoint, 64 acquisition
page attempts/two million source-row visits per endpoint, and 1..128 whole changes per output page.
Current Query authority, cancellation and journal custody are checked before replay and again before
return. Selected record bytes are reverified. The operation neither advances current state nor
samples watches, acquires a native capture, allocates a baseline, repairs history or dispatches effects.
A live source failure still permits verified history comparison. Archive schema discovery now has
thirteen executable variants; nested historical comparisons are not permitted.

Sixteen new logical Rust scenarios are registered: nine shared-comparison tests and seven actual
live/archive handler tests covering pagination, reopen, generation/presence semantics, epoch fences,
authority, unchanged watches, output refusal and same-length record corruption. **None has been
compiled or executed here**: Rust, Cargo and rustfmt are unavailable. The independent Python request
contract checker passed 66 cases (12 accepted, 54 rejected) in explicit `--envelope-only` mode. It
validates the new record/page envelope only, not delegated entity selectors, record existence/order,
digest agreement, comparison, filesystem custody, Rust pagination, MCP or native/live behavior.
Executed script/schema bytes match their committed Git blob identities. Executed script SHA-256:
`67ef27c59d25a5f83203b505c5e80f81fb487af178288f161622029cbdec69a9`.
No dependency, native protocol, production runner, mutation authority or admission is changed.

### Offline spatial/1.8 archive bootstrap and exact-record analysis

`docs/SPATIAL_ARCHIVE_RECOVERY.md` documents `fortress.open_session(recovery_only=true)` on the
existing spatial server. It opens an existing observation journal without DFHack, bridge credentials
or an endpoint. Query and optional Doctor grants are freshly scoped to the verified archive
fortress; Observe, mutation authority, journal creation/repair and watch recovery are refused.
The operator-configured path keeps the existing exclusive lock and private-file custody rules,
using a read-only descriptor whose write, flush, sync and truncate operations explicitly refuse.

The normal fixed-profile replay reconstructs the exact citizen, operations and terrain generation
history. Empty archives cannot bootstrap a world. Requested region/acquisition bounds remain in
force. Cached queries and diagnostics recheck current authority and custody; exact historical reads
also reverify their record bytes. No archive bytes are rewritten or migrated.

Archive queries reuse the ten stateless graph, terrain, inventory and workforce analysis variants,
plus bounded history listing and exact-record historical queries. Route drill-downs stay pinned to
the selected record without replacing the session's latest retained observation. Their Agent Turns
explicitly mark all facts historical and current freshness unproved. Whole-row output includes full
historical metadata within the response budget. Archive-only schema discovery excludes watch/baseline
mutations and uses session/head-bound history continuations. No live acquisition or monitor evaluation
is routed, even with injected Observe capability. Persisted watch evidence is not loaded; empty
active work is scoped to this archive session and does not prove that no persisted monitoring or
game actions exist.

Fourteen Rust scenarios are registered: six private-file adapter integration tests and eight archive
bootstrap/actual-MCP-handler tests. **None has been compiled or executed here**: Rust, Cargo and
rustfmt are unavailable. An independent Python JSON-size calculation passed 128 boundary cases for
exact-record route wrappers; every wrapper was smaller than its original anchor-bound request
(maximum growth -22 bytes). This limited check does not execute Rust serialization, archive replay,
filesystem custody, query engines, MCP or DFHack. No full qualification, native/live evidence,
production runner, admission, dependency or game-effect capability is changed by this increment.

### Coherent workforce candidates and simultaneous capacity planning

`docs/WORKFORCE_PLANNING.md` describes `workforce_candidates` and `workforce_plan` through the actual
spatial/1.8 `fortress.query` dispatcher. Previously disconnected candidate source is now integrated
with schema discovery, exact anchors, current authority and complete Agent Turn/active-watch budgets.

The shared adapter analysis reads observed sparse skills, job availability and terrain from one
capture. The existing integral allocator gives each citizen one worker slot across all declared
roles, with deterministic rerouting and a distinct-worker shortage certificate. Requests allow
1..16 demands and at most 128 simultaneous slots. The objective is maximum filled slots, not global
skill/travel/priority optimization, native labor eligibility, reservations or actual labor changes.
Unknown skill keys do not recruit every novice. The occupied-citizen endpoint exception relaxes only
unit occupancy, never hidden terrain, walls, liquid, buildings or unknown walkability.

Whole-row continuations bind the session, full anchor, model and work allowance. Current watches
remain attached without sampling. In live mode workforce queries use the latest retained capture;
the subsequent archive-only path described above also supports exact historical workforce analysis.
No native protocol, dependency, top-level tool, mutation capability, compatibility admission or
production map is changed.

Fifteen new Rust tests are registered: eight adapter tests (including all 512 three-worker/three-role
skill graphs against an exhaustive assignment oracle), four query tests and three actual MCP-handler
tests, including 8,192-byte pages with current watches. **None has been compiled or executed here**:
Rust, Cargo and rustfmt are unavailable. The checked-in Python reference passed 88 schema cases
(25 accepted, 63 rejected) and 4,096 independent mathematical allocation-oracle cases. Its executed
SHA-256 is `ff87fdd47943ad6b7dc88ec9cec7148fb0490280cf8ff597995973c050268d04`; committed script and
schema blob identities match the tested bytes. These checks do not execute Rust, the production
allocator, terrain analysis, MCP, native DFHack or a live fortress, and establish no qualification.

### Durable foreground monitoring bound to spatial/1.8 observation history

`docs/DURABLE_WATCHES.md` describes the new restart-safe monitoring path. The existing watch queries
retain their schemas and foreground execution model; no top-level tool, native bridge method,
dependency, game effect, or production admission boundary is added.

- Operator-only `DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL` selects a separate private file paired with the
  required spatial/1.8 observation journal. Query and Observe authority, exact-mode custody and an
  exclusive file lock are required; a process-local observation universe cannot back recovery.
- Registration, changed samples, terminal outcomes, cancellation and release are persisted as bounded
  hash-chained checkpoints. Complete response rendering precedes append/sync; durable sync precedes
  in-memory root publication and acknowledgement. Failed rendering does not commit a transition.
- Startup replays/syncs the fresh observation first, then verifies the exact watch profile/archive
  identity and every stored creation/evaluation/checkpoint anchor against retained observations.
  A wrong archive or missing required observation fails closed without migration or repair.
- Recovered definitions retain their original deadlines, prior samples and evidence links, but use
  fresh session-bound handles. Unfinished stability resets; bootstrap is not counted as a sample.
  `watches` rediscovers the handles and `await_watch` obtains the fresh evidence needed to continue.
- Epoch/identity discontinuity invalidates unfinished watches. Passed deadlines expire rather than
  move forward. Current grants and game-tick horizons are not widened by historical configuration.
  Previously terminal outcomes remain historical, even at an identical bootstrap anchor.
- Repeated unchanged reads do not append or manufacture samples. Historical queries never evaluate
  current watches against past game facts. Scoped persistence metadata does not make query baselines,
  ordinary results, plans or game effects durable. Session drop releases ownership, not durable intent.
- Partial writes and uncertain sync fence publication. A complete uncertain frame may recover on
  reopen; an incomplete watch frame is refused unchanged. There is no automatic tail repair, pruning,
  rotation or compaction. Limits are eight watches/session, 1 MiB/checkpoint, 64 MiB/4,096 checkpoints.
- The actual spatial startup/query paths include recovery metadata and active work. Corrupt watch
  storage is not mislabeled as output overflow when the usual active-work error projection fails.

Twenty-five new logical Rust tests are registered: seven binary-journal, thirteen durable-watch,
two configuration and three actual spatial startup/MCP-handler scenarios. They retain the existing
watch tests and cover restart stability, cancellation/release, current authority, paired history,
output refusal, storage faults, historical outcome labeling and fixed framing vectors. **None of
these Rust tests has been compiled or executed here**: Rust, Cargo and rustfmt are unavailable.

The checked-in independent Python framing reference passed 740 checks, including 363 single-byte
corruptions and 361 incomplete prefixes. Its tested source SHA-256 is
`6171594ba703f1b95a458826edcf6acfdf0af002ebe4ce3d8324d99ab3dfac9a`.
The committed script bytes were checked against that executed source. These checks use opaque
payloads and do not execute the Rust serializer, watch state machine, filesystem custody or MCP
runtime. No Rust/Clippy/stdio, power-loss, native/live-game, full-repository or admission claim is
made. This increment supplies development source and independent framing-reference evidence only.

### Pause recovery, verified receipts and bounded foreground reconciliation

The control/1.7 runtime now implements a complete discovery-to-reconciliation source path under the
existing eleven-tool interface. See `docs/LIVE_CONTROL.md` for exact request and evidence semantics.

- `fortress.open_session(recovery_only=true)` opens an existing private journal with Query authority,
  a read-only descriptor, no bridge credentials or connection, and no creation or repair. Query,
  explain and doctor can expose stored evidence while DFHack is unavailable. Recovery sessions
  cannot promote themselves to live sessions or invoke prepare, commit or live wait.
- `fortress.query` lists durable effects without prior knowledge of their keys. Whole-record pages
  and state filters are bounded; continuations bind the session, journal incarnation, exact head,
  filter and offset. Authority and custody are rechecked even for terminal/idempotent lookups.
- Native preparation identity now survives a later commit: desired pause and expected tick are
  immutable, separate from actual observed pause and tick. Duplicate preparation after time advances
  returns the retained token. Setter failure reports the actual opposite pause state, not the target.
- New live prepare tokens and terminal receipts are independently recomputed in Rust. Both applied
  and not-applied terminal results require a complete matching receipt and consistent observation.
  A known native prepare or interrupted commit without a terminal receipt leaves a durable attempt
  indeterminate; it is never interpreted as proven non-application.
- The wire requires explicit outcome/pause/tick fields for known records, exact token/receipt lengths,
  and fences every failed wire call, including unread oversized frames. TCP connect and handshake
  share one deadline allowance.
- `fortress.wait` runs one foreground reconciliation pass over 1..16 selected keys. It validates the
  complete selection and reserves the complete worst-case response before bridge work, processes keys
  canonically, skips prepared/terminal records, and uses a query-only recovery transport interface.
  A shared wall-time allowance and stop-on-first-error preserve earlier durable progress while
  reporting remaining work as deferred. No mutating commit is dispatched or retried.
- Previously persisted terminal records are not migrated or newly qualified by the live receipt
  validator. Historical evidence does not prove current pause state. Hashes are identity checksums,
  not signatures or authority. Synchronous filesystem calls have no claimed hard cancellation bound.

The offline recovery increment registered 14 Rust tests. The receipt/bounded-pass increment adds 19
Rust tests (10 coordinator, seven wire and two response-reservation cases) and extends the existing
MCP-handler tests to refuse live wait in offline mode, including injected clock grants. **None of
these Rust tests was compiled or executed here**: Rust, Cargo and rustfmt are unavailable. No whole
repository qualification is claimed.

The changed actual native producer passed `scripts/test_live_control_outcomes_native_mock.py` under
both GCC and Clang with C++17, `-Wall -Wextra -Werror -pedantic`: 75 C++ assertions and three independent
Python SHA-256 comparisons per compiler. Tested producer SHA-256:
`80bb0c427ce94ecee41b86c52ea721e979cd15b1f371e0a4a7a27f5aa410a6ff`.
The test injects no-op setters, invalid post-set clocks and post-set exceptions, plus time-advanced
prepare replay, duplicate suppression, generation loss and authentication refusal. Independent Python
also checked the Rust test-vector constants. These are mock-interface/source checks, not a real
DFHack/generated-protobuf build, a live campaign, or native qualification. Protocol 1.7 remains
unadmitted and all production admission boundaries are unchanged.

### Coherent citizen + operations + terrain spatial/1.8 source

`docs/LIVE_SPATIAL_CITIZENS.md` describes the citizen-inclusive read profile. It removes the
cross-observation ambiguity between the old citizen profile and spatial/1.6: the strict citizen
roster and the existing operations/terrain payload are serialized during one native DFHack RPC
suspension and transferred as one immutable retained capture.

- `dfmcp_spatial_v1_8` keeps the two-method `Handshake`/`ReadObservation` waist. Its request adds an
  explicit citizen bound while preserving fixed jobs/buildings/items/terrain/page/byte bounds.
  Native retained-cache ownership binds both the requested terrain region and citizen bound.
- A strict complete citizen component is bounded to 4,096 records, sorted by nonnegative native unit
  ID, and rejects duplicates, non-citizens and residents. Observed fields include bounded visible name,
  race, profession, position, alive/sane/active/visible and developmental status. Subsequent readiness
  and skill source fields do not establish qualified labor eligibility or full health coverage.
- The combined payload is at most 16 MiB and preserves the spatial/1.6 immutable-page semantics.
  Citizens are captured before the native RPC returns; later page transfer never dereferences DF
  pointers or mixes a newer citizen roster into the retained operations/terrain bytes.
- Spatial/1.8 uses a dedicated citizen entity namespace rather than the legacy protocol-1.0 unit
  encoding, which would collide with current job IDs. All projected facts and edges are rebound to
  one spatial/1.8 source digest and one combined observation anchor/generation universe.
- A job whose `Job::getWorker` ID appears in the complete strict-citizen roster gains a
  generation-checked `worker_entity` fact and a `performs` edge from that citizen to the job. A job
  with no worker records absence. An assigned worker outside the strict-citizen roster remains
  explicitly unknown rather than becoming a fabricated placeholder unit.
- The existing route-aware inventory allocator and candidate route queries were generalized over a
  coherent spatial-state interface; spatial/1.6 behavior is preserved while spatial/1.8 can run the
  same algorithms against its larger same-anchor graph.
- `dfmcp-live-spatial-citizens-dev-server` is registered with a distinct process-scoped session
  family, its own credentials/opt-in, two-session limit, read-only Observe/Query/Doctor grants, and
  the frozen eleven top-level tool names. Convenience modes expose citizens/jobs/buildings/items/
  tiles, while ordinary graph/baseline/watch/route/allocation queries all use the combined anchor.
- A separate sealed spatial/1.8 observation codec supports append-before-publication, exact restart
  replay and stateless historical queries. The new paired watch journal is described above; neither
  archive constitutes continuous game history, a game checkpoint, or mutation authority.
- The profile still does not establish complete needs/health, labor eligibility, complete unit
  navigation, non-citizen unit details, outside-region terrain, native material requirements or any
  game effect. Pause-control/1.7 remains separate and gains no authority from a spatial observation.

Four initial Rust integration scenarios are registered for worker joins, non-citizen uncertainty,
citizen-only advancement/generation continuity and strict-roster corruption. A reproducible native
mock harness is checked in at `scripts/test_live_spatial_citizens_native_mock.py`, covering immutable
retained bytes, strict citizen limits, hidden-terrain noninterference, generation invalidation and
fixed method registration.

Those spatial/1.8 Rust/native checks have **not been executed in this editing environment**. The
container has no Rust toolchain and no network-mounted repository. No real generated DFHack/protobuf
build, live-game campaign, full repository qualification or admission is claimed. This section is
therefore evidence rung 1: source present.

### Durable spatial/1.6 history and exact-record analysis

The optional spatial journal is integrated into authenticated bootstrap and
refresh. Complete coherent captures are synced before publishing changed live
anchors, and restart replays the exact combined generation chain. Historical
queries reconstruct an independent typed spatial state, not a replacement live
world. See `docs/SPATIAL_HISTORY.md`.

- The shared journal has sealed operations/1.3, operations/1.4 and spatial/1.6
  codecs. Legacy 1.3 APIs, framing and digest domains remain unchanged. Wrong
  profiles are rejected before incomplete-tail repair; no archive migration is
  inferred. The 1.4 codec is a library API, not 1.4 MCP journal integration.
- Spatial `history` lists committed captures with bound whole-row pagination.
  `historical_query` admits only eight stateless query kinds, including candidate
  routes and route-aware inventory allocation. Allocation route drill-downs stay
  pinned to their exact archived record and digest.
- Current Query authority is required; old captures do not revive expired grants.
  Current watches stay current and are not evaluated against archived facts.
  Acquisition/replay and response budgets remain separate. Native source failure
  still permits verified archive reads in an already-open session.
- Paths and explicit incomplete-tail repair are operator-only settings. Failed
  writes/syncs fence publication; complete corrupt frames are never silently
  discarded. Default retention is 64 MiB/1,024 changed captures with no pruning.
- No offline bootstrap, durable watches/baselines, automatic rotation, effect
  journal, anti-rollback floor, native bridge change, dependency change, or
  production admission is introduced by the spatial/1.6 history increment.
  The subsequent spatial/1.8 durable-watch path is separate and described above.

Fourteen new Rust scenarios are registered: eight journal/profile and six Unix
actual-handler scenarios. They have not run because Rust, Cargo and rustfmt were
unavailable. No Rust compilation, Clippy, stdio, filesystem crash campaign, live
DFHack or repository qualification is claimed. JSON Schema meta-validation and
50 new-wrapper/gate component checks passed; full composed schema validation did
not run. Sixteen independent JSON-size checks passed for archived route wrappers.
Those checks are not execution of the Rust implementation.

### Pause-control/1.7 durable development effect boundary

`docs/LIVE_CONTROL.md` describes the first bridge-backed live mutation source. The implementation is
strictly scoped to simulation pause/resume and remains unadmitted development functionality.

- The native bridge exposes only `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause` under
  fixed protocol 1.7 identities. It does not expose a generic command, Lua, keyboard, path, address,
  method selector, or any other action family.
- Prepare binds one idempotency key, 32-byte sealed plan digest, desired pause state, expected game
  tick, and bridge generation. It performs no game mutation. The 16-byte prepare token is derived
  from a fixed SHA-256 domain and that complete identity.
- A private durable coordinator journal is mandatory. It uses an exclusive locked exact-mode `0600`
  file under a canonical exact-mode `0700` directory, a hash-chained transition sequence, bounded
  retention, file sync, and explicit incomplete-tail repair. Complete corrupt records and broken
  predecessor chains are rejected.
- The coordinator syncs `Prepared` after nonmutating bridge prepare. Before any `CommitPause` RPC it
  appends and syncs `CommitStarted`. If this durability boundary fails, no game mutation is called.
  The runtime never retries a mutating commit automatically.
- After one commit dispatch, a complete bridge outcome includes observed pause state and a
  generation-bound full 32-byte SHA-256 receipt. The coordinator verifies that identity and must sync
  `VerifiedApplied` or `VerifiedNotApplied` before acknowledging a terminal result. Missing or invalid
  terminal evidence remains unresolved; failed terminal durability is `EffectIndeterminate`.
- Rust-process restart replays the exact durable transition chain. `CommitStarted` and
  `Indeterminate` remain reconciliation-required. Read-only reconciliation may reconnect to the
  bridge, but commit is not redispatched as recovery work.
- If the bridge retains a complete matching receipt, reconciliation records its result. A merely
  known key, missing receipt, generation loss or unknown key cannot prove a terminal outcome. The same
  effect is never reported safe to retry; a new observation and new plan/idempotency key are required.
  A merely `Prepared` effect may commit once only while its bridge generation still agrees.
- World load/unload advances bridge generation and clears native retained records, preventing native
  idempotency state from crossing a world boundary. The generation is also bound into tokens and
  receipts.
- The safe-Rust client binds only the four fixed methods and profile identity. The development MCP
  runtime retains one isolated control/recovery session family, grants only `ControlClock` at
  reversible risk in live mode or `Query` in offline mode, preserves the eleven top-level tool names,
  and refuses every non-pause mutation surface.
- Agent Turn metadata explicitly keeps `runtime_admitted=false` and `mutation_admissible=false`; the
  unadmitted development effect switch is represented separately. Protocol 1.7 remains absent from
  the production runner map and the compatibility registry remains empty.

The earlier native source was compile-checked in the editing environment against explicit mock
DFHack/protobuf interfaces with both GCC and Clang under C++17 and warning-denied flags. A local
state-machine mirror exercised prepare replay, one-shot commit, duplicate suppression, query
reconciliation, generation reset, and fixed method registration. Independent Python `hashlib`
calculations matched the generation-bound token and receipt identities after an embedded-NUL domain
separator bug was corrected. `scripts/test_live_control_native_mock.py` is checked in as the
reproducible baseline mock-native harness; current outcome-fix evidence is recorded above.

These checks are not native qualification. The Rust durable journal/client/MCP runtime have not been
compiled or executed because no Rust toolchain was available. No real generated DFHack/protobuf
build, filesystem power-loss campaign, disposable-fort control campaign, registry promotion,
deployment-floor advancement, server-artifact qualification, production runner, or admitted live
mutation capability is established. The source now contains restart-safe coordination semantics;
those semantics still require exact Rust/native/live qualification before broader effects or
production admission are considered.

### Coherent spatial/1.6 capture and route-aware inventory allocation

The spatial profile captures jobs, buildings, items, attachments and one bounded terrain region in
one native suspension, then transfers immutable serialized pages. `docs/LIVE_SPATIAL.md` contains the
full contract.

- Composite decoding requires matching world/site/clock/pause/software/generation identities.
- One source digest, observation cursor and entity-generation universe cover operations and terrain.
- Bounded terrain reachability feeds the existing integral allocator without double-counting stacks.
- `spatial_inventory_plan` returns same-anchor candidate routes, supply handles, exclusion reasons,
  and a flow/min-cut shortage certificate under the declared stack-unit model.
- The registered spatial development server preserves eleven tools, shared query/watch behavior and
  strict read-only mutation refusal.

Mock-native validation previously passed 786 GCC and 786 Clang checks, including a 40,000-item,
2,200,457-byte immutable capture across 135 pages. Twenty-three Rust scenarios remain registered but
unexecuted in this editing environment. Spatial/1.6 remains unadmitted.

### Bounded map/1.5 terrain observation and candidate routes

The map profile observes one fixed bounded terrain region, preserving hidden cells as redacted and
unallocated blocks as unknown. Deterministic dry cardinal floor/stair candidate routes are available
through the existing query surface. Routes are not unit-specific navigation or global reachability
proofs. The profile remains unadmitted development source.

### Immutable-paged operations/1.4

Operations/1.4 captures one coherent jobs/buildings/items state and transfers immutable bytes through
bounded pages with fixed token/generation/limit identity and whole-payload digest verification. It
remains unadmitted development source and does not inherit 1.3 qualification or archive identity.

### Durable operations/1.3 observation history

Operations/1.3 can optionally persist canonical changed observations before publication, replay the
exact generation chain on restart, list retained history, and execute stateless historical queries.
This is an observation archive, not durable effect-journal recovery, anti-rollback custody, or a
production MVCC claim.

### Query, graph, monitoring and production analysis

Public query continuations are snapshot/query bound. Structured entity inspection, aggregates,
search, graph traversal, SCC/dependency diagnosis, baseline changes, foreground condition watches,
production diagnosis, integral inventory allocation and spatial inventory planning are implemented
as described in their dedicated documentation. Optional spatial/1.8 watch persistence is now present;
query baselines remain process-local. These derived layers never grant mutation authority.

## Current registry and qualification state

The checked-in compatibility registry remains:

```json
{
  "schema_version": "dfmcp.live-compatibility-registry/1",
  "status": "no_admitted_live_tuples",
  "entries": []
}
```

No fresh full latest-nightly qualification receipt is checked in for the current head.

Consequences:

- no Dwarf Fortress/DFHack/plugin/source/protocol/platform tuple is currently admitted;
- the production launcher cannot authorize any newly added development profile;
- protocols 1.1 through 1.8 remain outside the production runner map;
- old or external receipts do not qualify this current source generation.

## Area matrix

| Area | Present now | Not yet established |
|---|---|---|
| Agent surface | Agent Turn envelope, eleven-tool waist, structured queries, restart-safe spatial/1.8 watches, production/spatial analysis, bounded durable control-effect discovery and recovery passes | complete durable handoff, durable baselines, complete counterfactual/VOI models |
| Protocol 1.0 | authenticated citizen read stack and production-runner source | current R1-R5 receipts and registry entry |
| Protocol 1.1 | retained announcements and development runtime | current native/live admission chain |
| Jobs/operations/map/spatial | coherent bounded development reads through citizen-inclusive spatial/1.8, same-anchor strict-citizen worker joins, exact spatial/1.8 history and paired durable watches | Rust qualification, real DFHack campaigns, complete needs/health/labor coverage, full unit navigation, production admission |
| Control/1.7 | pause prepare/commit, receipt-verified live reconciliation, bounded wait, mandatory private journal and offline Query-only recovery | Rust qualification, real DFHack build, crash/disposable-fort campaigns, production admission, any other live effect family |
| World | canonical snapshots, deltas, query/graph/path/allocation, operations history and durable spatial/1.6 and spatial/1.8 observation replay | admitted production durable backend and complete fortress coverage |
| Intent/effects | sealed plans, in-memory dispatcher laboratory, bridge-backed pause effect with durable pre-dispatch/terminal coordinator states | qualified/admitted effect journal, leases/checkpoints tied to live commits, dig/build/labor/etc. live effects |
| Security/admission | closed dependencies, protocol-bound tickets, monotonic floor machinery | admitted current tuple, hostile-host resistance, signed release provenance |

## Explicitly absent

- no current admitted live tuple;
- no supported production compatibility claim for protocols 1.1 through 1.8;
- no admitted live mutation capability;
- no live dig, construction, labor, burrow, stockpile, work-order, military, checkpoint, Lua,
  arbitrary command, keyboard, filesystem, or network effect;
- no Rust-qualified/native-qualified/live-qualified control effect journal or power-loss evidence;
- no Rust-qualified durable-watch implementation, continuous downtime monitoring, or automatic watch-journal compaction/repair;
- no proof that the current head passed every Rust qualification gate;
- no signed cross-platform release provenance.

## Next executable milestones

1. Run full Rust verification/qualification for the exact current clean head, including spatial/1.8
   citizen coherence, paired durable watches and their actual handler tests, the durable pause-effect
   journal, receipt-verified coordinator, control wire, bounded wait and all registered recovery tests.
2. Compile spatial/1.8 and control/1.7 against named real DFHack/protobuf generations and execute
   disposable-fort read/control campaigns for the exact plugin bytes.
3. Exercise spatial/1.8 with real citizen/job churn, non-citizen workers, large rosters, hidden terrain,
   immutable multi-page transfers, route/allocation analysis and foreground watches, including paired
   journal restart/failure cases, before treating its cross-domain joins as live-qualified evidence.
4. Execute control host/bridge failure campaigns at every durability boundary: before
   `CommitStarted` sync, after sync/before dispatch, after dispatch/before reply, after reply/before
   terminal sync, Rust restart with live bridge, DFHack restart, world load/unload, and incomplete/
   corrupt journal tails.
5. Only after those exact semantics are qualified should a control/1.7 admission proposal or another
   narrowly versioned mutation family be considered. Keep the production map unchanged until exact
   evidence supports widening it.

## Status rules

1. Source presence is not qualification, admission, support, or production evidence.
2. Development execution is not production admission.
3. A tuple is admitted only while its exact entry exists in the current registry generation.
4. A deployment is admitted only when its trusted floor matches that registry generation.
5. A protocol executes in production only when the V2 production map contains its reviewed runner.
6. A doctor report is diagnosis, never authority.
7. A server receipt qualifies one executable, not a bridge session or game state.
8. A single-use ticket authorizes one exact process/protocol start and does not grant unstated effects.
9. Negative evidence may reject a claim but cannot certify success.
10. Derived indexes, recommendations, history and path models never grant authority.
11. Unit tests never substitute for disposable-fort evidence where Dwarf Fortress behavior matters.

## Reality-check addendum (2026-09-22, `reality-check-for-project` audit pass)

An external agent pass executed the comprehensive reality check against this repository
(`docs/reality-checks/2026-09-22-phase-0c-reality-check.md`) and adds this dated, evidence-labeled
addendum rather than editing the phase table above:

- [FACT] The workspace now contains an authenticated read-only live plane beyond the phase-0C
  laboratory slice described earlier in this file: `dfmcp-mcp::run_live_stdio` (`serve-live`),
  `bridge_r0_authenticated_read_only` implementation phase, token + nonce bridge credentials,
  loopback endpoint parsing, single-use inode-bound admission tickets
  (`DFMCP_ADMISSION_TICKET`, 300 s lifetime), live capsule projection, and a DFHack-side plugin
  exposing `DFhackCExport RPCService *plugin_rpcconnect` (V1 and V1.1 wire codecs). The
  eleven-tool waist is preserved; mutation-stage tools fail closed.
- [FACT] The full workspace (7 crates, ~164k src LOC) materialized on the reference machine via
  the remote build-tree sync on 2026-09-22 and is coherent after one additive repair
  (`dfmcp-core::Digest32::new` constructor alias for adapter-revision call sites).
- [TARGET/UNVERIFIED] Whether this live plane passes the full workspace gates on the reference
  machine is recorded by the qualification receipts, not by this addendum. No receipt existed at
  addendum time; the run was in flight.
- [FACT] The bead database materialized from an unflushed frankensqlite WAL on first write
  (123 beads hidden behind `br stats` = 0); visibility/regression guards now run in
  `scripts/verify.sh` via `scripts/check_bead_coverage.py`.
- [FACT] Upstream fastmcp conformance defects DRAFT-A/B were reproduced with byte captures by an
  external stdio harness; DRAFT-C is a documentation question (era-refusal `supported` list is
  pinned intended); DRAFT-D is a facade API gap (`UriParams` not re-exported). See
  `docs/DOGFOODING_FASTMCP.md`.
