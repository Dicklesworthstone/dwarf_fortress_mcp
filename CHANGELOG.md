# Changelog

## Unreleased — Discover exact retained history for laboratory replay

- Add bounded history discovery to fortress_query, including full anchors,
  stable recording positions, newest/oldest order and exact continuation pages.
- Expose collected recording holes separately from original observation cursor
  gaps, and suggest current-epoch replay windows of at most 32 consecutive
  retained versions. The replay operation still validates all world sources.
- Bind cursors to the complete catalog, current anchor, owner and order; refuse
  stale tokens after observation or collection changes. Fit complete rows and
  their next cursor together, without a lossy history-success response.
- Keep discovery metadata-only and read-only, including its response rendering;
  preserve game work, retained query handles and the cached observed anchor.
  Add owning-session handoff discovery and portable-replay cursor refusals.
- Add retention, helper and public workflow regression sources. Execution and
  Rust formatting remain blocked by the dependency-fetch disk exhaustion and
  subsequent executor disconnection.

## Unreleased — Foreground waiting for multiple laboratory conditions

- Add optional watch_ids (one to eight unique owned handles) and watch_policy
  (all_satisfied or any_satisfied) to the existing fortress_wait tool, preserving
  the scalar watch, selected-plan and ordinary wait APIs.
- Keep every selected watch's original cadence, deadline and stable-sample
  count; evaluate and publish one complete candidate set at each frontier.
  Failure and unknown outcomes take precedence over group success.
- Return full retained records with historical/current anchors and aggregate
  sampling work. Group completion does not establish simultaneous current truth.
- Preserve pending game work, baselines and unselected watches. Admit current
  authority, shared source/wall allowances and the complete output packet before
  changing roots; retain actual elapsed progress on a later refusal.
- Add thirteen regression sources for public workflows, boundaries and atomic
  refusal. Tests and formatting remain unexecuted: dependency fetch exhausted
  disk space and the executor disconnected before project compilation.

## Unreleased — Recover oversized reviews in an existing session

- Add explicit `session_id` plus `max_output_tokens` negotiation through the
  existing opening tool, preserving the original Rust opening API and eleven
  tool names. Other opening arguments are rejected for this alternative.
- Preserve world, pending seals, original goals, action/query work, grant scopes
  and expiry, and every non-output allowance. Current owner/Observe/Query and
  combined world/retained-work admission precede a complete acknowledgement;
  refusal leaves budget and semantic request sequence unchanged.
- Read authoritative response limits outside the presentation cache. Order
  budget hints by semantic request ID without regressing newer world hints,
  and compare accepted previous/new allowances during portable replay.
- Pass 11 owner invariants, four semantic facade/replay cases and the public
  two-bedroom recovery workflow: default review refusal, explicit 8192-token
  allowance, exact complete review, commit and original room completion in the
  same private session, with harvested inputs and original proof preserved.
  All three existing portable replay compatibility cases also pass.
- Execution uses actual semantic implementations on current nightly with locked
  dependencies. Transport registration and an installed TaskService remain
  outside the isolated component evidence; full qualification stays open.

## Unreleased — Replay conditions across retained laboratory history

- Add `condition_replay` to the existing query tool, evaluating an exact
  retained interval through the real laboratory watch state machine.
- Preserve cadence, stable samples, unknowns, failure precedence, generation
  fences and first terminal evidence. Charge and validate every selected source,
  including observations after a terminal outcome.
- Refuse collected interiors, unretained endpoints and cross-epoch history;
  expose canonical cursor gaps without reconstructing missing observations.
- Preserve current authority, pending game work, baselines and watches. Read-only
  syntax refusals cannot publish durable progress or retain a new world version.
- Pass all 30 focused historical/watch/retention tests, including all 17 new
  regressions and three actual public facade workflows. The public labor-history
  regression caught and fixed omitted current baseline/watch handles; historical
  results now retain that catalog under the original shared deadline.
- Owned-source formatting and schema vectors pass. Original-source execution
  uses the latest nightly and exact locked dependencies, excluding transport
  startup/macros and an installed TaskService. The full pinned build stopped
  when unchanged Asupersync received SIGKILL. No full-runtime or live admission
  claim is added.

## Unreleased — Authorize future project stages before simulation accounting

Complete-project forecasts now check each newly compiled stage's exact effect
capabilities, risk, entity/map scope, expiry at its predicted anchor, and required
checkpoint grant before admitting that stage or forking its simulation. The
ordinary adapter still performs its own preparation and execution checks.

Previously a denied later stage could consume aggregate action/stage/fork
allowances and appear in the simulated-stage catalog before refusal. A tight
shared action allowance could consequently hide the actionable capability
failure behind `budget_exceeded`. The forecast now reports `capability_denied`
with only the earlier admitted stages and their real simulation costs. The
original adapter, source request and prepared plan remain unchanged.

Validation: all 12 original-source complete-project forecast tests pass,
including the formerly failing authority case and new missing, insufficient-risk,
expired and entity-scoped grant cases under tight aggregate action limits.
Existing staged room/production, reserve-frontier, quiescence and horizon cases
also pass. All 32 compiler-reported thin-component source/manifest/lock inputs
remain hash-stable; the run uses the complete cached core/world/intent/adapter/lab
crates and exact locked serialization dependencies on rustc 1.100.0-nightly
(f7d782a3b, 2026-08-19). This selected-source execution excludes the public facade,
transport and full current-head release/native qualification. Owning bead
df-lab-project-forecast-p7q remains in progress.

## Unreleased — Preserve full query sources through bounded live responses

Shared structured queries now carry independent source and response allowances
through the protocol-1.1, jobs, operations, map and spatial live handlers.
Previously each handler replaced the authorized source-byte limit with the
space left after its Agent Turn. Compound aggregates and projected entity
queries could therefore reject a small count or page solely because the
already-published source was larger than the answer.

The shared publisher preserves the original source context for entity,
spatial, relationship, ranked, aggregate, search and event executors and applies
the enclosing response reservation separately. Active-watch metadata remains
reserved and included. Source/record limits, current grants, cancellation,
whole-row continuations and atomic aggregate publication are unchanged;
output reservations cannot expand either allowance. Stateful watch and
baseline lifecycle operations retain their existing atomic output context.

Regression sources exercise large unselected input with compact compound
counts and pages, continuation identity, active-watch evidence and source,
record, output, authority and cancellation refusal before publication.
Independent source review found no authority or publication-boundary widening.
A zero remaining response allowance now returns `budget_exceeded` after current
authority validation and before any publication callback.

Validation passed 116/116 original-source query component tests, including two
direct source/output regressions. The merged MCP assembly then passed the real
spatial-handler regression and 16/16 shared-publisher executions: eight distinct
cases in both laboratory and spatial runtime instances. Forty-record pages and
compound counts fit complete 8 KiB responses, keep their anchored continuation,
and perform no bridge read; lowering the real source allowance still refuses.

All 76 query-run inputs and all 603 frozen MCP component files stayed unchanged
during their respective runs. All 11 external versions, sources and archive
checksums match the repository lock. The MCP assembly uses actual semantic
crates and production function bodies while excluding transport registration,
an installed Task service and unrelated test modules. These runs used rustc
1.100.0-nightly (f7d782a3b, 2026-08-19); they do not establish full current-head
workspace, native-game, latest-nightly release qualification or compatibility
admission. Owning bead df-lab-structured-query-f7q remains in progress.

## Unreleased — Wait for a retained laboratory condition

- Add optional watch_id to the existing fortress_wait tool, preserving plain and
  selected-plan callers and rejecting simultaneous plan/watch selection.
- Sample each published foreground frontier at the original cadence/deadline;
  stop on decisive or unknown evidence without rewriting the condition.
- Preserve concurrent game work, pending reviews and query catalogs. Admit the
  complete response before watch publication; retain elapsed time and historical
  evidence when later progression or publication fails.
- Add current-authority wait affordances and exact-selector replay refusal for
  process-local handles, plus public-facade and owner regression sources.
- Independently review recovered source through GitHub after executor loss.
  New Rust tests, compilation and final formatting could not be run; runtime
  and release qualification remain open.

## Unreleased — Read retained announcements through typed event queries

- Add explicit events source=announcements with original report-time ordering,
  typed fields, bounded windows and source-specific continuation pages.
- Attach exact published native retention/gap evidence through whole-AgentTurn
  page fitting; preserve unknown subjects/fields and tainted report text.
- Keep canonical events, default cursor identity, native projections/hashes and
  production admission unchanged.
- Pass 42 actual-source query/publisher/watch regressions before executor loss,
  including source admission before capsule validation and one shared deadline.
  Recover and independently review the feature through GitHub; final recovered
  code could not be rerun or reformatted after the environment became unavailable.

## Unreleased — Wait for a selected original laboratory plan

- Add optional plan_digest to the existing fortress_wait tool, preserving its
  name, legacy Rust API and ordinary duration-based behavior when omitted.
- Stop at original project completion or an actionable incomplete outcome;
  keep stage proof, physical work and exact reserve evidence distinct.
- Preserve older-plan custody, peer work, current authority, one shared deadline,
  cross-call exact reserve snapshots, replay and bounded response evidence.
- Complete 100 actual-source component tests and 529 original internal-crate
  tests before executor loss. Recover and independently review remaining edits
  through GitHub; final recovered bytes could not be rebuilt. Full MCP transport,
  generated wire-definition execution and release qualification remain open.

## Unreleased — Respect deferred pauses in plan forecasts

- Stop forecast game time when a deferred action pauses the fortress, matching
  the actual foreground wait owner and preventing invented stability samples.
- Preserve valid immediate-pause completion, same-round authorized resume and
  exact reserve observations; later reserve evidence stays unavailable while paused.
- Add three real planner/adapter regressions, including direct foreground parity.
  Formatting and source review pass. The 421 core/world/intent tests pass on the
  current locked workspace. All three new forecast regressions subsequently pass
  in the 100-test actual-source component described above.

## Unreleased — Compare complete resource-backed production and room projects

- Add explicit version2 complete_project comparisons that follow real modeled
  harvest yields through separately compiled production or room stages.
- Preserve the complete original semantic goal, stock-ledger identity/generation
  and exact reserve frontier; later stock cannot repair missed reserve evidence.
- Rank aggregate stage risk, checkpoint burden and actions under shared bounded
  work. Keep version1 behavior and fresh current-stage-only selection.
- Add eleven core, six comparison and one public-facade regression sources;
  decoded schema checks pass9accepted/53rejected cases. No new projectRust tests
  ran: full and component builds were interrupted by dependency/adapter SIGKILL
  and executor loss. Latest reserve refinements remain source-reviewed only.

## Unreleased — Query exact totals for related entities inside a region

- Compose optional positive relationship and spatial selectors with aggregate
  source filters, grouping and exact integer/fixed metrics.
- Count each selected source entity once; keep membership uncertainty separate
  from unavailable metric samples and avoid complete-world absence claims.
- Admit complete sources and combined work before selection, preserve unchanged
  no-selector/ranked/related behavior, and refuse partial aggregate publication.
- Add ten pure and three public-facade regressions plus closed schema/docs.
  All52 focused actual-source query tests pass with eleven exact locked registry
  dependencies. Full MCP/public-facade execution remains blocked by unchanged
  Asupersync SIGKILL and is not inferred from that run.

## Unreleased — Stop a selected original plan without stopping other projects

- Add exact plan:<digest> cancellation with complete session-owned original
  action custody, all-action current authority and aggregate budget preflight.
- Preserve terminal proof and unrelated plans; bind requested original identity
  into measured drain certificates and disclose witness-rebased aliases.
- Offer concrete current selected stops through read-only typed Explain, bound
  to its complete observation pin and source budget.
- Add eight real-adapter/public cancellation regression sources and operational
  documentation. Syntax/format/source and eleven-tool checks pass; the full
  locked MCP check was SIGKILLed in unchanged Asupersync before MCP compilation.
  New Rust test execution and broad qualification remain open.

## Unreleased — Explain current original-goal blockers without changing proof

- Extend the existing Explain selector with exact plan:<digest> and goal:<digest>
  reads for current requirements, original project/reserve truth, historical
  achievement, retained receipts and physical work.
- Return bounded follow-up calls pinned to the complete selected evidence;
  reject same-anchor history/receipt changes instead of mixing pages.
- Preserve the immutable current-authority read path and existing eleven tools.
- Add nine regression tests and finalize formatting of recovered changes.
- Verification remains limited to syntax formatting, source/integrity review
  and Git blob checks. The exact-source focused build was killed in unchanged
  dfmcp-world before tests ran; a later metadata check could not start after
  executor disconnection. No runtime qualification is inferred.

## Unreleased — Continue underfunded ordinary reserves without renewing their horizon

- Add isolated /6 continuation sources for original consumption_reserve_v1
  pursuits whose current minima hold but whose remaining reserve is underfunded.
- Preserve historical /3 eligibility and exact original request, stock ledger,
  permissions, lineage and absolute tick through recovery.
- Report production_reserve demand separately, retain unfinished goals at
  capacity and abandon old pursuits on restore.
- Wire complete bounded review publication and repeated current-source checks
  into existing Plan/Commit paths; add six pure and eight facade regressions.
- Parse/format recovered source and tests with nightly Rustfmt. Cargo execution
  remains blocked before compilation by dependency checkout/disk-space failures.

## Unreleased — Compact complete plan proof without dropping requirements

- Add a lossless, self-describing predicate table fallback for oversized
  resource-room, production and reserve-continuation reviews.
- Require an exact inverse, preserve literal data and sealed identity, and
  expand accepted proof into standalone later-turn orientation.
- Preserve complete current work and safe refusal before pending-plan changes.
- Add codec, publication, cache and four-variant public room regression sources.
  These tests are unexecuted after loss of the execution environment; default
  4,096-token room fit and full-runtime qualification remain open.

## Unreleased — Read exact original goals through bounded resource pages

- Expose retained original requests, requirements and historical custody through
  capability-checked manifests and source-pinned, lossless pages.
- Fit larger retained histories by moving only named historical artifacts
  behind per-goal detail links, preserving complete new proof and current truth.
- Preserve concurrent shared-proof/current-anchor work and correct reference
  counts when historical fields move out of a packet.
- Add six goal-resource cases and six publication/history regressions; retain
  incoming tests. MCP execution remains unestablished after dependency memory
  kills and loss of the execution environment.
- Complete ordinary-LLVM intent/lab execution: 544 tests passed, including all
  seven new canonical-kind cases. Recover reviewed source through GitHub with
  explicit formatting and full-qualification limits.

## Unreleased — Fit complete future-reserve reviews within the normal budget

- Share exactly equal current anchors in declared work/history fields with
  explicit same-response metadata; retain complete evidence and distinct history.
- Publish the representative complete reserve continuation in 15,919 bytes
  under the unchanged 4,096-token ceiling, including its original request,
  reserve tick, lineage, current work and full action proof.
- Measure complete two-bedroom projects with existing workshops within 8,192
  tokens and add public lifecycle/refusal regressions. Larger setup and default
  4,096-token furnished-room reviews remain explicit bounds.
- Pass seventeen focused proof, anchor, publication and orientation cases;
  preserve the documented full-runtime build and execution limits.

## Unreleased — Preserve complete resource plan reviews across turns

- Share repeated readable predicates in explicit production/room `/2` response
  envelopes, retaining every step and proof with bounded same-response references.
- Preserve exact typed archives and source/plan identities; reject incomplete,
  mismatched, cyclic and excessively expanding proofs before publication.
- Refresh pending production/room hints and cache expanded proofs for later
  Observe/Commit; retain stale room fresh-review instructions instead of an old
  commit affordance.
- Pass thirteen focused proof/publication/orientation regressions and execute
  actual two-stage room compilers and MemoryAdapter work. Record remaining
  default-budget overflows and the blocked full-runtime build explicitly.

## Unreleased — Keep identical source identities from simulating different worlds

- Refuse four reserved canonical entity-kind aliases before physical progress,
  preserving consumption, workforce, production, construction and combat rules.
- Reuse the bounded source scan and retain canonical bytes and valid custom kinds.
- Reproduce five failures on the original code; pass all 397 actual intent tests
  with the fix. Add real adapter regressions for atomic refusal and normal progress.
- Record the separate adapter-build memory limit without claiming unexecuted
  lab, MCP, release or live qualification.

## Unreleased — Keep whole-project truth when observations are compacted

- Retain exact stage scope, current room/output evidence and outstanding future
  reserves alongside historical achievement in compact Agent Turn briefings.
- Preserve unknown evidence, quiescence, continuation eligibility and explicit
  next calls; mark source detail omitted without inventing missing claims.
- Pass all 18 actual output-budget module tests, including four new lifecycle
  regressions; full runtime and native qualification remain separate.

## Unreleased — Read original plan actions and proof requirements in pages

- Discover readable step pages directly from retained-plan catalogs/manifests.
- Reuse the existing step renderer with exact plan/object identity, original
  anchors and at most 25 intact rows per page, clipped to complete wire budgets.
- Keep readable projections distinct from the full typed archive and current
  evidence; expose omitted contract fields and the authoritative archive link.
- Add two server regressions and pass all eight focused actual-consumer groups,
  including full paged reconstruction and current capability/source budgets.

## Unreleased — Refuse unfounded reserve work and unrelated compensation

- Require source-qualified remaining output demand before admitting harvest
  for currently satisfied production minima at an unchanged future tick.
- Reject ambiguous unit aliases, incomplete census/stock/metabolism evidence,
  ledger ambiguity and arithmetic overflow instead of inventing a funded reserve.
- Keep harvest compensation absent and labor compensation confined to its exact
  inverse, with ordinary capabilities and mandatory action proof preserved.
- Preserve published plan/source identities, zero minima and normalization.
- Pass all 247 actual intent tests and the selected compiler alias regression;
  record the independently discovered public packet-budget failures explicitly.

## Unreleased — Exercise finite-input stair rooms in one built-in world

- Add `starter_fortress_multilevel_resources`, retaining finite sources and
  worker assignments while supplying the five-level terrain needed for safe
  cross-level room access. Preserve previous scenario snapshots and indices.
- Exercise ordinary twenty-door production to consume starting wood, harvest
  exactly two finite logs, then furnish two lower bedrooms with paired stairs
  while preserving the original doors and conserving source quantities.
- Execute the actual source compiler and comparison consumer, including
  same-room acquisition/funded forecasts and rejection of different rooms or
  access domains sharing the same WOOD target. Correct the public test fixture's
  typed stock assignment.
- Pass 246 selected Rust tests with source-item and locked-dependency audits;
  this includes all 15 comparison tests. Public handler/facade execution and
  full qualification remain unestablished. Broader static verification was
  stopped during native-component contract tests after earlier gates passed.

## Unreleased — Build complete rooms through observed finite-input stages

- Add explicit finite-resource acquisition to provisioned rooms and both
  single-level and paired-stair connected rooms, retaining the full project.
- Preflight geometry, actual workshop sites, staffing, access and deferred
  action bounds before proposing harvest; manufacture and build only from
  observed stock in a separately reviewed stage.
- Preserve the original room, permissions and parent/root custody in additive
  continuation `/2`, including partial harvest and installed-furniture recovery.
- Separate complete semantic room truth from exact harvest/room stage proof;
  retain unfinished project custody and block duplicate same-root work.
- Require complete authority/source/publication budgets and fresh review after
  source drift. Bind comparisons to the same complete room request and label
  predictions as current-stage evidence.
- Pass 224 selected real-module Rust tests, including all 17 new compiler and
  continuation cases, with exact source and locked dependency audits. Validate
  17 schema vectors and scoped formatting. Nine public lifecycle/comparison
  tests are authored but unexecuted; full MCP and release qualification remain
  unestablished after the earlier dependency build memory kills.
- Preserve historical compiler bytes, the eleven-tool surface and current
  native/live admission boundaries. Track `df-lab-resource-harvest-h2m.5`.

## Unreleased — Read exact original plans after their first response

- Add session-bound retained-plan catalogs, manifests and bounded lossless
  archive pages through the existing MCP resource template.
- Bind both semantic and typed-object digests, preserve original anchors, and
  reject stale catalog pages or missing exact custody without fallback.
- Require current Observe and Query grants, enforce complete source and wire
  budgets, and keep reads free of dispatch or proof/history side effects.
- Add nine server regressions and execute seven focused consumer groups with
  the actual resource/codec modules, including typed-hash alias protection.
- Document the exact scope: focused execution does not establish full MCP
  transport, durable-owner plumbing or release qualification.

## Unreleased — Gather finite inputs for explicit future production reserves

- Combine `acquire_resources` and `reserve_until_tick` under the new fixed
  `resource_reserve_v1` compiler, preserving older request generations.
- Budget consumption through the later of the absolute reserve horizon and
  complete gathering plus production work; keep actual stage deadlines bounded
  to their own work.
- Compile only the current stage, using observed inputs before any production.
  Continue through the flat `/5` source with unchanged quotas, horizon,
  permissions, ledger generation and lineage.
- Keep future reserve work discoverable when present minima already hold.
  Protect unfinished or unknown reserve goals from capacity eviction, and
  reject expired horizons and active same-root work before effects.
- Preserve complete review rows and source binding in workflow report `/2`,
  compacting only duplicated or derivable metadata in the new generation.
- Add compiler, codec, public lifecycle and core admission regressions, the
  request schema and the laboratory contract/usage guide. Selected execution
  passed 111 production/continuation tests and eight core admission tests,
  including all 19 new compiler/codec/core cases. Production source identity
  and exact locked external dependencies were checked before and after.
- Latest-nightly rustfmt and 12 schema vectors passed. The seven new public
  facade tests remain unexecuted: both full test compilation and metadata-only
  type checking were killed in `asupersync`. No full workspace, release,
  native or live qualification is claimed.
- Track implementation and remaining execution evidence in
  `df-lab-resource-harvest-h2m.4`.

## Unreleased — Bound reserve acquisition admission separately from production

- Add a distinct laboratory-only typed admission for finite harvest and its
  relevant labor setup when original production stock minima currently hold.
- Keep the previous production-only reserve policy, mandatory harvest proof,
  finite sources, action deadlines and ordinary authority requirements intact.
- Add five real-planner regression groups for positive admission and refusal
  boundaries. Execution remains pending after the build host disconnected;
  no full Rust, native or live qualification is claimed.
- Track the public finite-input future-reserve workflow in
  `df-lab-resource-harvest-h2m.4`.

## Unreleased — Reach production goals through observed resource stages

- Add opt-in `acquire_resources` to the existing production blueprint, pinning
  the distinct `resource_aware_v1` compiler and the original output quotas.
- Derive finite WOOD/PLANT acquisition from actual recipe demand, pending input
  debt, deduplicated workshop costs and population consumption through both
  acquisition and production horizons.
- Review and commit harvesting first when inputs are missing; compile later
  production only from observed yield through an explicit `continue_goal`.
- Retain original policy and parent/root custody in flat production continuation
  generation 4, including canceled-work proof and same-root exclusion.
- Bind output truth to the original stock ledger ID and generation before any
  effect; reconstruct that binding from the sealed snapshot on durable reopen
  so replacement counts cannot create success, achievement or continuation.
- Admit current complete-source budgets and publish the entire review before
  replacing pending work; require a fresh explicit review after source drift.
- Fit complete acquisition and production-continuation reviews in the default
  response budget by consolidating duplicate analysis, retaining original
  request, staffing, allocations, step proof and lineage.
- Exercise the full public workflow, delayed population consumption, recovery,
  cancellation and historical source compatibility in 102 selected regressions;
  record full-build and qualification limits separately in implementation status.
- Preserve historical production generations and the existing native boundary.
  Future `reserve_until_tick` requests remain a separate unsupported combination.

## Unreleased — Continue original resource acquisition goals

- Extend `continue_goal` to original WOOD/PLANT stock targets, retaining the
  exact fixed request, labor permission and parent/root lineage.
- Recompute only observed residual stock after complete same-root work is quiet;
  refuse active or unknown siblings and suppress already-satisfied acquisitions.
- Keep canceled original task proof unchanged when a child meets the stock goal.
- Reuse complete atomic source/response publication and ordinary effect authority;
  return public parent-based continuation guidance after source drift.
- Persist the flat source through the additive `resource_acquisition_continuation`
  archive kind, with strict nested parsing and retirement/reopen coverage.

## Unreleased — Plan resource acquisition through the existing MCP loop

- Add the strict, versioned `resource_acquisition` blueprint for absolute WOOD
  and PLANT stock minima, with explicit optional labor setup.
- Publish original targets, current deficits, exact source allocations, worker
  choices, ordinary proof obligations and a separately labeled discarded forecast.
- Admit current source/authority budgets and the complete Agent Turn before
  changing a pending plan; preserve reviewed work on refusal or no-op requests.
- Keep complete action proof once per resource proposal, with a checked catalog
  reference, so a two-resource review fits the default response allowance.
- Support candidate comparison, selection and durable original-source replay;
  require a fresh reviewed allocation when the source anchor changes, retaining
  that exact next step through later observations and query/comparison catalogs.
- Add public workflow regressions for authority, cancellation and residual work,
  original identity, and acquisition of observed inputs for furniture production.

## Unreleased — Compile original finite resource stock goals

- Add bounded WOOD/PLANT target compilation from qualified observed stock and
  complete finite harvest sources, with deterministic multiple-source allocation.
- Propose labor enables only under explicit policy, preserving known source
  guards, ordinary dependencies and complete shared-worker queue deadlines.
- Keep original stock targets and mandatory physical harvest proofs separate;
  refuse incomplete supply, active overlapping work and unbounded candidates.
- Add conservation, provenance, workforce, action-bound, original-identity,
  no-work and partition-equivalence regression coverage.

## Unreleased — Rank one population selected by facts, relationships and location

- Add optional point/cuboid spatial selectors to ranked queries, usable alone
  or together with independent positive relationship and source-fact filters.
- Filter before top-K selection, preserve unknown geometry separately from
  outside-region records, and keep relationship targets independent of location.
- Retain mandatory geometry, numeric and relation evidence through whole-packet
  fitting; bind all selectors and cursor eligibility to one immutable source.
- Reuse standalone geometry semantics without changing existing spatial or
  absent-selector ranked identities; charge the full bounded source and cursor.
- Add combined leading-exclusion, geometry-oracle, stage-unknown, forged-cursor,
  provenance and output-budget regressions, plus executable combined examples.

## Unreleased — Recommend finite acquisition for missing ingredients

- Turn qualified FOOD/DRINK input shortages into bounded PLANT harvest
  proposals using exact sources, current supply, pending debt and enabled labor.
- Retain the ordinary Plan/Designate/Checkpoint, scope, expiry and budget checks
  before exposing an actionable recommendation.
- Wait for active original harvests and deduplicate identical food/drink advice
  without hiding either hazard or crediting unfinished yield.
- Add source-selection, unavailable-evidence, authority and acquisition-to-brew
  regression source; keep unexecuted public coverage explicit in status.

## Unreleased — Acquire finite laboratory resources

- Add guarded `harvest_resource` actions for exact WOOD and PLANT sources.
- Share finite workers and causal service with existing jobs; conserve source
  units, inventory, partial work and completed yield across cancellation.
- Bind original source and ledger identity through planning, proof, archives
  and recovery; reject unsupported native routing.
- Expose the opt-in resource scenario, strict request grammar, source/task
  progress and separately labeled forecasts through the existing tool set.
- Add resource-to-food/furniture, scheduling, conservation, request and recovery
  regressions. Exact executed evidence is tracked in `IMPLEMENTATION_STATUS.md`.

## Unreleased — Combine relationship selection with exact numeric ranking

- Add optional positive relationship selectors to `ranked_entities`, including
  independent typed target filters, canonical target IDs and all three directions.
- Reuse one bounded relation matcher; keep whole-source work admission and the
  existing bounded ranking heap, with explicit index-cost accounting.
- Preserve exact edge/endpoint witnesses alongside ranked facts through complete
  laboratory page fitting; distinguish unknown endpoints from unobserved matches.
- Bind all selectors into continuation identity while preserving existing
  unfiltered ranked and standalone relationship query behavior.
- Add exhaustive graph/ranking, unavailable-fact, cursor, authority and packet
  regressions; update the query schema and executable usage guides.

## Unreleased — Ground needs and threat advice in current evidence

- Qualify laboratory stock, population, need, worker and threat facts; retain
  known hazards while exposing incomplete totals and unknown arrival times.
- Fix false labor remedies for missing workshops, select known disabled
  workers and avoid production suggestions for an already met stock target.
- Check proposed remedies against actual current risk, scopes and checkpoint
  authority before recommendations or handoff; preserve unavailable diagnostics.
- Retain derived/unknown confidence and bounded source evidence through pulse
  and compact output, with distinct hostile/burrow attention identities.
- Add focused helper and public regression workflows; exact execution and
  remaining qualification limits are recorded in implementation status.

## Unreleased — Preserve original deadlines in laboratory predictions

- Schedule discarded-fork observations at each sealed action deadline and
  final stability window while respecting eligible cadence and deferred dispatch.
- Keep real deadline misses failed and count normal/reserve advances against
  one hard simulation-slice limit, returning unavailable evidence on exhaustion.
- Add planner/adapter and public comparison regressions; retain source purity,
  original timing and the existing eleven-tool interface.

## Unreleased — Fit selected event, spatial, relation and ranked pages

- Extend complete Agent Turn page fitting to the four remaining structured
  page families through the existing query tool.
- Render only selected fields, preserve whole rows and exact continuations,
  and refuse an oversized first row without silently skipping it.
- Keep source work separate from output, retain current catalogs and authority,
  and add helper/public regressions for source-pinned continuation and fitting.
- Record the exact executed validation scope in implementation status; broader
  transport, native and release qualification remain separate.

## Unreleased — Compose stock sizing with an explicit labor enable

- Add optional `assign_labor=true` to the existing native stock-target request.
  Calculate the whole finite deficit from one qualified census, then retain
  separately reviewed labor and order steps through the existing executor.
- Preserve the default existing-worker identity and no-work semantics; met
  targets cause no labor or order effect, and positive assignment plans require
  current ordinary capabilities and a two-action allowance.
- Seal and recover the complete assignment under `Stock target/2`, retaining
  exact census/queue checks before dependent insertion and original goal timing.
- Expose assignment intent and actual order StepId through bounded Agent Turns
  and recovery; update request guidance and both machine descriptions.
- Add six core regressions and a closed public request test, then two real
  Runtime/native-peer campaigns with eleven assignment scenarios and three
  bounded-presentation regressions. Cover original-key recovery, drift after
  labor assignment, met-target no-op, capability/action-budget refusal and
  compact/overflow retention without restored proof or confirmation.
- Verify uploaded-source identity and the complete published tree. Compilation,
  rustfmt and test execution remain pending because the execution service
  disconnected after disk/memory exhaustion; neither mandatory qualification
  script could start, and no live or full-qualification claim is made.

## Unreleased — Compare laboratory alternatives and select exact sources

- Add bounded two-to-four candidate comparisons through the existing `fortress.plan`
  blueprint field. Require the same original objective/reserve frontier and report
  real compiler/planner forecasts, explicit unavailable evidence and deterministic rankings.
- Share source, fork, action, simulated-tick, poll and wall-time bounds across the
  complete request while preserving ordinary single-plan forecast behavior.
- Bind selection to the exact source anchor and normalized candidate, prepare a
  fresh ordinary plan, and accept the entire response before replacing pending state.
  Preserve current plans, query catalogs and nonfinal recovered work without resampling.
- Add schema and usage examples, 13 comparison tests, six bounded-forecast tests,
  and nine public workflow tests. Correct canonical decimal fortress-ID selection,
  propagate remaining wall time and qualify retained recovery coverage.
- Validate the final integration: 64/64 selected Rust tests, six valid/36 invalid
  schema cases, the documented example and formatting/integrity checks. Exact-source
  laboratory execution does not establish transport, native or full release qualification.

## Unreleased — Expose laboratory watch workflows and stateless preflight

- Add closed watch registration, listing, single/coherent batch polling,
  cancellation and release to the existing temporal query wrapper.
- Add current/exact-history condition preflight using the same source-qualified
  evaluator, without allocating a watch or counting a stability sample.
- Publish complete candidate watch/baseline catalogs together with pending game
  work before retained state changes; preserve unknowns, grants and epoch fences.
- Recover query handles through observations and handoffs without sampling;
  keep all reported catalog entries intact under response fitting.
- Bind laboratory watch handles to a private process incarnation so restarted
  session/serial counters cannot alias stale handles; preserve native durable
  identity bytes and same-key registration replay.
- Add shared typed grammar, two schemas, usage guidance, six helper, two renderer,
  six public watch, four public preflight and one output-fitting regression.
  Add two projected-page catalog/interleaving regressions; preserve the concurrent
  complete-page fitter and retained work through late refusals. Consumed
  structured cursors are explicitly session-bound in replay logs.
- Pass all62 selected public tests and54 component tests at the feature
  checkpoint, plus9 refresh cases. The incarnation follow-up passes all152
  native/laboratory watch tests and reruns all6 public watch flows successfully.
  Final source checking passes; transport/full-qualification limits are explicit.

## Unreleased — Fit laboratory entity pages through the complete Agent Turn

- Choose whole entity pages against the exact laboratory wrapper, captured
  active work, coverage and Agent Turn. Automatically reduce page width while
  keeping each continuation aligned with the rows actually returned.
- Capture presentation once outside session locks, recheck current authority,
  anchors and budgets before executing, and publish successful orientation only
  after the final packet fits. Refusals retain the prior observation basis,
  including late deadline/output refusals and minimal budget responses.
- Bound source structure before canonical serialization, while preserving the
  exact negotiated canonical-byte allowance and admitting combined source/scan
  work before selection. Output-token limits remain separate from source costs.
