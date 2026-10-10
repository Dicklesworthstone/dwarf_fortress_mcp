# Semantic actions in the deterministic laboratory

The process-local laboratory (`dwarf-fortress-mcp`, `run_stdio`) executes every
semantic action family except extensions, through the same eleven tools. It is
a deterministic simulator for developing and evaluating agents and for testing
the plan → commit → wait → prove loop. **It is not evidence about Dwarf
Fortress or DFHack behaviour**; progress rates are a laboratory calibration.

## One definition of what an action means

`dfmcp_intent::effects` is the single reference model of each action:

| Action | Effect on canonical state | Default proof | Temporal |
|---|---|---|---|
| `pause` | fortress pause flag | `Paused(p)` | no |
| `designate_dig` | dig-designation entity; tiles converted over time in z/y/x order | `RegionTerrain{area, target tile}` | yes |
| `build` | building entity, `planned → under_construction → complete`; footprint must be observed floor | `construction_stage == complete` on the created entity | yes |
| `create_work_order` | work-order entity counting `amount_remaining` down | `amount_remaining == 0` | yes |
| `set_labor` | `labor.<LABOR>` on each unit | field equality per unit | no |
| `set_burrow_membership` | `burrow.<id>` on each unit | field equality per unit | no |
| `assign_squad` | `squad` on each unit | field equality per unit | no |
| `configure_stockpile` | `accepts`, optional bin/barrel/wheelbarrow limits | `accepts` equality | no |
| `set_standing_order` | `standing_order.<key>` on the fortress settings entity | field equality | no |

Entities an action creates get an identity derived only from the step's
idempotency key (`effects::created_entity_id`), so the sealed plan names them
before dispatch and a retry cannot create a duplicate.

The planner fills omitted postconditions, a bounded obligation for temporal
work (deadline scaled to the work, after every prerequisite's deadline, capped
at one game year), and an exact inverse compensation where the action fully
determines it (labor, burrow membership, pause). Explicit caller choices win;
everything is sealed into the plan digest.

## Agent workflow

```text
fortress_open_session(scenario="starter_fortress",
    requested_capabilities=[["observe","read_only"],["query","read_only"],
      ["plan","reversible"],["control_clock","reversible"],["checkpoint","guarded"],
      ["designate","guarded"],["construct","guarded"],
      ["configure_labor","reversible"],["configure_production","reversible"]],
    max_game_ticks=2000, paused=false)
fortress_query(mode='{"mode":"entities","kind":"unit"}')
fortress_query(mode='{"mode":"terrain","min":[0,0,10],"max":[9,5,10]}')
fortress_plan(actions='[
  {"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},
  {"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},
  {"action":{"kind":"create_work_order","name":"brew","job_token":"BREW_DRINK","amount":2},"depends_on":[1]},
  {"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":true}}]')
fortress_commit(plan_digest=...)
fortress_wait(max_game_ticks=100)   # repeat until open_actions_remaining is 0
```

Instead of explicit actions an agent can state an objective:

```text
fortress_plan(blueprint='{"template":"bedroom_cluster","origin":[3,5,10],"rooms":4,"room_size":[3,3]}')
```

The blueprint planner lays out rooms, doorways, corridors and an entrance,
refuses the objective when its one-tile hazard halo is not completely observed
or touches magma (or a span is unsupported), and adds furnishing steps — a bed
per bedroom, a table and chair in a dining hall — each depending on its room's
excavation. The result is an ordinary sealed plan with forecast; `actions` and
`blueprint` are mutually exclusive.

`scripts/lab_stdio_walkthrough.py` runs exactly this sequence against the real
`dwarf-fortress-mcp serve` binary over stdio.

`fortress_plan` also returns a `forecast` (epistemic state `predicted`): the
sealed plan is committed on a discarded fork of the world and laboratory time is
run forward to every obligation deadline, reporting each step's predicted
terminal state and tick, `predicted_completion_tick`, `blocked_by_pause`, and the
forecast's `resolution_ticks`. A step that would fail at commit (for example
digging unobserved terrain) shows up as `available: false` with the refusal,
before anything is committed. Advancement and polling refusals also make the
forecast unavailable with their actual reason; a failed partial simulation is
not reported as an available prediction. The final slice is clipped to the
requested obligation horizon. Forecasts assume no other agent acts and the
fortress stays as it is; real completion also depends on how often the agent
waits, because deferred steps dispatch when a wait observes their prerequisites.

The Agent Turn compares those forecasts with what is later observed. When a
step reaches a different terminal state than forecast, completes later than the
forecast by more than its resolution plus the last wait's step, or a commit
finds the anchor moved (and is replayed), the turn carries a **surprise record**
(`attention` category `surprise`: predicted vs observed, explanation, lesson
candidate), as the operating model requires: silent prediction error prevents
learning.

