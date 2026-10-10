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

The laboratory `blueprint` JSON string also accepts the closed
`{"template":"continue_goal","plan_digest":"<retained production goal>"}` form.
It compiles a new reviewed plan from the complete original quotas and setup
options under current Observe/Plan authority. Original and same-root work must
be proven quiet, and commit repeats the check before effects. Parent/root
lineage survives durable recovery; no old grants, actions or proof are reused
as authority. See `docs/PRODUCTION_GOAL_CONTINUATION.md` for the exact bounds,
refusals, source-seal binding and explicit commit workflow.

### `fortress.commit`

Requires exact plan ID/digest, expected anchor, and confirmation seal when policy requires.
Revalidates before effects. Returns per-step action states, checkpoint receipt, obligations,
evidence, observed delta, and prediction-versus-observation comparison. Timeout may produce
`indeterminate`.

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

In the reference laboratory, a positive `max_game_ticks` request requires
current `ControlClock` authority. An unpaused advance must remain within both
clock and observation grants through its final tick, and within the negotiated
game-tick budget. Zero-tick polling requires observation authority only.
Goal outcome and observed physical work are reported separately.

### `fortress.cancel`

Starts or advances request/drain/compensate/finalize. It never means “delete the record.” Active
work remains visible throughout the drain.

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
