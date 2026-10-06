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

