# MCP Surface

The public surface is intentionally smaller than the internal action and query registries. The
wire is **MCP 2026-07-28, modern-only**, carried by the owned `fastmcp_rust` sibling at an exact
pinned revision (ADR-013, `docs/FASTMCP_INTEGRATION.md`); the `legacy-2024-11-05` graph is never
compiled, and `dfmcp/0` semantic negotiation remains authoritative above MCP negotiation.

The eleven tools are stages of one agent operating loop, not independent mini-APIs. The
normative model is `docs/AGENT_OPERATING_MODEL.md`; the machine contract is
`architecture/agent_turn_contract.json`.

## Versioning

MCP protocol negotiation and `dfmcp` semantic negotiation are separate. Every session records:

- MCP protocol version;
- `dfmcp` protocol version;
- Agent Turn Packet schema version;
- JSON Schema catalog digest;
- bridge protocol version;
- canonical schema version;
- DF/DFHack/bridge manifests;
- compatibility level.

No tool is callable before `fortress.open_session` completes.

## Common Agent Turn Packet

Every success and error result contains an additive `agent_turn` object. Tool-specific fields
remain stable, while the common packet gives an agent one reliable orientation spine:

```json
{
  "schema": "dfmcp.agent_turn/1",
  "operation": "fortress.observe",
  "phase": "orient",
  "session_id": "…",
  "request_id": "…",
  "anchor": {
    "fortress_id": "7",
    "epoch": 3,
    "sequence": 18422,
    "game_tick": 9120031,
    "state_hash": "…"
  },
  "continuity": {
    "status": "continuous",
    "basis": {"epoch": 3, "sequence": 18418, "state_hash": "…"},
    "gap": null,
    "reset_reason": null
  },
  "profile": "briefing",
  "briefing": {},
  "changes": [],
  "attention": [],
  "active_work": {},
  "affordances": [],
  "recommendations": [],
  "uncertainty": [],
  "coverage": {},
  "budget": {},
  "references": []
}
```

The packet is a projection. Attention, affordances, recommendations, memory, and counterfactuals
cannot grant authority, satisfy a live precondition, or dispatch an effect.

### Continuity

`continuity.status` is one of:

```text
bootstrap
continuous
heartbeat
partial
gap
reset
stale
indeterminate
```

The server never silently bridges a gap, crosses a restore epoch, or mixes derived generations.

### Epistemic state

Agent-visible facts and recommendations distinguish:

```text
observed
certified_derived
inferred
predicted
assumed
stale
unknown
contradicted
indeterminate
```

Only observed and eligible certified-derived facts may satisfy mutation preconditions. Confidence
does not upgrade epistemic class.

### Active work

Every response exposes the bounded session-relevant set of pending plans, actions, obligations,
cancellation drains, indeterminate effects, publications, and confirmations. A caller is not
required to preserve an old context window merely to learn that unfinished work exists.

### Affordances

Affordances are typed semantic action templates derived from current state, compatibility,
capability grants, and risk policy. Each states whether it is enabled, why it is disabled,
preconditions, risk, reversibility, checkpoint/confirmation policy, estimated cost, and arguments.
An affordance is not a commit ticket.

### Recommendations

Recommendations are structured protocol next steps. They include evidence, expected utility,
expected information value, risk, reversibility, cost, prerequisites, invalidators, and
confirmation requirements. The server may return no recommendation; it never manufactures
busywork.

### Coverage and absence

Every result declares complete, partial, and omitted domains. An empty result proves absence only
inside a domain with a complete-domain witness. Budget exhaustion returns a complete bounded
prefix, explicit omissions, and a continuation rather than malformed or silently truncated JSON.

## Observation profiles

Profiles are semantic contracts:

| Profile | Purpose |
|---|---|
| `pulse` | Cheapest safe heartbeat: critical changes, active-work transitions, unresolved indeterminate effects, and top next steps. |
| `briefing` | Default cold-arrival/context-reset orientation: compact fortress summary, changes, attention, work, affordances, and important unknowns. |
| `tactical` | Decision-specific entity/region detail, causal neighborhood, witnesses, blockers, and candidate options. |
| `forensic` | Evidence-complete bounded reconciliation, diagnosis, audit, or replay. |
| `custom` | Explicit bounded union of registered projections; unknown projections fail closed. |

Laboratory presentation profiles operate over the process-local reference world
and protocol state. They do not establish the full live content described by
the target contract.

The laboratory also implements the separate canonical completeness profiles
`control-minimum`, `operations`, `spatial`, `historical`, and `research-full`.
Use `fortress.query` with
`{"mode":"observation","completeness_profile":"spatial","section":"chunks","limit":25}`.
The closed section selection accepts `entities`, `relations`, `chunks`, or
`events`. Pages retain distinct source and projected anchors, field-presence
metadata, explicit profile omissions, and a continuation bound to the original
source hash. Coverage certifies record membership only within the named
included section; excluded empty sections never prove absence. The bounded JSON
rendering is separate from canonical profile envelope bytes. Full semantics
and examples are in `docs/LAB_SEMANTIC_ACTIONS.md` and `docs/WORLD_MODEL.md`.

## Tools

The dedicated `dfmcp-live-semantic-dig-dev-server` uses the same eleven names for
[original mining goals](docs/SEMANTIC_DIG_DEVELOPMENT.md). It observes a complete
paused canonical source, prepares one original Mine rectangle and explicitly
confirms its paired designation. Optional independently authorized run/1.13
control adds `plan(kind="run")`, a distinct review confirmed through
`commit(scope="run")`, and the native owner's safety pause through
`cancel(scope="run")`. Tick permission defaults to zero; a positive allowance
is capped at 1200 per interval and further bounded by the original cadence and
deadline. Fresh exact-target readiness may accept pending dry Mine walls without
upgrading their unknown floor truth. Wait reconciles any unsettled clock before
sampling excavation, and never starts or chains runs. Clock receipts, current
excavation and historical achievement remain separate. Exact original/clock
custody survives errors and restart; unknown dispatch never permits another
unpause. This remains an explicitly unadmitted development profile. See the
[clock configuration and recovery guide](docs/SEMANTIC_DIG_CLOCK.md).