- Add six public-facade regressions, four focused projected-query cases and
  two deterministic late-refusal regressions; update the old expected-refusal
  test for automatic whole-page selection. Focused helper and renderer/finalizer
  suites pass 63/63 and 35/35. Full MCP execution remains blocked in the unchanged
  Asupersync dependency, as detailed in implementation status.

## Unreleased — Add laboratory evidence to the foreground watch engine

- Reuse the bounded predicate evaluator, cadence/stability state machine and
  atomic candidate store with an immutable laboratory evidence policy.
- Support registered laboratory fields, projected population counts, pause/tick
  predicates and Boolean/failure conditions; keep unsupported native evidence
  families and native durability outside this path.
- Add coherent current-snapshot batch polling and complete retained/active work
  catalogs, with publication/authority/deadline checks before any root changes.
- Preserve native identity/evidence/checkpoint bytes and pass all 138 native
  watch regressions. Record precise new-suite/public-path evidence in status.
- Repair shared anchor serialization across module contexts and execute the
  previously blocked comparison, baseline, response and query-budget tests.

## Unreleased — Retain selected laboratory query baselines

- Add capture, catalog, changes and release through the existing query tool,
  using bounded session-owned roots, exact retry keys and game-time expiry.
- Publish complete results, retained catalog and current pending game work
  before root changes; reject overflow or elapsed admission without hidden state.
- Preserve source budgets independently of output; reclaim roots on session
  disposal and expose restore invalidation. Mark process-local handles and
  consumed comparison cursors nonreplayable instead of reporting false divergence.
- Add 26 baseline helper/renderer/public-facade regressions and schema/usage docs.
  Schema/static checks and subsequent helper/renderer component execution pass.
  All11 public baseline flows now pass; precise execution scope is recorded in status.

## Unreleased — Page selected entity fields within the complete output budget

- Borrow canonical candidates and render only selected fields, so large
  unselected values no longer consume the projected response allowance.
- Fit complete rows, result metadata and exact continuations together. Never
  skip an oversized row or issue an empty advancing cursor; handle the smaller
  terminal envelope when its continuation disappears.
- Preserve existing cursor bytes and all ordering rules. Keep full source work
  admission separate from live active-work/output reservations, with a pure
  complete-packet fitting hook for enclosing presentation layers.
- Pass 16 actual world-query tests and 42 exact-source query component tests.
  Clock presentation validation also passes all 29 component cases.

## Unreleased — Combine lexical search with typed entity selection

- Add optional `where` predicates to structured search, sharing the existing
  entity/aggregate filter language and three-valued fact evaluation.
- Search only True selections; report False and Unknown exclusions separately.
  Missing, withheld and conflicting facts stay Unknown under negation.
- Bind filters to ranked continuation identity and charge predicate work before
  exclusions, pagination or empty kind selections can reduce the visible result.
- Preserve unfiltered response shape and ordering. Add seven component and two
  public-facade regressions, request schema and usable query documentation.

## Unreleased — Recover original clock workflows after review or source loss

- Replace ineffective Tree clock waits with authorized local retirement for
  unattempted intervals and inspection for terminal or unreconcilable outcomes.
- Offer clock confirmation only for a current, unfenced Prepared review; retain
  original-key reconciliation and authorized safety stop for dispatched runs.
- Recognize Workshop's real `intent_recorded` journal state, including recovery
  without retirement authority. Preserve original custody and goal evidence.
- Add five Tree regressions, correct the Workshop state fixtures, and exercise
  the matching static and mutation gates. Rust execution is recorded in status.

## Unreleased — Compare exact retained laboratory selections

- Add `query_changes` to the existing laboratory query tool: compare complete
  entity selections across exact retained endpoints, including field and
  generation changes, filtered membership changes and unchanged heartbeats.
- Bind pagination to both sources, selection, session and current authority;
  charge complete sources together and keep historical/current anchors distinct.
- Preserve whole comparison rows and cursors through Agent Turn publication;
  refuse overflow without exposing an unpublished comparison summary.
- Add request schema, usage guidance and focused helper/public-facade regression
  coverage. All13 comparison helpers and6 public workflows now pass; precise
  execution scope is recorded in status.

## Unreleased — Complete original Tree harvesting in one bounded MCP session

- Expose optional `run_tree` planning and separate clock-scope commit, Wait and
  cancellation through the existing eleven Tree tools. Explicit tick allowance
  defaults to zero; each interval inherits the original Tree deadline.
- Own exact Tree and Run custody together, require fresh durably acknowledged
  Tree guards after native control markers, and sample original supply after
  clock settlement. Pending or uncertain runs block competing goal samples.
- Admit complete remaining Tree capacity and final publication work before
  control. Preserve consumed floors, source fences, separate confirmations,
  safety stop and both recovery references through errors, overflow and close.
- Pass 15 focused host tests, including 13 actual Runtime/TCP scenarios plus
  recovery phases, all 424 host component regressions and actual MCP library/test
  typechecking. Live-game and whole-workspace qualification remain separate.

## Unreleased — Bind bounded game time to the original Tree goal

- Add a Tree-specific guarded clock adapter over the existing Run/1.13 owner.
  Bind the exact acknowledged Tree original and receipt, and inherit its existing
  deadline instead of allowing a replacement horizon.
- Verify original custody from private bytes and independent ACK; admit complete
  sample/frame/byte capacity before each interval and recheck fresh original
  root/felling evidence after durable Run Intent and DispatchStarted.
- Preserve separately scoped authority, one-attempt unpause, query-only recovery
  and independently authorized safety stop. Reject progress older than the
  consumed clock floor before it can establish Tree supply.
- Actual repository Tree tests pass 43/43, including 29 new clock, custody and
  native TCP cases. Same-session MCP integration follows as a separate increment.

## Unreleased — Plan and execute native stock targets through MCP

- Add a reachable `kind="stock_target"` request to the combined production
  server. Read a fresh complete census and derive one finite furniture order
  from the requested minimum using already-enabled, eligible workers.
- Return an explicit already-satisfied no-work result; refuse an oversized
  deficit or conflicting production instead of overordering or silently capping.
- Bind stock sizing to the original archive, deadline, worker predicate and
  optional Workshop prerequisite. Recheck the exact census and a fresh empty
  native manager queue at preparation and after the durable dispatch marker.
- Preserve the original amount and target through goal sampling, lost-reply
  reconciliation and Offline recovery. Expose sizing and blockers in the
  existing Agent Turn, without restoring confirmation or current proof.
- Add compiler, identity, public routing, budget and publication regressions,
  plus actual Runtime/TCP scenarios for planning, dispatch races and recovery.
  Evidence and current qualification limits are recorded in Implementation Status.

## Unreleased — Qualified native stock-target sizing

- Derive one bounded furniture deficit from a complete independently qualified
  original operations census, preserving a typed no-work result when the target
  already holds and refusing oversized or competing-production proposals.
- Seal the original count, target, limit and source lineage for exact-census
  revalidation; never resize a retained amount or credit future output.
- Bound all generated inventory-projection edges before publication. Actual
  latest-nightly adapter tests pass 18 new stock-target and six existing
  inventory-baseline cases. Combined MCP integration follows separately.

## Unreleased — Complete workshops in one bounded MCP session

- Integrate the original Carpenter clock into the existing eleven-tool Workshop
  server. Explicit tick allowance, independent permission and a separate exact
  review are required before each finite interval; default allowance is zero.
- Reconcile each run once, then acquire fresh original-workshop evidence after
  settlement. Preserve immutable source/horizon/custody and the original sample
  cap; unknown run outcomes cannot authorize more time or prove construction.
- Add clock recovery/offline inspection and optional attachment to an existing
  acknowledged workshop without restoring construction permission. Preserve
  hazards on failure/overflow/close and keep useful retirement/blocker guidance.
- Fence fresh advancement after failed source contacts while retaining authorized
  safety stop. Reject Workshop completion older than the independently consumed
  clock floor before it reaches durable goal evidence.
- Add 7 configuration, 12 presentation, one real RPC/journal regression and
  10 actual-server stdio scenarios. Their execution and the interrupted or killed
  qualification attempts are recorded explicitly in Implementation Status.

## Unreleased — Use completed original workshops in production

- Add an optional closed `workshop_prerequisite` to the combined production
  owner. Import the exact acknowledged original Carpenter workshop Offline and
  require a fresh complete native read before planning and at every original
  order preparation and post-dispatch-marker boundary, including portfolios and
  existing-labor plans.
- Bind the immutable origin into the complete plan identity and private
  `original.workshop` association. Original recovery checks the marked plan,
  exact donor, sidecar and external association digest together; dropping or
  substituting configuration cannot remove the original requirement.
- Preserve consumed tick, generation, allocator and fence state in caller-held
  recovery floors, including a witnessed checkpoint before any original exists.
  Consume decoded handshake generations before later budget or authority
  refusals. Query and Offline need no Workshop credential or native contact.
  Fresh-floor rollback beyond the immutable donor seed remains an explicit
  external-custody limit, not a local durable acknowledgement claim.
- Separate configured read-only custody reservations (768 MiB) from native
  dispatch reservations (2 GiB), so one bounded Wait can reconcile an uncertain
  original, initialize history and sample its goal within the unchanged public
  allowance. Preserve legacy custody and native frontier bounds.
- Preserve the concurrent Workshop clock implementation and pass all 393 tests
  again in the merged exact-source host component, including two actual
  Runtime/native TCP campaigns for fresh prerequisites, refusal boundaries,
  checkpoint restoration, token-free Offline custody and lost-reply original
  reconciliation without redispatch. The actual repository Workshop adapter
  target passes all 70 tests: 17 origin regressions, 20 concurrent clock cases
  and 33 existing cases. Static Workshop checks cover 56 source files and all
  nine inventory/refusal tests pass. The full verification script remains
  blocked by missing `br`; these passes do not qualify MCP transport, real
  DFHack, a live fortress or production admission.

## Unreleased — Add durable clock ownership for the original workshop

- Bind independently reviewed finite Run/1.13 intervals to the exact original
  Carpenter placement and a separately frozen absolute deadline.
- Recheck fresh original InProgress evidence after synchronized intent and
  dispatch markers, preserving separate named-fortress/NIL authority, source
  floors, one-use review, complete history and query-only recovery.
- Add 20 adapter regression cases. The actual 53-case workshop suite passed
  before the final zero-identity and pending-sampling refinements; the frozen
  rerun was killed during compilation. Static source checks and all eight
  inventory negatives pass; no final-source or live qualification is claimed.

## Unreleased — Hand off an original workshop to production

- Add a read-only adapter import of exact independently acknowledged Workshop
  custody. Qualify the original Carpenter building with a fresh progress read
  between custody checks; historical completion cannot authorize later effects.
- Keep the original descriptor immutable and preserve consumed tick, generation,
  allocator and identity-loss fences. Add one-way withdrawal for connection
  uncertainty and monotone caller-held recovery floors without donor writes.
- Expose independently accepted source pins and bounded private association
  storage for the importing host. Combined production host integration remains
  separate; no new tool, native protocol, canonical workshop fact or admission.
- Execute all sixteen new adapter regressions, the 53-source static contract and
  nine inventory checks. The preceding 48-case workshop run passed; the final
  49-case run passed 44 and failed five unchanged TCP fixtures, one of which
  passed separately afterward. Preserve that limitation in Implementation Status.

## Unreleased — Complete bounded production inspection before planning

- Expand both captured-production inspection paths to twelve pure query kinds,
  adding search, retained events, spatial, related and ranked entity queries.
  Keep exact public anchors, conservative coverage and source-bound pagination.
- Accept `include_operations` as an alias for `include_production`; reject
  duplicate aliases, preserve workforce choice identity and carry the capture
  option through fresh observation pages.
- Prepay the entire optional 160 MiB capture before native contact, bound the
  complete projection to 4096 entities and 8192 edges, and reject insufficient
  native entity budgets before credentials or network access. Retain accepted
  canonical session/cursor identity across source withdrawal and failed Observe.
- Prove retained inspection and independent growth planning with the actual
  Runtime/native TCP peer: Query keeps stock 7 without another read; a later
  plan independently captures 11 and seals a +2 target of 13. Exercise refresh,
  stale-source refusal and ordinary Observe withdrawal without native effects.
- Execute 125 distinct post-merge host component tests, 55 exact-source query
  tests and 58 latest-nightly workspace tests. All 29 previously authored growth
  regressions now execute within these explicitly scoped runs. The host/query
  components use the cached older nightly and exclude modern MCP transport.
- Discover 75 public stdio scenarios, with seven added here; they remain
  unexecuted. The full CLI build was OOM-killed in unchanged Asupersync under
  the shared 8 GiB limit. `verify.sh` reaches the missing `br` prerequisite;
  no full-workspace, native-game, live or admission qualification is claimed.

## Unreleased — Complete captured inventory totals

- Add typed `aggregate` queries to both combined production inspection paths,
  with a ready `inventory_totals_request` grouped by item type and exact observed
  stack-unit totals. Retain the same capture, public source anchor and digest.
- Distinguish observed stock from usable supply. Preserve the separate inventory
  allocator's eligibility rules, Unknown filter outcomes, null empty statistics,
  complete-group limits and final output bounds. No additional native read,
  reservation or goal evidence is created.
- Add six Rust regressions and four public stdio scenarios; the suite now
  discovers 68 cases. Rust/stdio execution remains pending the lower-memory
  diagnostic retry after a dependency compiler SIGKILL; no live pass is claimed.

## Unreleased — Keep small query pages usable on large captured sources

- Separate full-source admission from rendered-output reservation for spatial
  and relationship entity queries, matching the existing ranking path.
- Keep caller authority, source hash, entity/byte-work and deadline limits on the
  original context while clamping complete output pages to remaining tokens
  after active-watch metadata.
- Execute 55 exact-source regressions: 52 core/spatial/relationship/ranking cases
  and three actual semantic-publisher cases. Continuations and active watches
  survive small pages; real source/output overflow refuses before publication.
- Use the actual repository dependency crates and exact locked external package
  identities. This scoped result does not claim CLI, transport, native-game or
  full-workspace qualification.

## Unreleased — Inspect operations before an original production plan

- Add an immutable, bounded OperationsInspectionProjection that supports real
  job diagnosis and joint declared-inventory allocation without a plan, order
  or native effect.
- Keep the canonical parent separate from the operations anchor and preserve
  entity retirement generations and native allocator floors across refreshes.
  Refuse source resets, replaced parents, rewinds and cross-session reuse.
- Require whole-source Query/Observe authority, the same paused fortress tick,
  at most 4096 entities and 8192 edges, and prepaid bounded projection work.
- Add twelve adapter regressions for diagnosis/allocation consumers, immutable
  refresh, stale handles, lineage, authority and budgets. Source review and
  formatting are complete; executable validation is in progress. This adapter
  checkpoint does not add a native protocol, tool or production admission.

## Unreleased — Production inspection anchor routing

### Fixed
- Existing combined production inspection advertised source anchors using
  `tick`, while its structured query engine expected `game_tick`. Added a
  strict boundary adapter so copied item queries and diagnosis-generated
  entity/relationship follow-ups work with the advertised source anchor.
- Return query and graph-witness anchors in that same public representation,
  while preserving opaque continuation identity and rejecting changed or
  incomplete anchors. Added focused source-backed regressions; the real MCP build was killed in the existing asupersync dependency, before
  compiling the MCP crate. The new regressions remain unexecuted pending a
  lower-memory diagnostic retry. No live qualification is claimed.

## Unreleased — Inspect inventory and production before planning

- Add optional pre-plan `include_production=true` observation to the combined
  development host, using one complete independently pinned Operations/1.3
  capture at the paused citizen tick.
- Expose the existing six bounded inspection query kinds before order insertion,
  including joint declared-material allocation, with no native Query contact,
  mutation authority, reservation or goal-completion claim.
- Preserve source-local identities, allocation/time floors and source-bound
  continuations; withdraw current handles after errors, refresh, planning and
  recovery. Cache the qualified source digest for bounded descriptor rendering.
- Add source-aware Agent Turn guidance and 23 focused source-owner, presentation
  and public-stdio tests. Keep growth baseline acquisition independent of prior
  inspection and preserve the existing eleven tools and native protocols.
- Record the actual 138-test world-library pass. New MCP checks are in progress;
  no full-workspace, native, live-game or admission pass is implied.

## Unreleased — Request net inventory growth through production MCP

- Add server-derived complete inventory baselines at the exact original paused
  citizen/workforce tick, with independently pinned source qualification and
  explicit zero-versus-unknown handling.
- Accept the closed `observed_inventory_increase` objective in all five finite
  production compilers. Portfolios share one census and seal distinct per-recipe
  targets; client JSON cannot provide or replace the baseline.
- Retain baseline provenance and native allocator floors through original
  archives, repeated planning, first goal sampling and durable-history replay.
- Carry the new goal through selected/whole-parent monitoring, Agent Turn
  presentation and both original clock flows, including inventory stability
  after order retirement, without renewing deadlines or asserting causality.
- Preserve consumed preplanning time after rejection. Refuse selectively expired
  Observe authorization before reacquisition and block objective changes from
  bypassing the canonical time floor.
- Add twelve adapter and eight MCP regressions and update the operator/embedding
  contracts. Independent source reviews completed; these and the nine core
  growth regressions remain unexecuted while the executor is unavailable.
  Preserve the existing native protocols, eleven-tool surface and unadmitted
  development status.

## Unreleased — Preserve original inventory-growth baselines

- Add the typed `EntityQuantityIncrease` predicate and additive canonical/archive
  tag 15, sealing the original quantity, positive increase, anchor, source
  digest, generation and allocator floors without changing legacy identities.
- Share complete source-qualified quantity measurement with current evaluation;
  preserve Unknown inputs, checked arithmetic and conservative scan budgets.
- Bind all baseline fields into archives and query continuations, require a
  forward canonical lineage and refuse implicit historical-baseline rebasing.
- Author six world, two archive and one MCP witness regression. The recovered
  sources have independent review; new growth tests remain unexecuted after the
  executor disconnected. Native/MCP consumer work follows this core checkpoint.
- Fix the Tree-origin closure's unconstrained error type. Before the outage,
  the annotated/formatted baseline passed all 15 focused handoff component
  tests. Preserve the exact observed scope and limitations without claiming
  a full transport or current-head qualification.

## Unreleased — Select harvested material through the workshop MCP flow

- Add a closed opt-in workshop configuration that imports acknowledged Tree
  custody and discovers logs without a startup native item ID.
- Expose at most 64 opaque material offers and route
  `carpenters_from_tree_supply` through fresh Tree identity checks, independent
  Workshop qualification and the existing original-intent preparation flow.
- Verify read-only donor custody around native reads, preserve consumed
  authorization ticks across failures, and retain original-only recovery
  without Tree files, credentials or native calls.
- Prefer a spatially allowed initial candidate, retain the complete offer set,
  and recommend independently qualifying an alternative when appropriate.
  Recommendations never turn unknown host clearance or eligibility positive.
- Author five MCP cases and five actual two-binary stdio scenarios, register
  their Rust wrapper and exact source/contract gates, and document the complete
  operator and recovery flow.
- Preserve both native protocol generations, legacy fixed-item identities,
  original deadlines, causal uncertainty and unadmitted status. New sources
  received independent review and source-consistency checks; build, formatting,
  Python and stdio execution remain blocked by the unavailable executor.

## Unreleased — Import original Tree supply for workshop material selection

- Add a read-only acknowledged Tree-origin descriptor reconstructed from exact
  private journal history and a separate Tree-profile acknowledgement.
- Add bounded fresh supply discovery, deterministic opaque material handles and
  independent Workshop/1.20 qualification of the selected log at the same
  paused source frontier, preserving independent native generation pins.
- Retain original deadlines, custody and one-attempt workshop selection; do not
  grant Tree mutation permission or infer causal provenance.
- Author ten adapter regressions covering custody, source/authority drift,
  canonical evidence, independent eligibility and the 64-offer bound. New
  handoff source was reviewed but could not be executed after executor loss.
- Complete restored pre-handoff component validation of current main:
  102 query tests and 60 adapter tests passed with zero failures or ignored
  cases. These results cover the earlier ranking and existing-worker portfolio
  changes; they do not cover the new handoff or establish full qualification.

## Unreleased — Existing-worker finite production portfolios

- Add `existing_labor_portfolio` planning for one to four normalized, distinct
  finite furniture orders with already enabled workers and actual order StepIds
  starting at zero.
- Share one validated production layout across execution, selected and complete
  goal monitoring, both clock families, private storage and restart recovery.
- Preserve each original key, deadline and stability policy; revalidate current
  workforce eligibility/labor and custody for every native order.
- Compose complete-parent proof only from the full original goal cohort and
  coherent current workforce evidence; preserve negative prerequisite results
  without weakening positive proof or erasing historical achievement.
- Add eight adapter and nine MCP Rust regressions and six actual-stdio cases.
  Initial static/contract and Python discovery checks passed. Rust and actual
  stdio execution remain pending after resource-limited builds and executor loss;
  recovered final sources and the final presentation fix were source-reviewed.

## Unreleased — Bounded exact numeric ranking

- Add shared `ranked_entities` queries with up to three exact i64, u64 or
  fixed-scale ordering keys, mixed directions, optional age limits and stable
  entity-ID ties.
- Retain only the requested page in a bounded heap; prepay complete-source,
  predicate and ordering costs, and preserve whole-row continuation semantics.
- Keep unavailable/mistyped/old/future keys explicitly Unknown and return the
  ranking facts with their original provenance, without mutation authority.
- Wire the same executor into semantic and historical laboratory queries, with
  closed schema, guides and 15 component plus one public-façade regressions.
- Keep the semantic publisher's active-watch output reservation separate from
  the full source-read allowance, with a focused budget regression.
- Record the completed pre-outage component result as 100 passed / one source-
  budget fixture failure. The focused fixture correction and recovered final
  sources were reviewed but could not be rerun after executor loss; no full
  workspace or native qualification is claimed.

## Unreleased — Create finite orders using already enabled labor

- Add closed `kind=existing_labor` planning to the combined host: one original
  finite order at StepId 0, exact selected citizen identities and positive current
  labor requirements, without a redundant assignment or invented effect receipt.
- Discover eligible enabled workers through independently paged public options,
  including manual and broad work-detail configurations.
- Recheck current workforce facts, original custody and canonical source identity
  before preparation and insertion; preserve the actual key through restart.
- Join current labor with order or inventory goals, preserve historical
  achievement separately from current proof, and support both reviewed clocks.
- Pass all 52 actual-source native-executor tests and all 37 combined-host
  component tests. Fix nested read accounting within unchanged budgets and split
  choices response construction to respect the existing macro recursion limit.
- Add seven stdio scenarios (56 total); actual transport execution and full
  qualification remain pending after resource-limited full builds. Preserve
  legacy and portfolio semantics.

## Unreleased — Select entities through their relationships

- Add shared `related_entities` selection with source/target typed filters,
  target kinds and IDs, explicit edge kinds and all three directions.
- Return exact positive relation witnesses, endpoint generations, matched-edge
  counts and explicit witness omissions; preserve Unknown endpoint evidence.
- Bind whole-row pagination to every selector, the exact source and the current
  session; charge the complete source before hashing and a single bounded scan.
- Expose the query through laboratory and structured live-read consumers without
  granting negative existence proof or mutation authority.
- Pass all 87 exact-lock actual-source query/world/output component tests,
  including 13 new relation tests and an exhaustive independent graph oracle.
  Add a public facade/history regression; full MCP execution remains pending.

## Unreleased — Rebase independent edits to the same laboratory dwarf

- Add a bounded field witness to the real shared-session commit path for
  immediate labor and burrow setters after a whole-record conflict.
- Preserve exact predicate and prior-write facts, compensation, identity,
  generation, source lineage and the complete absence of implicit model reads.
- Replay the original request at the actual current source and require the full
  reviewed contract and current authorization before committing.
- Return deterministic certificates and retain original-digest idempotent retry;
  conflicting, unknown or unsupported reads still require explicit review.
- Pass 15 exact-lock actual-source component tests; add three public facade
  regressions, whose full MCP execution remains pending.

## Unreleased — Query entities by three-dimensional location and footprint

- Add shared spatial_entities queries to structured laboratory and live-read
  dispatch, using explicit typed point or cuboid fields and inclusive
  intersects/within/contains relations.
- Support typed filters, projected provenance, both ID orders and complete
  source/session/selection-bound continuation pages.
- Keep missing, unavailable, contradictory, future and invalid geometry Unknown;
  distinguish selection completeness, page truncation and world absence.
- Bound the complete source before hashing, including unselected records and
  nested values, under negotiated byte/record/time limits and fixed ceilings.
- Add thirteen component regressions and one public laboratory/history regression,
  plus the closed schema and usage documentation. An earlier component version
  compiled; final refinements/tests remain unexecuted after executor loss.

## Unreleased — Report complete bounded world-change domains

- Add relation creation, modification and removal to Agent Turn and historical
  change queries, including endpoint incarnation changes.
- Report retained-event arrival, revision and retention loss without inventing
  event occurrence, authenticity or subject generations.
- Include entity label/kind/revision changes and provenance-only field changes;
  preserve unavailable-value masking and exact comparison anchors.
- Bound entities, relations, events and terrain independently with explicit
  omission counts; report cross-epoch resets and reversed-endpoint gaps.
- Pass 19 actual-source component tests before executor loss. Add two public
  facade regressions; their execution and final-generation qualification remain
  pending. See docs/WORLD_CHANGES.md.

## Unreleased — Require known-clear failure evidence for goal success

- Preserve three-valued failure conditions in temporal obligations: Unknown
  interrupts unfinished stability instead of counting as evidence of safety.
- Require current failure clearance in excavation, construction, workforce and
  work-order goal monitors, including after a historical fulfillment.
- Apply the same guard to native clock readiness and agent Plan/Commit offers;
  retain cancellation and the original deadlines, receipts and history.
- Add five intent, four adapter-monitor and four clock-presentation regression
  groups, including off-cadence evidence loss, recovery and missing guard fields.
- Production intent compilation reached the library-test build before SIGKILL.
  New tests and full MCP qualification remain unexecuted after executor loss.

## Unreleased — Diagnose original production goals through MCP

- Add optional closed structured requests to the combined host's existing
  fortress_query tool: production diagnosis, declared inventory allocation,
  entity inspection, traversal and dependencies over the exact selected goal
  source. Expose distinct source and parent anchors without new native reads.
- Retain each portfolio leaf's own operations snapshot and require its exact
  current parent or complete-cohort association. Clock runs, failed calls and
  restart withdraw current inspection sources; recovery restores no snapshot.
- Offer bounded diagnostic guidance for pending goals, with observed conditions
  explicitly separated from causal blockers, native recipe feasibility,
  reservations and original-goal proof.
- Add six authentic-wire stdio/TCP scenarios plus parser and presentation tests;
  the combined production suite now contains 49 cases. These additions were
  source-reviewed but not executed after the shared executor went offline.

## Unreleased — Enforce budgets across every laboratory query mode

- Apply complete canonical source record and byte admission to summary, entities,
  search, observation, terrain, path, civilian recovery, historical reads and
  changes queries, using the same admission path as structured queries.
- Charge both distinct endpoints of a changes query together and check current
  Query authority before selecting historical sources.
- Carry the current session budget through success and refusal publication;
  bound the entire Agent Turn by bytes, tokens and elapsed wall time. Preserve
  whole-row legacy cursor repair and refuse lossy structured pages.
- Add six budget-helper tests and three public-facade regression groups. These
  tests have not executed: the shared executor became unavailable, and the full
  offline MCP check was already blocked by the missing exact FastMCP revision.

## Unreleased — Inspect the operations behind original production goals

- Make the exact immutable operations projection of a selected original production
  goal available to the existing bounded diagnosis and inventory-allocation
  engines, preserving its enclosing source digest and canonical entity generations.
- Add public-monitor regressions for source identity, observed job and inventory
  conditions, generation changes, and authority/budget refusals. The new cases
  were added and formatted but could not execute before the shared environment
  became unavailable.
- Type-check the full original adapter crate against real core/world/intent
  dependencies. This is component compile evidence only; it does not establish
  MCP transport, native gameplay, production admission, or whole-head qualification.

## Unreleased — Execute complete native production portfolios

- Add an explicit bounded portfolio request for one original labor assignment
  and up to four distinct finite furniture orders, preserving the single-order
  request and original native effect identities.
- Keep independent goal histories and both reviewed clock families for each
  exact original order; require explicit selectors and exclude overlapping runs.
- Prove the complete parent only from coherent current labor and all original
  goals, preserving their original deadlines and stability through bounded runs.
- Recover lost middle insertions and clock replies under the original keys;
  retain every journal without restoring confirmations or dispatching twice.
- Fit four-leaf dual-clock recovery within the existing default work budget by
  sharing immutable parent reads and bounding additional journal reservations.
- Pass nine new portfolio and ten existing single-order public-handler/TCP
  scenarios, 49 presentation/budget cases and 65 public adapter integration
  tests. Full MCP transport, real-game execution and admission remain separate.

## Unreleased — Carry civilian recovery through a guarded reviewed release

- Keep stable-clear monitoring in the private/shared world owner across normal
  calls and wait slices; reset it on unknown evidence, gaps, restore or restart.
- Replace one-snapshot all-clear actions with a closed civilian_release blueprint
  backed by current owned clearance and a complete exact civilian census.
- Seal current threat/roster/generation checks into both release effects using
  additive predicate/archive tag 14; retain original generations in compensation.
- Check complete-census budgets at Plan and Commit, refuse stale automatic rebase,
  and stop redundant protection advice once the configuration is complete.
- Pass full world/intent suites plus focused new core cases, eight actual adapter
  cases and 24 owner/public-workflow component cases; preserve native admission
  and transport qualification as separate evidence boundaries.

## Unreleased — Record laboratory events during real workflows

- Add opt-in starter_fortress_history with canonical publication events for
  jobs, units, construction, inventory, clock changes, cancellation and resets.
- Preserve old scenarios; publish world effects and bounded history atomically,
  with typed before/after presence and explicit eviction/omission coverage.
- Retain checkpoint branches and durable event suffixes; do not resurrect action
  authority or imply complete native history.
- Connect producer coverage to typed event queries and exact retained pagination.
- Pass 12 actual adapter tests, 23 actual-source query tests and the normal
  foreground public scenario workflow. Full MCP transport/admission stays separate.

## Unreleased — Connect furnished laboratory rooms across levels

- Add explicit connected_room_stairs_v1 routing that composes safe exact mining
  and paired stair shafts with production, staffing and exact-item furnishing.
- Prove current 3-D access with a distinct bounded laboratory predicate, correct
  per-level building occupancy, horizontal furnishing approaches and preserved
  Unknown evidence. Append archive tag 13 without changing legacy tag 11.
- Preserve original geometry, model and lineage through stair-aware continuation
  and repair; keep completed installations and avoid new wood consumption.
- Add starter_fortress_multilevel with finite workforce/economy/provisions and
  sufficient observed hazard halos; preserve all older scenario snapshots.
- Pass 587 world/intent tests and 230 actual-source laboratory component tests,
  including lower-level completion, forecast, existing stair reuse, repair,
  compatibility and independent 3-D graph oracles. Track the public facade
  regression separately from full MCP and native qualification.

## Unreleased — Run default combined production orders to their original goal

- Let reviewed bounded order-clock runs advance default retained-order completion
  and its original stability interval, preserving exact original custody and
  fresh joined labor/order evidence.
- Expose the corresponding agent Plan/Commit guidance and retain strict refusal
  for retired default orders or substitution of the separate inventory clock.
- Fix first-run logical-budget exhaustion for default and inventory objectives
  by deriving a smaller bounded empty-history reservation; public limits stay fixed.
- Pass 49 actual-source unit/helper tests and ten real Runtime/NativeShell tests
  against synthetic TCP peers, including lost-reply recovery with one unpause.
  Full modern stdio, live-game and admission evidence remain separate.

## Unreleased — Query bounded retained event windows

- Add shared typed event-window queries to laboratory and structured live query
  paths, with inclusive ticks, kind/subject filters, explicit fields and stable
  ascending or descending pages.
- Bind continuations to the complete source, session and selection. Keep whole
  rows under negotiated output limits and charge excluded source records.
- Report partial retained history and missing event provenance explicitly;
  empty projections do not prove that no event occurred.
- Pass 22 actual-source query component tests on current nightly. Track public
  facade and live-handler regressions separately from full MCP qualification.

## Unreleased — Expose structured queries in laboratory sessions

- Connect the existing entity, inspect, traversal, dependency, aggregate and
  lexical-search engines to fortress.query through a closed structured wrapper.
- Support exact current/retained sources with current authority checks and
  explicit historical coverage. Keep source-bound pages intact through output
  budgeting; refuse oversized complete packets without skipping unseen rows.
- Preserve legacy modes and the eleven-tool surface. Document typed filtered
  aggregates, source budgets and continuation use.
- Pass 44 actual-source helper/query/output-budget tests on current nightly.
  Track eight public-facade regression groups; their execution and the complete
  MCP check remain blocked by dependency-compiler SIGKILL under host memory limits.

## Unreleased — Reduce live observation publication memory

- Move canonical component graphs through the coherent jobs, operations,
  spatial and citizen projection chain, removing repeated full graph copies.
- Preserve exact snapshots, canonical bytes, generations, source checks and
  atomic refusal; add a frozen canonical-history regression.
- Pass 17 exact adapter integration tests on current nightly. A same-binary
  synthetic-frame comparison reduces peak RSS by 37.20% (190.07 MiB); native
  latency and SLO qualification remain open.
- Complete the integrated production follow-up: 162 component tests and seven
  new core lowering tests pass with unchanged inventoried inputs.

## Unreleased — Retain production dependencies in executable orders