`fortress_plan` returns each step's capability, risk, created entity, sealed
postconditions and obligation (terminal, deadline). Steps whose dependencies are
not yet verified stay `Prepared` and are dispatched by a later `fortress_wait`,
which polls every open action of every committed plan in commit order (so a
later plan never strands an earlier plan's deferred steps) and reports them in
`polled_actions` with `open_actions_remaining`.
Before that first deferred effect, the laboratory rechecks the current action
grant (including entity/region scope and expiry), action budget and sealed
preconditions. Observe-only polling can verify an already dispatched action,
but cannot dispatch its successor. A missing grant refuses the poll without
changing the world or the prepared receipt.

Failed, cancelled or compensated prerequisites fail their undispatched
descendants with evidence that those steps were never dispatched. An
undispatched step's own obligation deadline and failure predicate apply even
while dependencies are unresolved. Missing or indeterminate prerequisite
evidence blocks dispatch; it never proves a predecessor had no effect. Plan
preparation expiry is not substituted for a committed obligation's longer
deadline. Once a receipt is terminal, later polls return its original anchor,
digest and evidence unchanged. Completion first observed after the obligation
deadline is failed; sufficient proof at the deadline itself is accepted.

`fortress_wait` lets time pass only while the fortress is unpaused and within
the session's game-tick budget; a paused fortress reports `blocked`. Positive
tick requests require `control_clock`, and an unpaused advance must fit within
the clock and observation grants through the requested final tick. A zero-tick
poll needs observation authority without clock control.

On the modern stdio server, a client that negotiates Tasks can use
`fortress_commit(as_task=true)` to retain and supervise the original plan under
an opaque MCP task handle. `tasks/get` returns its status and terminal evidence;
`tasks/cancel` drains that original plan even after a later plan is committed.
Agent Turns and handoffs expose handles, with bounded discovery and detail at
`df://session/{id}/tasks`. Task reads do not advance game time or dispatch
deferred work. See [`LAB_MCP_TASKS.md`](LAB_MCP_TASKS.md) for negotiation,
cancellation, retention and the one-active-monitor process bound.

`fortress_cancel(mode="stop_future_steps", scope="plan")` drains the last
committed plan, dependents before prerequisites. `scope="session"` includes
retained open work from earlier plans owned by this session. If that exceeds
one call's budget, `scope="oldest_open_plan"` selects the oldest unfinished
original plan and returns its digest; repeated calls can drain older plans
after a later plan has finished. Each certificate covers only its selected
actions. Deferred steps
never dispatch during cancellation, and stopped work keeps completed progress
(excavated tiles stay excavated). Current observation and original scoped
effect grants authorize stopping; emergency pause additionally requires clock
authority and resets shared unpause consent after an authorized pause. A
partial drain refusal retains its actual clock, anchor and work progress in
the Agent Turn. The aggregate budget covers every stop, compensation and pause.

A Failed or early Verified goal can still own active physical work. Its
`work_state` remains visible in waits, Agent Turns, Tasks and handoffs. Cleanup
preserves that original proof receipt and emits separate `physical_drain`
evidence. Progress includes `remaining_nonterminal`, `remaining_work` and
`terminal_work_stopped`; a finalize certificate requires no unfinished proof
or active/unknown physical work. Unresolved work keeps spatial ownership after
its proof deadline and lease expiry. Without `scope`, cancellation addresses
the most recent action.

If compensation is refused, a pending cancellation may explicitly narrow to
`stop_future_steps` under current original work authority. This abandons the
compensation request and stops the remaining work without applying an inverse.

After checkpoint restore or durable recovery, physical entities may remain even
when their originating action handles are unavailable. `untracked_work` exposes
that active or unknown snapshot work; waits, observations, Agent Turns, handoffs
and doctor retain it. A complete eligible lifecycle observation is required to
remove it from the census. Unknown spatial geometry fences new spatial work
globally. Restoring or reading such a record grants no stop authority and creates
no historical goal proof. It must be reconciled or become observably quiescent
before the affected region is reused or a session-wide drain is certified.

Commit authority is the plan's own capability set: a session that did not
negotiate `designate` cannot commit an excavation, and an idempotent replay
re-checks that authority. Effect capabilities are never granted by default.

The `starter_fortress` scenario is 48x48 rock at z=9..11 with, at z=10, a carved
10x3 hall at (0..9, 0..2), seven dwarves (entity IDs 1001–1007), a stockpile
(2001), a burrow (3001) and a squad (4001).

## In-process dispatcher

`dfmcp_adapter::dispatcher::MutationDispatcher` executes the same reference
semantics for embedding and tests. Temporal steps are recorded as
`AppliedAwaitingVerification`; `reconcile(plan, snapshot, ctx)` proves or fails
them against whatever later observation the caller supplies (it never
simulates time) and then dispatches newly unblocked steps.

## Several agents in one fortress

`fortress_open_session(shared=true, fortress_selector="N", ...)` joins (or
creates) one shared fortress per selector. Members share a single canonical
world, game clock and lease book; each keeps its own grants, budget, plans,
receipts and Agent Turn. A joiner gets the existing world (naming a different
scenario is refused) and the response reports `shared_world.members`.

- **Regions are leased.** Committing an excavation or construction step takes
  an exclusive spatial lease on its area until the step's obligation deadline.
  Another member's commit that overlaps it is refused with `conflict` before any
  effect; a session never conflicts with itself. Leases are released once the
  holder observes the step terminal (verified, failed or cancelled).
- **Unrelated concurrent work does not force a replay.** A sealed plan's
  read witness is every entity, edge and pause flag its predicates and action
  scopes name, plus every terrain region they touch widened by a one-tile
  hazard halo. If the anchor moved but nothing in the witness changed between
  the version the plan was sealed on and now, the intent is replayed at the
  current anchor and, when the replay performs the very same actions, committed
  directly; the receipt carries `witness_rebase` (a certificate naming both
  digests, both state hashes and the witness) and a retry with the original
  digest returns the same receipt. Anything unbounded or outside the retained
  history falls back to the explicit replay below, with `witness_check`
  naming the first read that changed.
- **Stale plans are replayed, never committed blind.** If another member's
  action or the shared clock moved the anchor after a plan was sealed, the
  commit returns `stale_anchor` with a `rebased_plan` (the original request
  re-planned and re-sealed at the current anchor, preconditions rechecked) and a
  `rebase` record. The Agent Turn recommends committing the new digest; the old
  digest can never be committed.
- **Unanimous unpause.** Any member may pause a shared fortress at once (the
  emergency brake), which clears every unpause consent. Committing an unpause
  records the member's consent and returns `clock_consent` (votes of members)
  without dispatching until every current member has consented.
- **One clock.** Any member's `fortress_wait(max_game_ticks)` advances everyone's
  work; each member proves its own obligations when it next waits.
- **No unilateral rewrite.** `fortress_restore` is refused while other members
  share the fortress.

Calls are serialized per fortress: each tool call holds the world for its
duration, so every response is consistent with one anchor.

## Crash-durable fortresses

When the operator starts the server with `DFMCP_LAB_STATE_DIR=/absolute/dir`,
`fortress_open_session(durable=true, fortress_selector="N", ...)` makes fortress
`N` survive process loss:

- every state change is persisted after the tool call that made it, and every
  `fortress_checkpoint` is persisted (`durable: true` in its receipt);
- reopening `N` with `durable=true` after a restart resumes the last persisted
  world in a **new observation epoch** (`durable.resumed`,
  `recovered_from_anchor`, `restorable_checkpoints`); naming a different
  scenario is refused;
- checkpoints taken before the restart restore normally;
- designations, construction and work orders live in the world, so they keep
  progressing on `fortress_wait(max_game_ticks)` (which no longer needs a
  committed action). Adapter action handles do not survive restart; unfinished
  sealed obligations are recovered as observation-only proof monitors;
- **commits survive too.** Before a durable commit dispatches anything, its
  agent request and the exact world it was sealed against are journaled. After
  each call, submitted step transitions, the world head and completed-plan
  retirement are published in one atomic progress record. A crash exposes
  either the preceding complete frontier or the new complete frontier. New
  transitions retain their exact snapshot anchors through compaction; older
  step records remain distinguishable by their missing anchors. On resume each
  unfinished commit is deterministically recompiled from that request and world
  and must reproduce its sealed digest. Unreproducible plans, inadmissible proof
  specifications and legacy terminal states without atomic proof anchors stay
  `indeterminate`, retain their durable records and require reconciliation.
  Missing or unanchored legacy nondispatch/abandonment also remains indeterminate
  when the saved world differs from the sealed basis. New frontiers explicitly
  anchor deferred steps as `not_dispatched` while keeping their plans open.
  Ambiguous work is never silently retired or made eligible for blind retry.
  Dispatched steps come back as `carried_obligations`; their sealed postconditions
  and terminal predicate are proved through the normal obligation runtime.
  Recovery preserves the original absolute deadline and polling interval, resets
  an unfinished stability streak, and does not count the archived frontier as a
  fresh positive sample. Repeated reads at one tick cannot manufacture progress.
  Exact-deadline proof is eligible; first proof after the deadline fails.
  A current Observe grant is required, and an interrupted observation resets an
  unfinished stability streak. `recovered_commits` lists recovered states and
  original evidence anchors. Recovery never redispatches actions;
- **original goals survive completed commits.** Goal admission stores the exact
  original source, sealed snapshot and originating session with the action
  commit before effects. A separate goal record remains after all its action
  records retire. Reopening recompiles the original source against its archived
  sealing snapshot and verifies the sealed digest. It independently checks any
  first-achievement predicate against that exact archived proof snapshot;
- a restore publishes its restored world and abandonment of every old carried
  or in-flight commit and retained goal pursuit in one atomic frontier. First
  achievement remains historical evidence, and a restored world cannot newly
  complete an abandoned goal. Pending abandonment survives a failed save in
  memory for retry; a crash before publication recovers the preceding world
  together with its original work and goal records;
- reopening a private durable fortress fences its older sessions. The ownership
  check remains locked through each call and its save, including calls that
  resolved a session before the replacement opened. Shared durable sessions
  hold their unfinished plans, carried monitors, original goals and persistence
  fault in the common world. A peer cannot bypass another member's failed save.
  Joining an
  already running shared fortress does not reload the journal, bump its epoch
  or reconstruct its work again (`durable.joined_existing: true`,
  `durable.resumed: false`). Failed joins remove their new membership without
  dropping the world's outstanding progress. Mixing private and shared durable
  writers for one fortress in the same process is refused; a second server
  process on the same directory is refused by the store lock;
- `fortress_doctor` reports `durability` (persisted anchor, whether it is
  current, journal records, chain head, torn tail discarded at open).

Storage: `journal` (one `<chain> <record>` line per atomic progress frontier,
head, checkpoint or retained commit record, SHA-256
chained) plus `objects/<sha256>.snap` holding exact canonical snapshot bytes.
Objects are synced and renamed before the record naming them is appended and
synced. On open only an incomplete final record is discarded; a broken chain,
malformed record or corrupt object refuses the store. Progress records are
bounded at 8 MiB (at most 256 commits with 256 steps each); other records retain
their 64-KiB limit, and the journal is bounded at 64 MiB. The journal is compacted
to live records (and unreferenced objects removed) every 1,024 records or before
its byte limit would be exceeded. Sealed-plan, retained step-proof, first-goal-proof
and goal-abandonment snapshots remain pinned. An uncertain write, sync failure
or automatic compaction failure after publication refuses further writes,
including apparent no-ops, until the store is reopened and its complete journal
prefix is recovered.
`scripts/lab_durable_restart.py` demonstrates it across a SIGKILL.

### Original-goal history and current truth

The `objectives` projection on session open and subsequent observations requires
current Observe authority, including for historical metadata. Without it the
response contains an unavailable/unknown coverage entry and discloses no goal
source, owner, predicate, anchor or actual goal count. Recovery grants no
observation or dispatch authority.

| Field | Meaning |
|---|---|
| `original_source` | Exact retained request kind and request, used to reconstruct the original sealed goal. |
| `predicate_truth` and `observed_anchor` | Current authorized goal evidence, independent of historical achievement and action completion. |
| `historical_achievement` | Whether the retained first-achievement anchor has been verified against the original predicate; unverified recorded history cannot establish success. |
| `first_satisfied_anchor` | Immutable exact snapshot that first proved the goal. Consumption or later observations never move it. |
| `restore_abandoned_anchor` | Exact restored world at which the original pursuit was abandoned. Earlier achievement remains historical. |
| `owner_session_id` | Originating-session metadata. Numeric IDs can repeat across processes; recovered goals do not become owned by a new session with the same number. |
| `owned_by_current_session` | True only for the actual originating session in the same live process. Shared peers observe the same goal without acquiring its ownership. |
| `physical_quiescent` | Current inspection of every original effect identity; missing bookkeeping cannot stand in for completed work. |
| `needs_replan` | Original work is quiet and the original goal is currently false. No replacement work is dispatched automatically. |

A goal that first reached 60 drinks and later falls to 53 reports current false
truth and `no_longer_holds`, while preserving its original verified achievement
anchor. A never-achieved goal retains no invented historical success. Neither
the original preparation expiry nor a new session renews the goal into a
replacement plan.

At most 64 original goals are retained per fortress. Admission checks capacity
before consent, reservations, adapter preparation or effects. Ordinary eviction
requires verified historical achievement, a currently proven true original goal,
quiescent original effects and no unfinished durable commit. Unmet, unknown,
abandoned or unverifiable goals cannot silently yield capacity. Some recovered
never-dispatched work can remain conservatively unknown after its old action
bookkeeping retires; this can retain history longer, but cannot authorize
replacement work or unsafe eviction. Legacy action-only records gain no inferred
goal history.

## Live routing of sealed plans

Every `fortress_plan` response carries `live_routing`: how each sealed step maps
onto the live DFHack development families (`dfmcp_adapter::live_routing`), or
why it cannot.

| Semantic action | Live family | Limits |
|---|---|---|
| `pause` | control/1.7 | — |
| `designate_dig` (`mine`) | dig/1.16 | tiled into ≤8×8 single-level rectangles in z,y,x order; ordered coordinates 1..32766 on all axes for a complete halo; ≤64 rectangles per step; other modes refused |
| `build` `furniture:Bed/Chair/Table` | build/1.19 | single-tile footprint at its location and native coordinate bounds; needs an exact live item satisfying every retained material constraint |
| `create_work_order` `CONSTRUCT_BED/DOOR/TABLE/THRONE` | work-orders/1.10 | wooden only, amount 1..100, no conditions |
| `set_labor` | workforce/1.17 | ≤32 canonical units; evidence-bound native mapping, game paused and one selected-only detail containing exactly the requested labor |
| stockpile, squad, burrow, standing order, extension | none | refused with a reason |

Each routable step lists the typed request (for example the exact dig
rectangles), the live values still to resolve, the family's own live
preconditions, the unadmitted development server that executes the family
(`dev_server`) and, for excavation, the exact `fortress.observe` region call per
rectangle for that server (`dev_server_observations`). Routing is deterministic and pure: it grants no capability,
performs no I/O, and every family remains unadmitted development execution.

`execution_ready` remains explicitly false at both route and step level.
Workforce requests expose `canonical_units` as decimal strings, the exact
`labor`, and `native_units: null`; canonical IDs are never cast to native IDs.
Furniture requests retain all four material selector fields. A family match
alone cannot establish a unit mapping, item selection or semantic success.

The Rust `LiveRoutingEvidence` constructors bind actual V1 or spatial/1.8
projections to independently issued source/domain evidence. Workforce resolution
requires an exact paused native capture and one unambiguous single-labor detail;
removal refuses overlapping grants. Even native `Applied` must pass a separate
readback check preserving every other labor column. Furniture resolution binds
exact source item identity, material IDs, position, eligibility and map dimensions.
Non-default token, nearest-item or reservation selectors explicitly refuse until
the native family can establish them. These APIs produce reviewable native plan
candidates and grant no dispatch or original-goal authority. See
`docs/LIVE_SEMANTIC_ROUTING.md` for the complete source and identity contract.

## Deterministic replay bundles

Every tool call of a laboratory session is recorded (exact arguments, `ok`,
error code, resulting canonical anchor). `df://session/{id}/replay` (requires
`observe`) exports the log as a `dfmcp.replay.bundle/1` with a `calls_digest`
over the calls. `dwarf-fortress-mcp replay bundle.json` re-executes it in a
fresh session and prints either `ok` or the **earliest divergence**: the call
`seq`, tool, field (`ok`, `error_code` or `anchor_after`) and the expected and
observed values; it exits non-zero on divergence or refusal. A bundle whose
calls do not match its digest is refused. Sessions on a shared or durable
fortress depend on state outside their own log and are exported with
`replayable: false`. `scripts/lab_replay_roundtrip.py` demonstrates recording,
exact replay and localizing a tampered call over real stdio.

## Typed observation profiles and complete records

The laboratory query mode `observation` exposes the canonical completeness
profiles through the existing query tool:

```python
fortress_query(mode='{"mode":"observation","completeness_profile":"operations","section":"entities","limit":25}')
fortress_query(mode='{"mode":"observation","completeness_profile":"spatial","section":"chunks","limit":10}')
fortress_query(mode='{"mode":"observation","completeness_profile":"historical","section":"events","limit":25}')
```

The five profile names are `control-minimum`, `operations`, `spatial`,
`historical`, and `research-full`. The closed `section` selection accepts
`entities` (default), `relations`, `chunks`, or `events`; page limits are
1 through 100. See `WORLD_MODEL.md` for each profile's exact inclusion policy.
These are content profiles, separate from the Agent Turn presentation profiles.

Every page identifies its original `source_anchor`, separate
`projected_anchor`, profile envelope digest, source schema and laboratory
manifest. The JSON is a bounded rendering of records; it is not the canonical
envelope encoding. World-library callers can use the immutable profiled
snapshot and capsule codecs for exact canonical round-trips. These reference
observations establish no live Dwarf Fortress capability or action authority.

Rows preserve complete structured values, entity/relation identities, relation
endpoints, terrain RLE runs, event fields, and explicit field-presence metadata.
Unknown, absent, unsupported, omitted, redacted, and stale facts do not expose
retained compatibility values as current knowledge. Known null is distinct
from absence. Binary payloads retain the established byte-length rendering and
are explicitly marked `omitted`, including when nested inside a list, event
field, or chunk overlay.

Coverage names the selected **record-membership** domain. Complete membership
does not make every field known. Excluded sections report
`section_included: false` and explicit profile omissions; an empty excluded
section never proves absence. Historical event coverage refers only to the
supplied retained event window.

Pass the serialized `continuation` object as the next query's `mode` argument.
It preserves the source hash, profile, section, limit, and next offset. The
existing historical router resolves that exact source; an expired source
anchor is refused. A projected hash is never substituted for the retained
source address. If the output budget shortens a page, the continuation resumes
after the records actually returned. Empty, excluded, or exhausted collections
have no continuation.

Ordinary `entities` queries also return exact scope and pagination metadata.
A filter with unresolved rows cannot prove absence, including under negation.
Response reduction removes whole records and marks their domain partial; it
does not shorten semantic field lists, coordinates, predicates, or evidence,
and it preserves known null members and the coverage needed to interpret them.

## What changed, every turn

Each session retains a bounded history (32 versions) of the exact canonical
world versions it has seen. Every Agent Turn's `changes` now lists, besides
protocol events, the **observed world changes** between the anchor the agent
saw last and the current one: `game_time_passed`, `fortress_paused` /
`fortress_unpaused`, `entity_created` / `entity_removed`, `entity_changed`
(each changed field with its value before and after; at most 8 fields and 24
entities per turn, with `entity_changes_omitted` beyond that) and
`terrain_changed` per level (tiles changed, bounding box, tile transitions such
as `wall->floor`). If the previous anchor aged out of the history the turn says
`history_not_retained` (epistemic state `unknown`) instead of guessing.

The same history serves exact historical reads: structured `fortress_query`
requests, including observation profile pages, may add `"at": "<state_hash>"` to read that retained version
exactly (`historical: true`, with the current anchor alongside), and
`{"mode":"changes","since":"<state_hash>"}` lists the observed changes from
that version to now. A version outside the retained history is refused with
`stale_anchor`, never approximated.

## Fortress economy

`starter_fortress` has a stock ledger (entity 5001, kind `stock_ledger`) with
`stock.drink` 40 and `stock.food` 60. As game time passes every living dwarf
drinks one unit per 1,200 ticks and eats one per 2,400 ticks (laboratory
calibration). When a stock runs short the dwarves served last (highest id)
go without and their `need.drink` / `need.food` turns `thirsty` / `hungry`;
a later full round makes everyone `satisfied` again. Work orders produce
stock: each `BREW_DRINK` unit adds 5 drink, each `PREPARE_MEAL`/`COOK_MEAL`
unit 5 food. Production that completes on a meal boundary is available for
that meal; later production cannot feed an earlier meal.

`fortress_observe` and `fortress_wait` return `world_alerts` and the Agent Turn
raises them as `fortress_needs` attention: `high` when a stock lasts less than
four rounds, `critical` when it is exhausted or dwarves are deprived. Each alert
carries a `remedy` — the exact `fortress.plan` arguments for a work order
sized for about four rounds. Worlds without a ledger (`empty`) have no
metabolism. None of these rates are claims about Dwarf Fortress.

### Original production quotas

A production request such as
`{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}`
retains that original stock goal through planning, replay and the durable source
record. Stale-plan replay recalculates batches from current eligible inventory;
its `rebased_plan.production` reports the new stock and proposed production.
Requests accept 1–64 submitted quotas for exact `DRINK` or `FOOD` tokens. Duplicate
quotas combine by their greatest minimum and canonical token order. Explicit
production and its blueprint alias cannot both be supplied.

The plan-level terminal condition is the conjunction of every original stock
minimum, including a quota that was already satisfied and generated no action.
Individual work orders retain their exact action-completion postconditions.
Consumption can therefore leave a completed plan's original stock goal unmet.
`fortress_wait` still reports action progress in its top-level status; its
`objectives` and Agent Turn carry the separate original-goal truth.

A Task also exposes `original_goal`, `action_proof_status` and physical work. It
can complete only with a currently proven original goal and completed, quiescent
original work. Finished work with a false goal reports `needs_replan`; unknown
goal evidence requires reconciliation. Neither case dispatches a replacement
order. Rebase aliases resolve to the actual committed objective, and current
Observe authority gates the goal evidence. The original production source tag
does not reconstruct quota intentions from older action-only journal records.

### Conditional production

Work orders accept a bounded `conditions` array through the ordinary laboratory
action request. For example, this requests two brewing units, allowing work only
while the modeled drink count is below 50:

```json
[{"action":{"kind":"create_work_order","name":"brew to 50","job_token":"BREW_DRINK","amount":2,"conditions":[{"kind":"item_count_below","item_token":"DRINK","threshold":50}]}}]
```

All conditions must hold. Their closed JSON forms and reference meanings are:

| Condition | JSON fields after `kind` | Required evidence |
|---|---|---|
| `item_count_below` | `item_token`, `threshold` | The selected exact unsigned stock count is strictly below the threshold. |
| `material_available` | `material_token`, `minimum` | The selected exact unsigned material count is at least the minimum. This is an availability gate; it consumes and reserves nothing. |
| `completed_order` | `order_name` | Exactly one other work order has this established semantic name, `status="complete"`, and `amount_remaining=0`. |

`DRINK` and `FOOD` item tokens use the existing `stock.drink` and `stock.food`
fields. Other exact item tokens use `stock.item.<TOKEN>`; material tokens use
`stock.material.<TOKEN>`. These are separate namespaces on the existing
lowest-ID `stock_ledger`, not totals inferred from item records. The producer or
scenario must supply an explicit count: a missing field is unknown, including
when a material minimum is zero. Counts, condition records, and dependency
evidence must have eligible laboratory provenance and known, consistent,
nonfuture values. Retained assertions, replay values, omissions, or stale fields
cannot release production. This defines a laboratory inventory contract; it
does not infer native recipes, material consumption, path access, or DFHack
eligibility.

Conditions gate production units. For an order producing the stock it checks,
one indivisible unit may cross the threshold; the next unit is blocked. The
bounded event timeline checks the gate at every completion. A blocked order
remains active and records the first canonical reason in `blocked_by`.
Previously earned partial-unit work is retained, but blocked elapsed ticks are
discarded. A later stock or dependency event can release work for subsequent
time in the same advance. Completing the requested amount ends the order;
conditions do not create recurring orders.

Named dependencies use the source-qualified `order_name` field, never display
labels. Missing or ambiguous names, self-reference, cancelled predecessors, and
unknown completion evidence block. Establishing uniqueness also requires the
other work-order names in the supplied domain to be known. A prerequisite's
completion releases its dependent only for time after that completion. A
50-tick prerequisite followed by a 50-tick dependent therefore needs 100 ticks,
regardless of their entity IDs or how the caller divides those ticks into waits.

The reference boundary accepts at most 64 conditions and 256 non-control bytes
per token/name; the MCP request retains its existing 128-byte name limit and
16-KiB total action-request bound. Condition lookup requires a complete supplied
domain of at most 65,536 entities. Exact duplicates are removed and conditions
are sorted for both normalized action identity and the stored typed `conditions`
list. Compound conditions retain conjunction semantics. The production blueprint
now forwards its calculated stock thresholds into these executable conditions,
and refuses requested stock quantities without eligible current evidence.

New work orders always store an explicit condition list, including an empty list
for unconditional work, plus their semantic name. Their default completion proof
binds that exact condition list, name and job token as well as complete status
and zero remaining work. The created identity remains derived from the sealed
step key, so retries cannot replace an existing order with different conditions.

Older saved orders without a condition record remain blocked: the missing field
cannot reveal whether the old implementation discarded a nonempty request.
There is no automatic upgrade to an empty list. An explicit trusted migration or
observation is required to establish that configuration. Existing sealed plans
retain their original identity; if recompilation cannot reproduce a retained
seal under the stronger default proof, the existing recovery path keeps it
indeterminate. Conditional production adds no live mutation capability or
compatibility admission.

### Finite production capacity

Brewing and cooking share the observed labor pool. During each causal interval,
one living unit with the required eligible labor flag and one completed matching
workshop can serve one running order. The same multi-skilled unit cannot brew
and cook at once; adding orders alone does not create production capacity.

The deterministic allocator first prefers orders with more earned partial-unit
work, then lower canonical entity IDs. It finds a maximum set of simultaneous
assignments for the two registered recipe families, moving a flexible worker
when doing so lets a specialist serve another order. This is a throughput and
continuation policy, not a skill, travel or fairness optimizer.

Conditions are checked before allocating service. An order waiting for a stock
threshold or prerequisite consumes no worker or workshop slot. An eligible order
that cannot acquire service records a worker- or workshop-capacity explanation
in `blocked_by`. It retains earned partial work and earns no waiting ticks.
Completion, new workshop availability, stock changes and worker death release
or remove capacity only for subsequent intervals. Successful wait partitions
preserve the same physical outcome.

Capacity matching admits at most 65,536 canonical entities before its ready,
blocker and candidate allocations. Source scans, matching traversal and sorting
consume the same bounded advancement work allowance. A refused advance still
publishes no partial world through the ordinary transaction-shadow contract.

This pool covers `BREW_DRINK` and `PREPARE_MEAL`/`COOK_MEAL` only. Other
reference job tokens retain their abstract behavior. Mining, construction and
military activity do not yet compete for these worker slots; none of these
scheduling rules describes native Dwarf Fortress capacity.

### Causal time, competing work and bounded advancement

The reference model advances between actual work and world-event boundaries:
production units, construction completion, excavated tiles, meals, hostile
arrival and combat rounds. Eligibility is frozen at the start of each positive
interval. At a shared boundary, construction and excavation settle first,
production completions settle in ascending entity order, drink and food are
consumed, and then arrivals and combat settle in ascending hostile order.
Current production blockers are refreshed after those events. A newly completed
workshop, consumed stock, completed prerequisite or worker death changes the
next interval; it never grants or removes time from an earlier interval.

When two orders reach the same stock threshold together, the earlier canonical
entity may complete the unit that crosses it. The other order retains 49 of its
50 required work ticks; only the refused completion tick is discarded. Unequal
partial work instead completes in actual time order. This preserves earned
work and prevents arbitrary wait boundaries from changing which order wins.
Excavations cache their pending terrain once per advance and update overlapping
designations when a tile changes, avoiding a full region scan per excavated tile.

With the same source world and no intervening actions, successful advances
produce the same physical values under different wait partitions: stock,
remaining work, partial work, terrain, needs and life state. The public snapshot
cursor, revisions and observation timestamps still reflect actual publication.
Internal events do not poll obligations or add stability samples. Deferred plan
steps still dispatch only when a foreground poll observes their prerequisites,
so dispatch and proof cadence remain distinct from physical simulation timing.
These are reference-model semantics, not native Dwarf Fortress timing claims.

`advance_effects_with_limits` accepts an explicit `EffectAdvanceLimits`; the
ordinary `advance_effects` wrapper uses the hard defaults: at most 1,000,000
game ticks, 200,000 timeline intervals and 100,000,000 deterministic work units
per physical advance. Callers can reduce these limits. Aggregate excavation
footprints also have a hard 1,048,576-tile cache bound, checked before any region
allocation or terrain read. Source facts, entity and condition scans, population
scans and terrain visits consume the work budget.
Budget or counter overflow returns `BudgetExceeded`. Future-dated eligible facts
are rejected against the source tick before time moves. Active work with
noncanonical counters or unknown excavation terrain refuses instead of guessing.

The effect function operates on its caller's transaction shadow. `MemoryAdapter`
publishes only after the entire requested advance succeeds; refusal preserves
the original world, receipts and transcript even if earlier internal events
would have succeeded. Budget admission is per call: splitting an otherwise
oversized request does not grant an oversized single call admission.

## Threats

`scenario="besieged_fortress"` is the starter fortress plus a goblin raider
(entity 6001, kind `creature`, `hostile`, health 120) that arrives at tick
1,500. From arrival it fights in 100-tick rounds: every living dwarf in a squad
deals 10 damage per round; with no soldiers it kills one exposed dwarf (alive
and in no burrow, highest id first) every third round (`alive: false`,
`cause_of_death`). At health 0 it is `slain`. `world_alerts` report the raider
as `high` while approaching and `critical` while attacking, with a remedy that
assigns up to four unassigned dwarves to the squad; when the session holds the
remedy's capability (`configure_military`) it becomes the top recommendation.
Remedies are always gated by the capability they name (`requires`).

## Evaluating agent policies

`dwarf-fortress-mcp evaluate <scenario> <policy> <ticks>` plays a scenario for
a bounded number of game ticks through the same eleven tools, in 100-tick
steps, and prints a deterministic `dfmcp.lab-evaluation/1` report: dwarves
alive, dwarf-steps spent thirsty or hungry, hostiles slain, objectives
achieved, plus cost (tool calls, plans committed and refused). Policies:
`idle` (only waits) and `follow_recommendations` (after each wait, plans and
commits the Agent Turn's top recommendation when it is a concrete plan or
commit). On `besieged_fortress` over 20,000 ticks `idle` loses all seven
dwarves while `follow_recommendations` musters the squad, slays the raider,
keeps everyone supplied and achieves every objective it commits. The same
scenario, policy and horizon always produce the same report and final anchor.