The dedicated `dfmcp-live-semantic-work-orders-dev-server` uses the same eleven
names for [original native furniture-order goals](docs/SEMANTIC_WORK_ORDER_MCP.md).
Its closed `plan` request compiles one finite order under an exact canonical
observation; `commit` confirms the paired semantic/native review once, and
`wait` reconciles insertion and polls the original goal with durable history.
Optional order-run/1.14 control uses `plan` with `kind: "run"`, a separate
review confirmed through `commit` with `scope: "run"`, and `cancel` with
`scope: "run"`. Each bounded run targets the exact original Created receipt;
it cannot extend the original deadline or replace its cadence and stability.
`wait` first reconciles retained clock uncertainty and never starts a run.
`query`, `explain` and `doctor` expose complete retained custody, exact restart
seals, separate current/historical proof, clock receipts and physical quiescence.
Native clock receipts remain separate from original-goal proof. The runtime
requires explicit operator development source acceptance and remains unadmitted.

The dedicated `dfmcp-live-semantic-workforce-dev-server` uses the same eleven
names for [original native labor goals](docs/SEMANTIC_WORKFORCE_MCP.md).
`observe` acquires a complete bounded canonical citizen source. Optional closed
`query` mode `citizens` pages identities under that exact observation seal;
omitting the query inspects retained goal evidence. `plan` accepts canonical
`units`, one `labor`, `enabled`, the observation seal and original deadline,
cadence and stability. It prepares one paired semantic/native review without
assigning labor. `commit` requires that review and explicit confirmation;
`wait` reconciles the same assignment and samples fresh original labor facts.
Optional bounded-run/1.13 control adds `plan` with `kind:"run"`, its distinct
review confirmed by `commit(scope="run")`, and `cancel(scope="run")` for the
native owner's safety pause. Run preparation requires a historical Verified
assignment and a fresh true unfinished original goal. `wait` reconciles any
unsettled run before a paused goal sample and never starts or chains runs.
Historical assignment, current goal proof and durable first achievement remain
separate from clock coordination receipts. `cancel` retires preparation or
releases the session without undoing applied membership or elapsed game time.
Every error and handoff retains exact original journal, plan, citizen identity
and optional clock journal references without granting authority.

This isolated runtime supports 1–32 selected citizens from a complete source of
at most 128 and one original goal. Game-time permission defaults to zero;
`open_session(max_game_ticks=...)` can negotiate at most 1200 ticks for each
separately reviewed interval within the unchanged original deadline. Its logical
byte-work default is 2 GiB; `open_session(max_bytes=...)` can explicitly request
up to 32 GiB to replay a longer retained history, within the unchanged 60-second
wall ceiling. Recovery preserves the original absolute deadline and resets
unfinished stability. Operator source acceptance remains unadmitted development
acceptance, and no endpoint, path, native ID or effect key is a client input.
The independent clock gate supplies source-bound authority. Run/1.13 identifies
an incarnation, endpoint and software versions, not a named fortress lineage;
matching ticks and historical pause receipts do not establish current fortress
identity, current pause or original-goal proof.

The furniture development server also accepts a `furniture_request` JSON string
as an alternative `open_session` input. One complete operations/1.4 inventory
feeds an immutable exact batch preserving its original selection constraints
and item evidence. Shortage creates no partial batch; reopening does not
reallocate. Closed `query` mode `allocation` provides original `request` and
`items` views. The existing per-step review and commit boundary remains
required. This new source and its process tests await Rust execution after an
executor outage; see `docs/FURNITURE_HANDOFF_MCP.md`.

The isolated furniture/1.19 development server also uses these same eleven names
for [complete furnishing batches](docs/FURNITURE_BATCH_MCP.md) and
[whole-original-plan construction monitoring](docs/FURNITURE_COMPLETION_MCP.md).
The same development server accepts a complete bounded `room_request` through
`fortress.open_session`, preserving required floors/walls and the full furniture
request in original batch custody. See [room construction intake](docs/ROOM_CONSTRUCTION_MCP.md).
Declared room terrain is not construction-completion evidence.
The [joint room monitor](docs/ROOM_COMPLETION_MCP.md) subsequently evaluates the
complete retained terrain and every original furnishing in one sampled condition.
It uses query modes `room_completion_start` and `room_completion`, observe selection
`room_completion`, and wait/cancel scope `room_completion`, preserving the eleven
top-level tools and a separate acknowledged recovery history.
Its closed `query` modes `completion_start`/`completion`, `observe` selection
`completion`, and `cancel` scope `completion` add receipt-linked, Query-authorized
monitoring without adding tools. The complete original plan and all assessments
remain in each handoff. Operator-selected `completion-recover` and
`completion-offline` modes retain inspect/cancel access to the monitor after
original-file loss while withholding verified construction claims. These are
unadmitted development modes, separate from production protocol negotiation.

The isolated [Manager/1.22 development profile](docs/MANAGER_APPOINTMENT_MCP.md)
uses these same eleven names. `serve-gameplay-dev manager` observes one
host-offered eligible current Expedition Leader for vacant MANAGER, prepares
an exact assignment/HF/history witness, reviews and confirms once, and proves
the complete unchanged-tick transaction through both native noble getters.
Original-key lookup, independent journal/ACK custody and errors preserve
uncertainty without redispatch. Role/candidate/path/method selection is not an
MCP argument. Full fortress authority is required; production admission and
canonical world continuity remain unestablished. This legacy generation does
not prove the game's production-responsibility cache. The separate
[Manager/1.23 profile](docs/MANAGER_APPOINTMENT_V1_23.md), selected at host startup
with `serve-gameplay-dev manager-v1-23`, witnesses and updates that cache in the
same transaction. Its host names, wire identity and custody are distinct.
Neither historical appointment proof nor cache proof establishes bed production.