- Add opt-in `orders: "dependency_preserving_v1"` to laboratory production with
  source-bound unique order names, per-batch material gates and exact predecessor
  completion conditions alongside the action dependency graph.
- Preserve dependency indices across setup and shared workforce scheduling;
  retain original goals and policy through source replay and recovery continuation.
- Require explicit, nonaliased material bindings in the pure lowering API and
  refuse unrepresentable condition sets. Keep unmarked historical requests stable.
- Pass eight new actual-source production component tests on current nightly;
  track seven pure lowering regressions. Full facade/native qualification is separate.

## Unreleased — Filter structured aggregate queries

- Support typed Boolean row filters in canonical aggregate queries, reusing
  entity-query comparison and three-valued truth semantics.
- Count known False and Unknown exclusions separately; preserve exact statistics,
  unavailable metric samples, source identity and complete response bounds.
- Bound combined predicate/aggregate work before scanning. Preserve unfiltered
  result behavior and the frozen MCP surface.
- Pass all 28 actual-source query component tests, including seven new aggregate
  regressions. Full workspace and live qualification remain separate.

## Unreleased — Monitor default combined order completion

- Monitor the original retained-order counter/status goal when a combined
  production request omits an inventory objective, preserving original request,
  step, receipt and stability identities.
- Add separately labeled order progress to runtime, Agent Turn and recovery
  responses. Require fresh current labor for complete parent proof; preserve
  unknown disappearance, strict counter monotonicity and historical-only recovery.
- Keep inventory objectives and both inventory-specific reviewed clocks unchanged.
  Default retained-order sampling uses externally advanced game time.
- Pass all 60 combined component unit tests and five actual-runtime/TCP-peer
  scenarios. Add three tracked CLI scenarios (40 total); full CLI/native
  qualification remains separate.

## Unreleased — Staff complete production and room workflows

- Add explicit `prerequisites.workforce: "complete_project_v1"` to laboratory
  production, provisioned-room and connected-room requests. Plan missing qualified
  construction and mining capacity alongside existing recipe labor; reuse workers
  across sequential stages without bypassing finite scheduling or mutation grants.
- Preserve old source behavior, exact produced-item identities, all original room
  goals and complete-workload deadlines. Fully excavated rooms need no miner.
- Include constructor setup in consumption-aware and explicit future-reserve
  production, with a narrow registered-workshop/shared-model admission rule.
- Pass 212 intent tests and 220 actual-source MCP semantic tests, including eleven
  complete workflow regressions and the new reserve boundary test. Document the
  explicit request policy and its component-only validation scope.

## Unreleased — Plan qualified mining and construction setup

- Add bounded, deterministic setup staffing for complete laboratory projects in
  the explicit shared-workforce model. Reuse qualified enabled workers or propose
  authorized enables with source guards and exact inverse compensation.
- Reject missing or unqualified capacity, unsupported models, invalid source
  custody and oversized inputs. Sequential stages may share one worker; the
  existing engine still enforces finite capacity.
- Keep the helper authority-free and preserve existing compilers until an
  explicit consumer opts in. See `IMPLEMENTATION_STATUS.md` for validation scope.

## Unreleased — Complete combined inventory stability after order retirement

- Wire the independent Run/1.13 inventory clock into the combined labor/order
  owner. After an explicit Wait observes True original labor and inventory,
  `kind=inventory_run` and `scope=inventory_run` prepare, confirm and cancel
  bounded stability slices even after the native manager order retires.
- Preserve the complete original plan, selected key, Created receipt,
  predecessor custody, absolute deadline and sampled stability. Recheck exact
  custody and a fresh complete parent sample after every durable native marker.
  Neither clock can start while the other has pending or unresolved work.
- Retain independent configuration, authority and exact inventory journal
  recovery; withdraw confirmation and current proof after advancement, failure
  or reopening. Safety cancellation still works after the original deadline.
- Retain complete native observations and reply floors before later journal or
  authority refusals, and carry the maximum consumed tick through original
  recovery. A rejected newer clock read cannot revive older goal evidence.
- Add nine real-binary stdio/TCP regressions, bringing the tracked suite to 37.
  Focused actual-source runtime/native-TCP execution, presentation, configuration,
  parser and clock validation are recorded in `IMPLEMENTATION_STATUS.md`;
  full MCP transport and native-game qualification remain separate.

## Unreleased — Preserve complete original custody across production clocks

- Centralize selected original plan, step/key, Created receipt and deadline
  validation in an authority-free adapter descriptor used by the existing
  combined order-clock target, preserving its identity bytes.
- Validate optional order and inventory clock source/journal pins independently;
  reject omitted, swapped or substituted recovery custody while retaining
  existing entry-point restrictions.
- Add focused custody/configuration regressions. Execution remains unverified
  after the full workspace filesystem disconnected; the combined inventory-run
  consumer is still unfinished. See IMPLEMENTATION_STATUS.md for exact limits.

## Unreleased — Preserve combined inventory goals in independent clock targets

- Add a Run/1.13 inventory clock target for the actual selected order in its
  complete original labor/order plan. Reuse the existing selected-order custody
  validator and bind the full archive, selected key/review and verified Created
  receipt without requiring a present native order row.
- Keep the original absolute deadline and legacy standalone key/review bytes.
  Domain-separate combined inventory targets from standalone and order clocks;
  retain independent canonical and native source authority.
- Pass all 19 unchanged-source clock component tests, including three new
  deadline, identity and paused-source regressions. Combined host
  dispatch and recovery wiring follows in the next implementation checkpoint;
  this target constructor alone does not enable any new native effect.

## 2026-11-06 — Repair combined parent sampling and execute clock recovery

- Keep original-plan custody at its immutable original cursor/hash while
  authorizing private reads at the highest consumed game tick. Independent
  provisional labor/inventory projections no longer invalidate later source
  callbacks; session, source scope, expiry and backward-context refusals remain.
- Add a regression for distinct provisional hashes, intervening canonical
  callbacks, non-rewinding authority, cross-epoch refusal and unchanged files.
- Forward optional clock tick budgets through the shared modern-stdio client.
- Execute three actual-source Runtime/native-TCP scenarios: two reviewed runs
  through original parent stability; lost-reply restart/reconciliation with one
  unpause; and offline prepared-review/missing-journal refusal with no native
  calls or custody changes. The offline case used tmpfs; no disk/power-loss,
  MCP/Agent Turn, production Cx, live-game or admission evidence is claimed.
- Record verify.sh's passing preceding gates and its unrelated baseline Rust
  formatting failure. The 28-case actual-CLI suite, final adapter tests and new
  MCP unit regressions remain unexecuted under the documented full-build and
  resource limitations. Strict qualification could not start before the
  execution environment became unavailable.

## 2026-11-06 — Reviewed production clock in the combined MCP session

- Connect optional order-run/1.14 control to the original combined inventory
  goal. Preserve the whole original plan, actual selected step/key, Created
  receipt, predecessor custody and original deadline; retain legacy one-step
  clock target bytes.
- Require independently configured parent/child source pins, operator clock
  authority, an explicit session tick budget and a separately confirmed run
  review. Recheck original custody after each durable effect marker.
- Reconcile pending runs before goal acquisition; never retry unpause after a
  lost reply. Withdraw current goal proof after advancement and require a fresh
  joined sample. Retain exact external clock journal identity and withhold
  process-local reviews after errors or restart.
- Add actual-binary stdio/TCP scenarios for advancing false stock to complete
  parent proof, lost-reply recovery without another unpause, and offline review
  withdrawal/missing journal refusal. Native order retirement still requires
  independently managed time for further samples. Full runtime execution remains
  pending under the documented build resource blocker; no gameplay/admission
  claim is added. Normal complete adapter test-source type checking and an
  unchanged-source production runtime/native-shell component check pass; MCP
  facade/transport/presentation compilation and scenario execution remain pending.

## 2026-11-06 — Exercise parent-proof loss and read-only production recovery

- Add a real-CLI regression for workforce source-generation drift after complete
  parent success: withdraw current proof, retain historical inventory evidence,
  and preserve the original single-dispatch effect keys.
- Extend achieved-history reopen coverage to remove both write grants and verify
  no current proof, no restored authority, no native contact and unchanged
  private custody before the existing reconciled-recovery checks.
- Preserve the previous 17 scenarios and require 18 in both executable counters.
  Syntax and fixture checks pass; actual CLI execution is pending at this
  checkpoint, with no native-game or admission evidence claimed.

## 2026-11-06 — Keep original deadlines and approvals through stale-plan recovery

- Require automatic intent replay to preserve the complete reviewed contract,
  including preparation expiry, temporal deadlines, cadence, stability, guards,
  compensation, risk and authority requirements.
- Return a reviewable newly sealed plan when that contract changes. Independent
  same-tick work remains eligible for automatic witness replay; elapsed time
  cannot silently renew an old deadline or revive an expired preparation.
- Reproduce three failures against the old comparator and pass all six focused
  witness tests with the exact changed production module and real planner
  dependencies. Add two public shared-session regressions; their execution is
  pending the full MCP build. Structural fallback remains separate open work.

## 2026-11-06 — Complete original production goals in the combined MCP host

- Wire foreground Wait to fresh joined labor and inventory evidence for the
  complete original two-step plan. Preserve its original identity, native keys,
  deadline, cadence and stability; expose the separate selected inventory proof.
- Publish current parent truth only at its exact joined anchor. Require the
  selected temporal proof before reporting original-goal success; withdraw that
  success after failed publication, cancellation or reopening.
- Keep historical inventory achievement useful after native order retirement
  without claiming physical completion, causal output or general fortress proof.
  Make Agent Turn guidance distinguish unfinished stability, reversed labor and
  a fenced original source.
- Add three real-binary modern-stdio/TCP regression scenarios for retirement,
  labor reversal, bracket failure and recovery. All 32 normal adapter production
  tests pass, including the nine parent-goal groups. MCP compilation and the new
  stdio scenarios remain pending at this checkpoint; no live-game or admission
  qualification is claimed.

## 2026-11-05 — Explain current production labor blockers

- Derive a bounded, canonical-ID-ordered comparison for every original selected
  citizen under the independent workforce evidence policy. Report requested and
  qualified observed labor settings; missing or unqualified values remain unknown.
- Include the full comparison in current-parent evidence and summarize matched,
  mismatched and unknown citizens in Agent Turn, with up to eight exact blockers
  and an explicit omitted count. Invalid or stale projections expose no derived
  blocker claims, and guidance never redispatches the original completed keys.
- Extend adapter/presentation regressions and the actual combined CLI/modern
  stdio suite to 25 scenarios, including original inventory stability, retirement,
  labor reversal, bracket drift, custody failure, restart and source fencing.
  All 35 production adapter tests pass on the complete actual test crate with
  exact dependencies. MCP/CLI execution remains blocked by the Asupersync compiler
  receiving SIGKILL under shared memory pressure; the exact evidence and broader
  qualification limits are recorded in `IMPLEMENTATION_STATUS.md`.

## 2026-11-05 — Complete original production goals through MCP

- Connect the combined development host's foreground Wait to the executor's
  current-parent acquisition. Fresh matching labor captures bracket complete
  inventory evidence while the original plan, keys, deadline and stability remain
  unchanged.
- Expose separate selected inventory and complete-parent truth, anchors and source
  digests. Report original-goal proof only when current parent truth and selected
  temporal proof both hold; retain unknown physical completion and causal output.
- Withdraw current proof on failed reads/publication and reopening. Retain source
  fences and historical inventory achievement, with Agent Turn guidance that
  distinguishes completion, unfinished stability and contradicted current labor.
- Preserve the frozen eleven tools, existing native generations and development
  isolation. Executed validation for this increment is recorded in
  `IMPLEMENTATION_STATUS.md`; no live-game or admission claim is added.

## 2026-11-06 — Current original labor and inventory goal proof

- Add a bounded executor parent poll that joins fresh selected workforce evidence
  with the existing original inventory monitor at one coherent paused frontier.
- Require independent policies, exact original citizen/source/custody bindings,
  both workforce bracket captures and the original inventory stability before
  proving the complete parent. Historical effect receipts are never current
  labor evidence; retired orders do not become causal production witnesses.
- Retain consumed workforce floors through later failures and fence invalid
  identity/source lineage. Keep parent proof process-local and clear it on
  recovery; selected history and native wire formats remain unchanged.
- Add nine focused parent regression groups. Actual latest-nightly locked/offline
  adapter production checking and scoped formatting/diff checks pass. Test
  execution remains pending under resource pressure at this checkpoint; no full
  runtime, native/live or admission qualification is claimed.

## 2026-11-05 — Independently reviewed inventory stability clock

- Add explicit inventory_run planning/confirmation/cancellation over existing
  run/1.13 so an already true original inventory objective can finish its
  stability requirement after the native order retires. Bind exact original
  plan/archive/review/Created receipt and retain its deadline and cadence.
- Recheck complete qualified inventory and custody after each durable clock
  marker before native effect. Keep source-scoped native authority independent
  of named-fortress authority and preserve existing order-run/1.14 guards/bytes.
- Retain a separate private inventory-clock journal and exact external identity;
  fence the two owners against each other's pending work, reconcile both before
  sampling, preserve no-retry lost-reply recovery and deadline-independent safety
  cancellation. Expose actionable Agent Turn reviews and bounded recovery.
- Pass 296 distinct focused cases, including 12 inventory-clock cases inside 208
  MCP components, plus warning-denied production Clippy and formatting. Native
  game/stdio/full-workspace qualification and compatibility admission remain
  separate. The 64-run ceiling does not extend existing history or work budgets.

## 2026-11-05 — Combined labor/order inventory goals through MCP

- Accept an explicit recipe-matched inventory minimum in the original combined
  request while retaining the complete parent, original labor dependency and
  native keys. Preserve legacy request identity when the objective is omitted.
- Let foreground Wait sample the selected original inventory obligation after
  both effects settle, with durable history, exact source policy, historical
  achievement and additive external recovery custody. Separate selected proof
  from current labor and whole-parent completion.
- Withdraw stale success and unfinished stability on publication failures;
  retain consumed replay floors and use the existing configured replay allowance
  without spending response or custody reservations twice.
- Expose bounded Agent Turn polling/recovery guidance and document proof limits.
  Pass all 36 combined cases within 208 actual-source MCP component tests and
  scoped warning-denied production Clippy. Native TCP lifecycle, full transport,
  live-game and admission qualification remain separate.

## 2026-11-05 — Durable selected-step production goals

- Add an exclusive selected work-order monitor to the real labor/order executor,
  preserving the whole parent, original StepId/key/archive/review and exact
  Created receipt. Historical predecessor custody remains checked at replay and
  live policy boundaries.
- Evaluate selected postconditions and obligation separately from the complete
  parent terminal. Explicit inventory minima may survive native order retirement
  without claiming current labor, physical completion or order attribution.
- Reuse durable captures, first achievement and original timing; clear current
  proof and unfinished stability after recovery or failed publication. Keep
  legacy public one-step history/open/poll entry points strict and unchanged.
- Pass 23 focused production and 65 legacy work-order integration cases at stable
  source hashes. Full-workspace, transport, native/live and admission qualification
  remain unclaimed; the broader executor bead stays in progress.

## 2026-11-05 — Complete citizen rosters beyond a single native page

- Support complete rosters through 200 citizens in both semantic native shells
  and the combined host's reused workforce acquisition, with unchanged 128-row
  transport pages and 32-citizen mutation selection.
- Share the complete reader and evidence limits; refuse oversized or incoherent
  pages without publishing partial state or retaining stale routing evidence.
- Exercise the actual 200-citizen evidence refresh within its existing 16 MiB
  reservation, without changing grants, protocols or canonical capsule bytes.
- Pass all eight new roster cases and the existing 88 MCP component cases, plus
  warning-denied Clippy for both actual native Rust shells. The focused session
  total is 360; full transport, native/game and admission proof remain pending.

## 2026-11-05 — Original observed-inventory objectives through production MCP

- Add a closed, recipe-matched observed inventory minimum to the original finite
  work-order request, with a distinct identity and unchanged legacy goals.
- Prove stack quantities from the complete qualified operations roster; retain
  unknown order completion and attribution when the order has retired.
- Reissue conditional inventory policy during durable replay, retain first
  achievement, and withdraw current success after consumption or reopening.
- Expose separate inventory success and useful terminal guidance; preserve
  uncertain insertion custody and refuse unnecessary native clock runs.
- Clear current proof and unfinished stability after early Wait failures without
  unauthorized I/O, using the existing durable interruption representation.
- Pass 88 MCP component and 65 adapter cases plus scoped warning-denied Clippy
  on the November 5 nightly. Full transport/game qualification remains pending.

## 2026-11-05 — Exact entity quantities in goals and archived predicates

- Add bounded entity counts and unsigned quantity sums with complete-domain,
  independently qualified evidence and conservative Unknown semantics.
- Bind the new predicate through canonical bytes, query continuation identity
  and appended archive tag 12 while preserving legacy vectors.
- Capture complete kind-domain reads for MCP revalidation, detecting inserted,
  removed and replaced contributors and selector changes.
- Pass 199 focused core/world/archive cases and three actual-source witness
  cases on the November 5 nightly; preserve the full-runtime qualification gap.

## 2026-11-05 — Select any covered citizen and page workforce choices

- Add a closed optional Observe request for 1–32 canonical citizen IDs and a
  bounded option offset, exposing choices beyond the first 32 citizens/eight
  options without changing native selection or response limits.
- Bind selected/page request semantics in a distinct derived witness; retain
  default witness bytes, explicit omission counts and independently fresh
  observation anchors. Suggested next reads pin the exact selected citizens.
- Wire fresh-read suggestions through Agent Turns and reject malformed,
  unavailable or post-plan selectors without creating a mutation permit.
- Add seven choices regressions, one presentation regression and three public
  CLI/stdio scenarios; expand the latter to 14 and correct its peer for repeated
  selectors and complete multi-citizen receipts.
- Independently review source and check JSON/contracts/counters. Runtime tests,
  formatting and qualification remain unexecuted because this session has no
  usable execution environment; no new live or admission claim.

## 2026-11-05 — Presence-aware lexical search and kind discovery

- Exclude unavailable or contradictory fact compatibility values and their
  field names from lexical matches, snippets and relevance calculations.
- Include canonical entity kinds in the existing bounded search document,
  preserving known values, retained events and deterministic result ordering.
- Add four world regressions and two public query-route regressions, including
  current Query authority, exact anchors and later known-to-redacted withdrawal.
- Record source review only: compilation, formatting, executable regressions
  and full verification/qualification could not run without an execution
  environment. The owning FrankenSearch integration bead remains open.

## 2026-11-03 — Real control chosen through public workforce observations

- Exercise exact `a515efb` / executable `0e514de8…` with fresh Terra medium
  under NTM: choose MINE/citizen 252 through MCP, apply the assignment and its
  dependent finite bed-order insertion, and recover both receipts offline.
  Root replays all 15 exchanges, recomputes all choices against complete native
  before data, verifies both journals, unchanged seven-file recovery and fresh
  process/plugin/save cleanup. No ticks, produced bed, goal proof or admission.
- Retain two actor render-before-scan departures and the incomplete loading
  capture. The malformed-request negative is generic, not field-specific.
- Pass scoped warning-denied Clippy, 23 Linux cases and a separate eleven-case
  public stdio run. Preserve the subsequent Cargo wrapper's restart timeout;
  a passing traced diagnostic does not explain or erase it. Nonauthoring raw
  review independently agrees with the bounded live result; full qualification
  remains pending, with no task closure.
- Plant a cached-observation implementation in a disposable build: the positive
  control case still passes and both new stale-source/floor cases fail with
  zero setup errors or skips. The candidate source is untouched.

## 2026-11-03 — Observed workforce choices and real combined control

- Replay a real Terra/NTM combined development run: one MINE membership/labor
  change, then one dependent finite bed-order insertion, two Applied keys and
  zero ticks. Verify complete native captures, all 16 modern MCP exchanges,
  seven unchanged offline custody files and fresh cleanup. Preserve the dirty
  build guard failure, setup failures and scan-order deviations; no goods,
  original-goal, qualification or admission claim.
- Add bounded pre-plan workforce choices through Observe using the existing
  native reader and canonical identity resolver. Publish source witnesses,
  selected eligibility/membership and deterministic enable/disable candidates
  with omissions. Revalidate through existing preparation/commit owners;
  choices supply neither authority nor mutation preconditions.
- Retain consumed observation tick floors after failed combined reads, withdraw
  stale choices, and keep them absent after original planning and recovery.
  Extend component and public tests without removing prior cases. Final tests,
  no-scout game execution and full exact-source qualification are pending.
- Retain the earlier full qualifier's 21 adapter failures. Clean 49's scoped
  17 Rust and nine stdio cases pass; that is not full qualification.

## 2026-11-03 — Combined labor/order development MCP host

- Connect the real production executor and durable dispatcher to the eleven
  tools and standard `serve-gameplay-dev labor-order` profile. Retain one full
  original labor-then-order plan, exact keys, family sessions and parent custody.
- Seal all three operator configurations, keep independent family authority and
  source pins, and verify original archive/identity at child evidence boundaries.
  Lost replies retain original-key recovery; restart never restores a commit
  permit. No clock, original-goal proof or production admission is added.
- Fix compiler integration, nested budget, returned tool names/session IDs and
  first-labor canonical-evidence priming found through actual compiled execution.
  The Linux CLI passes eight public modern stdio component cases, separately
  rerun by Root, and 15 focused Rust cases. Loopback peers remain doubles;
  full clean qualification and new-host live execution are pending. Scoped
  `.4.1` and broader parents stay open.
- Fix two further fresh-eyes findings in source: check cancellation/deadline
  after successful response rendering and withdraw the review on failure;
  direct offline uncertainty to close-and-reopen reconciliation. The expanded
  eight-case suite reproduces the latter on the prior binary (seven pass, one
  intended failure). Corrected-source execution is pending. Track missing
  pre-plan workforce choices in `.4.2`; operator scouting is not agent competence.
- Execute both response regressions within 16 passing Mac component cases, with
  the older dated toolchain and platform warnings disclosed. Reproduce another
  pre-plan Commit/Wait guidance loop through actual stdio and fix its source
  classification: no original executor means no possible native dispatch.
  Add a ninth public case without weakening the existing eight; new execution
  and final clean-source qualification remain pending.

## 2026-11-03 — Independently replayed current-source job-control pair

- Prove real modern MCP suspend/resume by GPT-6.1 Terra medium under NTM on
  exact `4ed1091`, server `df7617d4…` and native plugin `e7a3f934…`. Root's
  unchanged strict auditors pass all 18 exchanges, six journal frames and
  both Applied receipts. Independent native captures show job 5 flags
  `0 → 2 → 0`, posting -1, all other captured fields unchanged and zero ticks.
- Verify exact offline preservation of the original 2894-byte journal, all
  347 export/supplement members, native/server/source identities, final pause
  and fresh host restoration/drain. Disclose the optional missing screenshot.
- Close `.8.1` for one live job class; seven other classes, unified MCP host,
  broader fortress competence and production admission remain unproved.
- Retain AD's incomplete pair, AE's expiry refusal and AF's operator error
  separately. No later documentation head inherits `4ed1091` qualification.

## 2026-11-03 — Preparation expiry and client workflow

- Independently replay AE's terminal refusal and unchanged real job: at least
  80.129179611 seconds elapsed after Plan returned, beyond native preparation's
  60-second lifetime. Preserve its negative result; AE did not close `.8.1`.
- Document reviewing Plan's complete effect and immediate Commit, followed by
  Explain. Their effect objects are identical; no native expiry or authority
  check is relaxed. A pipe-only relay is still unintegrated development work.
- Verify the complete 326-member export/supplement, canonical refusal receipt,
  exact offline journal preservation and paused owned cleanup. These later
  documentation changes do not inherit `4ed1091` qualification.
- Preserve AF's failed operator setup, including Root's incorrect header-X
  identification and the resulting queued demolition. Verify all 365 export
  members and cleanup; no MCP request occurred. Use the verified supported UI
  input route and leave the workshop panel open in the next trial.

## 2026-11-03 — Current-source live job suspension and harness repairs

- Prove one real MCP suspension on qualified `4ed1091`, source-matched native
  plugin `e7a3f934…` and development server `df7617d4…`: independent captures
  show job 5 flags `0 → 2` at the same paused tick with other captured fields
  unchanged. Preserve the uncommitted Prepared resume and keep `.8.1` open.
- Verify original-journal offline discovery and unchanged before/after bytes;
  retain cleanup evidence and the disclosed metadata scan-order violation.
- Repair the operator Lua evidence channel and independently verify it in
  actual DFHack. Keep failed startup, UI-pause and deadline attempts distinct
  from gameplay success, and preserve the production admission boundary.

## 2026-11-03 — Qualified cancellation and resource repairs

- Qualify clean `4ed1091`: both mandatory scripts pass, all 43 gates, and
  5255 tests per debug/release profile across 134 result blocks. Independent
  receipt, source and raw-log replay pass; receipt SHA-256 `6e2da780…`.
- Close the scoped cancellation bug `.25` after exact upstream and downstream
  conformance. Keep the broader resource and gameplay proof tasks open.
- Build a fresh non-test development job server, SHA-256 `df7617d4…`, bound
  to all 2695 exact source files. Track the composed policy live trial as `.8.1`;
  do not inherit the older live10 receipt or imply production admission.
- Correct the integration guide's tool-name scope: laboratory underscore names
  and explicit live-profile dotted names must be discovered and sent unchanged.
  Update the quickstart pin and describe local/self-hosted conformance authority.
- This later documentation and tracker update has no new full-source receipt;
  the qualification remains exact to `4ed1091`.

## 2026-11-03 — Resource inspection and cancellation review

- Reauthorize task candidates at owner-fenced publication after fetching them
  outside world/session locks. Add deterministic expiry/replacement component
  regressions for discovery, detail and evidence; execution is pending.
- Keep the replay test export test-only after resources adopt the fenced helper.
- Correct the mutex-guard binding in the new objective regression after a clean
  build refused it before running tests; retain that failed attempt.
- Keep resource reads from certifying objectives, recording observation events,
  consuming request IDs or publishing custody. Apply current shared-world
  authority and durable-owner fences to replay and capabilities inspection.
- Use one state-resource wall deadline through projection and escaped output
  accounting. Add allowed/denied process/durable, owner replacement, shared-clock
  expiry and deterministic deadline regressions; Rust execution is pending.
- Retain the clean e227 full release failure in Tasks cancellation. Track the
  minimized upstream race as fastmcp_rust#100 and take its reviewed narrow exact
  Git pin `13461caf` in all three dependencies, the lockfile and allowlist.
  Deterministic upstream regressions pass 4/4; downstream conformance and full
  qualification are pending. No new qualification is claimed.

## 2026-11-03 — Real wooden bed and main integration

- Record independently replayed W: real MCP Tree supply, completed original
  Carpenter, Manager 1.23 cache appointment, one finite order and newly observed
  PECAN-wood BED561 after 1200 bed ticks. Keep the disappeared original order's
  goal Unknown; do not infer causal linkage. Fresh host cleanup checks pass.
- Reconcile public main's immutable resource and job-control work with the
  gameplay branch, preserving entity references and the stronger guards from
  both job policies. Native job doubles pass 863/13 plus the 287-check guard and
  all five compiled mutants; require these checks in both standard scripts.
- Reserve the largest numeric wire ID for entity/task/state resource budgets
  once, and add a real entity-resource budget refusal/positive regression.
- Direct gameplay documentation to `manager-v1-23`; retain the legacy Manager
  route with its missing-cache limitation. Merged-source Rust checks are pending.

## 2026-11-03 — Manager append capacity and actual MCP follow-up

- Reserve HF-link/cache capacity before Manager 1.23 preparation and accept the
  last valid HF-link/history append during native terminal observation. Preserve
  legacy 1.22 native behavior; add boundary regressions and compiled mutants.
- Native SDK doubles pass 15 scenarios/343 assertions and eight compiled mutants;
  legacy passes 12/270 and three mutants with unchanged vectors.
- Record clean e899 focused execution: 23 adapter cases and actual dedicated/CLI
  1.23 suites of eight each plus seven legacy cases, using native doubles.
- Fix the test-only undeclared JSON import, stale session-family test constant
  and strict-Clippy hex parser diagnostic. Full current-source checks and real
  1.23 bed production remain pending.

## 2026-11-02 — Full qualification of the preceding repairs

- Clean `82f9a6c` passes both required scripts, all 43 gates and 5227 tests
  per Rust profile across 131 targets, zero failures or ignored cases.
- Record exact source/receipt scope; this evidence does not transfer to 1.23.
- Connect the new 1.23 source, native mutation gate and both eight-case actual
  MCP subprocess wrappers to the existing qualification path. Execution pending.

## 2026-11-02 — Manager 1.23 cache integration (early development cut)

- Witness group/civ responsibility caches and include the selected Manager
  assignment in MANAGE_PRODUCTION during the same native transaction.
- Preserve legacy 1.22 wire/vectors; add distinct 1.23 startup, package, native
  identity, codecs, custody and scoped public cache-proof field.
- Add native component/mutation and Rust cache/custody regression sources.
  Rust execution and live 1.23 production remain pending.

## 2026-11-02 — Keep standard CLI suite counts exact

- Correct the nested Tree/workshop standard CLI caller to require seven tests
  after adding both deadline regressions. Preserve every zero-failure/error/skip
  assertion; retain the failed574 verification as evidence of the missed caller.

## 2026-11-02 — Deadline regression execution and strict Clippy repair

- Record exact f922 focused GREEN: seven actual MCP cases per prepared profile,
  and 14/23/18 Tree/workshop/Manager adapter cases. Old sealed Tree reproduces
  exactly both new failures (five passed, two failed).
- Preserve latest-nightly Clippy failure and use an equivalent strict inequality
  for the newline-inclusive numeric-ID test bound. Full successor checks pending.

## 2026-11-02 — Prepared control review deadlines (.14.3)

- Use the existing 60-second ceiling by default for Tree, workshop and Manager
  review; preserve shrinking original deadlines and the native prepare expiry.
- Reject already expired Tree/Manager connections before recording dispatch;
  preserve typed workshop pre-dispatch read failures and original custody.
- Add actual MCP delayed-review/default and expired/no-dispatch regressions,
  including consumed permission on a later call. Execution remains pending.

## 2026-11-02 — Exact-source verification follow-up

- Record all 43 qualification gates and 5198 tests in each Rust profile on clean
  `924037df`; this qualification does not transfer to later source.
- Response-safety `cd3ef967` passes 11 resource, 5 Tree packet and 4 modern
  subprocess cases, including numeric-ID budget and zero-TTL behavior.
- Manager `63548fa7` dedicated and standard CLI executables each pass five
  actual modern subprocess scenarios with explicit native doubles. Real-game
  MCP bed acceptance and full integrated-head qualification remain pending.

## 2026-11-02 — Run Manager native regressions in standard checks

- Record independently replayed native-only R evidence: one Manager appointment,
  one history event, zero ticks, original-key duplicate/query stability and cleanup.
  MCP-only bed acceptance and production admission remain unproved.
- Wire the existing Manager native transaction tests into both mandatory scripts.
  Preserve exact vectors, independent commitments and three compiled mutation
  controls; source is present and execution remains pending.

## 2026-11-02 — Vacant Manager 1.22 development source (.14.2)

- Add the fixed six-method native generation and lossless bounded Rust
  capture/plan/receipt codec for one vacant MANAGER appointment, preserving raw
  assignment flags and the generated event constructor's one-zero-byte flags.
- Reuse one-attempt journal/independent ACK custody and same-connection permission.
  Revalidation, the complete assignment/HF/history transaction and dual-getter
  proof are required; uncertain effects retain the original key without retry.
- Wire the existing eleven tools and `serve-gameplay-dev manager`, with full
  fortress authority, one explicit review/confirmation and zero game ticks.
  Add native-vector, custody, TCP, MCP presentation and actual subprocess test
  sources; these are not live-game evidence.
- Correct public-adapter stale-tick authority after independent source review:
  retain the connection's observed tick floor, reauthorize all native plan
  edges at max caller/plan/observed time, and give both dispatch guards and the
  writer the journal's retained tick. Add paired TCP/real-lease regressions;
  keep one shrinking budget and post-dispatch uncertainty unchanged.
- Root reports native SDK attempt 4 build success. Retain actual Rust cut 2
  E0603/zero-test evidence; cut 3 corrects test visibility and scope policy.
  Final Rust/MCP execution, live Manager/bed acceptance and qualification remain
  pending. No registry, production runner, admission or item closure changes.

## 2026-11-02 — Account for maximum numeric MCP request IDs

- Tree result sizing now reserves the pinned codec's 4096-byte numeric ID bound,
  alongside framing and newline. A new full-envelope/chunk-preservation regression
  targets the actual 512KiB boundary; Rust execution is pending.

## 2026-11-02 — Revalidate session resources on every read

- Session and doctor resource responses now specify a private zero TTL, so
  clients are not invited to reuse state or authority after it changes. Modern
  stdio and grant-revocation regressions are added; execution remains pending.

## 2026-11-02 — Direct compilation and bounded live gameplay evidence

- Direct Linux execution on exact `64fd25f` passes 2,842 MCP/wrapper and 1,330
  adapter cases. Real J replay returns a usable session and ACK; two raw content
  blocks reconstruct all 30,489 native bytes.
- Live K semantic MCP effects designate the observed tree and complete original
  Carpenter workshop 1. The finite bed order remains inactive/unvalidated, with
  zero beds after four bounded production intervals. No manager was observed;
  that is a blocker hypothesis, not a proved sole cause. No flags were forced.
