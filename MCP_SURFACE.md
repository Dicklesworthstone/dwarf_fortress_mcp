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

The laboratory `blueprint` JSON string also accepts the closed
`{"template":"continue_goal","plan_digest":"<retained goal digest>"}` form.
It compiles a new reviewed plan from complete original production quotas or
furnished-room requests, preserving setup options and any connected-room
entrance/domain under current Observe/Plan authority. Valid installed furniture
is retained; a connected continuation can repair access with excavation alone.
Original and same-root work must
be proven quiet, and commit repeats the check before effects. Parent/root
lineage survives durable recovery. A verified reopen binds the retained goal
to the exact recovered world while preserving its old seal; arbitrary epoch
changes and checkpoint-abandoned pursuits remain refused. No old grants,
actions or proof are reused as authority. See `docs/PRODUCTION_GOAL_CONTINUATION.md` for the exact bounds,
refusals, source-seal binding and explicit commit workflow; furnished-room
continuations are detailed in `docs/ROOM_GOAL_CONTINUATION.md`.

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
committed plan), `"oldest_open_plan"` (the oldest retained plan with unfinished
work), and `"session"` (retained open work across the session's plans).
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

```text
df://session/{id}/summary
df://session/{id}/capabilities
df://session/{id}/handoff
df://session/{id}/tasks
df://session/{id}/task-{task_id}
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