The explicitly unadmitted [combined labor/order host](docs/COMBINED_PRODUCTION_MCP.md)
is selected at startup with `serve-gameplay-dev labor-order`. It uses these
eleven logical operations (underscore wire names) for one complete original
two-step plan. Plan first compiles the closed labor/furniture request; Plan with
`kind="step"` and `original_plan_digest` then prepares one dependency-satisfied
original step. Commit requires its current `paired_seal` and confirmation. Wait
reconciles one original key without dispatch or clock progress. Historical
all-applied is distinct from current state or terminal-goal proof. Separate
family source pins, write grants and custody remain mandatory. This host adds
no native method, production runner or admission.

### `fortress.open_session`

Negotiates fortress, versions, compatibility, requested capability scopes, observation profile,
and hard budgets. Returns a concrete initial anchor, grants, bootstrap briefing, supported
profiles, current affordances, implementation uncertainty, and the safest first inspection step.

The semantic work-order development server accepts optional `max_game_ticks`,
defaulting to zero and capped at 1200; opening never unpauses. Original-order
insertion and optional clock control have separate operator gates and
credentials. Optional operator `clock` configuration pins its own generation
and private `clock.journal`; every reopen requires the externally retained
exact journal identity. These are operator configuration, not client tool
arguments. See `docs/SEMANTIC_WORK_ORDER_MCP.md` for initialization and the
complete recovery handoff for the original goal and clock.

The semantic workforce development server accepts the same optional
`max_game_ticks` range and never unpauses on opening. Labor and clock control
have independent operator gates and credentials. Optional `clock` configuration
pins the run/1.13 generation and fixed private `clock.journal`; each reopen
requires its externally retained identity, including `recovery.clock_journal_id`
when an original goal exists. A clock-only Control reopen cannot regain labor
authority. See `docs/SEMANTIC_WORKFORCE_MCP.md` for the complete workflow and
`docs/SEMANTIC_WORKFORCE_CLOCK.md` for the source-identity limit.

### `fortress.observe`

Accepts an exact cursor or no cursor, interest set, profile/projection, freshness, and limits.
Returns a snapshot, delta, heartbeat, or reset. Partial results carry an opaque continuation tied
to the session and target anchor. The default product is orientation, not a world dump.

### `fortress.query`

Executes bounded structured DfQL at a concrete anchor. Query plans are statically costed.
Deterministic result ordering is mandatory. Coverage says exactly what an empty or partial answer
can establish.

Structured `aggregate` accepts optional `relation` and `spatial` selectors,
independently or together, using the same closed objects as `ranked_entities`.
Source kinds and typed `where` run first, positive witnessed relationships next,
then source point/cuboid geometry, grouping and exact integer/fixed metrics.
Each source entity contributes once even when several edges match; targets may
be outside the region or have no geometry. Source False/Unknown, missing or
unavailable targets, no observed relationship, outside geometry and Unknown
geometry have separate counts. Totals cover selected known members only,
with metric sample exclusions independent of membership completeness.
Complete-source, combined-work and wall admission precede selection; excessive
groups or a complete packet that cannot fit refuse atomically. Exact retained
reads still require current Query authority. See
[`docs/QUERY_EXECUTION.md`](docs/QUERY_EXECUTION.md) and
[`docs/LAB_STRUCTURED_QUERY.md`](docs/LAB_STRUCTURED_QUERY.md).

Structured `dfmcp.query/1` requests also accept `kind: "events"` for inclusive
retained-event tick windows, exact kind and historical subject filters,
explicit typed field projection and ascending/descending chronological pages.
Continuations bind the complete source anchor, session and selection. The
response identifies every canonical event while keeping unknown retention
coverage, source authenticity and per-field provenance explicit. It neither
collects new native events nor proves absence outside the supplied retained
projection. The default laboratory uses the existing `mode: "structured"`
wrapper, including exact retained `at` hashes; see
[`docs/LAB_STRUCTURED_QUERY.md`](docs/LAB_STRUCTURED_QUERY.md).
The explicit `starter_fortress_history` scenario supplies events through normal
atomic laboratory action, tick, cancellation, restore and recovery publication.
Its `coverage.producer` describes bounded retention and omitted details;
[`docs/LAB_EVENT_HISTORY.md`](docs/LAB_EVENT_HISTORY.md) defines the source model.

### `fortress.plan`

Compiles an objective/intent without effects. Returns immutable candidate plan(s), digest, action
DAG, affected scopes, assumptions, witnessed preconditions, postconditions, obligations, risks,
capabilities, checkpoint policy, predicted diff, cost, invalidators, alternatives, and
explanation. The executable laboratory supports every non-extension semantic
action family and furnished blueprint objectives; see `docs/LAB_SEMANTIC_ACTIONS.md`.

After verified insertion, the semantic work-order development server also
accepts a closed `request` JSON string with `kind: "run"`,
`original_paired_seal`, `game_ticks` and `wall_millis`. It acquires fresh
original-goal evidence and prepares a separate review without unpausing. The
runtime chooses the receipt-linked native order and stop guard. Bounds are
1–1200 ticks within the negotiated allowance and original deadline, and
1–60000 milliseconds for the native stop bound. A complete inactive order can
receive another reviewed slice for original sampled stability or global job
quiescence, with unexpected activation as its safety stop.

After a Verified labor assignment, the semantic workforce development server
accepts the same four-field closed run request, bounded to 2 KiB. It requires
fresh true original labor predicates and an unfinished original goal. The run
is bound to the unchanged plan, original citizen capsule and assignment receipt;
it cannot substitute another citizen or work detail. Game ticks are 1–1200
within the negotiated allowance and original deadline; the native wall stop
bound is 1–60000 ms. Suggested intervals respect the original cadence. Native
callbacks may overshoot either stop bound, and each interval needs separate
confirmation. No native preparation unpauses the game.