- Independently replay all 306 exports, six exact 1,200-tick intervals without
  overshoot and nine unchanged offline custody files. Fresh host checks confirm
  cleanup, seven original plugin states, both 30-file saves and port closure.
- Actual standard CLI integration passes on exact `34c8c53`. Its warning-denied
  Clippy run finds three existing Tree custody APIs over the argument-count
  style limit. Add three local, reasoned lint annotations, retaining explicit
  authority/frame/ACK parameters and behavior, and name the test fixture tuple
  type to resolve the separate type-complexity finding. Final successor qualification
  and original bed acceptance remain pending; no broad lint/test gate changes.
- Fix the all-target integration build: shared workforce tests had imported a
  private unit-test clock from the ordinary production library. Preserve the
  interrupted-read assertions in the public-API suite and retain deterministic
  delayed/pre-I/O/unwind assertions in a unit-only include. The test read hook
  defaults to absent; production clock and code remain unchanged.


## 2026-11-02 — Connect standard CLI to existing gameplay development runtimes

- Add `serve-gameplay-dev <tree|workshop|bounded-clock|finite-bed>` as a closed
  operator startup choice. Invoke the existing guarded runtime in-process;
  preserve its eleven modern tools, plans, confirmation, custody and authority.
  Unknown/missing profiles and extra arguments refuse before startup.
- Apply the finite-order runtime's existing environment guard before stdio
  startup as well as on requests. Production state remains refused; no native
  generation, production registry or protocol-1.0/1.1 mutation route changes.
- Add one Rust integration target driving three actual CLI scenarios. Reuse the
  existing five Tree and five workshop subprocess cases via optional test-only
  server arguments; retain their assertions and counts. Execution is pending
  Root's direct build. This selects one runtime per process; shared cross-profile
  sessions, original .14 gameplay acceptance and final qualification remain open.


## 2026-11-02 — Correct Tree result encoding and entropy authorization

- Preserve candidate `527587a` compiler exit 101 and zero executed tests.
  Measure complete results through the pinned public modern tools/call codec;
  use it in the packet regression as well. Keep empty ordinary metadata and
  all existing response, string, evidence and custody limits.
- Check Query authority and the 32-byte budget before Tree entropy I/O, and
  remove the unused Record import. Extend source-boundary mutation checks.
  Corrected-source execution and live gameplay proof remain pending.
- Preserve direct `8eb7754` test-compilation exit 101 and zero executed cases.
  Annotate chunk-test sizes as `usize` and write the identical newline-inclusive
  bound as a strict comparison. Production encoding already compiled; complete
  corrected-source execution remains pending.
- Retain J replay's capability-denied result: the new Tree entropy check
  requested an unscoped Query against a map-scoped grant. Pass the configured
  area from both callers and enforce those call sites in mutation checks.
- Integrate both corrections into the private CLI branch. Root reports exact
  `64fd25f` direct Linux passes: 2842 MCP/wrapper cases, 1330 adapter cases and
  five Tree plus five workshop subprocess scenarios, all without failures or
  ignored/filtered cases. Its sealed-executable J replay returns a usable ACK
  and losslessly reconstructs all 30489 bytes; offline reopening preserves
  custody without native calls. These results belong to 64fd. The merged CLI
  candidate, both final full commands and gameplay acceptance remain unproved.

## 2026-11-02 — Package complete Tree evidence within modern MCP bounds

- Preserve Live I's rejected open and original custody, and independent old-cfcc
  replay RED of J's unchanged 30489-byte canonical capture. Its 60978 hex
  characters fit individually; complete Agent Turn wrapping crosses the pinned
  result-string bound. A separate valid 67676-byte protocol-double capture also
  reproduces the old failure with header-only custody and zero writers.
- Return typed modern complete results with the full Agent Turn, independent ACK
  and original uncertainty in content[0]. Recursively replace raw capsules with
  explicit ordered block/path/length/SHA256 descriptors and preserve every byte
  in separate bounded hex content blocks. Keep dependency, native bytes, eleven
  tools, custody, authority and transport guards unchanged.
- Check complete typed serialization with empty ordinary-tool metadata and
  worst-case admitted JSON-RPC framing against the existing 512KiB product
  ceiling. Preserve the
  bounded custody/uncertainty fallback. Extend the same five stdio scenarios with
  large/max-cell positives, actual longest-ID envelope and corrupt-block refusal;
  add two registered Rust packet regressions. Repaired-source Rust, new J replay
  and full qualification are pending. Original .14 and owned bug .14.1 stay open.
- Preserve the broad validator RED for two forbidden `expect` calls in the
  existing test-only citizen clock seam. Use an equivalent installed-clock
  assertion and conditional update; production time, deadlines and cases stay
  unchanged. Keep the validator and its forbidden-call rule intact.


## 2026-11-02 — Fix the first executed Tree adapter compiler errors

- Preserve exact `45b2a4e`'s admitted ts1 compiler exit 101, worker-verified
  source-content receipt and matching clean before/after source snapshots.
  Zero tests ran. Keep this RED separate from transport/admission refusals.
- Annotate the post-ack dispatch closure as `Result<Record>` and rename the
  TCP peer factory and all callers to avoid handle shadowing. Preserve guards,
  indeterminate key/cause mapping, peer joins, assertions and all test counts.
  Corrected-source execution and final full qualification remain pending.

## 2026-11-01 — Make citizen deadline regressions deterministic

- Retain frozen `08fca4c` verifier exit 0 and qualification exit 101. Debug and
  verifier release profiles each pass 5176 cases across 125 targets; qualification
  release stops at 1314 passes / one failed citizen deadline case. Preserve the
  failed official receipt, all raw output and exact-source snapshots.
- Add a test-only per-thread monotonic clock with RAII unwind restoration.
  Keep the existing 10ms request and exact one-read/no-retry assertion; expire
  the fake source only after recording read entry. Require released resources,
  withdrawn old scope and no final authorization/publication. Add one registered
  pre-I/O expiry/unwind case requiring zero I/O. Production time and budgets are
  unchanged. Focused execution and both final full commands remain pending.

## 2026-11-01 — Add supported one-tree supply development generation

- Add isolated unadmitted tree-harvest/1.21 native, safe-Rust and eleven-tool
  modern MCP source using SDK `markPlant` at its computed southeast trunk tile.
  Preserve unknown stable plant identity, complete visible body witnesses,
  observed CUTWOOD/axe offers and separate designation/supply proof.
- Share the pure native one-attempt engine while preserving workshop/1.20 wire
  vectors and ground-only policy. Reject PERM borrowing and forced item/job state.
- Synchronize independent acknowledgement before one dispatch; withdraw failed
  preparation, enforce scoped grants and one shrinking budget, preserve consumed
  source frontiers and refuse rollback or uncertainty-clearing negative history.
- Register 14 adapter, two MCP packet and one five-scenario actual-stdio Rust
  integration cases. Execution remains pending. Three Tree and seven workshop
  inventory regressions pass; earlier native-double cuts retain their exact
  component scope and failed attempts. Keep both final full commands, exact SDK
  artifacts, game proof and original `.14` acceptance open. Preserve all 43 gate
  IDs and production admission boundaries.

## 2026-11-01 — Preserve workshop startup failure status and distinct test logs

- Return process failure when workshop startup validation refuses the closed
  environment. The strengthened existing real stdio case detects exit zero on
  `08fca4c`; retain its four-pass/one-failure result. Corrected-source execution
  and full qualification remain pending.
- Append log-file extensions to production markers so `1.0` and `1.20` retain
  separate stdout, stderr and exit receipts. Keep all five stdio scenarios and
  their original positive, recovery and refusal assertions.

## 2026-11-01 — Connect original Carpenter workshop construction through MCP

- Implement the distinct unadmitted workshop/1.20 native generation, safe-Rust
  adapter and registered eleven-tool modern MCP consumer for one exact eligible
  WOOD log and one original 3x3 Carpenter workshop. Preserve complete review,
  scoped authority/leases, fresh revalidation and one native construction attempt.
- Synchronize journal dispatch and independent acknowledgement before the writer;
  reject prefix rollback, source/clock regression and stale or foreign prepared
  connections. Withdraw old knowledge and preparation after failed calls.
  Preserve original identity through recovery without redispatch or new keys.
- Connect callable Observe/Plan/Commit/Wait recommendations and terminal reasons;
  preserve the full Agent Turn spine, null canonical anchor and explicit unknown
  world continuity, including errors and bounded overflow.
- Pass final focused 23 adapter cases, seven MCP unit cases, one integration
  case running five actual stdio scenarios, and seven source-inventory cases,
  with zero failures, ignored cases or skips. Strict native component execution
  passes 13 scenarios / 27,596 assertions, 12 vectors and 12 individual compiled
  mutation checks using SDK/protobuf doubles. Clippy and formatting pass; retain
  earlier failed attempts and their raw output.
- Record Terra's separate actual-SDK configure/protobuf/build-link diagnostic,
  with exact unchanged C++/header/proto hashes and probe location in
  `IMPLEMENTATION_STATUS.md`. This is not accepted native R1, game execution,
  full final-source Rust qualification, artifact or runtime admission.
- Keep both mandatory full qualification commands and live acceptance pending.
  Original `.14` and finite-bed manager/activation prerequisites remain open;
  preserve the production registry, runner and floor. Document the isolated
  profile in `docs/WORKSHOP_CONSTRUCTION_MCP.md`.

## 2026-11-01 — Withdraw failed-call item views and bound test setup

- Exact private R29 `b6d0e626` passes both required commands on the November 1
  nightly: each 5,145 debug plus 5,145 release cases, 123 targets per profile,
  zero failed or ignored, all 43 gates. Parent checks the official receipt,
  all 31 critical cases in both profiles, all 2,592 source files and the emitted
  release laboratory CLI. Report bytes do not inherit source qualification;
  no native, live or production/runtime admission follows.
- On exact qualified R28, live E exposes a new bed through fresh Wait and
  same-anchor Query, then withdraws those rows after failed Observe. Independently
  replay all 139 members, 23 modern exchanges, native journals and complete
  before/after rosters. Two reviewed runs advance 1,200 and 743 ticks without
  overshoot; original goal remains Unknown after order disappearance. Actual
  five-file offline recovery and fresh host cleanup pass. Preserve D's earlier
  failure and E's two operator diagnostic limitations; no admission follows.
  Independent CC review accepts the original narrow `.5.2` item-view criteria
  at R28; broader production and resource-family work remains open. Preserve
  E's manifest-only exported process-to-binary binding: no opened-executable
  identity is retained, and no process/runtime qualification follows.
- Wire entity-query discovery to a session-scoped entity resource consumer,
  binding the exact entity generation and complete current anchor. Require
  Query authority for the requested ID; preserve field presence and refuse
  stale references or output that exceeds the negotiated bound. Preserve semantic
  request IDs and pending goal history/custody on successful and denied reads.
  Four new focused cases and the complete four-case modern stdio MCP target
  pass, including the new entity lifecycle. Corrected old-production RED
  detects the missing route; earlier test/setup failures remain retained.
  Exact R29 full Rust qualification passes as recorded above. Broader
  fortress, historical, knowledge and memory resource families remain open.
- Clean private R28 `e7b93a4` passes all 43 qualification gates and the
  separate verifier on the November 1 nightly: each 5,140 debug plus 5,140
  release cases, 123 targets per profile, zero failed or ignored. Parent
  verifies the official receipt, all 26 critical cases, all 2,592 source files
  and the exact emitted development executable. No native or production/runtime
  admission follows.
- Actual live D inserts one reviewed bed order and advances exactly 1,200 ticks
  without overshoot. The early anchor-floor refusal preserves prepared custody;
  the second clock run is refused as stale before unpause. No bed is observed in the final rosters,
  the original goal remains false, and the positive nonempty item and late
  changed-source probes are unachieved. Preserve this unsuccessful result and
  undifferentiated refusal cause. Parent replay verifies all 105 exported files;
  actual five-file offline recovery and fresh full host cleanup pass. Independent
  review closes the scoped repairs only; `.5.2` and broader goals remain open.
- Preserve the failed R27 latest-nightly Clippy receipt and all raw output;
  full Rust tests, verifier, server build and live trial are unrun. Replace
  only the redundant lazy default-capability selection with its equivalent
  eager fallback. Independent review confirms omitted, explicit-empty and
  explicit requests retain their meaning. No warning suppression or gate,
  profile, budget, dependency or test change. The subsequent exact R28 source
  qualification above passes; the R27 attempt remains failed.
- Further review adds a pre-publication cursor/tick/state-identity floor guard
  to canonical Observe, including failed goal-acquisition cursor reservations.
  Three actual original-source regressions fail on late-error cursor rollback,
  backwards tick publication and old planning evidence below a consumed native
  tick. R27's repaired MCP library passes all 2,825 cases, zero failed or ignored,
  including prepared-runtime reconstruction over memory-backed codecs and
  forward progress. R27's newer-nightly qualification fails as recorded above;
  the exact R28 guard passes both full commands and the early live D probe.
- Preserve five executed regressions failing on unchanged R25 production code:
  failed Observe/source/budget, Query revocation/regrant, final presentation
  authority and unverified copied rows. Reopen `.5.2` and `.20` for discovered
  omissions; keep their prior positive evidence and failures intact.
- Invalidate optional item rows on failed acquisition, inspection and the real
  facade's pre/post-dispatch checks. Recheck optional authority after custody
  I/O and mask copied rows in unverified error/close output. Preserve original
  goal/history policy, recovery identities, effect authority and native wire.
- Bound direct TCP fixture setup, including transient nonblocking accept, with
  unchanged I/O and production limits. Fix the standalone reference checker's
  rustfmt sensitivity; eight Python source-shape/mutation tests pass. The
  outer-boundary mutant fails as intended; restored source passes all 2,822 MCP
  and 1,292 adapter library tests, zero failed or ignored.
- Clean private R26 `5ce23acd` passes all 43 qualification gates and the
  separate verifier: each 5,137 debug plus 5,137 release cases, 123 targets
  per profile, zero failed or ignored. Parent verifies exact source, receipt
  and emitted development executable. R26 does not qualify the later anchor
  change; exact R28 and its bounded live result are recorded above.

## 2026-10-31 — Prove the public item view against the actual game

- On qualified R25's exact development executable, Terra inserts one reviewed
  bed order and runs one bounded simulation slice: 1,031 of 1,200 requested ticks,
  zero overshoot. Thirteen modern MCP exchanges expose BED 540 through a fresh
  item view and same-anchor Query. Independently replay all 129 export members,
  native receipt/history chains and complete-roster ID/type/material evidence.
- Preserve Unknown for the removed original order and for public material
  classification, eligibility and attribution. Lua separately corroborates
  cherry wood; it does not independently supply subtype or stack. Operator
  workshop/manager setup earns no MCP prerequisite or fortress-competence credit.
- Verify actual B and C offline custody without stale rows, then independently
  check drained processes, closed port, restored plugins and both thirty-file
  saves. Close only scoped item-presentation `.5.2`; retain original production,
  workshop and admission gaps, failed runs and the excluded zero-request client.

## 2026-10-31 — Preserve bounded TCP test-peer lifetime and intended effects

- Reproduce two short-idle-timeout counterexamples within unchanged caller
  budgets, then separate peer polling from fixed overall deadlines. Preserve
  partial I/O, cancellation and joined shutdown; add scoped real-socket tests.
- Prove exactly one retained stopped commit before disk recovery and retain
  peer error/event diagnostics. Preserve the full maximum-roster, identity,
  attachment, digest, release and no-replay assertions.
- Pass the full 1,292-case adapter suite, then qualify exact clean R25
  `bc5bcdfe28a895f9334fd8a116270be36650ee3a`: 43 gates and the separate verifier,
  each 5,131 debug plus 5,131 release tests, zero failed or ignored. Independently
  verify all 2,591 files and seal the exact Cargo-emitted development executable.
- Reopen actual B custody with that executable while the game is off; preserve
  all five files and historical identities while withholding current goal and
  stale item rows. Close scoped test-reliability `.20`; preserve R24's verifier
  failure. The later bounded C proof is recorded above; no admission follows.

## 2026-10-31 — Prove bounded real output and retain the completion gap

- Expose bounded source/anchor-bound furniture candidates from the existing
  operations sample, independently of the original order's receipt link.
  Preserve original goal policy and history; withdraw stale or unauthorized
  rows and leave material classification and production attribution unknown.
  Frozen R24 passes twenty focused cases and all 43 qualification gates with
  5,127 debug plus 5,127 release tests. Preserve the separate verifier's two
  adapter failures; the required command set is incomplete and the driver does
  not build a server or start a new game trial. Strengthen peer diagnostics and
  intended-effect assertions under Bead `.20` without changing production limits.
- Through actual public MCP, insert one original bed order and execute two
  reviewed bounded runs on an explicitly operator-prepared fortress. Independent
  replay verifies the insertion, clock receipts, six complete observations and a
  new cherry-wood bed. Preserve the original goal as Unknown after the game removes
  its order, with semantic completion unproved and an unrelated job remaining.
- Document the separate native-plugin opt-ins after a retained pre-effect launch
  failure. Keep broader workshop/production Beads open; operator setup and source
  qualification do not establish autonomous fortress competence or admission.

## 2026-10-31 — Restore the positive native job-control path

- Replace job-control's unconditional unsupported/refusing adapter with a closed
  actual-SDK policy for four idle furniture-job types at their exact completed
  workshop. Validate the entire holder queue and preserve all original effect,
  witness, authority, replay and readback checks; change only suspension.
- Remove fictional lookup/support APIs from the native test doubles. Execute
  234 policy checks, the 397-assertion complete handler suite under UBSan and
  all three engine mutation checks. Preserve codec goldens and disclose the
  corrected stale throwing-setter expectation.
- Compile/link the changed plugin against the real DFHack 53.16-r3 SDK. Fix
  the independently found hidden-worker reference case with exactly one valid
  holder reference. Through actual public MCP, suspend and resume one real idle
  bed job; independently verify native receipts and unchanged captured fields.
- Qualify exact clean `7bba4d01debdc7069e081adb824a4ace4fa199b3` on TRJ:
  43 gates plus verifier, 5,124 debug and 5,124 release tests per command,
  zero failed/ignored and 2,591 unchanged source hashes. Preserve the earlier
  documentation and TS2 budget failures. Operator workshop/job fixture setup
  earns no MCP construction or production credit; admission remains unchanged.

## 2026-10-31 — Complete exact-source and fresh server qualification

- Qualify private clean commit `742e5a1dd419110979c42647ae7d88cb54696363`:
  43 local gates, verifier success, 17 announcement-source gates, and 5,124
  debug plus 5,124 release tests per aggregate, zero failed or ignored.
  Preserve all 2,591 source hashes and clean final custody.
- Complete four real Cargo routing fixtures and the actual fresh release build.
  Contract, doctor, demo and independent opened-descriptor receipt checks pass
  for executable SHA-256
  `112523b7c419f9fa45769b6e0a1426a1f65dcd8da15302092e6270ab0cb030c1`.
  Close the bounded composition, gate, lock, publication, cache, artifact-selection
  and executable-mode repairs with exact evidence; keep broader gaps open.
- Preserve earlier failed/interrupted runs, live procedural departures and the
  old unreconciled Prepared record. Source/artifact qualification grants no
  native campaign, registry, production-runner or runtime admission, and does
  not transfer to the later shared checkout or report updates.

## 2026-10-30 — Add immutable world resource consumers

- Expose current and exact retained world manifests plus bounded entity,
  relation, map-chunk and event pages through the existing session resource
  template, reusing canonical presence-aware observation rendering.
- Keep continuations on their original source hash, distinguish current from
  historical anchors, and explicitly refuse unknown or collected versions.
- Recheck current session authority and expiry on every historical read;
  enforce source/output budgets and truthful partial/absence coverage.
- Override the transport's default resource cache lifetime with private
  zero-lifetime hints so changing grants and continuity are checked afresh.
- Add ten focused unit cases, a real modern-stdio traversal/authority/budget
  regression, and an exact implemented/deferred URI inventory. Rust execution
  remains blocked by host memory exhaustion during the actual dependency build;
  no live qualification or production admission is claimed.
- Pass the complete production adapter Rust check on latest nightly. Attempt
  both required repository commands; each stops at nine existing bridge-contract
  findings in unchanged paths, without weakening those gates.

## 2026-10-30 — Restore guarded workshop and furnace job control

- Replace refusal-only native job suspension with a closed eight-type policy
  using actual DFHack job, holder, posting and activity fields. Restrict writes
  to idle, unposted, unclaimed jobs in completed matching production buildings.
- Validate the complete bounded job list and exact holder queue membership before
  the sole suspension-bit setter; retain stale-plan refusal, original-key replay
  and immediate exact readback without retrying uncertain effects.
- Replace fictional SDK test helpers and `BrewDrink` enum assumptions with
  SDK-shaped lists and real enum values. Pass 863 handler/engine assertions under
  UBSan, 26 independent SDK-guard assertions, and reject five safety mutations.
- Retain explicit source/test scope: matching-SDK build, live MCP execution and
  full Rust qualification remain open. No bridge method or admission is added.

## 2026-10-30 — Repair executable wiring and prove a bounded live run

- Repair laboratory composition, Tasks recovery of original receipts, partial-wait
  obligations, cancellation selection, bounded lossless schema discovery and stale
  executable gates. Keep all eleven tools and the exact owned FastMCP pin.
- Compile all 20 native plugins against the named real DFHack 53.16-r3 SDK;
  replace fictional job APIs with bounded lookup and fail-closed eligibility.
  Preserve complete UTF-8 citizen names from native code-page text.
- Fix shared clock safety: a modal UI pause cannot release ownership without a
  successful raw simulation-pause write/readback. Exercise failure vectors and an
  actual game modal; compile run/order-run/excavation-run against the real SDK.
- Execute an exact-source ten-tick MCP run with Sonnet 5.5 through NTM, zero
  overshoot, fresh pause observation and independent native ownership checks.
  Preserve the earlier forty-seven-of-sixty wall-limit stop and hidden-halo mining
  refusal as limits. No sustained fortress or production-goal success is claimed.
- Record real wiring gaps and owning beads in the current reality-check matrix.
  Private R7 passed static/Python/format gates and failed Clippy; full qualification
  repairs continue. Native development builds and live trials grant no admission.
- Preserve Terra's later read-only mining search and native hidden-context refusal;
  repair the 64-row spatial text overflow at pre-pagination packet budgeting.
  Direct explicit Python syntax caches under ignored `target/` so qualification
  preserves clean source custody. R15 supplies the source-bound tile replay.
- Close source-bundle Git subprocess pipes on success and failure; exercise actual
  child exit, bounded oversized output and ResourceWarning-denied cleanup.

- Record the source-bound R15 live 768-tile query success and independent cleanup.
  Preserve its subsequent six private-journal lock failures; add actual-backend
  duplicate-descriptor regression candidates under reality bead .13. Full
  qualification remains incomplete.

- Reproduce private lock retention after owner drop in all three actual affected
  backends, then add owned cleanup immediately at every adapter acquisition.
  Preserve nonblocking refusal, context checks and journal bytes. R17 passes Linux
  regressions 3/3, guard tests 3/3 and the 1,288-test adapter suite at 32 threads;
  full debug qualification passes 5,120 tests before a host reboot interrupts
  release compilation. R17b repeats the full checks; no interrupted pass credit.
  No sleeps, retries or test serialization are used to repair ownership.
- Record the missing live workshop construction prerequisite as reality bead .14.
  Furniture placement and queue insertion do not establish productive bed output.
- Route the separate protocol-1.1 source qualifier's explicit Python compilation
  cache under `target/` as well. Its actual nine-file syntax command dirties an
  isolated clean checkout before the repair and preserves it after; full updated
  source qualification remains separate from frozen R17 evidence.
- Bind announcement source qualification to the full clean HEAD snapshot before
  gates and before/after create-only publication. Reject hidden, untracked and
  HEAD drift; remove a published snapshot or receipt when final source reading
  raises. Actual missing-file regressions fail 0/2 before repair; the common
  receipt suite now passes 22/22 and announcement custody passes 5/5. Failed OS
  cleanup reports all possible residual identities as indeterminate; every Cargo
  gate is forced offline.
- Preserve the live7 workforce refusal: one Prepared record, no commit and
  byte-identical independent captures. Add a compact source-bound native review
  for semantic Plan/Explain and a truthful 65,535-byte transport fallback. Rust
  execution and the new live replay remain pending for those later bytes.
- Preserve R17b's passed 43-gate qualifier and both 5,120-test configurations,
  while rejecting its dirty combined completion: sanitized read-only tracker
  children generated eleven caches after qualification. Disable bytecode in the
  CLI itself and reject special journals before locking. The new regression is
  red before repair; all 64 tracker suites pass on macOS and isolated Linux after.
  Archive generated worker caches without relabeling the failed final check.

- Reproduce artifact qualification selecting a stale canonical executable after
  target-aware fake Cargo builds elsewhere. Build into a fresh private directory
  under the create-only run and refuse redirected target-triple output before
  checks. All 13 wrapper tests pass; real Cargo routing and full exact-source
  qualification remain pending under .18. No artifact or runtime admission follows.

- Prove R20 native review through actual public MCP Plan/Explain and one Terra
  confirmed assignment. Independent captures show exactly the intended MINE and
  detail-membership change, no other captured changes and zero ticks advanced.
  The original temporal goal remains active and unproven; retain that distinction
  and the disclosed export-order limitation. Full R20 debug passes 5,124 tests.

- Record R20's full 43-gate local qualifier and verifier success, with 5,124
  passing tests in each configuration. Preserve its separate source qualifier
  launch failure (0644 mode, exit 126); make all six direct qualifier entrypoints
  executable and add actual-process coverage, red before and 14/14 after.
- Prove one deliberately single-sample labor goal through actual public MCP,
  one separately reviewed ten-tick run and a fresh paused fulfilled sample.
  Independently replay native captures and durable history, preserving the
  configuration/message and pre-scan screenshot-copy departures. Broader
  fortress competence and productive work remain unproved.

## 2026-10-30 — Execute complete original labor and work-order plans

- Connect the durable frontier to the real workforce/1.17 and work-orders/1.10
  owners for up to sixteen labor and finite wooden-furniture-order steps.
- Select actual original StepIds while retaining the complete sealed parent,
  original keys, anchor, dependencies, terminal predicate and obligations.
  Preserve public single-step family entry points and reject selected multi-step
  reviews at legacy whole-goal monitors.
- Release dependencies only from exact verified labor receipts and retained
  semantic/native custody; recheck predecessor custody after the order dispatch
  marker and before insertion. Reject unsupported whole-plan requirements before
  family contact and prevent direct-trait route/dependency bypasses.
- Reconcile lost native replies with original keys under Query-only recovery,
  without restoring a commit permit or crediting queue insertion as goal proof.
- Carry narrower opening/current authority into native calls and retain decoded
  queue clock floors through later failures, including executor and durable pins.
  Preserve later-epoch historical Query while refusing cross-epoch native control.
- Pass 352 focused adapter tests, including 44 new groups across this executor
  and the preceding durable core, plus 58 public work-order and 47 public
  workforce regressions (457 passing tests across three targets). The original
  historical Query-after-restore regression is unchanged.
  Existing repository gates still block full qualification. Combined MCP exposure,
  native/live qualification and production admission remain open.

## 2026-10-30 — Recover mixed-family execution from the complete original plan

- Add a private bounded `DFMLDJ01` execution journal retaining the complete
  original PlanArchive, actual StepIds, family routes, keys and dependencies.
- Synchronize preparation, dispatch and cancellation intent before family
  contact; retain lost replies and accepted results without blind mutation retry.
- Require external journal/head/floor recovery pins and complete semantic replay;
  reject rollback, corruption and replacement without repairing history.
- Preserve Query-only recovery after original expiry, consumed tick floors,
  opening/current entity, map, risk and expiry intersections inside every family
  callback, shrinking deadlines, and room for complete
  prepare/commit or cancellation publication before native contact.
- Bind explicit confirmation to a process-local current-head review. Reopening
  never recreates that review or a native commit permit; family receipts never
  prove the whole original goal.
- Pass 260 focused adapter tests, including 20 new crash/authority/retention and
  real private-file groups. Full workspace/native/live qualification remains open.

## 2026-10-30 — Complete reviewed room-clock control through MCP

- Add explicit room-clock plan/commit/wait/cancel and exact-head history query
  through the existing eleven tools, preserving the complete original joint goal.
- Require fresh durable full-room readiness at both native boundaries without
  positive goal credit; reconcile each original run before another room sample.
- Add independently configured room clock custody, credential and opt-in in the 24-name
  environment. First-source bootstrap creates no owner under Query-only sampling;
  explicit Plan opens it under current retained authority.
- Keep room and ordinary furniture clock authority/identities separate and enforce
  reciprocal prepared/uncertain-history exclusion before native contact.
- Preserve original safety cancellation after source-file loss, goal expiry and
  zero future ticks; allow local monitor cancellation after failed initialization.
  Recheck final custody and render current proof flags, review and exact page limits.
- Pass 233 adapter plus 118 MCP/Agent Turn component tests (351 total), including 38
  new groups across the core and MCP increments. All 291/784 source inputs remain
  unchanged. Scoped formatting passes; required repository gates retain 77 existing
  findings across 63 unchanged paths and the source-bundle documentation blocker.
- Document configuration, complete run/reconcile/sample calls and recovery limits.
  Full ambient-runtime/native/live qualification and independent room-only startup
  remain open; no compatibility or production admission is claimed.

## 2026-10-30 — Bind reviewed native clock intervals to complete room goals

- Add a separate original-room clock owner over the existing native run engine,
  binding complete geometry, furnishings, first sources and private clock custody.
  Preserve exact ordinary-furniture clock identity bytes and public behavior.
- Require full paused joint readiness after durable intent and DispatchStarted;
  disclose known terrain deficits and refuse hidden/missing required terrain.
- Retain full readiness as room-journal frame 5, charge one original acquisition
  per boundary, and award no positive completion credit. Preserve original limits,
  first sources, terminal negatives and conservative floors through recovery.
- Reuse explicit one-use review, one-attempt unpause, original-key reconciliation
  and separately authorized safety cancellation after goal/future-budget expiry.
- Pass all 233 focused adapter tests, including 21 new groups, with 291 unchanged
  source inputs. Public MCP integration follows separately; full workspace,
  ambient runtime, native/live qualification and admission remain open.
- Document the joint readiness and clock contract without claiming excavation
  causality, present room usability, assignment or pathfinding.

## 2026-10-30 — Expose durable whole-room completion through MCP

- Add original-room start, inspect, one-sample observe/wait and local cancellation
  through the existing eleven tools, with complete geometry, furnishing assessments,
  terrain deficits and truthful Agent Turn guidance.
- Require exact acknowledged recovery pins, preserve original timing and sample
  allowance, and recheck source, monitor and optional clock custody after the last
  callback. Failed acquisition clocks narrow authority without positive proof.
- Preserve Query-only local cancellation after source loss within an open owner;
  support normal original-custody Recover/Offline reopening and explicit recovery
  release without introducing independent room-only recovery.
- Keep ordinary furniture clocks separately bound; unresolved clocks block fresh
  room samples. Reserve complete output and large-history replay within existing
  limits, including a tested 4096-cell, 20-sample journal over 2 MiB.
- Pass 212 adapter and 101 MCP/Agent Turn component tests (313 total), including
  17 new joint-room lifecycle groups. Correct new core test-helper validator
  findings while preserving incoming main work. Existing repository-wide gates
  still block full qualification; no ambient-runtime/native/live admission claim.
- Document exact configuration, public calls, recovery and historical proof limits
  in `docs/ROOM_COMPLETION_MCP.md` and the machine interface contract.

## 2026-10-30 — Prove original room terrain and furnishings together

- Add a durable joint room monitor with one complete original condition and
  stability streak, preserving every floor, wall and furnishing through recovery.
- Acquire one fixed receipt/map/operations/map/receipt bracket on one connection,
  with exact paused endpoints, full released pagination and no effect methods.
- Retain partial-read clock floors on errors; preserve immutable source history,
  acknowledged ancestry and local cancellation after original-file loss.
- Pass 212 focused adapter tests, including 24 new joint-core/native-reader groups;
  preserve the concurrent mixed-family dispatcher. The subsequent public MCP
  monitor routing is recorded above; no native/live or full runtime admission is claimed.

## 2026-10-30 — Coordinate sealed mutations across live native families

- Add one deterministic cross-family dispatch frontier over the existing control,
  dig, furniture, work-order and workforce routes.
- Recheck each semantic step's exact capability, risk and entity/map scope at
  preparation and commit without widening family authority.
- Record DispatchStarted before native commit; any lost or invalid commit result
  becomes indeterminate and blocks all later family work until the exact original
  idempotency key is reconciled.
- Permit a step to become pending again only when its family explicitly proves
  the durable dispatch marker was never sent. Prepared cancellation cannot undo
  an effect that may have started.
- Bind plan, family, dependencies, semantic keys and every frontier state into a
  stable domain-separated digest suitable for durable host handoffs.
- Keep all native I/O, preparation journals, confirmations and effect-specific
  verification inside the existing narrow family owners. Generic MCP execution
  wiring and live qualification remain separate.

## 2026-10-30 — Preserve original room geometry through native furnishing intake

- Add bounded `room_request` intake with complete floor/wall masks and all
  original furniture constraints; allocate the whole request or return shortage
  without partial custody.
- Retain room identity in new DFMFBD03 batches and original placement keys while
  preserving exact legacy generations and existing parent/origin storage bounds.
- Carry complete compact geometry and request identity through batch responses,
  current scope checks, immutable reopening and permanent stop.
- Pass 188 adapter and 84 MCP/Agent Turn component tests, including 16 new groups;
  preserve concurrent independent clock-recovery work on main. Ambient runtime,
  native/live and full release qualification remain unverified. The subsequent
  joint room observation increments are recorded above.

