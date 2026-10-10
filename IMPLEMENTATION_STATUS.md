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
source when reconstructing history. MCP recovery and shared-session wiring are
a subsequent increment. The format and crash tests use actual snapshots,
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
retry. Historical objective custody beyond completed-plan retirement remains a
separate gap at this increment.

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