The laboratory production template compiles original DRINK/FOOD and wooden
BED/DOOR/TABLE/CHAIR quotas with the
fixed `consumption_aware_v1` generation on new intake. It budgets observed
population consumption through the complete sealed schedule, including setup,
queued work, shared-worker dependencies and originally satisfied quotas. Reports
keep original minima separate from predicted reserves. Saved sources retain
their exact compiler generation and modern seals bind the complete request.
The discarded-fork forecast requires Verified actions, a True original goal and
physically quiet work together before `predicted_complete` is true. See
`docs/PRODUCTION_RESERVE_PLANNING.md` for inputs, reports and limits.

In the explicit resource laboratory, the closed `resource_acquisition`
blueprint accepts one or two WOOD/PLANT `quotas` with absolute `minimum` stock
targets and optional `assign_labor`. It installs fixed compiler
`resource_acquisition_v1`, allocates complete qualified finite sources, and
seals ordinary guarded harvests plus explicitly permitted labor enables.
Current Observe/Query/Plan and full source budgets precede compilation;
complete publication precedes pending-plan replacement. Original stock truth,
action proof and physical completion remain separate. Stale commits require
a new reviewed Plan; unknown supply, overlapping harvests, consuming work,
insufficient finite sources and action/output bounds refuse explicitly.
Candidate comparison and selection use the same source. See
`docs/LAB_RESOURCE_ACQUISITION.md` for the request, analysis and acquire-then-
observe-then-produce workflow. `continue_goal` preserves the original resource
minima and labor policy after canceled or interrupted work is proven quiet;
completed yield contributes only through observed stock. This adds no native
route.

Furniture quotas share a finite WOOD supply and completed carpenter workshops.
The planner accounts for existing orders, assigns up to six output jobs jointly,
and serializes work sharing a selected citizen or workshop. Each completed
carpentry unit consumes one modeled log and produces one canonical inventory
item with its original order and ordinal. A shortage or unknown material count
refuses the plan. See `docs/CARPENTRY_PRODUCTION.md` for a complete example and
the distinction between manufacturing inventory and furnishing a room.

For furnished rooms with an explicit access requirement, the laboratory
`connected_room` blueprint wraps the complete `provisioned_room` request and
adds `access.entry` plus a bounded same-level `access.region`. It plans safe
connector excavation, waits for circulation before placing furniture and
proves current FLOOR access from the retained entrance to a cardinal approach
for every furnishing. Current and planned building occupancy constrain the
route. The terminal goal combines complete terrain, exact installed furniture
and current access; later obstructions can invalidate current success.
See `docs/LAB_CONNECTED_ROOM_ACCESS.md` for the complete JSON example,
deterministic routing policy and reference-model limits.

An explicit nested `acquire_resources` policy selects the additive resource
room compiler. `provisioned_room_resources_v1`, `connected_room_resources_v1`
and explicit `connected_room_stairs_resources_v1` retain the complete project
through finite-input acquisition and separately reviewed room stages. Responses
use `dfmcp.resource-room/2`, kind `resource_room` or
`resource_room_continuation`, and complete `room` analysis. Goal observations
separate `semantic_room_goal` from exact historical stage truth; a finished
harvest cannot complete or evict an unfinished room. Current Observe/Query/Plan,
full source/output bounds and same-root quiescence are enforced at the relevant
review and commit boundaries. See `docs/LAB_RESOURCE_ROOM_PROJECTS.md`.

Resource-aware room and production Plan responses share repeated readable
proof predicates through `dfmcp.plan-predicate-references/1`. Complete
`predicates` and `steps` remain in the same response; canonical local `$ref`
pointers expand to every original terminal, postcondition and obligation
predicate. The typed sealed plan and archive are unchanged. Later observation
hints retain expanded proofs, so they do not depend on a previous response's
predicate table. Complete publication still precedes pending-plan replacement.
Oversized complete packets can select `dfmcp.plan-predicate-references/2`,
using zero-based integer references and tagged rows with explicit columns.
Its checked inverse restores every original readable requirement; literal
leaf values and source requests stay unchanged. Fitting `/1` packets retain
their original representation. See `docs/COMPACT_PLAN_REVIEWS.md` for grammar,
bounds and qualification limits.
`anchor_encoding` declares same-response references to `/current_anchor` for
equal typed work/history anchors; full canonical anchors, differing history,
unknown evidence and literal source/predicate values retain their meaning.

The laboratory `blueprint` JSON string also accepts the closed
`{"template":"continue_goal","plan_digest":"<retained goal digest>"}` form.
It compiles a new reviewed plan from complete original production quotas,
resource stock targets or furnished-room requests, preserving setup options,
resource labor policy and any connected-room entrance/domain under current
Observe/Plan authority. Resource continuations also require current Query
authority and the complete bounded source and output. Valid installed furniture
is retained; a connected continuation can repair access with excavation alone.
Original and same-root work must
be proven quiet, and commit repeats the check before effects. Parent/root
lineage survives durable recovery. A verified reopen binds the retained goal
to the exact recovered world while preserving its old seal; arbitrary epoch
changes and checkpoint-abandoned pursuits remain refused. No old grants,
actions or proof are reused as authority. See `docs/PRODUCTION_GOAL_CONTINUATION.md` for the exact bounds,
refusals, source-seal binding and explicit commit workflow; furnished-room
continuations are detailed in `docs/ROOM_GOAL_CONTINUATION.md`.

The laboratory `blueprint` JSON string additionally accepts closed version-one
`compare_candidates` and `select_candidate` requests. Comparison evaluates two
to four ordinary sources against the same normalized original terminal condition
and reserve frontier, sharing one source, fork, action, simulated-tick, polling,
and wall-time budget. Results separate unavailable models from complete
predictions and use an explicit deterministic ranking policy. Forecast forks
advance physical world jobs and the candidate's own deferred chain; other
retained plans' deferred actions are outside the model. Predictions remain
conditional and grant no effect authority.