## 2026-10-30 — Recover original furniture clock runs without placement files

- Add `clock-offline` and `clock-recover` through the existing eleven tools,
  opening only acknowledged original monitor and run histories without placement
  custody, a furniture parent, native bootstrap, inventory capture or a new lease.
- Reconcile one original outcome with Query only; permit safety cancellation
  under separately retained/current canonical and native clock authority, with
  zero future ticks and no restored review, new run or construction sample.
- Match immutable first-source history after ordinary or readiness invalidation;
  preserve exact ancestry, consumed floors and final publication custody.
- Keep original cancellation reachable after later monitor loss, refuse native
  work after clock damage, and allow redacted explicit release after Query expiry.
- Make recovery Agent Turn recommendations respect offline/query-only modes and
  current authority, and remove follow-up actions from closed sessions.
- Pass all 79 actual-source handler/Agent Turn tests, including 19 new recovery
  groups. Combined with 113 adapter tests, 192 focused tests pass; all recorded
  compiler inputs remain unchanged. Full public/native wrappers type-check with
  nonexecuted ambient stubs. Full runtime/native/live qualification remains open.
- Preserve both concurrent readiness increments on main and document the paired
  recovery pins, 18 environment variables and seven operator modes.

## 2026-10-30 — Preserve original construction-monitor recovery custody

- Add exact external monitor/head recovery references backed by complete semantic
  replay and synchronized appends; accept valid descendants and reject rollback,
  unrelated or divergent history, file substitution and malformed trailing frames.
- Retain the first accepted operations source through later diagnostic source
  changes, so an invalidated monitor cannot substitute the original clock target.
- Preserve the private journal format, bounded verification and existing authority
  checks; this core prerequisite restores neither sampling nor clock control.
- Preserve incoming readiness core and MCP work; pass all 113 actual-source adapter
  tests (91 construction-plan and 22 bounded-run), including seven new recovery
  groups. All 276 adapter source hashes remain unchanged. Public recovery integration
  follows separately; full runtime/native/live qualification remains unverified.

## 2026-10-30 — Complete durable furnishing run/sample cycles through MCP

- Wire both clock guards to durable whole-plan readiness with write-ahead read
  intent, complete receipt-bracket acquisition and guarded acknowledgement.
- Charge original acquisition limits once, preserve a valid existing streak
  across reviewed finite intervals, and keep ordinary samples responsible for
  positive construction proof.
- Retain terminal negative readiness and source/stage/allocation floors through
  restart; refuse preparation or unpause when the original budget expires.
- Recheck placement, batch, monitor and clock histories after the final opening
  callback under prepaid bounds and the original deadline. Preserve the highest
  consumed tick in current and fallback clock metadata.
- Pass 106 focused adapter tests and 60 MCP/AgentTurn component tests, including
  whole 2/32-target multi-interval completion, tiny budgets, durable negatives,
  and final opening corruption. MCP ambient runtime uses nonexecuted stubs;
  full runtime/native/live qualification remains separate. Required repository
  gates still stop on unchanged baseline documentation/validation findings.

## 2026-10-30 — Retain furnishing readiness without crediting completion

- Add durable whole-plan readiness publication and replay using the original
  receipt, source, identity and fixed temporal checks.
- Charge each completed readiness acquisition to the original observation limit,
  preserve only an already valid stability streak, and give no positive proof
  credit. Retain contradictory and terminal outcomes across reopen.
- Expose a pure readiness probe for clock integration without a second
  hypothetical acquisition charge or premature completion claim.
- Withdraw a prepared review on all final publication failures, including
  deadline expiry before the final custody replay.
- Pass 104 focused adapter tests, including durable replay, readiness budgets,
  valid streak preservation, final publication expiry and 14 TCP fault tests.
  The offline component view retains all production bodies and selected tests;
  full workspace/runtime/native/live qualification remains separate. Preserve
  incoming clock and MCP work; host use of the new journal path follows.

## 2026-10-30 — Drive whole-batch furniture construction through MCP

- Add finite clock preparation, separately confirmed commit, original-outcome
  reconciliation, safety cancellation and exact-head history through the same
  eleven tools used for furniture placement and completion.
- Require the complete original batch, fixed construction monitor and an accepted
  operations sample before eager activation of independent run/1.13 custody.
- Keep game-time allowance zero by default, with separate operator authority,
  complete receipt-bracket readiness and current checkpoint policy per new run.
- Block completion behind unsettled clock history, withdraw unfinished stability
  on contradictions or failed publication, and retain exact recovery and consumed
  floors without restoring an unpause permit.
- Recheck final clock custody after worker runtime callbacks, retain joined-worker
  acknowledgement and pin the native connection credential at each call.
- Pass all 54 actual-source handler/Agent Turn tests, including 16 new integration
  and three helper groups, with unchanged compiler inputs. Complete async/native
  wrapper bodies type-check with nonexecuted ambient stubs. Combined with the core,
  147 focused tests pass; full runtime/native/live qualification remains unverified.
- Document operator activation, complete workflow and independent-source limits;
  preserve the broader mixed-action and verified checkpoint/restore gaps. Existing
  source-bundle/static repository gates and an uncached pinned FastMCP dependency
  continue to block full qualification.

## 2026-10-30 — Reviewed finite clock control for complete furnishing batches

- Retain the whole original furniture goal, complete Placed receipt set and
  independently accepted operations source under an original run/1.13 journal.
- Recheck complete current construction readiness after durable preparation and
  dispatch boundaries, with separate review and one finite unpause attempt.
- Preserve exact ancestral recovery, paired retained/current authority, consumed
  tick floors and safety cancellation after future budgets or goal time expire.
- Block fresh completion behind unresolved runs and withdraw unfinished stability
  after contradictory readiness, interrupted publication or reopening; retain
  terminal history and the unchanged fixed observation policy.
- Pass all 93 focused actual-source adapter tests, including 22 new clock and
  five new continuity groups; all 273 adapter Rust inputs remain unchanged.
  Full FastMCP/native/live qualification and production admission remain unverified.

## 2026-10-30 — Recover blueprint clock runs without original mining custody

- Add `clock-offline` and `clock-recovery` through the existing eleven tools,
  reopening exact monitor/run histories without the native dig journal, parent,
  terrain acquisition or a paused bootstrap.
- Permit original outcome reconciliation with Query only and separately enabled
  safety cancellation with retained named/native Clock authority; keep future
  game ticks at zero and permanently exclude new runs or restored review seals.
- Allow cancellation after post-startup monitor damage while refusing native
  work after clock damage; preserve unresolved histories on explicit release.
- Verify both histories after the final opening callback, preserve consumed
  floors and normal recovery shapes, and publish only historical coverage.
- Add 19 regression groups; pass all 100 focused MCP/Agent Turn tests and 59
  valid/205 invalid schema cases. Full wrapper bodies type-check with nonexecuted
  ambient stubs. Existing repository-wide gate failures and full native/runtime
  qualification limits remain explicit.

## 2026-10-30 — Recheck clock custody at the final publication boundary

- Reserve the final exact clock-history replay before work, then verify custody
  after the last runtime callback and before publishing the rendered response.
- Withdraw stale review/history success on a late change and preserve possible
  commit/cancel effects instead of implying the action can safely be retried.
- Carry the highest consumed shared authority tick into every clock handoff,
  including map captures rejected by later original-custody checks.
- Add three regression groups; pass 81 focused MCP/Agent Turn component tests
  and the current 59 valid/205 invalid query-schema cases. The full production
  build remains resource-blocked; native/live qualification is unclaimed.

## 2026-10-30 — Whole-blueprint clock control through the existing MCP tools

- Add explicit finite clock preparation, separate confirmed commit, one-query
  reconciliation, safety cancellation and paged history for the original monitor.
- Keep positive game time opt-in, independently scoped authority, current
  checkpoint policy, original deadlines/cadence and full sparse readiness.
- Block fresh completion while clock work is pending or unresolved; withdraw
  known-broken stability and retain exact recovery through failed publication.
- Preserve intended creation identity, reject mislabeled initialization and
  rollback, and never restore a recovered preparation's dispatch permission.
- Pass all 194 focused tests on the final restored source (122 adapter and
  72 MCP), plus 59 valid/205 invalid query-schema cases.
- Document the complete operator workflow and remaining independent-source,
  callback-stop and combined-restart limits.

## 2026-10-30 — Reviewed finite whole-blueprint clock core

- Bind the whole original excavation monitor, Designated receipt set and fixed
  temporal policy to independent run/1.13 custody and separately reviewed runs.
- Recheck fresh sparse terrain and original custody after durable intent/dispatch,
  then reverify the clock journal before native prepare or unpause.
- Preserve exact ancestral recovery pins, single-attempt dispatch, consumed clock
  floors, retained authority and safety cancellation after future budgets expire.
- Pass 122 focused actual-source adapter tests (100 blueprint and 22 bounded-run),
  including 19 new clock groups, with 390 compiler/source inputs unchanged.
  Full MCP/native/live qualification and compatibility admission remain unverified.

## 2026-10-30 — Withdraw unfinished monitor continuity after failed MCP publication

- Integrate local monitor withdrawal with failed request preflight, runtime and
  authority checks, failed publication, response overflow and timeout. Persist
  the reset before a later sample without renewing the original goal or deadline.
- Track owned-worker acknowledgement in each native session. A missing successful
  outer-runtime join withdraws unfinished monitor continuity and stale local
  preparation permission before the next request; client delivery is not implied.
- Preserve updated failure handoffs and mark older accepted target pages as
  historical after a source-invalidating acquisition.
- Preserve the incoming independent monitor recovery implementation. Add two
  handler regression groups, one runtime test and stronger source-drift assertions.
- Pass query-schema validation for 49 valid and 175 invalid cases. New Rust test
  execution remains unverified after SIGKILL and filesystem exhaustion blocked
  compilation; full FastMCP/native/live qualification remains unclaimed.

## 2026-10-30 — Preserve blueprint stability across contradictory and unpublished samples

- Clear the whole-project matching streak when capture bytes change at one
  paused tick. Equal-tick corrections and restarted observations cannot seed
  another streak; a later advancing tick must establish a new matching window.
- Add local withdrawal of unfinished progress after an enclosing runtime or
  publication failure. Persist the continuity reset before another read while
  preserving the original deadline, attempt budget and terminal history.
- Keep already consumed raw evidence and clock floors from an abandoned read,
  fence its publication, and preserve interruption and Query-only cancellation.
- Add four regression groups. Source review and formatting are complete; the
  attempted 51-case monitor run was blocked by SIGKILL during complete adapter
  test compilation. No new test execution, full-runtime, native/live or admission
  qualification is claimed at this checkpoint.

## 2026-10-30 — Independent whole-blueprint monitor recovery

- Add explicit `monitor-offline` and `monitor-recovery` opening modes that retain
  only the existing proof history and Query authority. Missing native journals
  and damaged blueprint parents no longer prevent historical monitor recovery.
- Preserve exact original monitor, source, binding and acknowledged-head checks;
  require no region, game connection or mutation authority. Offline inspection
  never appends; writable recovery additionally permits local monitor stops.
- Restrict schema discovery and all actions to history inspection and permitted
  cancellation. Recovered satisfaction stays historical, with native inventory
  and current terrain explicitly unverified. Original grant expiry still applies.
- Support verified close and explicit unverified recovery release after damage,
  preserving exact handoffs even when a durable stop outlives its failed response.
- Pass all 128 focused tests (77 adapter, 51 MCP), including nine new recovery
  groups; 388 adapter and 404 pipeline inputs remain unchanged. Independent async
  routing review passes. Full FastMCP/native/live qualification remains unverified.

## 2026-10-30 — Whole-blueprint excavation monitoring through MCP

- Expose exact monitor registration, one-capture sampling, head-pinned progress
  and separate cancellation through the existing eleven-tool mining server.
- Join every original terminal Designated receipt to the shared sampled-floor
  condition without equating independent map/dig generations or native effects.
- Create private history lazily at registration and carry learned clocks across
  exact restart. Keep deadlines and attempt limits fixed, preserve raw negative
  evidence and never restore current truth from historical satisfaction.
- Preserve local monitor inspection/cancellation through damaged native custody
  in an open session, and retain exact recovery through failures and release.
- Prepay complete source/history/final-audit/output work, enforce bounded socket
  deadlines and expose actionable selected-cell diagnostics and Agent Turns.
- Pass 119 focused actual-source tests (77 adapter, 42 MCP), including 58 new
  monitor cases; 402 pipeline inputs remain unchanged. JSON Schema 49/175 passes.
  Full FastMCP/native/live and release qualification remain unverified.

## 2026-10-30 — Durable whole-blueprint excavation monitoring core

- Bind complete original sparse geometry and all terminal Designated receipts
  to one independent typed map source and a fixed sampled completion policy.
- Evaluate the entire selected mask together: visible dry floors, no magma and
  cleared dig designations. Preserve unknown cells and exclude unselected gaps.
- Retain started reads, raw observations, published outcomes and local stops in
  a bounded private history with exact monotonic recovery and clock floors.
- Add a foreground bounded wire reader using the unchanged map/1.5 protocol;
  no new native command, automatic game advancement or mutation authority.
- Pass 77 actual-source adapter tests (47 new and 30 prior blueprint cases),
  with 388 compiler/source hashes unchanged. Full MCP/native/live qualification
  and compatibility admission remain unverified.

## 2026-10-30 — Execute retained sparse mining projects through MCP

- Join complete original floor blueprints to the existing one-attempt native
  coordinator through the frozen eleven tools. Fresh observations and distinct
  paired reviews advance only the exact next child after a verified prefix.
- Expose complete head-pinned progress pages, canonical layout export, exact
  offline/reconcile/control restart and permanent stopping. Preserve original
  native query/cancel recovery through damaged or stopped parent custody.
- Retain current source tick and sequence on Control reopen before issuing a
  new lease; reject expiry/rewind and restore no preparation permit. Preflight
  predictable initialization refusals before exclusive native file creation.
- Preserve project and native identities through error/overflow/close/final
  runtime failures, and expose actionable next/pending work in Agent Turns.
- Pass 79 focused final-source Rust tests and 39 valid/145 invalid JSON Schema
  cases. Full FastMCP/native/live and release qualification remain unverified.
  Multi-region designation does not establish excavation completion.
- Required gates reproduce the unchanged source-bundle failure and 76 baseline
  static findings, with no new findings from this feature.

## 2026-10-29 — Durable Rust sparse mining projects

- Added exact floor-blueprint geometry and immutable native source/journal-bound
  parent custody, with Python-compatible mask/layout digests and deterministic
  region partitioning that preserves every unselected gap.
- Added complete original-prefix auditing, independently reviewed child keys,
  permanent durable stopping and exact monotonic recovery identities. Historical
  designation remains separate from excavation completion.
- Added one-pass bounded complete mining-journal inventory and scoped private
  storage without broadening existing grants or changing native protocol bytes.
- Verified 75 actual-source model/store and adjacent regression tests. This
  checkpoint does not claim MCP integration, native/live or release qualification.


## 2026-10-30 — Advance original mining goals with reviewed finite runs

- Join the exact original Mine plan and verified designation to optional,
  independently pinned run/1.13 clock custody and exact private restart identity.
- Expose closed run planning, distinct confirmed `commit(scope="run")`, safe
  `cancel(scope="run")` and reconcile-before-sample wait through the eleven tools.
  Preserve original deadlines, cadence and stability; never chain runs or retry
  an unknown designation or unpause.
- Qualify pending dry visible Mine walls independently for advancement while
  preserving unknown canonical excavation truth. Recheck fresh original terrain,
  custody and authority after durable intent and immediately before native effect.
- Default explicit positive-clock sessions to 3 GiB logical work, keep ordinary
  sessions at 2 GiB and respect caller limits. Refuse known underfunding before
  clock contact; retain current-proof suppression, clock floors and safety pause.
- Preserve original/clock handoffs through errors, overflow and close. Permanent
  native clock loss stays indeterminate and offers release instead of futile waits.
- Pass 102 actual-source component tests without warnings, with 285 input hashes
  unchanged. The welfare increment also passes 14 current cases, for 116 total.
  Required checks reach the existing source-bundle gate and 76 baseline static
  findings; full FastMCP/native/live qualification and admission remain unverified.

## 2026-10-29 — Compare historical citizen welfare through MCP

- Add `historical_citizen_welfare` to the existing spatial query surface, joining
  two exact retained citizen observations by canonical identity and generation.
- Report complete endpoint stress-category and job-availability distributions,
  category movement, elevated-category crossings and observed status changes,
  with bounded pages and runnable exact-record citizen inspections.
- Bind continuations to the session, archive head, endpoints, selection and work
  allowance. Preserve unknown evidence, new/absent citizens and replaced
  generations; never infer health, causes, raw stress or continuous trends.
- Pass all 13 focused actual-source model/archive-handler tests and the schema
  catalog regression (14 cases in total).
  Historical queries perform no native capture, journal write or watch advancement.
  Full FastMCP transport/native/live qualification and admission remain unverified.

## 2026-10-28 — Preserve mining review and proof across foreground failures

- Give newer trusted requests bounded wall windows on the original preparation
  socket, allowing human review without renewing its permit, credentials, prepaid
  bytes or game-time deadlines. Failed or regressed requests cannot revive it.
- Withdraw cached current proof after every failed wait, including facade failures
  before polling. Persist the interruption before continuing unfinished stability;
  preserve historical achievement and the exact original recovery handoff.
- Remove cached proof/preparation/enablement flags from retained response
  references, including overflow. Keep verified results and historical identities
  distinct, and update the operator workflow and commit guidance.
- Pass all 66 current-source runtime/native/presentation/Agent Turn/adapter cases
  without warnings. Preserve the concurrently landed mining implementation;
  full FastMCP, native/live and admission qualification remain unverified.

## 2026-10-28 — Match MCP registration to documented tool calls

- Explicitly register the eleven dotted `fortress.*` names in the spatial/citizen
  and original-mining servers, matching the calls emitted in agent guidance.
  The pinned FastMCP macro otherwise uses underscore-separated Rust names.
- Preserve every handler body. Replace forbidden job-test fixture helpers with
  propagated errors and rerun all 15 model/archive-handler/schema cases successfully.
  Macro/stdio execution remains subject to the existing full-build limitations.

## 2026-10-28 — Expose original mining and recovery through MCP

- Add a dedicated eleven-tool original mining server with exact observation-bound
  tile pages, closed Mine requests, paired explicit confirmation, original-goal
  waits, offline/reconcile recovery and separate goal/session cancellation.
- Compose independently pinned dig/1.16 and spatial/1.8 sources with current
  operator gates, protected shared-block leases, exact private custody and a
  pre-seal development checkpoint exception. Retain the bootstrap clock floor.
- Preserve original recovery identities through partial publication, errors,
  response overflow and session release. Recovered preparation cannot regain
  dispatch; explicit retirement needs current authority but no new checkpoint.
- Prepay all three source acquisitions and complete native/archive work before
  original publication. A larger source that exceeds the allowance leaves its
  original observation reusable. Use the existing 60-second wall cap as the
  default review window without renewing native connection deadlines.
- Pass 113 distinct focused cases: 17 adapter, four world, 15 joined runtime,
  nine native, 13 presentation, seven facade and 48 adjacent regressions. The
  final 35 affected cases pass without warnings with 413 inputs unchanged;
  ordinary joined workflows use the actual facade-reserved default budget and
  a realistic 430 MiB allowance for each evidence acquisition.
- Preserve concurrent historical job-progress work on main. Final required
  verification reaches the existing source-bundle assertion; integrated static
  qualification reports 77 findings (76 starting findings plus one incoming
  job-progress test expectation), with no mining findings. Failed development
  checks do not establish release qualification.
- Document complete operator setup, exact restart, original receipt versus
  current excavation, bounded history initialization and nonrenewable review
  deadlines in docs/SEMANTIC_DIG_DEVELOPMENT.md. Game-time advancement remains
  external; full FastMCP/native/live qualification and admission are unverified.

## 2026-10-28 — Compare source-qualified historical job progress

- Add `historical_job_progress` through the existing spatial query surface for
  exact retained job identities, worker/suspension changes, holder construction
  stages and complete qualified attached-item membership.
- Preserve disappearance/reused IDs and unknown evidence without inferring
  completion, stalls, intermediate events or current game freshness.
- Return whole-roster summaries, bounded whole-row pages, session-bound cursors
  and runnable inspections pinned to each exact archived record/generation.
- Pass 15 focused actual-source model, archive-handler and schema tests; queries
  do not capture native state, write journals, advance watches or confer admission.
  Full FastMCP transport/native/live qualification remains unverified.

## 2026-10-28 — Execute and verify exact original mining goals

- Bind one original canonical Mine rectangle to the existing dig/1.16 native
  owner, with independently qualified spatial/1.8 terrain, the unchanged
  original key/archive/predicates/deadline and a paired confirmation seal.
- Persist original/native association before native intent; revalidate source,
  authority and exact journal custody after the dispatch marker, before RPC.
  Reopening and lost replies reconcile the original key without redispatch.
- Prove current excavation only from fresh known dry, undesignated target floors
  and the original temporal goal. Keep historical designation, current terrain
  and historical first achievement separate.
- Retain original proof history in bounded private custody. Replay requires new
  historical policy, clears current truth and breaks unfinished stability while
  preserving consumed source/cursor/tick floors and the original deadline.
- Add exact-create versus exact-reopen native journal custody and reject broad
  or laboratory evidence at the typed terrain-routing boundary.
- Pass all 17 actual adapter integration cases without warnings; all 396 recorded
  source/fixture/manifest inputs stay unchanged across the final run. The four
  prerequisite world cases also pass. Dedicated MCP composition follows in the
  next checkpoint; full-server/native/live qualification remains unverified.

## 2026-10-28 — Preserve unknown terrain in partial canonical observations

- Represent hidden, unsupported and unobserved cells explicitly in canonical
  terrain chunks, while keeping observed cells usable by original predicates.
- Preserve unknowns through canonical snapshot encoding. Even broad evidence
  coverage cannot prove those cells or their negation; reference mutation cannot
  fill them and spatial indexing cannot infer safe paths through them.
- Pass all four actual-world regression cases without compiler warnings. This
  supplies the terrain prerequisite for original-plan mining integration; it
  does not itself expose a new native mutation or MCP workflow.

## 2026-10-28 — Expose original construction clock runs through MCP

- Add explicit zero-default game-tick negotiation, closed original-bound run
  planning, distinct clock confirmation and safety cancellation through the
  existing eleven tools. Wait reconciles retained runs before fresh construction
  reads; it never starts or chains an interval.
- Recommend cadence-sized intervals only for qualified viable unfinished work,
  within the original deadline and negotiated allowance. Stale prepared clock
  reviews offer explicit retirement instead of an endless wait loop.
- Preserve original and clock recovery identities and independent effect
  uncertainty through errors, response overflow and session close. Historical
  clock receipts never become current pause or original completion proof.
- Document optional operator clock setup, complete tool flow, exact restart and
  the bounded 64-sample workflow. Keep development responses free of production
  admission provenance.
- Pass 122 distinct focused tests: 74 actual-source runtime/native/presentation/
  facade cases, 26 adapter integration cases and 22 existing bounded-run cases.
  All pass without warnings; the final test-helper correction also passes its
  16 affected owner cases. No full FastMCP, native/live or admission claim.
- Run required repository verification and static development qualification.
  The source-bundle assertion and all 76 static findings are identical to the
  prior baseline; there are no new findings. These failed development runs do
  not establish release qualification.

## 2026-10-28 — Advance original furniture goals with separately reviewed finite runs

- Compose original construction proof with the existing run/1.13 coordinator,
  binding the unchanged original plan, archive, placed building/item/job receipt
  and absolute deadline to each separately confirmed finite interval.
- Permit known viable unfinished construction while rejecting unknown identity,
  blocked work and failed predicates. Recheck original custody and independently
  authorized construction evidence after durable clock intent, before unpause.
- Retain at most 64 runs, one unsettled run, exact private clock custody and
  offline recovery. Lost replies never permit another unpause; independent safety
  cancellation remains available without renewed placement authority or deadline.
- Keep current construction proof separate from clock receipts and retain consumed
  clock/canonical floors across refusals. A denied evidence policy cannot strand
  safety cancellation behind an unpublished observation cursor.
- Raise bounded goal history from 512 to 1024 events, retaining its 64 MiB file
  limit; allow explicit logical replay budgets up to 64 GiB, with 2 GiB unchanged
  as the default. Add full 64-run stability/recovery and denial-path regressions.
- Final core/native evidence passes 97 focused tests: 49 actual-source runtime/
  native, 26 adapter integration and 22 existing bounded-run cases. The full
  64-run stability/recovery and private-authority-floor regressions pass. The
  subsequent MCP checkpoint records complete integration evidence; no full
  FastMCP, native/live qualification or admission is claimed.

## 2026-10-28 — Expose original furniture construction through MCP

- Join canonical spatial observation, exact-item original Build planning, paired
  native review, one confirmed placement, fresh construction proof and exact
  restart recovery through the frozen eleven tools.
- Add the operator-owned spatial/1.8, furniture/1.19 and operations/1.4 composition
  with independent source pins, bounded credentials, exact private create/reopen
  custody, scoped construction leases and a pre-seal development checkpoint exception.
- Preserve original recovery identities through partial publication, errors,
  close and response overflow. Reopening cannot restore placement permission;
  placement, current construction and historical first achievement stay distinct.
- Read original archives at the retained authority-clock floor without rewriting
  their anchors or reviving expired Query authority. Prepay complete fresh
  opening work before creating custody files.
- Pass 63 focused tests: 32 actual-source MCP runtime/native/presentation/facade,
  26 public adapter/goal/archive and five concrete spatial wire cases.
  Full FastMCP executable/stdio, native/live qualification and admission remain
  unverified; existing repository-wide static gate failures remain.
- Document setup and the complete tool/recovery workflow in
  docs/SEMANTIC_FURNITURE_DEVELOPMENT.md. Game-time advancement remains external.

## 2026-10-27 — Prove original furniture construction and retain its history

- Assess current construction from the exact original Placed receipt and one
  complete bracketed operations/1.4 capture, including original building/item
  identity, installed-item state and construction/removal/job-link evidence.
- Project a narrowly scoped certified derivation onto the original action's
  canonical building identity; require independently issued evidence policy.
- Evaluate all original postconditions, terminal/failure predicates and temporal
  requirements, preserving the original deadline, cadence and stability. Current
  truth, historical first achievement and physical completion remain distinct.
- Persist read intent, complete samples and proof publication in a bounded
  owner-private history. Restart independently reauthorizes history, clears
  current proof and breaks unfinished stability without replaying placement.
- Retain consumed source, clock, allocation and retired-identity floors across
  denied proof and monitor replacement. Failed publication cannot expose a new
  current-goal result.
- Actual adapter compilation and 28 focused tests pass: 23 joined placement,
  goal and recovery cases plus five concrete spatial wire-codec cases. Native,
  live and compatibility admission remain unqualified.

## 2026-10-27 — Execute original furniture plans with durable placement recovery

- Carry one original canonical single-tile bed, chair or table `Build` through
  paired native review, explicit confirmation, preparation and one placement.
- Retain the full original plan archive identity, action key, native journal,
  source witness, predicates and obligation; native insertion remains historical
  evidence and does not establish original construction completion.
- Add exact association custody and restart/query/cancel recovery. Lost replies,
  missing native records and repeated calls cannot create redispatch permission.
- Reacquire complete spatial/1.8 evidence after durable Intent and immediately
  after DispatchStarted, before the native writer; keep independent source
  generations, caller policy and consumed source floors across refusals.
- Add focused lifecycle/fault coverage and a concrete bounded spatial RPC owner.
  Production adapter compilation and 20 focused tests pass: 15 coordinator/guard
  cases and five concrete spatial wire-codec cases.
  Native/live qualification and compatibility admission remain unverified.

## 2026-10-27 — Expose separately confirmed labor clock runs through MCP

- Add explicit zero-default tick negotiation, closed run planning, distinct
  clock confirmation and safety cancellation through the frozen eleven tools.
- Recommend cadence-sized intervals within the unchanged goal deadline and
  negotiated allowance; reconcile unsettled clocks before paused labor reads.
- Preserve clock uncertainty and exact recovery identities through failed calls,
  bounded response overflow and session close without restoring control grants.
- Document the complete operator setup, run workflow and exact restart,
  including the new stability samples required after an unfinished streak resets.
- Pass 146 actual-source runtime/presentation/facade tests, including the full
  Action dispatch and nine facade boundaries, plus 97 core/adapter/bounded-run
  tests. Full FastMCP, native/live qualification and admission remain unverified.

## 2026-10-27 — Advance original labor goals with separately reviewed finite runs

- Compose original labor proof with the existing run/1.13 coordinator, preserving
  original citizen identity, assignment receipt, deadline, cadence and stability.
- Separate clock permission and source scope from labor permission and named
  fortress evidence; recheck original custody and fresh labor after dispatch intent.
- Keep one unsettled run, at most 64 retained runs, exact offline recovery and
  independent safety cancellation. Never repeat unpause after an uncertain reply.
- Reject labor observations older than retained clock evidence before publishing
  goal proof, and suppress current-goal display while the clock may be running.
- Add exact create-versus-reopen clock-file custody and pass all 49 actual-source
  workforce runtime tests, including 64 samples through 63 finite runs and exact
  offline recovery. Existing suites also pass 28 core, 47 workforce integration
  and 22 bounded-run cases. Full FastMCP, native and live remain unqualified.

## 2026-10-27 — Carry original labor goals through the complete MCP loop

- Register the dedicated eleven-tool semantic workforce development server:
  canonical citizen pages, original labor planning, paired confirmation, single
  assignment, fresh goal verification and exact offline/reconcile recovery.
- Keep new recovery identities and history presence through partial-operation
  failures, close and output overflow without promoting cached proof or authority.
- Exercise the full 32-citizen goal against a 128-citizen roster and correct its
  bounded archive profile to 1,024 nodes; the 64 KiB byte limit stays unchanged.
- Support explicit bounded replay allowances for long histories while retaining
  the 2 GiB ordinary default. A 64-sample goal recovers with an 8 GiB logical-work
  allowance; insufficient allowance refuses safely, and no assignment is replayed.
- Pass 114 distinct current-feature tests, including 43 actual-source runtime,
  presentation and facade-boundary cases. Full FastMCP executable/stdio, native
  and live qualification remain outside this component evidence.
- Document configuration, closed tool inputs, the separate proof claims,
  fixed custody, loss recovery, capacity and explicit work-budget negotiation.

## 2026-10-27 — Execute and recover original native labor goals

- Join canonical citizen acquisition, a closed `SetLabor` compiler, paired native
  review, exact original-plan custody and durable goal history in one Rust owner.
- Preserve original canonical identities, deadline, cadence and stability across
  restart; keep historical assignment evidence separate from current labor truth.
- Require exact journal and capsule identities, explicit creation/reopen modes,
  complete retained-file discovery and independent native source pins.
- Revalidate original custody and current authority after dispatch intent and
  before native assignment. Lost replies and Unknown outcomes never redispatch.
- Pass 85 distinct adapter/core tests, including ten joined runtime fault and
  lifecycle cases. Dedicated MCP dispatch is the next increment; full server,
  native and live qualification remain unverified.

## 2026-10-27 — Advance native work-order goals through reviewed MCP runs

- Expose finite run planning, separate confirmation, reconciliation and safety
  cancellation through the existing eleven tools. Keep explicit game-tick
  budgets and independent production/clock authority.
- Recheck live runtime cancellation and I/O capability at native source/effect
  boundaries, including after durable dispatch intent and before unpause.
- Retain original and clock identities through opening failure, close and
  overflow. Recommend cadence-sized continuation for already-true goals that
  still need stable samples or physical quiescence.
- Document complete operator setup, both review seals, lost-reply recovery and
  callback-bound limits in the native goal guide and MCP surface.
- Pass 155 distinct ordinary adapter and actual-source tests, including seven
  joined runtime scenarios and six verbatim facade-boundary cases. Full FastMCP
  handler/stdio, native and live qualification remain unverified; see status.

## 2026-10-27 — Bind bounded simulation runs to retained native order goals

- Compose the real conditional-run coordinator with exact original plan,
  archive, insertion receipt and deadline; separately seal and confirm each
  finite run, retain uncertainty and forbid repeated unpause after dispatch.
- Support further finite samples for completed inactive orders when original
  stability or independent physical quiescence still needs evidence.
- Add optional independent clock authority, exact private-journal reopening,
  source pins and offline recovery without native credentials.
- Pass 145 distinct real-adapter and actual-source component tests, including
  six complete joined runtime scenarios. Full MCP compilation remains blocked
  by upstream compiler SIGKILL; no native or live qualification is claimed.
- Stage clock-aware Agent Turn projection; public MCP run dispatch follows in
  the next integration increment.

## 2026-10-27 — Execute and recover original native work-order goals through MCP

- Add a dedicated eleven-tool development server joining exact canonical
  observation, finite furniture-order compilation, paired semantic/native
  review, one confirmed insertion and durable original-goal monitoring.
- Preserve original deadlines, cadence, stability and action identity across
  exact archive recovery. Separate insertion certainty, historical achievement,
  current goal truth and physical quiescence; unknown outcomes never redispatch.
- Add closed operator source acceptance with independent profile incarnations,
  four separate credentials and fixed minimal evidence scopes. Keep paths,
  source policy and production authority outside MCP arguments; reject ambient
  production admission and unrelated profile state.
- Expose bounded complete Agent Turn handoffs, authority-aware next steps,
  explicit offline/reconcile/control modes and resource release after fencing.
  Retain recovery identities even when cancellation follows successful work.
