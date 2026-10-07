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
before anything is committed. Forecasts assume no other agent acts and the
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
the session's game-tick budget; a paused fortress reports `blocked`.

`fortress_cancel(mode="stop_future_steps", scope="plan")` drains every
nonterminal action of the last committed plan, dependents before their
prerequisites. Deferred steps are never dispatched, temporal work stops without
undoing progress (excavated tiles stay excavated), and verified actions are
history that is never rewritten. The response reports `drain_progress`
(total, already terminal, drained, compensated, cancelled, remaining) and a
`finalize_certificate` digest only once nothing nonterminal remains. Without
`scope` the historical single-action behaviour is unchanged.

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
  committed action); action handles, plans and obligations from before the
  restart are not carried and must be re-established from observation;
- **commits survive too.** Before a durable commit dispatches anything, its
  agent request and the exact world it was sealed against are journaled; step
  states are journaled after every call, before the world head. On resume each
  unfinished commit is deterministically recompiled from that request and world
  and must reproduce its sealed digest (otherwise it is reported `unverifiable`
  and abandoned, never trusted). Dispatched steps come back as
  `carried_obligations` whose sealed proof (obligation terminal predicate or
  postconditions) is evaluated against every later observation until they are
  `verified`, or `failed` at their failure predicate or deadline; steps that
  were never dispatched are reported `not_dispatched` (they had no effect).
  `recovered_commits` lists every step's recovered state. A restore abandons
  every carried and in-flight commit;
- reopening a durable fortress fences every older session of it (`conflict`),
  so two writers never interleave; a second server process on the same
  directory is refused by the store lock;
- `fortress_doctor` reports `durability` (persisted anchor, whether it is
  current, journal records, chain head, torn tail discarded at open).

Storage: `journal` (one `<chain> <record>` line per head/checkpoint, SHA-256
chained) plus `objects/<sha256>.snap` holding exact canonical snapshot bytes.
Objects are synced and renamed before the record naming them is appended and
synced. On open only an incomplete final record is discarded; a broken chain,
malformed record or corrupt object refuses the store. The journal is compacted
to live records (and unreferenced objects removed) every 1,024 records.
`scripts/lab_durable_restart.py` demonstrates it across a SIGKILL.

## Live routing of sealed plans

Every `fortress_plan` response carries `live_routing`: how each sealed step maps
onto the live DFHack development families (`dfmcp_adapter::live_routing`), or
why it cannot.

| Semantic action | Live family | Limits |
|---|---|---|
| `pause` | control/1.7 | — |
| `designate_dig` (`mine`) | dig/1.16 | tiled into ≤8×8 single-level rectangles in z,y,x order; x,y ≥ 1 (complete halo); ≤64 rectangles per step; other modes refused |
| `build` `furniture:Bed/Chair/Table` | build/1.19 | single-tile footprint at its location; needs an exact live item |
| `create_work_order` `CONSTRUCT_BED/DOOR/TABLE/THRONE` | work-orders/1.10 | wooden only, amount 1..100, no conditions |
| `set_labor` | workforce/1.17 | work-detail membership for ≤32 units, game paused; needs the live detail carrying the labor |
| stockpile, squad, burrow, standing order, extension | none | refused with a reason |

Each routable step lists the typed request (for example the exact dig
rectangles), the live values still to resolve, the family's own live
preconditions, the unadmitted development server that executes the family
(`dev_server`) and, for excavation, the exact `fortress.observe` region call per
rectangle for that server (`dev_server_observations`). Routing is deterministic and pure: it grants no capability,
performs no I/O, and every family remains unadmitted development execution.

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

The same history serves exact historical reads: any `fortress_query` entities
or terrain request may add `"at": "<state_hash>"` to read that retained version
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
unit 5 food, applied before consumption in the same interval.

`fortress_observe` and `fortress_wait` return `world_alerts` and the Agent Turn
raises them as `fortress_needs` attention: `high` when a stock lasts less than
four rounds, `critical` when it is exhausted or dwarves are deprived. Each alert
carries a `remedy` — the exact `fortress.plan` arguments for a work order
sized for about four rounds. Worlds without a ledger (`empty`) have no
metabolism. None of these rates are claims about Dwarf Fortress.

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