An explicit `compare_candidates` version 2 with
`forecast_scope: "complete_project"` compares production, provisioned_room or
connected_room sources through up to four discarded stages per candidate.
Harvested supplies must actually appear in the simulated world before the
unchanged original request is compiled for production or construction.
Each stage must verify and become physically quiet before another begins.
The comparison binds the original semantic output/room/access goal and ledger
generation, compares complete project costs, and preserves the absolute reserve
frontier. Later stock cannot repair a missed reserve observation; a known-True
exact frontier may precede later stable action verification. Version-one
current-stage comparison behavior and shape remain unchanged.
Version-two selection still emits an ordinary version-one selection of only
the current executable stage; future model plans never become effect authority.

Comparison preserves the pending plan and exposes current game work, baselines,
and watches. Each prepared candidate includes a source-bound selection request.
Selection checks the exact current anchor and candidate contents, recompiles an
ordinary plan under current Observe/Query/Plan authority, and accepts the entire
Agent Turn before replacing pending state. A publication refusal preserves the
prior plan. Selection still requires ordinary `fortress.commit`; it does not
commit or advance the canonical world. See
[LAB_PLAN_COMPARISON.md](docs/LAB_PLAN_COMPARISON.md) for inputs, ranking, budgets,
model assumptions, and the explicit validation limits.

### `fortress.commit`

Requires exact plan ID/digest, expected anchor, and confirmation seal when policy requires.
Revalidates before effects. Returns per-step action states, checkpoint receipt, obligations,
evidence, observed delta, and prediction-versus-observation comparison. Timeout may produce
`indeterminate`.

In the semantic work-order development server, omitted `scope` or
`scope: "goal"` confirms the original paired review for insertion. `scope: "run"`
instead confirms `clock.active_run.review_seal` through the existing
`paired_seal` argument. Both require `confirmed: true` and current authority
for that effect. The original archive and Created receipt remain bound to
every run; durable dispatch intent precedes the native effect. Lost replies
require exact reconciliation and cannot authorize another insertion or unpause.

The semantic workforce development server uses the same `goal`/`run` scopes:
the default confirms the original labor review, while `scope:"run"` confirms
`clock.active_run.review_seal`. The exact original archive, citizen capsule,
Verified assignment receipt and current true labor evidence are checked again
at the durable clock effect boundary. Assignment and clock grants are
independent. Lost replies never permit another assignment or unpause attempt.

The laboratory modern stdio registration accepts optional `as_task: true` with
an explicit session ID and sealed plan digest. A client must negotiate the
Tasks extension. This returns a real modern task handle whose original plan is
supervised through completion or cancellation. It is an opt-in transport
projection; the ordinary commit response and semantic plan identity remain the
same. See `docs/LAB_MCP_TASKS.md` for the exact bounded implementation.

### `fortress.wait`

Polls or follows plans/actions/obligations under bounded wall/game time. Returns meaningful
progress only and may provide continuation. Stable no-change progress becomes a compact
heartbeat.

The semantic work-order and workforce development servers first reconcile a
retained clock run, then attempt a fresh paused original-goal sample only when
the clock state permits it. Running or unresolved clock effects remain visible and prevent
that sample. This `wait` never starts a run or advances time itself. Original
cadence, absolute deadline and stability remain authoritative; a native stop
or historical pause receipt cannot satisfy them. Further runs each require
another explicit plan and confirmation.

In the reference laboratory, a positive `max_game_ticks` request requires
current `ControlClock` authority. An unpaused advance must remain within both
clock and observation grants through its final tick, and within the negotiated
game-tick budget. Zero-tick polling requires observation authority only.
Goal outcome and observed physical work are reported separately.

Laboratory waits interleave bounded time slices with dependency-ordered polling
of retained original actions. Original obligation cadence/deadlines constrain
the slices; ready successors can start and finish in the same call. Each round
publishes world, proof and original-goal history before another round proceeds.
A newly dispatched pause stops the remaining advancement.
`wait_progress` reports requested, advanced and remaining game ticks, successful
observation rounds, action polls and deferred dispatches. A partial refusal
retains those counters and, under current Observe authority, the actual anchor,
pause state and work in the Agent Turn. A persistence failure fences further
progress and is reported as `durability_pending`. Each polled action carries
its immutable proof `observed_anchor` separately from current physical work.
See `docs/LAB_FOREGROUND_PROGRESS.md` for the bounded implementation and evidence.

The laboratory tool also accepts an optional `plan_digest` selecting one exact
original commit owned by the session. It returns early when that original goal,
verified actions and quiet physical work agree, or when a stage, failure or
unknown outcome needs attention. Other retained work progresses during the same
shared time; selection does not follow descendants or dispatch a continuation.
`selected_plan_wait` separates original goal truth, stage proof, physical work
and exact reserve evidence. The optional selector is recorded in replay, and
omitting it preserves duration-based waiting. See `docs/LAB_SELECTED_PLAN_WAIT.md`.

### `fortress.cancel`

Starts or advances request/drain/compensate/finalize. It never means “delete the record.” Active
work remains visible throughout the drain.

The semantic work-order development server uses `scope: "goal"` by default to
retire a prepared insertion. `scope: "run"` retires a prepared clock run or
requests the dispatched native owner's safety pause under current clock
authority. It retains the manager order and original goal. `scope: "session"`
releases resources without cancelling game effects; preserve the complete
recovery handoff, including `clock_journal_id` when configured, before closing.

The semantic workforce development server likewise uses `goal`, `run` and
`session` scopes. Clock cancellation preserves the original labor assignment and
goal. It requires current clock authority, but no true labor predicate, labor
control grant or unexpired original deadline. It cannot undo elapsed time, and
a historical stop receipt does not prove present pause. Session close retains
both original and clock identities without cancelling game effects.