- Pass 28 focused tests across the actual runtime, Agent Turn projection and
  verbatim facade-boundary helpers. Verify concrete session ownership is Send.
  The full MCP check was killed while compiling upstream `asupersync`, before
  reaching the server; exact evidence scope is in `IMPLEMENTATION_STATUS.md`.
- Document configuration, review/commit/polling and restart in
  `docs/SEMANTIC_WORK_ORDER_MCP.md`; retain the existing wire and custody formats.

## 2026-10-26 — Execute semantic actions across independent native profiles

- Stop equating canonical-read and mutation-plugin incarnation numbers during
  labor, work-order and furniture resolution. Preserve both exact identities,
  source witnesses and original sealed plans independently.
- Pin the workforce incarnation across failed citizen reads and invalidation;
  reject replacement of either original source before native effects.
- Exercise distinct generations throughout prepare, commit, original-goal
  monitoring and durable recovery. Pass 20 routing, 58 semantic work-order and
  46 semantic workforce tests, including reload and no-redispatch regressions.
- Retain the recorded component-only evidence scope and existing native wire,
  journal, plan and production-admission boundaries.

## 2026-10-26 — Inspect citizen welfare and elevated stress

- Add `mode: "welfare"` and `citizen_welfare` to the existing spatial/1.8 query
  surface. Rank observed stress categories with current job assignments and
  social-preserving availability from the same complete strict-citizen capture.
- Support live, exact-record historical and offline archive queries with
  source-bound pagination and inspection drilldowns. Retain current Query
  authority, source fencing, journal custody and bounded response/work budgets.
- Add living-citizen elevated-stress attention under the separately versioned
  `dfmcp.spatial-situation/2` policy. Explicitly leave needs and physical health
  unknown; filtered absence does not prove population wellbeing.
- Pass the 189-test actual-source MCP semantic harness, including nine welfare,
  twelve pure situation and four workforce schema/pagination cases. Full server,
  facade and stdio runtime tests, DFHack and live-game execution remain outside
  this evidence. Exact scope is recorded in `IMPLEMENTATION_STATUS.md`.

## 2026-10-26 — Plan explicit future food and drink reserves

- Accept `reserve_until_tick` in the existing production blueprint. Budget
  consumption through the requested future tick and actual work horizon,
  including currently satisfied minima, finite ingredients, queued work,
  shared labor and authorized workshop setup.
- Keep action deadlines tied to work. Seal the absolute tick into a distinct
  compiler generation and `/3` continuation source while preserving historical
  request bytes and refusing implicit renewal of expired horizons.
- Expose a separate forecast at the exact requested tick. Preserve action
  completion, current stock truth and predicted future stock as distinct results.
  Wire the forecast through ordinary planning and reviewed stale-plan replay.
- Admit already-satisfied original stock minima through a typed laboratory-only
  reserve rule. Require qualified production stock predicates and registered
  production/setup actions; seal the bounded horizon while preserving prior
  valid plan bytes and ordinary planner refusal behavior.
- Pass all 203 intent library tests and the same 189-test focused MCP harness,
  including twenty reserve-specific cases and six new intent admission groups.
  Public-facade regressions are present but unexecuted. Exact scope and the
  remaining full-server/native/live limits are in `IMPLEMENTATION_STATUS.md`.

## 2026-10-24 — Complete food and drink reserve plans with finite ingredients

- Add `starter_fortress_provisions`, combining twenty observed PLANT inputs
  with the existing shared workforce and finite workshop WOOD. Preserve the
  prior scenarios and saved-source interpretation.
- Budget new FOOD/DRINK production jointly after active original orders, then
  include every batch added for consumption reserves and workshop setup. Keep
  ingredient use separate from direct metabolism and finite construction logs.
- Expose current Observe-gated session, world and original-action ingredient
  reports, plus separate predicted forecast accounting. Suppress low-stock
  remedies whose necessary inputs are short or unknown, without inventing a
  shortage when a target is already met.
- Preserve independent stock-goal truth, completed-action proof and physical
  quiescence. Continue interrupted reserve plans after canonical reopening
  without refunding consumed inputs or repeating completed output. Document
  the complete workflow in `docs/LAB_FINITE_PROVISIONS.md`.
- Pass all 191 focused MCP semantic-module tests, including 15 new provisions
  cases; the complete focused set has 488 passing tests. Production Clippy
  passes under the three recorded baseline allowances. Full server/facade
  compilation and repository qualification retain the recorded limitations.

## 2026-10-23 — Consume finite ingredients for brewing and cooking

- Add explicit `plant-provisions/1` food-order semantics over the shared
  workforce. Each completed brewing or cooking batch consumes one qualified
  PLANT and produces five existing output units; both families share stock.
- Gate input before worker selection and again at causal completion. Preserve
  earned partial work through shortages, cancellation and recovery without
  inventing refunds or output. Retain original per-order consumption history.
- Budget new and already queued food/drink batches jointly, bind original
  inventory in sealed steps, and require full consumption proof at action
  completion. Preserve legacy worlds and explicit caller deadlines. Document
  the bounded model in `docs/LAB_FINITE_PROVISIONS.md`.
- Pass 197 intent, 17 dispatcher and 83 laboratory unit tests, including
  wrong-family cancellation ownership, material conservation and original
  generation proof. Production Clippy passes under recorded baseline
  allowances; existing repository static gates still block full qualification.

## 2026-10-23 — Run complete room and production goals with finite workshop inputs

- Add the opt-in `starter_fortress_economy` scenario, composing finite shared
  labor and workshop materials while preserving earlier scenario bytes.
- Budget actual new workshop setup and queued carpentry before admitting
  furniture or food/drink production. Reuse completed workshops without a new
  input, and charge shared setup once.
- Expose current Observe-gated construction inventory and exact per-action
  allocations, plus separate predicted forecast reports. Require completed
  material custody before certifying the original plan goal.
- Recover cancelled workshop allocations and permit a newly reviewed room
  continuation to reuse the released site and refunded input exactly once.
  Document scenario selection, reports and interrupted projects in
  `docs/LAB_WORKSHOP_MATERIALS.md`.
- Pass all 176 focused MCP tests, including nine new material-economy cases;
  the complete focused set has 451 passing tests. Production Clippy passes
  under the existing baseline allowances. Full server/facade compilation and
  repository qualification retain the recorded limits.

## 2026-10-23 — Allocate finite materials for workshop construction

- Add an explicit `wooden-workshops/1` model composed with finite shared labor.
  Still, Kitchen and Carpenters construction allocates one qualified WOOD log
  at dispatch, consumes that held allocation on completion and refunds it
  exactly once on unfinished cancellation.
- Preserve legacy Build semantics and reject unsupported material selectors,
  missing or ambiguous stock, malformed custody and implicit history upgrades.
  Bind physical ownership and completion proof to the original allocation.
- Check joint workshop, new furniture and queued carpentry demand before
  sealing an action program. Require material completion in sealed Build proof
  while preserving explicit deadlines. Document the bounded model in
  `docs/LAB_WORKSHOP_MATERIALS.md`.
- Validate original material custody at dispatcher and laboratory proof
  publication, including generations and the unique stock-ledger domain.
  Pass 177 intent, 15 dispatcher and 83 laboratory unit tests; focused intent
  production Clippy is clean under the recorded baseline allowances. Full
  repository qualification remains blocked by the existing static gates.

## 2026-10-24 — Run room and reserve goals through the shared workforce

- Add the opt-in `starter_fortress_workforce` scenario while preserving the
  canonical legacy starter. Existing connected/provisioned room and production
  requests use its finite mining, construction and production workers.
- Expose bounded current assignments and shortages at session opening,
  observation and per-action progress, gated by current Observe authority.
  Keep forecast scheduling explicitly predicted and avoid carrying an old
  workforce report into a later orientation packet.
- Include queued construction in consumption-aware production reserves and
  bind the exact same horizon into sealed obligations. Require reviewed replay
  after changed anchors for every plan using global workforce reads.
- Execute all 167 focused MCP module tests, including seven new whole-workflow,
  shortage/resume, reconstruction, authority and consumption-horizon cases.
  Production Clippy passes with the documented existing world-source allowances.
  Two new full-facade tests remain unexecuted; exact validation limits are in
  `IMPLEMENTATION_STATUS.md`.
- Document the scenario, report semantics and authorized labor remedy in
  `docs/LAB_SHARED_WORKFORCE.md` without adding a top-level tool.

## 2026-10-23 — Share finite labor across construction and production

- Add the explicit `shared-workforce/1` laboratory model. Mining, building,
  exact-item furniture installation and registered production now compete for
  one qualified living-worker pool, with deterministic maximum matching and
  completed-workshop limits. Unstaffed work cannot accumulate elapsed service.
- Preserve original world semantics without the marker. Stamp new activities
  and refuse unknown model versions, misplaced declarations, implicit migration
  of active work and removal of a model with surviving task history.
- Expose pure current assignments, labor/capacity blockers and bounded remaining
  service. Retain causal completion, cancellation, restart and budget behavior.
- Account for queued and peer work in synthesized plan deadlines; retain exact
  caller obligations and bind the model into temporal preconditions. Document
  the complete-domain replay requirement and reference-model limits in
  `docs/LAB_SHARED_WORKFORCE.md`.
- Execute all 157 actual-source intent unit tests, including 20 new workforce
  and planning groups. Intent production Clippy passes with the documented
  existing world-source lint allowances. Exact scope and limits are recorded
  in `IMPLEMENTATION_STATUS.md`.

## 2026-10-23 — Build and resume entrance-connected furnished rooms

- Add `connected_room_v1` through the existing blueprint input, preserving the
  complete item-backed room request plus an observed entrance and bounded
  routing domain. Route deterministic safe connectors around current buildings,
  planned workshops, furniture and bedroom wall rings; excavate only the exact
  selected cells and order placement after all required circulation.
- Seal the complete room and current access proof as one terminal goal. Reuse
  open corridors, retain work-scaled dig deadlines and keep ordinary room
  compiler sources and seals unchanged.
- Extend `continue_goal` with fixed `connected_room_residual_v1` source custody.
  Retain installed furnishings and original access choices through cancellation,
  durable reopen and compaction, including excavation-only access repair.
- Recheck authority, original goal truth and same-root work before dispatch;
  require reviewed replay after changed anchors and keep historical exact goals
  separate from replacement pursuits.
- Refuse aliased building occupancy over original room geometry or an actual
  proposed workshop, while still routing around unrelated aliased buildings.
- Add seven compiler groups, five source/seal cases, nine public MCP lifecycle
  cases and two durable cases. All 38 focused connected, archive, durable, witness and
  existing-room runtime tests pass, along with final production-module Clippy.
  The full MCP check was killed during dependency compilation before project
  code; its nine new facade cases remain unexecuted. Exact evidence is recorded
  in `IMPLEMENTATION_STATUS.md`; this does not change native/live admission.

## 2026-10-23 — Prove current access to room furnishings

- Add a bounded same-level FLOOR access predicate with an explicit entrance,
  routing domain and every furnishing target. Require a reachable cardinal
  approach while accounting for current building occupancy and planned targets.
- Preserve Unknown for missing route evidence or incomplete qualified occupancy;
  only the laboratory evidence owner can certify this reference-model policy.
- Append canonical and archive encodings without changing older predicate bytes,
  charge bounded census/search work, and include phantom building insertions in
  complete-domain read witnesses.
- Add focused access, authority, bounds, archive and witness regressions. All 119
  current world unit tests pass; world/intent production Clippy and actual
  library/test-target type checks pass. See `IMPLEMENTATION_STATUS.md` for scope.

## 2026-10-23 — Resume interrupted furnished-room projects

- Extend the existing `continue_goal` request to retained item-backed room
  goals. Preserve every original layout slot and setup option, retain validated
  complete installations, reuse released items and manufacture only deficits.
- Exclude retained furniture tiles from new excavation, remap dependencies to
  every required residual piece, and keep the full original terrain goal.
  Retained furniture consumes no new stock or material.
- Bind a separate fixed residual compiler and flat parent/root lineage into a
  new private source seal and appended durable `room_continuation` tag. Existing
  room compiler bytes and historical action outcomes stay unchanged; replacement
  placements prove the newly reviewed pursuit's complete goal.
- Recheck current authority, original False evidence and all same-root work at
  proposal and commit. Require reviewed replay after every anchor change and
  suppress duplicate continuation when another pursuit currently satisfies the
  complete room request.
- Add eighteen room compiler, source, durable and MCP regression cases.
  Current adapter/laboratory and actual-source room module targets pass type
  checking; warning-denied laboratory and module production Clippy passes.
  Runtime execution and the full MCP facade remain unqualified in this
  resource-constrained session. Document the complete workflow in
  `docs/ROOM_GOAL_CONTINUATION.md`.

## 2026-10-23 — Prove retained furniture with exact current custody

- Expose validated completed-installation inspection and reusable exact
  completion predicates, preserving every existing placement predicate byte.
- Strengthen current and archived goal proof for mandatory completed-furniture
  claims even when a plan does not dispatch that furnishing again. Revalidate
  original generations, production provenance and exclusive reciprocal custody
  through one bounded attachment census; optional predicate branches remain
  optional.
- Add five regression groups, including canonical fingerprints for all four
  furniture kinds, completion/reopen reuse, contradictory custody and generation
  substitution, optional branches and bounds. The actual intent library and
  unit-test target pass type checking; warning-denied production Clippy passes.
  The environment terminated the fresh test build before execution, so no new
  runtime test pass or full-workspace qualification is claimed.

## 2026-10-23 — Provision complete furnished-room goals

- Add the existing blueprint argument's versioned `provisioned_room` wrapper
  for bedroom clusters and dining halls. Reuse eligible exact furniture first;
  manufacture only missing items, then excavate and install them with ordinary
  verified-completion dependencies.
- Derive future item identities from the original work-order step and ordinal.
  Legacy aggregate stock cannot stand in for eligible items; procurement adds
  the real missing count to the observed aggregate target.
- Seal every original terrain region and exact installed furnishing as the
  final goal. Intermediate stock quotas are discharged before installation
  consumes inventory; already-excavated rooms can still be furnished.
- Retain complete canonical layout, compiler marker and optional prerequisite
  choices for durable source replay. Bound compilation to sixteen furnishing
  slots and sixty-four combined actions; refuse unknown supply, unsafe or
  occupied geometry, inadequate logs and overlapping workshop setup.
- Scale excavation deadlines with actual tile work, allowing accepted large
  rooms to finish while preserving legacy blueprint seals.
- Add ten actual-source compiler/execution regression groups, including
  forecast parity, source immutability, released-item reuse, generation
  substitution and a 105-tile room. Nine groups and the room-inclusive module
  passed type checking and production Clippy before the final deadline fix;
  that narrow fix and tenth regression are source-reviewed and formatted but
  not recompiled. Runtime execution remains blocked by shared-storage exhaustion.

## 2026-10-23 — Install and recover exact manufactured furniture

- Add a distinct typed `place_furniture` action for Bed, Door, Table and Chair,
  binding one canonical item and one tile under existing Construct authority.
  Preserve original Build semantics and append new canonical/archive tags.
- Allocate reciprocal item/building custody and debit one loose stock unit
  atomically. Completion installs the same item; unfinished cancellation
  releases it and refunds once. Completed furniture remains installed.
- Validate original producer/ordinal/generation, eligible WOOD singleton data,
  complete attachment and occupancy scans, and explicit stock evidence. Legacy
  items without custody are ineligible; ambiguous evidence refuses atomically.
- Bind action verification, original-goal truth, forecasts, physical-work
  inspection and historical achievement to exact completion evidence. Retain
  original allocation through durable restart and fence identity substitution.
- Expose the closed action grammar and reviewed exact item/tile through the
  existing MCP tools. Reserve its footprint, retain its physical work and
  require reviewed stale replay. Native routing explicitly refuses laboratory
  canonical custody.
- Include focused conservation, cancellation/reuse, corruption, generation,
  restart, authority, codec and parser regressions. Component validation is
  recorded in the implementation status; no native/live admission is changed.

## 2026-10-23 — Plan and verify complete furniture production goals

- Extend the existing production blueprint with BED, DOOR, TABLE and CHAIR
  quotas, including mixed requests with FOOD and DRINK. Seal the original
  stock goals, preserve canonical source replay and include them in forecasts
  and real completion checks.
- Budget one shared finite WOOD supply across all requested furniture and
  outstanding marked carpentry orders. Never count future production as stock;
  refuse unknown inputs, overlapping producers and insufficient material.
- Select workers jointly across three workshop/labor capacity families. Reuse
  one carpenter across serialized furniture outputs, avoid unnecessary labor
  enables, and deduplicate explicitly requested workshop/labor setup. Shared
  workers and workshops create ordinary completion dependencies.
- Include a carpenter workshop and explicit wood/furniture stocks in the
  starter scenario; expose the new quota grammar and document complete usage.
  Historical FOOD/DRINK requests retain their original source/seal behavior.
- Pass all 125 actual-source MCP component tests and warning-denied production
  module Clippy, including 12 furniture planner groups, seven joint-staffing
  groups, source replay and forecast-versus-execution regressions. The final
  engine also passes all 124 unit tests and production-library Clippy.
- Full-workspace qualification remains blocked by existing static gates;
  component evidence does not establish MCP facade, native or live admission.

## 2026-10-23 — Manufacture wooden furniture from finite laboratory inventory

- Close the reference-model carpentry loop for beds, doors, tables and chairs:
  each completed unit consumes one explicit WOOD log, increments its stock
  counter and creates a canonical item with stable original-order identity.
- Share finite citizen and completed-workshop capacity across carpentry,
  brewing and cooking. Preserve conditional gates, partial work, cancellation
  and causal timing without double-counting a multi-skilled citizen.
- Require source-qualified input/output counts and a new `carpentry/1` order
  marker. Legacy abstract orders cannot retroactively create inventory; item
  collisions, overflow and bounded-world exhaustion refuse transactionally.
- Correct four existing alert-state-machine test fixtures to construct typed
  digests, enabling the actual intent regression suite to compile.
- Pass all 239 actual-source intent tests (124 unit and 115 integration),
  including 12 new carpentry groups and 512 three-family worker configurations;
  scoped formatting passes. This is laboratory behavior; native item allocation
  and live admission are unchanged. Full-workspace qualification is not claimed.

## 2026-10-11 — Recover original native work-order goals across restart

- Add a bounded, operator-owned private goal-history store and exclusive durable
  monitor. Sync read intent, both queue/progress endpoints, complete operations
  and original creation context before publishing proof.
- Rebuild original achievement and obligation history under twice independently
  reissued exact source policy. Preserve original deadlines/cadence; reset
  unfinished stability and current truth on reopen. Earlier Unverified insertion
  stays unlinked after later Created reconciliation.
- Retain partial progress and operations allocation/source floors. Fence source
  changes, unretained oversized captures and incomplete crash prefixes. Preserve
  current authority floors across monitor replacement; ambiguous storage fences
  publication until independently verified recovery.
- Keep raw custody and source authority separate, with distinct bounded codecs,
  private-file identity/locking and immutable read-only recovery. Generic MCP
  dispatch and native/live admission remain separate work.
- Pass all 56 actual-adapter work-order integration tests, including 16 new
  durable-history cases, and warning-denied library/integration Clippy with 416
  unchanged inventoried inputs. This establishes component behavior with
  synthetic native responses and real storage, not native/live qualification.

## 2026-10-11 — Retain source history when original-goal sampling fails

- Retain selected-order counters from both progress endpoints before rejecting
  an inconsistent queue or operations bracket. A later counter increase cannot
  hide behind the failed sample and become eligible goal evidence.
- Evaluate original-goal proof on a private pending transition, after retaining
  source identity and before exposing the result. Internal acquisition and
  publication boundaries support durable history without granting source policy.
- Pass 40 actual-adapter work-order integration cases, including both rejected
  bracket regressions. Goal history remains process-local in this increment.

## 2026-10-11 — Preserve undispatched original goals through repeated recovery

- Retain exact anchored `not_dispatched` steps in the original objective when
  its action commit retires. Interrupted workshop setup can finish after a
  crash, survive another reopen, and permit a fresh reviewed continuation of
  the same stock goal without dispatching the original deferred order.
- Atomically transfer custody through new progress/retirement journal records;
  compact the bounded step map separately from its source and pin every exact
  archived frontier. Preserve legacy semantics, latest-state precedence and
  publication fencing on uncertain writes.
- Apply the same original-seal, archive/hash, chronology and creator checks on
  both recovery paths. Missing or unanchored legacy custody remains unresolved.
  A failed or contradicted custody claim cannot fall through to generic
  inspection of a completed creator and authorize duplicate pursuit.
- Keep current Observe/Plan, active recovery lineage and all same-root physical
  quiescence checks. Expose authorized retired step evidence without restoring
  original action handles, dispatch keys, grants or preparation lifetimes.
- Pass 35 durable store/objective tests, including eight new cases; 13
  durable-progress integration tests; and all-target lab Clippy with warnings
  denied. Pass all 109 actual-source module tests, including five new recovery
  cases, and warning-denied all-target harness Clippy on unchanged inputs.
- Add three facade regressions and update existing recovery fixtures. These are
  source-reviewed/formatted but unexecuted because unchanged Asupersync blocks
  the full MCP build. No full-workspace, native-build or live qualification is
  claimed. See `docs/PRODUCTION_GOAL_CONTINUATION.md`.

## 2026-10-11 — Prove original native work-order goals

- Add bounded foreground original-goal monitoring to `SemanticWorkOrderSession`
  using concrete existing operations/1.3 and selected progress/1.12 read clients,
  bracketed by identical current work-orders/1.10 queue captures.
- Bind the original Created receipt and key-derived order identity to complete
  item/job/building evidence under independently issued policy. Preserve both
  progress endpoint witnesses and distinct native/canonical chronology.
- Evaluate all original postconditions, plan and obligation terminals, fixed
  deadlines, cadence, failure and sampled stability. Keep current truth and first
  achievement separate; Unknown native insertion never becomes verified through
  independent goal truth.
- Require exact finite-order zero remaining, validated/inactive status and a
  qualified complete empty job roster for physical quiescence. Missing orders
  and zero counters alone remain insufficient.
- Bind each non-cloneable monitor to its unique current session opening;
  replacement or reopen cannot reuse an old streak. Goal history remains
  process-local, with fresh observation after read-only reattachment.
- Pass all 38 actual-adapter integration tests, including 17 new original-goal
  cases, and warning-denied library and focused test Clippy on unchanged compiled
  inputs. This extends the operator embedding API; generic MCP dispatch,
  durable native goal history and native/live admission remain incomplete.

## 2026-10-11 — Complete dependent work within one foreground wait

- Interleave real laboratory advancement with dependency-ordered observations
  so one bounded wait can execute dig, build and production successors. Preserve
  original actions, deadlines, cadence and scoped dispatch authority.
- Make each simulated round atomic, with aggregate action/poll/round limits and
  a cooperative wall deadline. Stop on deferred pause and report actual elapsed
  and remaining time on partial refusal.
- Publish each frontier and original-goal achievement before further progress;
  retain an intermediate achievement through later consumption and durable
  reopen. Fence further effects when publication fails.
- Render Task status from the final retained receipt without an extra poll.
  Keep original per-action proof anchors and current physical work distinct;
  Agent Turns adopt authorized partial-wait observations on errors.
- Pass 104 actual-source module tests, including eight new progression cases,
  and warning-denied all-target Clippy. Four MCP integration regressions and
  existing durable fixtures are source-reviewed/formatted but unexecuted; the
  full MCP build remains blocked before MCP in the unchanged Asupersync graph.
  No native/live or full-workspace qualification is claimed.

## 2026-10-11 — Continue original goals after verified durable recovery

- Fix continuation rejecting every durable reopen because the new observation
  epoch differed from the immutable original plan anchor. Verify the exact
  persisted head and its reset/rehashed recovery anchor once per fortress, then
  bind each fully reconstructed active goal to that process-local frontier.
- Keep original source, seal and historical achievement unchanged. Require
  current evidence to advance the verified active lineage and reject arbitrary
  epoch changes, backward clocks/cursors and same-cursor forks. Checkpoint
  restore clears active continuity immediately and persists abandonment.
- Preserve current Observe/Plan authority and complete original/sibling work
  quiescence at both planning and commit; recovery restores no dispatch grant.
- Pass all 96 actual-source production/continuation/forecast tests, including
  three new recovery regressions with real two-epoch execution, and warning-denied
  all-target Clippy. All 644 inventoried source/build inputs remain unchanged.
  Add two facade regressions and strengthen the repeated durable-reopen test.
- The full MCP check remains unexecuted: its bounded attempt was stopped while
  unchanged Asupersync was compiling at the shared 8 GiB memory ceiling, before
  reaching MCP. No full-workspace or native/live qualification is claimed.

## 2026-10-11 — Execute original semantic work orders through native custody

- Add `SemanticWorkOrderSession` for one original unconditional wooden-furniture
  work order through the existing work-orders/1.10 coordinator. Preserve the
  original action key, full semantic plan, native review and pending obligation.
- Resolve against complete queue capture and independently authorized canonical
  evidence. Reuse the complete citizen acquisition owner; refresh evidence at
  preparation and after durable DispatchStarted immediately before insertion.
- Add a separate private immutable semantic/native association journal. Sync and
  verify it before native preparation, reject unassociated old keys and refuse
  uncertain native work before persisting a fresh association.
- Retain distinct canonical and native chronology across goals/restart. Validate
  sorted retained histories before merging current floors; query-only historical
  reattachment/reconciliation neither renews authority nor dispatches again.
- Report strict Created readback as native order insertion only. Original goals
  remain unproved; prepared-only cancellation cannot remove a created order or
  discharge Unknown effects. Bound source, history, acquisition and call budgets.
- Pass all 21 new actual-adapter public integration tests, including complete
  citizen acquisition, lost Created reply, sync failure, restart chronology and
  real private-file custody. All 18 existing work-order coordinator regressions,
  all 43 workforce integration tests and all 93 production module tests pass;
  warning-denied adapter-library Clippy passes. All 644 inventoried source and
  build inputs remain unchanged across final checks. Independent source review
  and scoped formatting pass. No generic MCP dispatch or live admission is added.

## 2026-10-11 — Reach original production reserves through consumption

- Add fixed `consumption_aware_v1` planning for new production requests and
  continuations. Budget observed population consumption through the complete
  sealed schedule, including setup, existing queued service, shared workers and
  quotas already satisfied at the source. Keep original terminal minima intact.
- Apply joint read-only staffing to ordinary quota requests too. Serialize
  orders that must share their only eligible worker; setup remains opt-in.
  Bound reserve iteration and quantities, and refuse unknown source evidence.
- Preserve historical production and continuation `/1` compiler bytes and seals.
  New production seals bind the complete canonical request; continuation `/2`
  selects the fixed new compiler without changing the original retained request.
- Require Verified actions, True original goal and physically quiet original
  work together for whole-plan forecast completion. Expose separate predicted
  proof, goal and physical-work results, preserving Unknown and immutable proof.
- Pass all 93 actual-source production/projection/continuation/forecast tests,
  including 28 new cases with the actual StaticPlanner and MemoryAdapter, and
  focused warning-denied Clippy for those source modules. Update public MCP
  expectation tests for new reserve behavior and later consumption of completed
  goals; these facade tests remain source-reviewed/formatted, not executed.
  No full-workspace qualification or native/live admission is added.

## 2026-10-10 — Continue retained production goals through a new reviewed plan

- Add the closed continue_goal blueprint form to existing fortress.plan. Retrieve
  all original quotas and setup permissions/sites, require current authority and
  False goal truth, and prove the original and same-root work quiet before
  proposing fresh deficits. Unknown, active, abandoned and legacy sources refuse.
- Recheck lineage at commit before reservations/effects. Preserve exact-seal
  idempotent retries while forbidding their candidate exemption from carrying
  into a new seal after the world anchor changes.
- Persist a flat production_continuation source with parent/root identity and
  the complete original request. Bind even unused setup fields into the plan
  seal, reconstruct historical plans purely, and preserve original achievements,
  receipts and action identities. Expose lineage and explicit next-plan requests
  in goal, handoff and Task evidence without dispatching replacement work.
- Pass 65 actual-source module tests, including ten new continuation cases;
  pass all 96 laboratory tests, including all 27 durable-store cases and the new
  continuation retirement/compaction regression. Pass focused warning-denied
  Clippy for the laboratory, adapter library and actual source modules on
  unchanged inventoried inputs.
  Add ten MCP integration regressions and pass focused source review/formatting.
  Full MCP execution remains blocked in unchanged Asupersync under the shared
  memory limit. No native/live admission or full-workspace qualification added.

## 2026-10-10 — Recover native original-goal history across restart

- Add an operator-opened private goal-history store and durable foreground
  workforce monitor bound to the exact original semantic/native review.
  Retain complete raw captures, interrupted reads and observation floors.
- Reconstruct historical achievement by replaying the original predicates and
  temporal runtime under independently reissued historical policy and current
  authority. Stored fingerprints compare policy; they cannot restore grants.
- Preserve first achievement and original deadline/cadence while resetting fresh
  evidence and unfinished stability. Keep Unknown native effects unresolved.
- Sync intent before contact, raw capture before proof and publication before
  returning state. Reserve all possible events before contact; ambiguous writes
  fence the owner. Reuse private-file custody with explicit 8 MiB/1,024-event
  bounds and no repair/pruning.
- Pass all 43 actual adapter integration tests, including eleven new history,
  crash, authority, event-budget and real private-file cases. Independent review
  found and resolved the pre-contact event-budget gap. The final native and
  queue55 runs used unchanged semantic inputs. No generic MCP execution path,
  native/live admission or full-workspace qualification is added.

## 2026-10-10 — Plan production against the existing work queue

- Read current registered physical orders before compiling new quotas, including
  work whose original process handles are gone. Refuse overlapping same-output
  producers with exact observed work details; never count future output as stock.
- Bind conservative queued service, partial progress and setup dependencies into
  actual step obligations, within the existing one-year horizon. Preserve source
  and action bytes for uncontended plans, bound scans/output, and reject missing
  workload evidence or overflow before publishing any changed intent.
- Require reviewed source replay for stale production plans. The current witness
  omits production-domain absence/progress/setup reads, so matching action bytes
  cannot authorize changed deadlines automatically.
- Pass 55 actual-source production/projection tests, including twelve queue
  regressions and the fifteen prerequisite cases. The real planner/adapter cases
  demonstrate old-deadline failure and new-deadline success behind existing work.
  Add an MCP reviewed-replay regression; its execution remains blocked by the full MCP dependency build.
- Execute the previous exact `57c0cd6` feature generation on pinned nightly:
  32 native integration, 21 timeline, 95 laboratory and 43 actual-source production
  tests passed. Earlier outage-era unexecuted notes are superseded for those
  tested targets only; no full-workspace or native/live admission is claimed.

## 2026-10-10 — Compile missing production staffing and workshops

- Add explicit prerequisite options to original DRINK/FOOD production sources:
  permission to enable known-disabled labor and bounded Still/Kitchen sites.
  Compile real setup actions and per-order dependencies under existing action
  authority, compensation, construction checks and checkpoint rules.
- Select workers jointly across both roles, maximizing distinct capacity before
  minimizing labor changes. Preserve scarce specialists and treat unavailable
  labor and military evidence explicitly. Require known life and known-false
  prior flags for new assignments; keep other labors and squad membership.
- Serialize orders that must share one worker. Use verified completion
  dependencies so a short order retains its full sealed horizon after waiting
  behind a larger order, while preserving established FOOD-first ordering.
- Retain every original quota and explicit site through canonical source storage
  and replay. Reuse newly ready setup, recompute batches from eligible current
  inventory, and report the required setup and action capabilities.
- Add fifteen focused Rust regressions, including real planner/adapter setup,
  authority refusal and unequal-order execution. Two independent source reviews
  passed; an independent 1,103,762-case staffing oracle matched exhaustive
  selection. Current Rust compilation/tests/rustfmt remain unverified because
  the execution service is disconnected. No native/live admission is added.

## 2026-10-10 — Prove original workforce goals from fresh observations

- Add a foreground goal monitor to the existing semantic workforce handoff.
  Preserve original canonical/native/historical citizen identity while projecting
  current labor facts from fresh workforce/1.17 captures, never receipt payloads.
- Require independently issued source policy, close acquisition before both
  policy checks, and refuse unsupported completeness or unrelated domain grants.
  Preserve canonical/source clock floors and check Observe authority at fresh
  game ticks.
- Evaluate every original goal component together under its fixed deadline,
  cadence and stability count. Preserve first achievement separately from
  current truth; reset unfinished streaks after gaps or failed publication.
  Keep Unknown native effects unresolved even when the current goal is true.
- Support Query/Observe-only recovery with exact original-plan reattachment and
  a new process-local monitor. Reserve read-only work and output before contact;
  provide no automatic clock advancement, dispatch, retry or imported proof.
- Add seventeen integration tests to the ordinary production-adapter target and
  correct two fixture ID casts in the finite-capacity tests. Rust compilation,
  tests and rustfmt remain unverified during the execution-service outage.
  No new protocol, generic MCP dispatch or native/live admission is claimed.

## 2026-10-10 — Make production compete for finite workers and workshops

- Allocate one eligible living worker and one completed matching workshop to
  each running brewing or cooking order. A multi-skilled worker cannot count
  twice, and additional orders no longer manufacture extra service capacity.
- Use bounded deterministic matching to keep specialists productive, preferring
  earned partial work and then canonical order identity. Preserve canonical
  completion ties and successful wait-partition equivalence.
- Expose worker/workshop capacity blockers, retain earned work without banking
  waiting time, and apply workshop completion or worker death only to later
  causal intervals. Charge matching against the existing advancement budget
  and reject oversized domains before the new matching allocations.
