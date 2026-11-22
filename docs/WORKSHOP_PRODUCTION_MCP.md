# Staged workshop and production MCP session

The `workshop-production` development profile keeps one session from an original
Carpenter workshop into the combined workforce and finite-order owner. It makes
the existing construction and production paths usable together: finish the
original workshop, settle its clock, import its exact acknowledged custody,
observe the current workforce, and review a finite production plan.

This profile is explicitly unadmitted. It preserves two original goals and their
independent journals. Workshop native observations and the production owner's
canonical observations do not establish a shared canonical timeline. Completed
construction, current workshop readiness, order completion and actual furniture
inventory remain separate evidence.

## Configure the fixed owners before startup

Use the existing closed operator configurations from:

- [Workshop construction](WORKSHOP_CONSTRUCTION_MCP.md), optionally including
  its acknowledged Tree material donor and
  [original-workshop clock](WORKSHOP_CLOCK.md).
- [Combined production](COMBINED_PRODUCTION_MCP.md), including its workforce
  and work-order configurations, optional Manager donor, input-supply policy and
  separately authorized production clocks.

Supply all five configuration variables:

| Variable | Purpose |
| --- | --- |
| `DFMCP_WORKSHOP_PRODUCTION_CONFIG` | Parent stage and retained recovery floor |
| `DFMCP_WORKSHOP_MCP_CONFIG` | Original Carpenter footprint, material policy, source pins and workshop custody |
| `DFMCP_COMBINED_PRODUCTION_CONFIG` | Combined original-plan custody and optional production policies |
| `DFMCP_SEMANTIC_WORKFORCE_CONFIG` | Workforce native custody and independently accepted source |
| `DFMCP_SEMANTIC_WORK_ORDER_CONFIG` | Finite-order native custody and independently accepted source |

For a new workflow, the parent configuration is:

```json
{"schema":"dfmcp.workshop-production-development/1"}
```

All four child configurations must use `initialize: true` together. If a
Workshop clock is configured, it must also initialize in that opening. Use
distinct, nonnested private custody directories for Workshop, combined
production, workforce, orders and any Tree or Manager donors. The children must
pin the same fortress and software; workforce and orders must agree on their
citizen source and endpoint. Native family generations retain their own meanings.

Omit `workshop_prerequisite` entirely from the combined configuration, including
a null placeholder. The staged owner derives that prerequisite from its own
completed Workshop original. A tool argument cannot substitute a directory,
building, source binding, acknowledgement or handoff digest.

Enable these exact development opt-ins:

```text
DFMCP_ALLOW_UNADMITTED_WORKSHOP_PRODUCTION=1
DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20=1
DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION=1
DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE=1
DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS=1
```

Keep the independent child credentials and effect permissions required by their
guides. Configure a Tree donor or Workshop clock only with its corresponding
opt-in. Production clock and Manager credentials are accepted only when their
fixed child policies are configured. Production admission state and unrelated
`DFMCP_` profile variables are rejected.

The parent pins the complete bounded operator configuration, including optional
authority and credential presence, for the session. Changing it requires closing
and reopening with the latest complete recovery references. The server never
changes environment variables to switch owners.

Start the standard executable:

```sh
dwarf-fortress-mcp serve-gameplay-dev workshop-production
```

## Run the two original goals

The profile exposes the same eleven tools. The selected stage determines which
existing closed child requests are available.

### 1. Open and construct the original workshop

Call `fortress_open_session` with `mode: "control"` and explicit bounds. The
default per-call allowance is 60 seconds, 8 GiB of bounded work, and 32,768 output
tokens. `max_game_ticks` defaults to zero; explicitly supply at most 1,200 when
using configured clocks.

The parent opens the empty production custodians and the Workshop owner before
a construction request becomes reachable. All initialization belongs to this
opening. A later handoff uses the same production owners.

Follow the Workshop Agent Turn recommendations to observe the configured
material, plan the fixed Carpenter footprint, review and confirm its exact
`review_seal`, and wait for original construction progress. Optional
`run_workshop` intervals keep their separate reviews, clock permission and
durable recovery.

In the Workshop stage, commit arguments use `review_seal` and omit `scope`.
The exact retained seal selects the construction or clock review. Wait and
cancel retain the existing optional `scope: "clock"`; their default scope is
the workshop goal.

### 2. Import the completed original

After the original is resolved and historically complete, and every Workshop
clock interval is settled, call `fortress_plan`:

```json
{
  "session_id": "<staged session ID>",
  "request": "{\"kind\":\"begin_production\"}"
}
```

This closed request accepts only its `kind`. The owner verifies the exact
Workshop journal and independent acknowledgement, original placement identity,
completion sample and settled clock history. It retains a deterministic
descriptor, releases the exclusive Workshop writer and imports that exact
original Offline into the already-open production owner.

The transition performs local custody work. It grants no construction, labor,
order or clock permit, and it preserves the session's mode. It is available in
Offline and Reconcile modes for custody transfer as well as Control.