The laboratory supports `scope: "last_action"` (default), `"plan"` (the latest
committed plan), `"plan:<64-lowercase-original-plan-digest>"` (one exact original
commit owned by the current session), `"oldest_open_plan"` (the oldest retained
plan with unfinished work), and `"session"` (retained open work across the
session's plans). Exact original selection checks complete commit/action custody,
all current scoped effect authority and aggregate budgets before its first
effect. It leaves other plans, continuation descendants and pending review
untouched; witness-rebase aliases disclose both requested and actual digests.
Typed original-plan/goal Explain includes a currently authorized selected-stop
request when available, bound into its complete observation pin.
See `docs/LAB_PLAN_CANCELLATION.md`.
Use `oldest_open_plan` to drain individually bounded original plans when the
whole session exceeds one call's budget; the response identifies that plan's
original digest and certifies only its actions.
Cancellation requires current observation and original scoped effect authority;
emergency pause also requires clock authority and invalidates earlier shared
unpause votes once authorized. Partial refusals report the resulting anchor,
clock and work state in the Agent Turn. Stops, compensation and emergency
pause share the call's aggregate action budget. Terminal Failed/Verified proof
receipts remain immutable; their continuing physical work can be stopped with
separate evidence. Finalize certificates require both terminal action states
and observed physical quiescence. Active or unknown work retains ownership
fences even after an obligation deadline or a spatial lease TTL expires.

### `fortress.checkpoint`

Creates and verifies a content-addressed recovery point and reports its publication and evidence
state.

### `fortress.restore`

Guarded global operation. Drains work, restores a sealed checkpoint, creates a new observation
epoch, and explicitly invalidates stale plans, actions, continuations, recommendations, and
handoff anchors.

The reference laboratory also inspects active or unknown entities restored
without their original action handles. This untracked work remains visible,
fences its spatial footprint, and blocks a session-wide quiescence certificate.
The census is observation-only; it neither recreates historical proof nor
restores cancellation authority. Snapshot persistence keeps that custody
observable after another restart.

### `fortress.explain`

Returns evidence-backed rationale for a fact, score, attention item, affordance, recommendation,
plan, transition, compatibility decision, error, surprise, memory item, or doctor finding.

The laboratory currently accepts a decimal `entity_id` for graph dependencies,
an omitted `entity_id` for recent transcript events, and two typed selectors for
a bounded explanation of an exact original plan or goal:

- `plan:<64-lowercase-plan-digest>` resolves a pending plan or retained original.
- `goal:<64-lowercase-plan-digest>` requires a committed original still retained
  by the goal book. An uncommitted pending plan cannot satisfy this selector.

A successful typed read returns `dfmcp.lab-goal-explanation/1`. It separates
current terminal-clause truth, original room/output project truth, resource or
ordinary production reserve status, retained first-achievement proof, original
step receipts, and current physical work. A finished resource-acquisition stage
can therefore explain the still-unfinished original room or production goal.
When the existing continuation gate permits a new reviewed pursuit, `next_step`
contains that original-goal request; inspection itself prepares or dispatches
nothing. Unknown facts remain unknown. Historical achievement never substitutes
for present goal truth or physical quiescence.

`requirements_summary` counts true, false and unknown conjunctive terminal
clauses. Nested `All` clauses are expanded; `Any` and `Not` remain intact, so
a false alternative inside a satisfied disjunction is not labeled a blocker.
`original_steps` and `original_requirements` contain complete follow-up
`fortress.explain` calls. Follow their `next` calls for subsequent pages;
do not construct offsets or mix pages from different discovery responses.

Each page selector binds the semantic plan digest, exact typed object digest,
inspecting session and an observation pin over the full anchor, original source,
complete selected goal projection, all clause diagnostics and all step evidence.
Receipt, historical-proof or custody changes invalidate the pin even when the
world anchor stays unchanged. A mismatch returns `cursor_gap`; restart with
the original `plan:` or `goal:` selector. `complete_domain` refers only to the
selected requirements or steps domain. Agent Turn work coverage is explicitly
partial for the selected original, with the session handoff linked for all work.

Typed reads require current whole-fortress Observe and Query authority before
looking up a digest. They admit the complete world and retained source scan,
validate the exact typed archive, cap source/diagnostic work and elapsed time,
and reduce pages from at most eight intact rows to fit the complete Agent Turn.
An indivisible result that cannot fit refuses explicitly. Malformed typed
selectors never fall through to transcript or numeric-entity handling.

These reads use the immutable session owner fence: they do not allocate a
semantic request ID, poll obligations, record replay, update orientation or
historical proof, publish durable progress, notify Task progress, or move the
clock. Exact plan archives and lossless original source remain available through
the returned `df://session/...` references; see
[Resource URI consumers](docs/RESOURCE_URIS.md#explaining-current-original-goal-truth).

### `fortress.doctor`

Checks bridge, compatibility, canonical state, ledger, active work, leases, checkpoints, indexes,
replay, and presentation continuity. May propose a sealed repair plan but does not silently apply
it.

## Sparse mining project development profile

With operator `DFMCP_DIG_BLUEPRINT_CONFIG`, the existing
`dfmcp-dig-control-dev-server` retains one original sparse floor blueprint and
its complete native dig/1.16 prefix. The frozen eleven tool names remain:

- `open_session` adds `mode="control"|"offline"|"reconcile"` for exact retained
  projects. Offline/reconcile opening makes no native call. Control reopening
  takes a fresh exact-source clock/sequence read before issuing its new lease,
  retaining neither that selection nor an old dispatch permit.
- `plan(blueprint=<closed JSON string>)` publishes the original bounded shape.
  Individual regions use the existing full key/witness/hidden-choice request,
  followed by their exact paired review confirmation through `commit`.
- `query` adds `blueprint` (complete-prefix-verified pages) and
  `blueprint_layout` (canonical original geometry) to `dig-control-query/1`.
  Continuation queries bind both native and parent heads.
- `cancel(scope="blueprint", blueprint_id=...)` permanently stops future
  advancement. Exact original native query/retirement remain available.

At most one native commit is attempted per request. Every next region requires
a fresh capture and separate review; unresolved/refused/cancelled predecessors
cannot be skipped. Agent Turns retain the original project, next or pending
child and recovery pins. `all_designated` certifies historical designation
receipts only, with excavation completion explicitly unproven. The separate
single-original-Mine goal/clock profile is not widened into a multi-step goal.
See `docs/DIG_BLUEPRINT_MCP.md` for bounds, operator setup, exact recovery and
component-test limitations. No production admission or native wire is changed.

Optional `DFMCP_DIG_BLUEPRINT_MONITOR_CONFIG` adds whole-original-blueprint
sampled excavation monitoring through those same tool names:

- `open_session(mode="monitor-offline"|"monitor-recovery")`, with no region,
  reopens only exact retained monitor history. Native/parent files and tokens
  are unnecessary; Query-only authority permits inspection, and the writable
  recovery mode permits local monitor cancellation. It cannot resume sampling.

- `plan(monitor=<closed JSON string>)` registers the exact completed designation
  set, independent map/1.5 source and immutable deadline/stability/sample policy.
- `wait(monitor_id=...)` takes one complete paused enclosing capture and
  durably publishes progress after current source, authority and original
  receipt checks. Native key/plan arguments are omitted for this alternative.
- `query(mode="blueprint_monitor", monitor_id=...)` inspects verified historical
  progress and selected-cell diagnostics, with exact history-head page pins.
- `cancel(scope="monitor", monitor_id=...)` stops only read-only monitoring.

Every selected cell must be visible dry floor without magma or a dig designation
in the same capture and shared stability window. Unselected gaps are ignored.
No game clock is advanced; equal-tick reads cannot accumulate samples. Reopen
preserves history and clock floors, resets unfinished stability and never
renews the deadline. Fresh acknowledged satisfaction, historical achievement,
original native outcomes and safety remain distinct. The map/dig source link
does not claim shared-process identity or mining causality. See
`docs/DIG_BLUEPRINT_MONITOR.md` for exact configuration and recovery limitations.

Optional `DFMCP_DIG_BLUEPRINT_CLOCK_CONFIG` adds separately reviewed finite
run/1.13 intervals for that original monitor. `plan(clock=<closed JSON string>)`
prepares the interval, and `commit(scope="clock", monitor_id=..., review_seal=...)`
confirms its distinct review. `wait(scope="clock", monitor_id=...)` reconciles
the retained clock before a separate excavation sample. Each interval requires
its own review; no wait starts or chains intervals. Clock receipts remain
historical coordination evidence. See `docs/DIG_BLUEPRINT_CLOCK_MCP.md`.

When original native or blueprint-parent custody is unavailable, explicit
`open_session(mode="clock-offline"|"clock-recovery", max_game_ticks=0)` omits
the region and opens only the exact existing monitor and clock histories.
Both histories must be intact at opening; no original parent file, native dig
bootstrap, map read or designation token is used. `clock-offline` permits local
history queries. `clock-recovery` additionally permits one original clock
reconciliation through `wait(scope="clock", monitor_id=...)`. Explicit clock
opt-in at opening can retain separate named-fortress and native-source
ControlClock grants for `cancel(scope="clock", monitor_id=...)`; cancellation
rechecks current opt-in and never requires a positive future-tick budget.
Enabling clock control later cannot add missing opening grants. These modes
cannot plan another interval, sample terrain or recover designation authority.
`query` exposes `blueprint_clock` and `blueprint_monitor` histories with their
independent recovery pins, and explicit session release preserves unresolved
work without claiming a current pause. See `docs/DIG_BLUEPRINT_CLOCK_RECOVERY.md`
for recovery after file loss and the complete operator sequence.

## Common mutation state

```text
prepared
committing
applied_awaiting_verification
verified
compensation_pending
compensated
cancel_requested
cancelled
failed
indeterminate
```

Only `verified`, `compensated`, `cancelled`, and `failed` are ordinary terminal states.
`indeterminate` blocks blind retry and requires reconciliation.

## Resources

This inventory includes both implemented consumers and target families. The
laboratory registers session and doctor templates; the exact implemented views,
authority rules, returned schemas, retention behavior and deferred families are
documented in [`docs/RESOURCE_URIS.md`](docs/RESOURCE_URIS.md). Fortress-scoped,
knowledge and memory URI families below are not yet registered consumers.

```text
df://session/{id}/summary
df://session/{id}/capabilities
df://session/{id}/handoff
df://session/{id}/replay
df://session/{id}/anchor
df://session/{id}/tasks
df://session/{id}/task-{task_id}
df://session/{id}/entity-{entity_id}~{generation}~{anchor_digest}

df://session/{id}/state
df://session/{id}/state-{state_hash}
df://session/{id}/state-{state_hash}~{section}~{offset}
df://session/{id}/plans
df://session/{id}/plans-{catalog_digest}~{offset}
df://session/{id}/plan-{plan_digest}~{object_digest}
df://session/{id}/plan-{plan_digest}~{object_digest}~steps~{offset}
df://session/{id}/plan-{plan_digest}~{object_digest}~archive~{offset}
df://session/{id}/goal-{plan_digest}
df://session/{id}/goal-{plan_digest}~{source_digest}
df://session/{id}/goal-{plan_digest}~{source_digest}~source~{offset}
df://fortress/{id}/anchor
df://fortress/{id}/entity/{entity}
df://fortress/{id}/map/chunk/{x}/{y}/{z}
df://fortress/{id}/events
df://fortress/{id}/objectives/{objective}
df://fortress/{id}/plans/{plan}
df://fortress/{id}/actions/{action}
df://fortress/{id}/obligations/{obligation}
df://fortress/{id}/checkpoints/{checkpoint}
df://fortress/{id}/attention/{attention}
df://fortress/{id}/surprises/{surprise}
df://fortress/{id}/compatibility
df://knowledge/{document}/{span}
df://memory/{item}
df://doctor/{bundle}
```

Resources are capability-checked; URI knowledge does not confer authority.

The laboratory session template serves summary, capabilities, handoff, replay,
anchor and task views. Entity query rows also discover the session entity URI
above. It requires `query` for the requested entity ID and an unchanged complete
V2 anchor digest, including fortress, restore epoch, sequence, tick and state
root. It returns one existing presence-preserving canonical entity projection;
changed anchors or generations require fresh discovery, without a historical
fallback. The resource enforces the negotiated byte and token bounds including
the resource wrapper; an oversized record is refused without silent field loss.
Inspection preserves semantic request IDs and does not certify objectives,
retain versions or run the durable-persistence postlude, including after denial.

Normal prepared plans also have a session-bound catalog, original-plan manifest,
readable step pages and lossless DFMPAR01 archive pages, independently of MCP Tasks. Current
whole-fortress Observe and Query grants precede even digest existence checks.
Exact references bind the semantic plan digest and the distinct typed archive
object digest. Catalog continuations additionally bind the current anchor and
entire retained membership; collection or pending replacement never silently
changes an exact object. Manifest fields are inspection projections, with
current goal truth unevaluated and active work linked to handoff. Step pages
retain whole readable rows, identify omitted contract fields, and distinguish
sealed requirements and predicted IDs from current evidence. Archive bytes
retain original steps, predicates, obligations and idempotency keys without
granting commit authority. Source work, output and elapsed-time bounds apply to
every read. See the resource consumer document for reconstruction and retention.

One retained committed original can also be inspected through `goal-{plan_digest}`.
It resolves to a source-pinned manifest and lossless paged original request,
summary and terminal condition. Exact source identity includes the typed-plan
object digest, preventing substitution under a repeated semantic digest.
Current whole-fortress Observe and Query grants precede existence checks;
source work, elapsed time and complete escaped output remain bounded. The
manifest keeps sealed, historical achievement and restore-abandonment anchors
distinct. Its current predicate/project truth is explicitly unevaluated, and
reading never certifies proof or writes custody. A stable original source pin
does not freeze subsequent historical progress; every read reports its current
authorized anchor and retained history. These links make prior-goal details
retrievable when a bounded plan response omits the complete original request.

The advertised `df://fortress/...`, knowledge and memory families remain
unimplemented resource targets pending explicit session authority binding.
The existing doctor resource is `df://doctor/{session_id}`. This laboratory
resource subset does not establish any live-game or production admission.

## Error behavior

Every error includes:

- stable code;
- human message;
- retry/recovery class;
- whether the prior anchor remains usable;
- whether an effect may have occurred;
- current anchor when known;
- affected scope;
- active work that still exists;
- evidence/findings;
- the minimum safe next protocol step.

Recovery classes:

```text
never_unchanged
safe_read_retry
refresh_and_retry
rebase_required
backoff
reconciliation_required
confirmation_required
operator_action_required
```

An error does not force the agent to start over unless continuity is genuinely lost.

## Output shaping

Safety, continuity, active work, uncertainty, and recovery guidance have priority over optional
detail. The server never truncates mid-object or omits a warning to meet a token budget. It
returns continuations or drill-down resources. Omission order and continuation boundaries are
deterministic under the pinned state, profile, policy, and budget.

## Schemas

The schemas in `schemas/` freeze the naming and bounds of the logical surface. They are transport-
independent: the MCP wire layer renders the dotted tool names with underscores, and the modern-era
request routing is provided by the pinned `fastmcp-rust` facade rather than by this repository.
The Agent Turn Packet is currently an additive executable projection and will move into the shared
schema catalog as Gate A1 hardens.

### Multi-level connected laboratory rooms

The existing `fortress.open_session` accepts `starter_fortress_multilevel`, which
extends the finite-workforce/economy/provisions scenario with complete hazard
coverage around the entrance level and its immediate neighbors. The existing
`fortress.plan` connected-room blueprint accepts the explicit
`planner: "connected_room_stairs_v1"` marker. Its bounded 3-D routing composes
ordinary mining with paired `UpDownStair` shafts outside original room floor
masks, then installs exact furnishings and proves current entrance access.

`continue_goal` retains that exact model and volume through the distinct
`connected_room_stairs_residual_v1` compiler. Existing single-level source and
archive bytes remain unchanged. See [multi-level connected rooms](docs/LAB_CONNECTED_ROOM_ACCESS.md#connect-rooms-across-levels)
for the complete request, geometry limits and laboratory-only stair semantics.

`starter_fortress_multilevel_resources` adds the same finite harvest sources,
worker assignments and opening stock as `starter_fortress_resources` to that
five-level terrain. With `connected_room_stairs_resources_v1` and the inner
room's explicit `acquire_resources` policy, one retained project can gather
missing inputs, then manufacture, excavate stairs and furnish from observed
yield. Each stage keeps its own review and commit. Existing scenarios retain
their original opening snapshots.

### Owned civilian recovery and guarded release

The laboratory `fortress.query` mode `civilian_recovery` reports process-local
clearance shared by every session of the same world. It accepts no historical
selector or caller-supplied threat/clearance claim. Up to 16 burrows and 64
civilians per release are monitored; the complete census and response must fit
current query allowances. Duplicate reads do not add stability; restore/restart
rebuild the interval.

The closed blueprint JSON string `{"template":"civilian_release","burrow":"3001"}`
uses the owner's current complete census and stable 100-tick clear interval.
Its private source is sealed for exact historical reconstruction. Commit repeats
current owned-clearance and complete-work allowance checks; any changed anchor
requires a fresh Plan/review. Membership removal precedes restriction release,
and each effect checks current threats and exact civilian/burrow generations.
The ordinary ConfigureLogistics/Checkpoint grants remain required. Protective
compensation preserves original generations while permitting returned danger.
Existing explicit operator action arrays retain their original semantics.
See `docs/MCP_CIVILIAN_PROTECTION.md` for the complete workflow and limitations.