- Add nine Rust regression functions, including 512 exhaustive reference-timeline
  configurations. An independent 60,016-case JavaScript assignment oracle passed;
  it validates the allocation algorithm separately from the Rust implementation.
  Rust compilation, tests and rustfmt remain unverified while the execution
  service is disconnected. No native or full-workspace qualification is claimed.

## 2026-10-10 — Recover and share original goals through MCP

- Connect the durable objective store to actual laboratory commit, progress,
  restart, shared-session and restore paths. Recover goals independently of
  unfinished commits by reconstructing the exact original sealed plan and
  verifying historical proof against its archived snapshot.
- Keep current goal truth, immutable first achievement and restore abandonment
  separate. Current Observe authority gates both new proof and historical
  source/owner/anchor disclosure; persistence alone cannot certify a goal.
- Retain the common fortress goal book across shared-session calls. Preserve
  historical owner metadata without assigning ownership to a new process whose
  numeric session ID happens to repeat. Restore retains first achievement and
  atomically abandons the original pursuit with the restored world.
- Check the 64-goal bound before reservations or effects. Eviction requires a
  currently true, historically verified goal and proven quiet original work;
  consumption, unknown evidence or missing work identities cannot free capacity.
- Preserve historical achievement and abandonment in compact Task results.
  Add restart, compaction, shared-world, authority, owner-collision, capacity,
  restore-crash and invalid-proof regressions. The full MCP build remains blocked
  in unchanged pinned Asupersync under the environment memory limit; their MCP
  runtime execution is not claimed.
- Pass 28 actual production/observation module tests, including the physical
  timeline where four completed brewing batches leave only 53 drinks against
  the original 60-drink goal, and the joint quota initially needing no work.
  Correct elapsed-tick fixtures to include consumption at game tick 1201.

## 2026-10-10 — Fresh evidence for semantic workforce execution

- Add a concrete citizen evidence owner for the existing semantic workforce
  coordinator. Reacquire the complete original paused roster after native
  capture, invalidate prior evidence before every refresh, and require an
  independently supplied source policy before publishing routing evidence.
- Preserve the exact original capsule, canonical anchor, fortress, generation,
  software identity and citizen IDs. Changed or partial evidence cannot justify
  the original plan; failed refreshes and revoked authority leave no old scope.
- Connect through the authenticated V1 codec with one absolute connect/handshake
  deadline, bounded page work, configured read/write timeout caps and a shrinking
  byte allowance. Close acquisition sources before both policy checks; permit
  only two prepaid borrowed routing views before another refresh is required.
- Pass all 15 public-API integration tests, including actual semantic/native
  prepare and commit, evidence loss after synchronized DispatchStarted, strict
  repagination, source/authority failures and real wire-codec timeout handling.
  The target compiles the ordinary production adapter and shares only test
  fixtures with the existing unit suite. The earlier twenty handoff/store unit
  tests remain unexecuted under the monolithic test-build memory limit.
- This supplies an adapter evidence owner. Independent compatibility authority,
  original labor-goal proof, broader semantic MCP dispatch and live qualification
  remain separate work; no native protocol or admission is added.

## 2026-10-10 — Durable original-goal storage

- Admit the exact original goal source, sealing snapshot, owner and action
  commit atomically before effects. Retain the goal after action retirement and
  journal compaction; legacy action records gain no invented goal history.
- Publish first satisfaction and restore abandonment with the exact world and
  action frontier. Keep both historical anchors immutable and pin their
  snapshots independently of unfinished work. A later world cannot create a
  first achievement for an abandoned goal.
- Bound retained goals to 64 per fortress. Refuse capacity exhaustion before
  admission and allow only achieved, retired history to be removed; the caller
  must also establish current quiescence of the original effects. Identical
  admissions preserve history without recreating retired action commits.
- Validate source and evidence objects on reopen, keep uncertain writes fenced,
  and exercise every byte boundary of admission and goal-progress publication.
  Fence post-publication automatic compaction failures too, so an error cannot
  leave an admitted goal invisible to its caller while further writes continue.
  All 95 laboratory tests pass, including 26 durable unit cases and the existing
  progress/deferred/full-action suites. This increment provides the durable store
  APIs; MCP lifecycle wiring follows separately and no additional live behavior
  or admission is claimed.

## 2026-10-10 — Preserve original production goals

- Keep normalized original DRINK/FOOD quotas through planning, stale-plan replay
  and durable source recovery. Recompute required batches from the current
  eligible stock, and return fresh production analysis with an explicit rebase.
- Seal every original stock minimum into the plan-level goal, including quotas
  that needed no work order. Work-order completion retains its own proof and
  can no longer stand in for the requested stock level after consumption.
- Require task completion to establish the original goal as well as completed,
  quiescent work. Expose current goal evidence separately from action receipts;
  exhausted work with an unmet goal requires explicit replanning, and unknown
  goal evidence requires reconciliation. Resolve witnessed rebase aliases to
  the actual committed goal; never dispatch replacement work automatically.
- Bound submitted quotas before normalization, validate the exact template, and
  reject simultaneous explicit production and blueprint-alias requests. Add a
  distinct durable production source tag without upgrading legacy action lists.
- Pass all 15 durable-store unit tests, including the two new source-codec cases,
  and all 27 actual production/observation module tests through a temporary
  direct-source harness. Full MCP compilation remains blocked by SIGKILL in the
  unchanged pinned Asupersync dependency; MCP facade/Task regressions are present
  but their execution is not claimed. Durable objective history is a separate
  increment; this change preserves the request of retained unfinished commits.

## 2026-10-10 — Bounded foreground room supervision

- Add `track_room_readiness.py wait` to supervise several complete joint samples
  under one original goal, held source custody and shrinking operation budget.
  Preserve original deadlines and lifetime observation counts across wait calls.
- Reserve the complete response and all five additional map RPCs before read
  intent. Continue checking query authority and original files throughout short
  foreground delays and before each sample publication.
- Return explicit terminal, sample-limit, stalled-tick, RPC-allowance and
  wall-allowance outcomes. Keep terminal waits offline and failed acquisitions
  unretried; no background work or game-time control is introduced.
- Extend the shared scheduler with a bounded nonnegative extra-RPC reservation
  while retaining its existing default behavior for selected-receipt and
  furnishing monitors. Add actual custody/TCP/process room-wait regressions.
- Pass ten new room-wait methods, all 20 shared-scheduler tests and two legacy
  process regressions, plus both final-publication revocation scenarios through
  the wait path. Tests use synthetic peers and actual private files.

## 2026-10-10 — Durable whole-room readiness

- Connect the existing joint terrain/furnishing condition to an executable
  `start`/`sample`/`inspect`/`cancel` workflow. Retain the complete original room,
  all placement receipts and source-file custody across process restarts.
- Require every original floor, required wall and furnishing to satisfy one
  shared advancing-tick stability window. Preserve the original deadline and
  observation allowance; an interrupted read has zero effective streak.
- Persist read intent before query-only acquisition and full raw map/operations
  evidence before acknowledgement. Reserve the complete original-plan response,
  synchronize file and parent, and recheck source custody before publishing.
- Recheck query authority after final source verification and owner close.
  Revocation withholds the response while preserving synchronized history for
  offline inspection.
- Keep offline cancellation available after original batch loss and withhold
  combined readiness without verified original custody. Refuse torn or altered
  histories, ambiguous writes, source changes and substituted receipt subsets.
- Add private-file and subprocess recovery tests, including actual process death
  at the operations-release boundary and the maximum 32-furnishing room. Scope
  remains Python development evidence using existing unadmitted query profiles;
  native room assignments, live qualification and production admission remain open.

## 2026-10-07 — Durable semantic-to-native workforce execution

- Hand an original sealed single-step SetLabor plan to the actual workforce
  coordinator with one review seal binding semantic and native plans, source
  evidence and native journal identity. Sync their immutable association before
  native preparation; repeat evidence and scoped authority guards immediately
  before native prepare and commit.
- Preserve exact original-plan custody across restart and reject changed
  predicates or obligations under an existing native key. Recover lost responses
  through original-key queries; Unknown never permits another dispatch.
- Add a bounded hash-chained association store and an operator-owned durable
  opener using the existing private-file custody implementation. Fence ambiguous
  writes and altered bytes, refuse orphan native-key adoption, and permit
  read-only association recovery without file creation or sync.
- Report verified historical single-labor readback separately from broader
  native Applied and the still-pending original goal/obligation. Keep expired
  plans cancellable under current original-unit authority and reserve native,
  evidence and journal work under one shrinking call budget.
- Add twenty end-to-end coordinator, restart, lost-acknowledgement, scope,
  deadline and durable-store regressions. Final production adapter Cargo check
  passed with no diagnostics and 386 unchanged scoped file hashes. The unit
  build was OOM-killed before any test ran; all twenty remain unexecuted. Scoped
  Rustfmt passes. This adds no evidence issuer, generic live MCP writes or admission.

## 2026-10-07 — Evidence-bound native routing and truthful forecast refusal

- Preserve canonical workforce IDs, exact labor and all furniture material
  constraints in advisory routes. Resolve native IDs through the real V1 or
  spatial/1.8 projection with exact source and canonical generation evidence.
- Resolve workforce candidates only from an exact paused capture and an
  unambiguous single-labor detail. Reject overlapping disable grants and require
  independent readback that no other labor changed. Bind furniture candidates
  to exact item/material/position/flag/map evidence and refuse unsupported selectors.
- Validate native coordinate and tiling bounds before route construction. MCP
  output exposes canonical IDs and unresolved native values, retains material
  constraints, and explicitly marks advisory routes as not execution-ready.
- Return unavailable forecasts on actual simulation/poll refusal and clip the
  final slice to its horizon. Add two MCP regressions; their execution remains
  pending under the shared-memory build limit.
- Frozen routing execution passed all 17 focused tests, zero failures/ignored,
  with unchanged source hashes. No generic native dispatcher or live admission
  is claimed; full semantic execution custody remains a separate increment.

## 2026-10-07 — Causal, bounded reference time advancement

- Advance construction, excavation, production, meals and combat on one causal
  event timeline. Freeze interval eligibility so completed prerequisites and
  workshops release only later work and killed workers stop producing.
- Preserve physical outcomes across successful wait partitions. Resolve
  simultaneous stock completions canonically, retain earned partial work, and
  keep internal simulation events separate from obligation observations.
- Add explicit tick, event and aggregate work budgets; reject future source
  facts, invalid active counters, unknown excavation terrain and counter
  overflow. Adapter refusal remains atomic across the entire requested advance.
- Cache pending excavation tiles and update intersecting designations instead
  of repeatedly scanning whole regions, with a separate aggregate cache bound
  checked before allocation. Add twelve timeline regressions and three full-action
  laboratory regressions. Frozen execution: 163 intent and 82 lab tests passed,
  with zero failures or ignored tests. Full workspace/native/live qualification
  and MCP compilation remain pending.

## 2026-10-07 — MCP cleanup, retained ownership and recovery of running work

- Keep Failed and early Verified actions visible until their physical work is
  quiescent. Connect real terminal cleanup to single-action, plan and new
  session-wide drains; preserve historical proof receipts and issue certificates
  only for fully inspected, quiescent work. Retain unknown work in refusals.
- Keep older plans individually drainable with `scope="oldest_open_plan"` when
  a session exceeds one call's action budget. Reset shared unpause votes after
  authorized emergency pause and report current work/clock on partial refusals.
- Hold MCP Task monitors open for cleanup while exposing the original goal
  outcome separately. Preserve physical progress and proof evidence in bounded
  task summaries and both cancellation phases.
- Require clock authority for positive waits through the entire requested tick
  span. Ordinary stopping needs observation and original effect authority;
  emergency pause additionally needs clock authority. Bound aggregate stops,
  compensation and pause, and allow a failed compensation request to narrow
  explicitly to stopping without compensation.
- Acquire spatial reservations atomically and retain ownership beyond proof
  deadlines/lease expiry until work is proven quiet. Preserve durable terminal
  proof anchors while separately tracking continuing work across restart.
- Observe and fence running or unknown snapshot work whose action handles were
  lost during restore/recovery. Expose that custody in observations, Agent Turns,
  handoffs and doctor; unavailable ownership never becomes implicit stop authority.
- Add 25 focused MCP tests (clock/lease, complete tool flows, Tasks, snapshot
  custody and durable terminal recovery), extend the real checkpoint/restart
  scenario, and migrate the existing deferred Task cadence fixture. Source and
  Rustfmt checks passed. The final frozen core passed all 79 lab tests,
  including condition-identity and compensation-narrowing regressions. MCP
  execution and full qualification remain pending after shared-memory
  exhaustion; the final MCP check is being retried with recovered headroom.

## 2026-10-07 — Executable conditional production

- Enforce typed stock thresholds, material-availability gates and uniquely
  named completed-order dependencies in actual reference production and its
  forecasts. Persist explicit canonical conditions and bind them, the order
  name and job identity into completion proof and work ownership inspection.
- Carry conditions through MCP requests, sealed step views and production
  blueprints. Require eligible exact inventory counts; unknown evidence blocks
  production. Cap same-product output at the first per-unit threshold crossing
  and discard blocked elapsed time without discarding prior partial work.
- Document bounds, inventory namespaces, model timing and legacy compatibility.
  Existing orders without condition records stay blocked for reconciliation.
- Actual intent suite: 151 passed, including ten new regressions. Four MCP
  condition tests are present and their execution is pending. The added
  identity-mutation drain regression passed in the final 79-test lab run.

## 2026-10-07 — Physical work inspection and authorized terminal cleanup

- Distinguish immutable goal outcomes from running reference work. Inspect the
  exact dispatched dig, construction or work-order entity using eligible
  observations; missing, changed or untrusted ownership remains unresolved.
- Add a separately authorized, transactional stop for physical work whose goal
  has already failed or verified. Preserve the original proof receipt and emit
  independent drain evidence. Cancellation finalization now requires physical
  quiescence and refuses compensation that would start fresh temporal work.
- Add ten lab regressions and migrate four deferred adapter fixtures to the
  canonical later-observation cadence. The isolated core increment passes all
  78 lab tests and the four deferred tests against the prior effects source.
  MCP drain/lease integration and complete workspace qualification remain next.

## 2026-10-07 — Atomic refusal of unverified immediate laboratory effects

- Require established postconditions for non-temporal laboratory actions at
  initial and deferred dispatch. Return `AdapterRejected` for false or unknown
  evidence instead of retaining an action without a completion monitor.
- Restore the whole commit or deferred poll on refusal, including world state,
  receipts, dispatch flags and transcript. Preserve explicit temporal waiting.
- Add two initial/deferred rollback regressions and correct the malformed
  predicate fixture to exceed the actual field-size limit. Final executable
  verification remains pending during the execution-service outage.

## 2026-10-07 — Canonical temporal proof and atomic deferred polling

- Replace both adapters' duplicate stability counters with the shared obligation
  runtime, registered at actual dispatch. Require eligible later game-tick
  samples for the complete sealed goal and retain immutable terminal receipts.
- Preserve scheduled cadence, same-tick refusal, off-cadence contradiction
  resets, failure precedence, fixed deadlines and exact observation lineage.
- Validate the original source before laboratory time advances; future-dated
  registered facts, forged hashes and lookup aliases cannot become trusted
  through clock movement. Preserve untrusted prediction metadata and real new
  model observations.
- Roll back deferred-poll effects, proof monitors, receipts, dispatch flags and
  transcripts on error. Add ten focused regressions and repair fresh-observation
  fixtures. Final source is reviewed and GitHub-verified; executable gates
  remain pending while the execution service is unavailable.

## 2026-10-07 — Source-authorized planning, simulation and recovered goals

- Require explicit admitted evidence for planning, deferred admission, obligation
  and recovery proofs; default raw-snapshot APIs remain conservative. Validate
  all obligation evidence before publishing any member of a batch.
- Prevent reference simulation from consuming untrusted counters, lifecycle,
  population, workforce, stock or combat inputs and producing trusted progress.
  An explicit same-value effect can establish reference provenance once.
- Thread the same evidence through both adapters and MCP carried recovery.
  Display true, false and unknown goal truth without claiming untrusted goals
  are achieved; keep proof-frontier publication atomic on errors.
- Add 26 focused regressions and migrate legitimate reference callers/fixtures.
  Production libraries compiled before resource exhaustion; final reconstructed
  source and the new tests await executable verification after executor loss.

## 2026-10-07 — Source-qualified predicate evidence

- Bind execution evidence to one immutable canonical snapshot, exact admitted
  source digests and explicit observed/complete domains. Default scopes grant
  nothing; replay, assertions and profile metadata cannot issue authority.
- Keep unavailable, future and unadmitted facts unknown through negation and
  boolean composition. Require observed relation endpoints and bounded terrain
  coverage; distinguish positive membership from complete-domain absence.
- Give canonical kind aliases identical truth and coverage semantics, reject
  ambiguous duplicate grants, and validate lookup keys against hashed identities.
- Provide explicit registered laboratory input eligibility without granting
  native effects. The initial implementation passed 178 actual world Cargo
  tests including 13 new evidence cases; two alias follow-up tests and the
  final generation await executable verification after the executor outage.

## 2026-10-07 — Usable MCP profiles and faithful bounded observations

- Expose all five typed completeness profiles through the existing laboratory
  query, with bounded entity, relation, chunk and retained-event sections.
  Keep source/projected identity separate and bind continuation to the exact
  source hash, profile and section.
- Preserve explicit field presence in entity and change views. Withhold stale,
  redacted and otherwise unavailable compatibility values; distinguish known
  null, absent and unrecorded fields. Mark nested binary length summaries as
  omitted content.
- Make query coverage reflect pagination, unresolved predicates, selected
  record domains and profile omissions. Excluded empty sections never prove
  absence; historical coverage follows the source of the returned records.
- Fix output reduction to preserve complete semantic values, remove whole
  records, retain and downgrade coverage, and repair continuations without
  skipping records or looping over exhausted sections.
- Compile the four actual MCP source modules and pass all 29 of their tests
  with real dependencies, including 22 new regressions. This is module-level
  execution evidence; complete server/transport qualification remains blocked
  by the recorded memory limit.

## 2026-10-07 — Typed completeness profiles and exact-base capsules

- Implement all five typed observation profiles with explicit field omission,
  separate canonical source/projected anchors, and identity-bound source schema,
  manifest and optional extensions.
- Add immutable profiled snapshot/capsule codecs and exact-base delta
  construction/application, rejecting profile, provenance, cursor, epoch and
  reconstructed-target mismatches. Preserve unknown optional payloads.
- Add strict canonical decoding for all seven existing state-delta variants;
  keep existing unprofiled v1 bytes unchanged. Bound profile encoding and
  decoding to 16 MiB with schema, extension and nesting limits.
- Pass all 290 actual world/intent Cargo tests, including eight new profile and
  codec regressions. Keep source authenticity, completeness and action
  authority separate from envelope integrity.

## 2026-10-07 — Presence-safe temporal execution

- Add a shared consistent-known-value accessor and expose three-valued predicate
  results. Incomparable ordered values stay unknown through negation instead of
  becoming satisfied mutation preconditions or obligation predicates.
- Require known, correctly typed work counters, job tokens and positive building
  durations before reference progress. Unavailable work can no longer turn into
  zero remaining work or manufactured completion.
- Refuse population-dependent stock and combat updates when explicit life, squad
  or burrow inputs are unavailable. Preserve documented missing optional defaults
  for older reference worlds and keep unknown worker eligibility blocked.
- Add ten regressions across world and intent, including 70 unavailable temporal
  input cases and real deferred-admission/obligation checks. The actual locked,
  offline Cargo world and intent suites passed 282 distinct tests, with no
  failures or ignored tests, on the recorded available nightly.

## 2026-10-07 — Shared durable execution and conservative recovery

- Wire carried obligations to the canonical recovery monitor under current
  Observe authority. Keep unreproducible plans, invalid proof specifications and
  ambiguous legacy records indeterminate, retaining their durable provenance.
- Explicitly anchor deferred nondispatch without prematurely finishing its plan.
  Missing legacy records at a changed world head cannot silently prove absence.
- Share outstanding plans, recovery monitors, pending restore state and save
  faults with the common world. Reuse existing shared worlds without recovery or
  an epoch bump; remove failed admissions without leaving unreachable members.
- Fence private replacement through complete calls and publication, and reject
  mixed private/shared durable writers. Publish checkpoint restore and old-plan
  abandonment atomically, retaining pending retirement for retry after failure.
- Add focused MCP restart, authority, sharing and restore regressions. Reapply
  the reviewed integration through GitHub during the execution-service outage;
  the resulting source is independently reviewed but not formatted, compiled or
  process-tested in that environment.

## 2026-10-07 — Canonical observation-only obligation recovery

- Add `RecoveredObligation` around the canonical runtime, retaining original
  deadlines and cadence while discarding unfinished stability across restart.
- Reject stale/forked observations, keep exact-deadline proof and immutable
  terminal anchors, and reset unfinished stability on observation interruption.
- Pass all 118 ordinary Cargo intent tests, including nine new recovery cases.
  This pure primitive grants no authority and never dispatches or advances time;
  its MCP integration is a separate increment.

## 2026-10-07 — Atomic laboratory recovery frontiers

- Publish world state, step progress and completed-plan retirement in one durable
  record, eliminating crash windows between execution evidence and its world.
- Retain exact step-proof anchors and their snapshot objects through compaction;
  keep legacy unanchored records distinguishable. Validate and canonicalize the
  complete frontier before writing, with bounded records and idempotent retries.
- Compact before journal byte limits are exceeded and preserve objects awaiting
  publication. Fence uncertain writes and sync failures until store reopen.
- Add every-byte crash-cut, compaction, corruption, capacity and I/O-failure
  regressions. The actual store module and real core/world dependencies passed
  26 tests; full Cargo and MCP integration validation remain environment-blocked.

## 2026-10-07 — Real modern MCP Tasks for laboratory plans

- Add opt-in `fortress_commit(as_task=true)` through the pinned modern Tasks
  runtime and an application-owned Asupersync supervisor. Each task retains its
  originating session and sealed plan, with normal authority and idempotency.
- Expose actual task handles through Agent Turns, handoffs and bounded session
  resources; `tasks/get` returns terminal results or failure evidence. Only
  explicit foreground waits advance laboratory game time.
- Route task cancellation to the original plan's request/drain/finalize path,
  preserving progress and proof after later commits. Remove effectful polling
  before cancellation and mark replay logs incomplete after transport-owned drains.
- Bound supervision to one active monitor and 256 retained process-local task
  records, refusing capacity overflow before effects. Add real stdio lifecycle
  regressions and document the laboratory-only scope in `docs/LAB_MCP_TASKS.md`.

## 2026-10-07 — Bounded deferred plan execution

- Revalidate current mutation authority, action budget and sealed preconditions
  before laboratory polling can dispatch a deferred step.
- Share deferred-step decisions between the laboratory and in-memory dispatcher:
  failed/cancelled/compensated prerequisites close their undispatched descendants,
  and a waiting step's own obligation deadline cannot be bypassed.
- Preserve terminal receipt anchors and evidence across later polls; refuse
  completion first observed after its deadline while retaining exact-deadline proof.
- Add deterministic lifecycle and dispatcher regressions for these boundaries.
  Scope is laboratory development execution, with no native or admission change.

All notable changes will be documented in this file. Until the first stable protocol release,
versions describe design, implementation, and compatibility milestones rather than production
readiness.

## [Unreleased]

### Added

- Reference semantics for every non-extension action (`dfmcp_intent::effects`)
  and a `RegionTerrain` map predicate. The laboratory adapter, the in-process
  dispatcher (with observation-driven `reconcile`) and the MCP laboratory now
  execute excavation, construction, labor, work orders, stockpiles, squads,
  burrows and standing orders with sealed postconditions and game-time
  obligations; `starter_fortress` scenario, `entities`/`terrain` queries,
  `fortress.plan(actions)` and `fortress.wait(max_game_ticks)`. Laboratory only.
- Laboratory agent loop: Agent Turn tracking of every committed action and open
  obligation, plan-scoped cancellation with drain progress and a finalize
  certificate, a `df://session/{id}/handoff` resume packet (all session views now
  resolve on the real `serve` binary), a bounded world briefing from
  `fortress.observe`, deterministic plan forecasts on a discarded fork, and
  surprise records when observations contradict forecasts.
- Shared laboratory fortresses (`shared=true`): several agent sessions on one
  world and clock, exclusive spatial leases at commit, intent replay of plans
  made stale by another member, unanimous unpause with an any-member emergency
  pause, and no unilateral restore.
- Objective decomposition: `fortress.plan(blueprint=...)` compiles a blueprint
  template into hazard-checked excavation steps and dependent furnishing steps
  with sealed obligations (`BlueprintPlanner::compile_furnished_blueprint_intent`).
- Crash-durable laboratory fortresses (`fortress_open_session(durable=true)` with
  the operator-set `DFMCP_LAB_STATE_DIR`): every state change and checkpoint is
  persisted as content-addressed canonical snapshots named by a locked,
  hash-chained journal (`dfmcp_lab::durable`); reopening after a restart resumes
  the world in a new observation epoch with checkpoints restorable and older
  sessions fenced. Strict canonical snapshot decoding
  (`WorldSnapshot::from_canonical_bytes`). `fortress.wait(max_game_ticks)` now
  lets time pass without a committed action. `scripts/lab_durable_restart.py`
  proves it across a SIGKILL. Laboratory only.
- Semantic-to-live routing (`dfmcp_adapter::live_routing`): every sealed plan
  step is translated onto the live development families — pause → control/1.7,
  mining → dig/1.16 rectangles (tiled ≤8×8 per level in z,y,x order), bed/
  chair/table → build/1.19, wooden bed/door/table/chair orders →
  work-orders/1.10, labor → workforce/1.17 work-detail membership — validated by
  each family's own constructors, with named live resolutions (exact furniture
  item, work-detail index) and explicit refusals for everything else.
  `fortress.plan` returns it as `live_routing`. Routing grants no authority.
- Durable obligations across restarts: durable commits journal their request
  and sealed world before dispatch and their step states after every call; a
  resumed fortress recompiles each unfinished commit, verifies it reproduces
  the sealed digest, and re-proves dispatched steps against observation
  (`recovered_commits`, `carried_obligations`).
- Deterministic replay bundles: every laboratory call is recorded and exported
  at `df://session/{id}/replay` (`dfmcp.replay.bundle/1`); `dwarf-fortress-mcp
  replay <bundle>` re-executes it and reports the earliest divergence by call,
  tool and field (outcome, error code or resulting canonical anchor).
- The Agent Turn's `changes` now reports observed world changes since the
  agent's previous anchor (entities created/removed/changed with field
  before/after, terrain per level with bounding box and transitions, clock and
  pause), from a bounded per-session history of canonical world versions.
- Exact historical reads over retained world versions: `fortress.query` accepts
  `"at": <state_hash>` and `{"mode":"changes","since":<state_hash>}`. Structured
  queries are no longer silently capped at 64 bytes.
- Laboratory fortress economy: a stock ledger, drink/food consumption over
  game time, explicit `thirsty`/`hungry` needs on shortage, work orders that
  produce stock, and `world_alerts` raised as `fortress_needs` attention with
  an exact remedy plan.
- Objectives: every committed intent is tracked and its terminal condition
  re-evaluated against each observation; the Agent Turn briefing's
  `objective_status` (and observe, wait and handoff) report `achieved` or
  `not_yet_observed` separately from action dispatch state.
- Laboratory threats: `besieged_fortress` schedules a hostile raider that
  fights squad members in rounds or kills exposed dwarves, with `hostile`
  alerts and a capability-gated squad-assignment remedy.
- Policy evaluation harness: `dwarf-fortress-mcp evaluate <scenario> <policy>
  <ticks>` scores `idle` or `follow_recommendations` deterministically through
  the eleven tools (survivors, deprivation, threats, objectives, cost).
- Witness-based revalidation: a plan made stale only by unrelated concurrent
  work (unchanged read witness of entities, edges, pause and haloed terrain)
  commits directly via identical-action intent replay with a deterministic
  `witness_rebase` certificate; otherwise the explicit replay names the first
  changed read.
- Output budgets are enforced: every laboratory response is fitted to the session's negotiated
  `max_output_tokens` (estimator ceil(bytes/4)) through deterministic tiers — compact Agent Turn,
  truncated arrays with recorded paths and totals, omitted optional sections, minimal envelope —
  and reports `output_budget`. Lab sessions default to 4,096 output tokens.
- Lab queries: `where` filters, lexical `search`, and `path` walkability routes over observed
  terrain with honest certified/unknown absence; `fortress.explain` returns a verified Merkle
  inclusion proof; doctor reports real lease and obligation counts.
- Certified top-k attention: every Agent Turn ranks attention (severity, urgency, category, id),
  keeps six items, and records the selection certificate under `coverage.attention_selection`.
- Production objectives (`{"template":"production","quotas":[...]}`) compile against observed
  stock; work orders need a completed Still/Kitchen and a worker with BREW/COOK, stall with
  `blocked_by`, and fail their obligation at the deadline; supply alerts remedy the actual
  blocker. The starter fortress opens with a still, a kitchen, a brewer and a cook.
- Reachability-based retention of world versions (`dfmcp_world::retention`): live plans and
  checkpoints keep their versions readable past the 32-version window.
- PUB-OBS root-last `CapsulePublisher`, seed-pinned `FaultSchedule` campaigns (replacing
  `chaos.rs`), the `dfmcp.replay.bundle/1` JSON schema with a golden replay regression, an
  adversarial threat corpus, and a capability noninterference test.
- `dwarf-fortress-mcp bench [iterations]`: laboratory SLO-001..015 scorecard (latency, output
  tokens, fault-schedule correctness, RSS, response identity; live-only rows marked N/A). Pulse and
  briefing observation profiles are now semantic shapings with named omissions.
- `dfmcp_core::StateAnchorV2`: the complete anchor tuple with strict canonical bytes and a total
  continuity classification, exposed on the session anchor resource.
- Trace-class exploration of two-agent shared-fortress interleavings (`dpor_tests`): lease
  exclusivity, independence-relation soundness, per-trace determinism and absence of write skew.
- Objectives carry readable action summaries and a `no_longer_holds` status; the evaluation
  report lists every objective.
- Renamed capability-implying names: `table_ledger` (was `sqlite_ledger`),
  `http_resumption_lab` (was `http_transport`), `e2e_lab_fortress_tests`, and
  `DelegationToken.delegation_digest`.
- Durable laboratory sessions fail closed: commit, wait, cancel, checkpoint
  and restore are refused while the latest state could not be persisted.
  The `starter_fortress` now has observed rock at z 9..11 so hazard halos are
  complete. Laboratory only.

### Fixed

- `fortress.cancel` refuses sessions whose every grant is read-only before disclosing session
  state (its entry gate had been lowered to `observe`); warning-denied Clippy is clean again.