If import fails after the writer is released, the stage remains `transition`
with the same handoff descriptor and production custodians. Repeating
`begin_production` retries that same read-only import. Inspection and close
remain available; ordinary observation, planning and effect requests wait for
the import to finish.

The existing Workshop `fortress_query` request with
`mode: "production_handoff"` also works in the Workshop stage for inspection.
That query exports the descriptor without changing the stage.

### 3. Observe and review finite production

After the stage becomes `production`, call `fortress_observe` to obtain a new
production observation. The construction observation cannot serve as a
production planning basis.

Use the existing combined-production request families, including workforce
choices, already-enabled labor, finite stock targets, portfolios and configured
input-supply checks. Order preparation and dispatch still freshly qualify the
exact original workshop. An optional Manager donor remains an independently
configured prerequisite.

Production commits use `paired_seal`. Supply exactly one seal field, matching
the current stage. Production `scope` and `order_step_id` retain their existing
meanings for steps and independent order or inventory clocks. Each original
effect needs its own current review and explicit confirmation.

Wait and inspect using the production owner's existing original-goal and
inventory semantics. An order receipt proves its own insertion or progress;
actual furniture output requires the corresponding fresh inventory evidence.
The parent does not infer that a specific workshop caused a particular item.

## Stage routing

| Tool operation | Workshop | Transition | Production |
| --- | --- | --- | --- |
| Observe | Existing Workshop observation | Refused pending exact import | Existing workforce and optional production observation |
| Query / Explain / Doctor | Retained original and optional handoff export | Retained transition custody | Existing combined inspection and selected order |
| Plan | Existing Workshop and configured Workshop clock; `begin_production` when eligible | Retry the same `begin_production` | Existing combined planning and preparation |
| Commit | Exact `review_seal`, no scope | Refused | Exact `paired_seal`, existing production scopes |
| Wait | Original construction or selected clock | Local inspection with no scope | Existing combined reconciliation and progress |
| Cancel | Original preparation or selected clock safety stop | Close with `scope: "session"` | Existing combined cancellation or clock safety stop |
| Checkpoint / Restore | Unavailable game operations | Unavailable game operations | Unavailable game operations |

`fortress_cancel(scope="session")` is available throughout. It releases the
session owners after the complete recovery acknowledgement fits the response.
It preserves journal files and does not itself stop native work or establish a
paused game. Settle an outstanding native clock through its existing safety
operation before closing when current authority allows that.

## Retain the complete workflow checkpoint

Every normal response includes `workflow` with schema
`dfmcp.workshop-production-recovery/1`:

- `parent` contains the stage, exact handoff digest when applicable, and the
  greatest consumed authority tick floor.
- `workshop` retains the independent acknowledgement, original identity and
  any Workshop clock recovery reference.
- `production` retains native custody IDs, original-plan recovery when present,
  clock IDs and required Workshop, Manager and input-supply checkpoints.
- `production_handoff` retains the exact verified donor descriptor once the
  transition starts.

Preserve the complete newest checkpoint and the private files together. The
parent has no separate durable journal. Loss or rollback of the external
checkpoint can therefore lose parent-stage and consumed-floor knowledge; this
profile does not provide independent protection against coordinated rollback of
all custody and external pins.

To reopen, set every child `initialize` flag to false and supply its exact
retained references using the existing child configuration shapes. Put
`workflow.parent` into the parent configuration's `recovery`:

```json
{
  "schema": "dfmcp.workshop-production-development/1",
  "recovery": {
    "stage": "production",
    "handoff_digest": "<exact retained digest>",
    "authority_tick_floor": 15000
  }
}
```

For stage `workshop`, omit `handoff_digest`; stage `transition` or
`production` requires it. Recovered Workshop or Transition stages cannot carry
a production original. Production recovery admits only the existing exact
combined original, when one exists. Reopening validates the completed donor
and settled clock Offline against the retained digest before importing it.

Recovery restores custody and monotone authority floors. It does not restore
old review permissions or authorize another construction original. Production
source anchors retain their exact observed values; a later consumed authority
tick is kept separately rather than rewritten into canonical state.

Complete responses include escaped transport bytes in their publication bound.
If an operation or publication fails, current reviews are withdrawn and the
remaining custody is retained. An oversized recovery object is withheld as a
whole with an explicit incomplete marker, so a truncated object cannot be
mistaken for a complete checkpoint. Retain the last complete checkpoint and
inspect the same session before further work.

## Evidence boundary

The source reuses the actual Workshop, Run, workforce, work-order and combined
production owners. Focused configuration, routing, publication and custody
regressions accompany it. Execution results are recorded in
[Implementation Status](../IMPLEMENTATION_STATUS.md); authored tests alone do
not establish an executed native campaign.

Untouched-game construction through actual finite furniture output, full modern
MCP execution and production admission remain separate acceptance work under
`dfmcp-reality-wiring-proof-4bf.14`.