- Plan-scope `compensate_reversible` no longer applies the inverse of deferred steps that never
  dispatched (it could flip a dwarf's labor nobody asked to change).
- Lab `path` queries bound their coordinates instead of overflowing `i32` arithmetic.
- Warning-denied Clippy is clean across the workspace (`--all-targets --all-features`).

- The whole Rust workspace test graph compiles and passes again on the pinned
  nightly: 3,054 tests, 0 failed (debug, `CARGO_PROFILE_DEV_DEBUG=0` to keep the
  asupersync build under ~8 GB). Nine development MCP servers refused every
  request because they required `cx.io()`, which asupersync 0.5's native runtime
  never installs; they now check effective `capabilities().io`. Closed
  `schema`/`baselines` queries now reject unknown fields. Three-way world merges
  accept branches several observations ahead of their base while still rejecting
  dangling edges, and epoch-crossing deltas name the epoch change. This is
  development execution, not warning-denied Clippy, native, live or production
  qualification.

- Furniture preparation now checks native-global uncertainty and retention
  capacity after its fresh-key query, before writing local intent. A known native
  refusal cannot strand a local operation that was never prepared. The latest
  validated native summary remains inspectable after connection abandonment;
  original-key Query and retirement still work under an unrelated native fence.
  All 45 adapter tests pass, including four new actual TCP regressions.

- Furniture placement now honors credential removal or rotation at the final
  native write boundary, after allocation and full state revalidation. A revoked
  request never enters the writer; its retained uncertainty cannot be retried by
  restoring credentials or changing keys. Actual-handler GCC/UBSan checks pass
  53 groups / 27,699 assertions and reject five compiled mutants, including the
  removed credential recheck. Native wire and golden fixture bytes are unchanged.

### Added

- Inventory-driven furnishing through the existing MCP open/query/observe/plan/
  commit workflow. Requested slots allocate from one complete operations/1.4
  capture and retain the original constraints and chosen items through restart
  and completion monitoring. Shortage creates no partial batch. Empty-child
  restart now initializes its lease from the retained allocation clock, and
  new custody rechecks current Plan opt-in. Nine process test methods are
  implemented; execution remains blocked by dependency memory failures and
  an offline executor. See `docs/FURNITURE_HANDOFF_MCP.md`.

- Source-bound furniture allocation handoffs in Rust. Complete operations/1.4
  inventory now feeds an immutable exact batch retaining request constraints,
  selected-item evidence and the original source. Fresh placement captures and
  retained child audits enforce those constraints; shortages expose no partial
  plan. The new batch representation preserves all legacy placement bytes.
  Source and 33 new test groups are present, but executor disconnection blocked
  compilation and execution. See `docs/FURNITURE_HANDOFF_RUST.md`.

- Source implementation of whole-original-plan construction monitoring through
  the existing furniture MCP tools, including fixed-goal creation, one explicit
  shared sample, complete progress and durable local cancellation. Monitor-only
  recovery preserves inspect/cancel access after original placement files are
  lost, with historical origin evidence clearly unverified. Local terminal state
  now removes only the monitor obligation; full 32-target result admission no
  longer reserves an unrelated optional native-record payload. The 13-scenario
  process suite is present, but MCP compilation and execution remain blocked by
  persistent shared memory exhaustion inside the pinned runtime dependency.
  The adapter below is tested; this MCP integration is not yet runtime-validated.
  See `docs/FURNITURE_COMPLETION_MCP.md`.

- Receipt-linked whole-plan construction monitoring in safe Rust. Every original
  Placed target shares one operations capture, one stability streak and a fixed
  deadline. Same-connection original receipt brackets, strict native identity
  checks and a private origin-bound journal connect sampled construction evidence
  to the complete original batch. Durable interrupted reads survive restart;
  local cancellation works without original placement custody. All 44 new
  adapter tests pass, including 32-target Python byte parity and real TCP faults.
  This remains development functionality and grants no placement authority. See
  `docs/FURNITURE_COMPLETION_RUST.md`.

- Complete exact furnishing batches through the existing furniture MCP server.
  Operator-configured parent custody retains the full plan and original step
  keys; `open_session(furniture_plan=...)`, `query` mode `batch`, `observe`
  selection `next`, and `cancel` scope `batch` connect durable planning to fresh
  review and one native placement per step. Reopening needs no bootstrap call
  and restores no commit permit. Parent loss blocks advancement while preserving
  original-key recovery, including after native dispatch. Every response retains
  the complete plan and progress within an admitted 64 KiB envelope. This is
  development functionality; all-placed remains distinct from completed
  construction. All 32 furniture MCP Rust tests, two shared runtime tests,
  nine new batch process tests and seven existing process regressions pass;
  the 32-placement response stays complete at 36,407 bytes. See
  `docs/FURNITURE_BATCH_MCP.md`.

- Durable whole-furnishing-plan coordination in safe Rust. The existing exact
  1..32-step JSON plan now has a Python-compatible parser/digest and a complete
  original-journal prefix audit. Only synchronized native Placed outcomes unlock
  the next exact item and target. A separate private parent retains the entire
  plan, source and journal identity plus permanent stop state without changing
  the existing single-placement journal. All 24 new adapter tests and 45 existing
  placement regressions pass, including exhaustive dependency ordering, 32-step
  recovery, interrupted publication and Linux custody. This model and storage
  increment grants no native effect authority. See `docs/FURNITURE_BATCH_RUST.md`.

- Complete inventory-driven furnishing through the typed MCP
  `furniture_allocation` query in operations/1.3, paged operations/1.4 and
  citizen-inclusive spatial/1.8. Full constrained layouts produce the existing
  exact-item plan artifact with canonical item generations, independent Python
  digest parity and source evidence; shortages produce no partial plan. The
  query respects current authority, published anchors and whole-result budgets
  without another native capture. Nine new Rust tests and five real MCP
  stdio/TCP scenarios pass, including a complete 32-slot result from six native
  pages, refused stale/fenced sources and smaller-budget recovery. Development
  functionality only; placement still uses separate fresh review. See
  `docs/FURNITURE_ALLOCATION_MCP.md`.

- Global furniture selection in safe Rust over the complete published operations
  inventory. Typed 1..32-target requests preserve exact furniture/material/subtype
  constraints and select distinct items with a global distance optimum and exact
  ID tie breaks. Infeasible layouts return complete shortage witnesses without
  partial executable assignments. Conservative native supply classification,
  fortress/anchor validation and original canonical generations bind the result
  to its source under one Query/work allowance. All 32 focused adapter Rust tests
  pass, including exhaustive oracles, all observed flag words, source reuse and
  the 65,536-item boundary. Development functionality only; fresh placement review
  remains necessary. See `docs/FURNITURE_ALLOCATION.md`.

- Complete original furnishing-batch construction goals through
  `track_furniture_batch.py`. The monitor imports every registered Placed child
  from the locked original action DAG and binds its full plan, source, endpoint
  and exact step-to-receipt mapping into durable monitoring. Every original
  target must satisfy its condition in the same sampled captures; omitted
  furnishings cannot be hidden by a smaller receipt selection. Restart retains
  the fixed goal, and successful inspection or sampling rechecks original batch
  custody even after terminal completion. Cancellation can stop monitoring when
  the source is unavailable while explicitly withholding source verification.
  Existing placement and selected-receipt formats are unchanged. Python
  development functionality only: 183 tests pass, including 40 new tests, and
  four weakened implementations are rejected by assertions. The 32-placement
  boundary workflow returns a complete 53,320-byte result. See
  `docs/FURNITURE_COMPLETION.md`.

- Whole-plan receipt-linked construction condition core for 1..32 original
  furniture/1.19 Placed records. Canonical unique target identities and all
  original receipt boundaries bind one shared operations capture. Every target
  must satisfy its condition together throughout one advancing-tick stability
  streak; independently timed successes cannot complete the plan. This Python
  semantic increment grants no game effects, placement retry or production
  admission. All 17 new core and 16 existing single-receipt test functions pass.
- Same-connection native acquisition for complete construction plans: query every
  original receipt around one complete operations capture, verify every page,
  capture digest and release acknowledgment, and preserve one total allowance
  across all targets. All 14 whole-plan and 13 existing TCP test functions pass,
  including fragmented 32-target acquisition and refusal of missing/substituted
  final receipts, source drift, insufficient retention, lost replies and revoked
  credentials. This is Python/TCP development evidence against explicit peers,
  not a real DFHack SDK/live-game campaign or durable workflow qualification.
- Executable `track_construction_plan.py` start/sample/inspect/cancel workflow,
  private complete receipt-set import and a separate append-only plan journal.
  Full sample/output reservation, synchronized read intent and file/parent
  publication preserve unknown reads across interruption; restart never renews
  policy or restores publication permission. Terminal evidence is immutable and
  offline. The aggregate checker passes all 108 actual Python functions: 59 new
  whole-plan tests and 49 existing single-receipt tests, and rejects five weakened
  implementations. The source-bound report records all 20 input hashes in
  `docs/evidence/construction-plan-monitor.json`. Whole-plan sampled satisfaction
  does not discharge placement effects or prove present usability, live DFHack
  operation or production admission. See `docs/RECEIPT_CONSTRUCTION_PLAN.md`.

- Executable furniture placement through the isolated
  `dfmcp-build-placement-dev-server`: the existing eleven tools now connect exact
  observation, policy-bound review, one-attempt commit, original-key recovery,
  Query-only preparation retirement and offline durable evidence. Real host
  item/target leases, protected regions and explicit checkpoint policy guard
  effects; cached evidence requires current Query authority. Typed insertion
  IDs support follow-up observation, and immutable native uncertainty remains
  visibly unresolved. The binary built on the pinned nightly; all 25 focused MCP
  Rust tests and seven real stdio/TCP/private-file scenarios pass. Correct effective runtime-capability
  checks and minimal older compilation repairs make the path executable without
  changing dependency pins or enabling production admission. See
  `docs/BUILD_PLACEMENT_MCP.md`.

- Executed Rust furniture/1.19 placement backend: strict canonical capture and
  receipt validation, current-authority bounded TCP, a private append-only
  coordinator, and a session retaining the original preparation connection.
  Intent/preparation/dispatch synchronization precedes effects; terminal receipt
  synchronization precedes acknowledgment. Reopened evidence never restores
  commit permission, indeterminate history blocks new keys, and Query-only
  recovery can retire an unattempted preparation after placement revocation.
  All 41 focused Rust tests pass on the pinned nightly, including real TCP and
  Linux custody. Four old test-fixture compilation blockers were corrected to
  enable execution; broader existing assertion failures remain explicit. See
  `docs/BUILD_PLACEMENT_RUST.md`.

- Native furniture/1.19 DFHack handler with exact-item bed/chair/table observation,
  fresh preparation, single commit, query and preparation retirement. Full capture
  revalidation surrounds private allocation; exact construction-job, native
  building, Hauled item/reverse links and final readback gate Placed evidence.
  Partial effects remain indeterminate and block new keys. Refuse hidden payload,
  zone-association writes, secondary item flags and production-profile markers.
- Native handler: 52 groups / 27,429 executed assertions under GCC/UBSan with
  explicit SDK/protobuf doubles, four rejected compiled mutants and five
  actual-handler byte vectors accepted by the strict Python codec. Updated engine: 23 groups / 1,133 assertions,
  eight unchanged independent vectors and five rejected compiled mutants.
  Full Python furniture suite: 52 passing tests. Current-source Clang, real SDK,
  live fortress, Rust/MCP and production qualification remain unestablished.

- Executable furniture/1.19 Python review and one-shot placement client, strict
  native capture/insertion/receipt codec, and private append-only recovery journals.
  Persist intent/preparation/dispatch before effects; preserve immutable uncertainty
  and block new keys until original work resolves. Offline pending-first discovery
  and inspection never dispatch. All 51 codec/POSIX/loopback/CLI tests pass; this is
  not native SDK, live-game, Rust/MCP or production qualification. See
  `docs/BUILD_PLACEMENT_CLIENT.md`.

- Executable read-only excavation floor-goal monitoring over unchanged map/1.5:
  strict capture decoding, distinct advancing-tick sampled stability, fixed
  deadlines, unknown terrain and source/clock invalidation without native mutation.
- Durable `track_excavation.py` start/sample/inspect/cancel workflow with complete
  native evidence, read-intent-before-connection synchronization, file/parent
  synchronization before acknowledgement and restart-safe interruption handling.
  Cancellation stops only the monitor; no native effect obligation is cleared.
- Thirty-five actual Python/loopback/POSIX/subprocess tests pass and four weakened
  implementations are rejected by regression assertions. Complete Agent Turn
  output is bounded. This is not Rust/MCP, live DFHack or power-loss qualification;
  see `docs/EXCAVATION_PROGRESS.md` for the standalone workflow and evidence limits.

- Policy-guarded mining control through the separate `dfmcp-dig-control-dev-server`:
  observe, prepare, exact-plan/review-seal commit, query reconciliation and native
  preparation retirement over the existing Rust session/coordinator and six-method wire.
- Actual host spatial-lease verification, protected shared-block footprints and
  explicit checkpoint policy. Required is the default and refuses without a real
  verifier; only trusted disposable-fortress configuration permits uncheckpointed
  development designation. The query-only recovery server remains unchanged.
- Complete Agent Turn output reservation and pending-work discovery, inherited
  Asupersync blocking ownership and one-attempt original-connection dispatch.
  Thirty-two new core/policy/MCP/runtime Rust tests are registered but uncompiled
  and unexecuted. The published query-schema suite passed 28 valid and 102 invalid
  cases; this is not Rust or native execution. See `docs/DIG_CONTROL_MCP.md`.

- Durable dig/1.16 Rust coordination with intent/dispatch-before-native sync,
  terminal-proof-before-acknowledgement sync, non-restorable commit permission,
  cross-key unresolved-work fencing, fixed recovery modes and typed offline
  record discovery. Require supervising runtime guards at native edges.
- Linux private-file backing with exact owned modes, exclusive locking,
  descriptor-pinned opens, append-only checks, file/parent sync and strictly
  read-only offline replay. No tail repair, native wire or admission changes.
  Thirty-two coordinator/storage tests are registered but uncompiled/unexecuted;
  the passing independent Python framing checks are not Rust or filesystem
  execution. Subsequent ownership, recovery and control integrations are documented
  in `docs/DIG_SESSION.md`, `docs/DIG_RECOVERY_MCP.md` and `docs/DIG_CONTROL_MCP.md`.

- Typed dig/1.16 Rust capture, sealed-plan and native-effect validation, plus a fixed
  six-method RPC client with full-halo/shared-block authorization, source/region
  pinning, nonrenewable connection budgets and same-connection one-attempt commit.
  Query/replayed preparations cannot grant dispatch. Twenty-two Rust regression
  groups are registered but UNCOMPILED AND UNEXECUTED; independent Python checks
  reconstruct the four existing native fixture identities only. The subsequent
  coordinator/storage increment is described above; neither increment changes
  native wire or grants production admission. The subsequent MCP route is described above.

- Read-only work-order-progress/1.12 capture and MCP monitoring: complete selected
  presence, approval/activity flags, remaining-work counters, conservative finite
  template recognition, exact-witness foreground refresh and epoch/reset-safe
  endpoint comparisons. No insertion receipt or counter is promoted to completed
  goods. Includes 24 unexecuted Rust regression groups, 1,358 executed C++ assertions
  on each of GCC/Clang with SDK/protobuf doubles and UBSan, three rejected mutants
  per compiler, and 4,800 independent Python phase cases. See
  `docs/WORK_ORDER_PROGRESS_MCP.md`; this is not production or live-game admission.

- Native pause preparation fencing, bounded token lifetime, map-incarnation invalidation
  and setter/readback exception confinement. The actual handler regressions pass on GCC
  and Clang with native/protobuf doubles, not a real plugin or live game qualification.
  See `docs/NATIVE_PAUSE_FENCING.md` and `CHANGELOG.d/native-pause-fencing.md`.

- Operational situation briefings and deterministic attention in the live spatial/1.8 loop:
  seven native-evidence rules, explicit unknown counts, bounded generation/anchor-bound inspection
  links, compact tactical alerts and `fortress.query` mode `situation` for detailed orientation.
  Observe/wait include bounded endpoint-count changes without inventing causality or event history.
- Current-authority and historical-source separation, watch-specific response reservation through
  the final renderer, and fifteen registered Rust projection/actual-handler scenarios. Those tests
  remain uncompiled/unexecuted; six independent reference JSON packet shapes fit 8,192 bytes.
  Behavior, limitations and evidence status are documented in `docs/SPATIAL_SITUATION.md`.
- `historical_changes` in journal-backed live and archive-only spatial/1.8 sessions: exact record
  pairs, complete selected endpoint projections, generation-aware entered/left/changed rows,
  separate provenance refresh counts, and whole-change pagination without process-local baselines.
  Replay/comparison share one cooperative deadline, current watches are preserved without sampling,
  and full historical Agent Turn metadata is reserved before replay. Sixteen registered Rust
  scenarios remain uncompiled/unexecuted; 66 Python record/page-envelope checks passed without
  validating delegated selectors or runtime semantics. See `docs/HISTORICAL_CHANGES.md`.
- Offline spatial/1.8 archive sessions through `fortress.open_session(recovery_only=true)`, using
  the existing operator-configured observation journal without DFHack, credentials or a connection.
  Fixed-profile replay retains exact entity generations and historical anchors under Query and
  optional Doctor authority; read-only custody refuses creation, repair and write operations.
- Archive-only graph, terrain, inventory and workforce analysis, bounded history discovery and
  exact-record historical queries. Route drill-downs stay pinned to their source record. Full
  Agent Turns label archived evidence as historical and never currently fresh; watches, baselines,
  live acquisition and effects remain unavailable. Fourteen registered Rust scenarios are unrun;
  the independent JSON wrapper-size check passed 128 cases. See `docs/SPATIAL_ARCHIVE_RECOVERY.md`.
- Coherent `workforce_candidates` and `workforce_plan` queries wired into the spatial/1.8 MCP
  dispatcher and schema discovery. Observed skill/readiness and terrain approaches feed the
  existing integral allocator, with capacity one per citizen across 1..16 simultaneous demands,
  at most 128 worker slots, and distinct-worker shortage certificates. These are read-only models,
  not labor assignments, reservations, native eligibility proofs or globally quality-optimal plans.
- Strict occupied-endpoint and unknown-skill handling, whole-row capture/session/model-bound
  pagination, current active-watch reservation and fifteen registered Rust tests. The independent
  Python reference passed 88 schema cases and 4,096 mathematical oracle cases without executing
  Rust. Usage, defaults, exclusions and exact evidence limits: `docs/WORKFORCE_PLANNING.md`.
- Optional restart-safe foreground condition watches for spatial/1.8, paired with its exact
  observation archive through operator-only `DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL` configuration.
  Registration, samples, terminal outcomes, cancellation and release use bounded hash-chained
  checkpoints with render-before-sync and sync-before-publication ordering.
- Recovery under fresh session-bound watch handles, retained definitions/deadlines/evidence,
  reset unfinished stability, explicit downtime uncertainty, current-authority and horizon checks,
  and historical labeling for old terminal outcomes. Existing watch queries and eleven-tool
  interface remain unchanged. Baselines and game effects are not made durable by watch metadata.
- Twenty-five new logical Rust durability/configuration/actual-handler tests and the executable
  independent Python checkpoint framing reference. The reference passed 740 checks; Rust tests
  remain uncompiled and unexecuted here. Full workflow, limits and evidence: `docs/DURABLE_WATCHES.md`.
- Foreground condition watches integrated into protocol-1.1 `fortress.query`:
  `watch`, `poll_watch`, `await_watch`, `watches`, `cancel_watch`, and
  `release_watch`. Typed predicates retain generation fences, failure guards,
  game-tick deadlines, sampling cadence, distinct-observation stability, and
  immutable terminal evidence. Unknown facts never become false under negation.
- A one-observation await path with pre-I/O handle/anchor validation, Query plus
  Observe authorization, and post-refresh reauthorization. It performs at most
  one adapter observation, never controls game time, and skips terminal reads.
- Active-watch projection in query Agent Turns, output-budget reservation,
  render-before-publication state changes, and source-stale metadata/cancellation
  after bridge poisoning. This is not background work or durable mutation state.
- Twenty-five registered Rust watch/refresh/packet scenarios, including an
  8192-byte integrated response path. JSON Schema validation passed 73 local
  cases and preserved every prior definition and ten original query variants;
  the embedded schema now has sixteen variants. Rust execution remains unverified.
- The condition-watch workflow and limitations in `docs/CONDITION_WATCHES.md`.
- Foreground query baselines and endpoint change monitoring through the existing
  protocol-1.1 `fortress.query` tool: `capture`, `changes`, `baselines`, and
  `release_baseline`. Captures retain complete bounded entity selections, carry
  exact anchors and game-time deadlines, and are isolated by session.
- Generation-aware entered/left/changed results with before/after facts, immutable
  baselines, target-bound whole-change pagination, explicit endpoint-only history
  coverage, and separate accounting for selected-view provenance refreshes.
- Transactional response rendering before baseline creation or release. Failed
  full Agent Turn construction does not create, consume, advance, or release a
  baseline. Agent Turns carry the true comparison basis and compact change summary.
- Ten registered public-dispatcher history tests, including full 8192-byte response
  pagination, filtering, generation reuse, presence changes, retries, session/epoch/
  deadline fences, retention bounds, and failed-publication recovery. These tests
  have not been compiled or executed in the editing environment.
- Foreground monitoring workflow and limits in `docs/QUERY_HISTORY.md`; embedded
  query-schema discovery includes the original ten structured query variants.
- Bounded `dfmcp_world::graph_query` reference APIs for canonical multi-source BFS,
  outgoing/incoming/undirected traversal, edge-revision path witnesses, explicit
  depth frontiers, and bounded path reconstruction.
- Iterative strongly connected components, prerequisite-first condensation order,
  cycle-member/dependent blocker diagnosis, and deterministic longest unweighted
  dependency chains for acyclic projections. Results retain exact source and
  authorization-scope identities without granting authority or claiming game
  walkability, complete-world absence, or timed production scheduling.
- Twenty-one query/graph Rust regression tests, including exhaustive directed
  three-vertex graph checks against independent closure/distance oracles, plus
  executable pagination coverage in the existing query truth-table suite.
- Query and graph execution contract documentation in `docs/QUERY_EXECUTION.md`
  and `docs/GRAPH_QUERY_EXECUTION.md`.
- Protocol-bound V2 production admission contract
  (`architecture/live_admission_ticket_v2.json`). The exact bridge protocol now travels from the
  deployment manifest through the compatibility decision, launch record, single-use ticket,
  `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, Rust admission provenance, and final private runner lookup.
  Launch and ticket digests both cover the protocol. The production map currently contains only
  protocol 1.0; protocol 1.1 and unknown protocols fail before live-server startup.
- Protocol-1.1 retained-announcement read generation with a distinct protobuf package, plugin,
  bridge version, source qualification contract, native receipt contract, A1-A6 acceptance
  contract, evidence journal, diagnostic probe, source qualification contract, and development MCP runtime.
- Canonical retained-announcement batches with strict report-ID ordering, bounded UTF-8 text,
  retained-window oldest/latest identities, explicit gap evidence, continuation progress, and
  complete-through-latest semantics without a complete-history claim.
- Transactional protocol-1.1 publication across citizen pagination and announcement continuation.
  No combined capsule is published until the citizen roster and configured retained suffix are
  complete and every page reproduces the same observation state.
- Single-publication protocol-1.1 bootstrap. One complete combined capsule now supplies fortress
  identity, source digest, initial world projection, and adapter bootstrap through a primed replay
  layer without a duplicate underlying bridge read.
- Explicit two-dimensional primed replay over citizen pagination and announcement continuation,
  with cursor, projection, limit, source-manifest, and final-snapshot drift checks.
- `dfmcp-live-v1-1-dev-server`, a separately named, exact-opt-in, read-only development runtime
  preserving the frozen eleven-tool waist and a protocol-specific session namespace. It rejects
  production admission state and cannot consume or impersonate a production ticket.
- Protocol-1.1 world projection, briefing, bounded attention, certified-derived report-ID change
  summaries, and query modes for `summary`, `citizens`, `announcements`, and `all`.
- Aggregate protocol-1.1 source checker that executes the core isolation, transactional
  publication, single-read bootstrap, and development-MCP isolation checkers.
- Mutation suites for production-protocol-map widening, inherited admission, method-waist widening,
  history-coverage overclaim, development-guard removal, process-test loss, and mutation
  contamination.
- Canonical clean-commit source-bundle contract, stable no-follow repository-file reader,
  Git-object-derived deterministic tar creation, hostile archive verification without extraction,
  create-only receipts, and atomic sibling-directory publication after complete verification.
- Authenticated read-only DFHack bridge protocol 1.0 with exactly `Handshake` and
  `ReadObservation`, bounded loopback bearer authentication, canonical protobuf validation,
  generation/version/nonce fencing, stable citizen pagination, and no mutation RPC surface.
- Canonical immutable live-observation capsules whose identity is independent of transport
  pagination, plus deterministic fortress/citizen graph projection with fact provenance and
  explicit complete, conditional, and omitted coverage.
- Live read-only adapter and MCP path covering session bootstrap, observation, heartbeat, query,
  wait, explain, doctor, restart/reset classification, and fail-closed mutation-stage tools.
- R1 native-plugin qualification and R2-R5 disposable-fort acceptance machinery, including
  source/binary receipts, secret scanning, append-only evidence journal, capture guidance,
  deterministic exact-tuple promotion, and registry-generation-bound resolution.
- Owner-private monotonic compatibility floor with exact `0700`/`0600` custody, no-follow reads,
  exclusive initialization, expected-file-digest compare-and-swap, atomic fsynced advancement,
  monotonic sequence and digest chain, and preservation of all previously accepted entry IDs.
- Deterministic authority-free live-admission doctor with fixed registry, floor, exact-tuple, and
  optional server-artifact stages; canonical reports expose `compatibility_ready` or
  `artifact_preflight_ready` without reading a bridge secret or executing a process.
- Source-bound release-server receipt contract sealing the exact clean commit, complete local gate
  order, admission machinery, source digests, platform, executable checks, size, and SHA-256.
- Descriptor-bound admitted launcher that repeatedly verifies registry/floor generation and
  executable bytes, rejects dynamic-loader override variables, emits a secret-free launch record,
  and refuses path-based execution fallback.
- Owner-private, short-lived, single-use admission tickets bound to process ID, bridge protocol,
  exact compatibility entry, registry, decision, monotonic-floor file/content/sequence, server
  receipt, launch digest, executable identity/SHA-256, read-only capabilities, and an empty mutation
  set.
- Rust V2 ticket consumer that validates protocol, rejects legacy V1 tickets, revalidates and hashes
  the current executable, consumes and proves deletion of the ticket, retains admission provenance,
  and only then invokes the exact reviewed private runner; direct `serve-live` invocation fails
  closed.
- Live Agent Turn provenance exposing bridge protocol plus exact compatibility, floor,
  server-receipt, launch, ticket, and executable identities after successful admission.
- Repository source-integrity checks rejecting symbolic links, special files, invalid UTF-8,
  NUL-corrupted or oversized source, machine-local placeholders, recovery debris, and files that
  change while being inspected.
- Adopted the owned `fastmcp_rust` sibling as the MCP presentation plane (ADR-013): modern-only MCP
  2026-07-28, `default-features = false` with `tasks`, pinned to an exact upstream revision for
  dogfooding; upstream defects return through `docs/DOGFOODING_FASTMCP.md`.
- Adopted the owned `eidetic_engine_cli` sibling as an advisory, evidence-linked agent campaign
  memory layer with no canonical-state or authority path.
- Deep source-level audit of asupersync, FrankenSQLite, FrankenFS, FrankenSearch,
  FrankenMarkdown, FrankenGraphDB, FrankenNetworkX, and Doodlestein Self-Releaser.
- Three-plane architecture separating authoritative world/evidence state, derived cognition, and
  narrowly fenced DFHack effects, plus an explicit deployment-admission boundary.
- One observation-capsule version universe for history, projections, subscriptions, branches,
  checkpoints, evidence, and replicas.
- Multi-version world-state specification with positive, negative, range, aggregate, spatial, and
  epoch witnesses; hierarchical conflict refinement; deterministic semantic rebase; and
  proof-carrying merge.
- Canonical fortress graph algorithm plan with explicit tie-break policies, complexity witnesses,
  tiered projections, incremental maintenance, and capability non-interference.
- ATP state/evidence plane covering content-addressed manifests, RaptorQ repair, path racing,
  anti-rollback rules, proof of retrievability, and an explicit prohibition on mutation authority.
- Root-last immutable publication primitives, same-binary performance experimentation doctrine,
  local-only qualification, and DSR release specifications with machine-readable receipts.

### Changed

- Public world query execution now uses `q1` continuations binding the exact
  fortress, epoch, sequence, game tick, state hash, selector set, predicate tree,
  and ordering. This closes cross-query and same-cursor-fork reuse. Legacy query
  cursors require restart; delta encoding remains unchanged. Page width and byte
  budget can vary without changing query identity. Digests are not authentication.
- Query execution verifies the snapshot hash and bounds combined scan/predicate
  work, aggregate identity bytes, and nested kind names before evaluation.
- Migrated the pinned FastMCP dependency to `180a7c88890705217bb8e202d19555adabf24187`
  and published Asupersync 0.5.0. Four modern-only stdio entry points now retain an
  explicit runtime owner and preserve inherited caller context restrictions.
  The existing real subprocess harness has bounded response waits and joined
  reader cleanup; conformance validation is tracked in `dfmcp-k2y`.
- Closed a protocol-confusion defect in process admission: an admitted compatibility decision can no
  longer execute an implicitly selected protocol-1.0 server. Production startup now requires exact
  protocol agreement at every representation and an explicit runner in the V2 production map.
- Hardened ticket custody so both Python issuance and Rust consumption require a real exact-mode
  `0700` directory and exact-mode `0600` ticket. Owner-only but noncanonical modes such as `0500`
  fail closed.
- Moved the protocol-1.1 production-environment refusal to the public `dfmcp-mcp` development API
  seam. External callers cannot bypass the guard by invoking the library wrapper directly.
- Replaced the stale monolithic announcement checker coupling with an aggregate that executes all
  specialized layers while the core checker owns shared protocol, source-map, native, model, and
  qualification invariants.
- Protocol-1.1 source identity now includes the bootstrap checker, development MCP contract/server/
  binary/process tests, and the production V2 admission contract whose runner map it must remain
  outside.
- Local verification compiles every specialized announcement checker and runs the protocol-1.1 MCP
  mutation suite explicitly.
- Local qualification receipts now hash the complete protocol-1.1 publication, adapter, bootstrap,
  transaction-test, development-MCP, and production-isolation source graph rather than only the
  initial wire and batch layers.
- Recovered a corrupted checked-in server-receipt verifier from a known-good source generation,
  rebuilt its exact contract validation, and hardened it around stable no-follow opens, duplicate
  JSON-key rejection, exact gate/source maps, opened-inode verification, and repeated SHA-256
  checks.
- Local verification and qualification now include source-bundle, announcement, compatibility-floor,
  admission-doctor, server-qualification-wrapper, launcher, V2 ticket, source-integrity, and binary
  process gates in one exact source-bound order.
- Workspace moved to Rust 2024 and the latest nightly channel.
- GitHub workflow files target controlled self-hosted machines and serve as locally executable
  specifications; GitHub-hosted execution is not release evidence.
- README, architecture, security, agent rules, roadmap, live-admission documentation, announcement
  documentation, changelog, and implementation status distinguish source presence, development
  execution, qualification, compatibility admission, local floor acceptance, artifact
  qualification, production-protocol dispatch, and runtime admission.

### Security

- Bridge-protocol confusion across manifest, launch, ticket, environment, Rust provenance, and
  runtime dispatch now fails closed.
- Legacy V1 tickets, unknown bridge protocols, protocol 1.1 production attempts, and mismatched
  protocol representations fail before server startup.
- Protocol-1.1 development execution refuses every production admission marker, including the V2
  protocol environment field.
- Compatibility registry rollback relative to an accepted local generation fails closed.
- Same-size executable byte substitution is checked before ticket issuance, before descriptor
  execution, and again by the Rust process before live MCP startup.
- Dynamic-loader injection variables, permissive or symbolic custody, stale compare-and-swap
  identities, direct live-server bypass, and mixed registry/floor generations fail closed.
- Credentials remain absent from receipts, floor files, decisions, launch records, tickets, Agent
  Turns, and admission-doctor reports.

### Current evidence status

- There is no admitted live tuple: the checked-in compatibility registry is empty, and protocol 1.1
  remains explicitly unadmitted development source.
- Spatial situation/attention has fifteen registered but uncompiled and unexecuted Rust scenarios.
  Six independent watch-packet JSON reference cases fit the minimum output budget; this is not
  execution of Rust rules, watch transitions, actual MCP or native/live-game behavior.
- Historical endpoint comparisons are source-present with sixteen registered but uncompiled and
  unexecuted Rust scenarios. The 66 Python checks validate the record/page envelope only, not the
  delegated selector schema, Rust comparison, journal replay, MCP or native/live qualification.
- Offline spatial archive recovery is source-present with fourteen registered but uncompiled and
  unexecuted Rust scenarios. The 128 independent JSON-size checks cover route-wrapper sizing only,
  not Rust, replay, filesystem custody, MCP, native/live-game behavior or qualification.
- Workforce queries are source-present with fifteen registered but uncompiled/unexecuted Rust
  tests. The 88 passing Python schema cases and 4,096 independent allocation-oracle cases do not
  execute the production allocator or establish Rust, MCP, native/live or admission evidence.
- Durable spatial/1.8 watches are source-present with 25 newly registered Rust tests, not executed
  here. The 740 passing Python framing-reference checks do not execute Rust or establish runtime,
  filesystem power-loss, native/live-game, repository qualification or admission evidence.
- Condition-watch schema checks passed 73 local cases (34 accepted, 39 rejected),
  plus three documentation examples. The 25 original Rust scenarios are registered but
  unexecuted: no Rust compiler, Cargo, or rustfmt is available in this editing
  environment. These checks do not establish Rust, stdio, native/live-game,
  repository qualification, or admission evidence.
- The foreground-history tranche is source-present, including ten registered
  regression scenarios. No Rust compiler, Cargo, or rustfmt was available;
  compilation, Rust tests, Clippy, repository qualification, stdio execution, and
  live-game execution have not been established for it.
- The initial query/graph tranche includes an independent Python design check over
  512 exhaustive graphs and 200 seeded multigraphs. That is not Rust-execution or
  qualification evidence. Graph modes subsequently gained protocol-1.1 MCP source
  integration alongside typed inspection, aggregate/search queries, and baselines;
  broader live observation domains remain unfinished.
- The checked-in compatibility registry remains `no_admitted_live_tuples` with zero entries.
- No current live tuple is admitted by that empty registry.
- The V2 production runtime map contains protocol 1.0 only; later profiles remain explicitly
  unadmitted development source.
- No admitted live mutation capability exists. The separate control/1.7 pause-only development
  source and its exact mock-test limitations are recorded in `IMPLEMENTATION_STATUS.md` and
  `CHANGELOG.d/2026-09-15-pause-reconciliation.md`; it is not production admission.
- A fresh full latest-nightly qualification receipt, exact native/live evidence, registry
  promotion, deployment-floor advancement, qualified server artifact, and admitted launch evidence
  are still required for the final current head.

## [0.0.1] - 2026-08-29

### Added

- Initial public design corpus and executable contract scaffold.

## Reality-check source corrections (2026-10-30, in progress)

- Qualified six laboratory server module paths; escaped string fields in the
  legacy EE JSONL helper and added hostile-text record-boundary regression
  coverage. The legacy output is not the E09 batch schema or EE integration.
- Scoped the production Rust validator to exclude explicit test-only modules
  and lexical comments and literals while retaining production checks; repaired
  source-bundle documentation phrase formatting. Room-example schema findings
  remain separate and are not suppressed.
- Native build-system and protobuf API repairs are in progress and unqualified.
  The earlier disposable-fort trial used `6e608849dc2b418c85ea508c382469110744b79d`
  plus isolated source/link overlays and does not qualify the later dirty
  `f502dd7bc9e1ddf084a46b37bf962ee673bd1635` worktree. Search, ledger, ATP,
  EE export, structural rebase, mixed-family dispatch and resource wiring retain
  the boundaries recorded in `IMPLEMENTATION_STATUS.md`.
