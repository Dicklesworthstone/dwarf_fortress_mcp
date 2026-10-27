# Original native work-order goals through MCP

`dfmcp-live-semantic-work-orders-dev-server` connects one original semantic
furniture-order goal to native preparation, a single confirmed insertion,
optional reviewed bounded simulation runs, foreground completion checks and
durable recovery. It composes the existing canonical citizen, work-orders/1.10,
selected-order progress/1.12 and
operations/1.3 clients with `SemanticWorkOrderSession` and its original-goal
history. Optional order-run/1.14 control advances that same receipt-linked
order through the frozen eleven MCP tools.

This is an **explicitly unadmitted development runtime**. The operator accepts
an exact, narrow source contract for this process. That acceptance does not
populate the compatibility registry, qualify a DFHack build, consume a
production admission ticket or add a production runner. The generic laboratory
MCP route remains separate.

## What the goal means

The closed request creates one unconditional order for 1–100 `wooden_bed`,
`wooden_door`, `wooden_table` or `wooden_chair` units. Completion requires the
original receipt-linked order to retain its requested total, have zero remaining
units, be validated and be inactive. The original absolute deadline, polling
cadence and required number of stable samples remain part of the sealed plan.
Semantic completion additionally requires qualified complete global job
quiescence. An unrelated active job can therefore keep physical completion
unproved even when the selected order's predicate is true.
If the game removes the order before a qualified completion sample, disappearance
remains unknown; it cannot substitute for the required counter and status proof.

The response separates these questions:

| Response field | Meaning |
|---|---|
| `insertion.verified` | The original native insertion has an exact Created receipt. |
| `current_goal.current_truth` | Original predicates at the latest published sample anchor. |
| `current_goal.original_goal_proven` | The original goal and its temporal requirement are satisfied at that sample. |
| `first_satisfied_anchor` | Historical first qualified achievement; it does not assert current truth. |
| `current_goal.physical_quiescence_proven` | The qualified complete global job domain is quiet. |
| `current_goal.semantic_completion_proven` | Original goal proof, receipt linkage and physical quiescence hold together. |
| `current_goal_available` | A current-process goal sample exists; exact restart does not fabricate one. |
| `clock.active_run` | The separate reviewed native run, if one remains unsettled. |
| `clock.may_be_running` | Retained clock uncertainty still prevents a fresh paused original-goal sample. |
| `clock.runs[].native.historical_pause_verified` | That native receipt verified a pause at its recorded tick; it does not establish the game's current pause state. |

This is a finite native order goal. It does not prove an inventory quota,
produced-item quantities, material availability or workshop feasibility. Brewing,
cooking, order conditions, multiple goals and automatic chaining of runs are
outside this runtime. Clock receipts retain their own source anchors and never
become original-goal proof or replace its canonical sample anchor.

## Operator configuration

Build and run the dedicated executable:

```sh
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-semantic-work-orders-dev-server
```

The process environment has exactly these permitted `DFMCP_` names:

| Variable | Purpose |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS` | Must equal `1`. |
| `DFMCP_SEMANTIC_WORK_ORDER_CONFIG` | Closed JSON configuration, at most 16 KiB. |
| `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION` | Absent, or exactly `1` to permit original-order insertion control. |
| `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK` | Absent, or exactly `1` to permit separately reviewed native clock control when configured. |
| `DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN` | Credential for the canonical citizen source. |
| `DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN` | Credential for work-orders/1.10. |
| `DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN` | Credential for progress/1.12. |
| `DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN` | Credential for operations/1.3. |
| `DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN` | Separate credential for optional order-run/1.14 acquisition and control. |

Credentials are separate operator secrets, each bounded to 32–256 bytes. MCP
clients cannot supply credentials, file paths, endpoints, raw plans, source
grants or protocol selectors. The process rejects other `DFMCP_` state,
including production tickets, launch state and admission provenance. The
production and clock gates and immutable configuration are rechecked during
their respective operations, including immediately before native effects.
Insertion authority does not grant clock control, and clock authority does not
grant insertion control.

Example configuration template, with illustrative versions and incarnation
numbers that must be replaced with the operator's exact accepted sources:

```json
{
  "schema": "dfmcp.semantic-work-order-development/1",
  "directory": "/absolute/operator-owned/semantic-order",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "sources": {
    "kind": "operator-development-source-acceptance/1",
    "world_folder": "region1",
    "site_id": 1,
    "df_version": "<exact DF version>",
    "dfhack_version": "<exact DFHack version>",
    "citizen_bridge_version": "<exact citizen bridge version>",
    "citizen_generation": 701,
    "work_order_generation": 7,
    "progress_generation": 801,
    "operations_generation": 901
  }
}
```

The directory must already be a real, exclusively controlled `0700` directory.
The private regular files use `0600`, locked descriptor custody, exact codec
checks and bounded verification. Symlinks, replaced files, corrupt histories
and ambiguous persistence are refused. The four original-goal filenames are
`creation.journal`, `semantic.associations`, `original.plan` and `goal.history`.
Optional clock custody adds the fixed filename `clock.journal` in the same
directory.
The endpoint must be a numeric loopback address with a nonzero port.

Each plugin has its **own** incarnation namespace. Canonical, creation,
progress, operations and optional clock numbers need not be equal; each must
remain equal to its own accepted identity. This configuration cannot silently bless a source
reload for an old plan. There is no cross-incarnation or restore-epoch migration.

`initialize: true` requires control mode, production authority and missing
initial custody files.
Reopening requires `initialize: false` and existing files. Missing recovery
files are never recreated as empty histories. Use a separate directory for a
new goal after completing or retiring the retained one.

### Optional clock custody

To initialize the same goal with bounded simulation control, include this
additional `clock` field in operator configuration and enable the separate
clock gate:

```json
{
  "clock": {
    "generation": 1001
  }
}
```

The example generation must be replaced with the exact order-run/1.14 source
incarnation. It uses the existing pinned endpoint, fortress and exact DF/DFHack
versions. Initialization also requires the clock credential and creates its own
journal identity. Omit `journal_id` entirely when `initialize: true`.

On every reopen, including offline inspection, set `initialize: false` and
supply the independently retained identity:

```json
{
  "clock": {
    "generation": 1001,
    "journal_id": "<exact 64-character clock journal identity>"
  }
}
```

Take this identity from `result.clock.journal_id`; once an original plan exists,
it is also present in `result.recovery.clock_journal_id`. The two configuration
locations must agree exactly. Clock recovery requires the existing file and
refuses missing, substituted or corrupt custody. Omitting `clock` cannot hide
an existing `clock.journal`. Existing configurations without clock custody
continue to work, with game progress supplied externally. If opening inspection
fails before the first plan exists, its error can still retain the startup
clock journal identity as an explicitly unverified recovery reference; it does
not verify custody or authorize an effect.

Control reopening can retain clock authority after production authority has
been removed; the creation journal then opens for reconciliation only. A clock
journal opened without clock authority likewise remains reconciliation-only
for that session. Reenabling an environment gate cannot promote either journal
in place. Offline opening reads local custody without native credentials or
connections.

## MCP workflow

The logical names below use dots. Their wire names use underscores, for example
`fortress.plan` is `fortress_plan`. Use the modern MCP metadata described in
[`FASTMCP_INTEGRATION.md`](FASTMCP_INTEGRATION.md); the server uses MCP
2026-07-28. Tool results contain the canonical `agent_turn` packet and the
tool-specific `result`.

1. **Open an authorized control session.** Call
   `fortress.open_session` with `{"mode":"control"}`. To permit optional
   bounded runs, negotiate a positive allowance, for example
   `{"mode":"control","max_game_ticks":1200}`. This allowance defaults to
   zero and cannot exceed 1200. Opening never unpauses the game, and client
   parameters cannot create authority absent from the operator environment.
   The default mode is `offline`, which opens existing custody without native
   connections.

2. **Observe while paused.** Call `fortress.observe` with the returned
   `session_id`. Preserve `result.observation.seal` and the complete anchor.
   This acquires the exact canonical source used to compile and revalidate the
   original intent. The tool does not pause the game for the caller.

3. **Compile and prepare the original goal.** Call `fortress.plan` with
   `session_id` and a `request` string containing this closed JSON shape:

   ```json
   {
     "expected_observation": "<64-character observation seal>",
     "recipe": "wooden_bed",
     "amount": 4,
     "deadline_tick": 12000,
     "poll_interval_ticks": 10,
     "stable_samples": 2
   }
   ```

   Choose an absolute future deadline within one game year of the observed
   tick. Cadence and stability must fit inside that original horizon. The
   default cadence is ten ticks and the default stability requirement is one
   sample. Requests are bounded to 4 KiB, reject unknown fields and preserve
   their original identity on repetition. Preparation archives the complete
   original plan and binds it to the native review; it does not insert an order.

4. **Review and confirm exactly once.** Check `original_plan`, the paired
   `review` and the recovery handoff. Call `fortress.commit` with `session_id`,
   `paired_seal` from the review and `confirmed: true`. Omit `scope` or use
   `scope: "goal"`. Native insertion intent is durably recorded before the
   insertion call. A confirmed receipt proves
   insertion only. Save `result.recovery` outside the private archive so a later
   process has an independently supplied exact handoff.

5. **Advance and poll original completion.** With optional clock custody, use
   the separately reviewed run workflow below. Otherwise let the game progress
   through an explicitly managed external control path and pause for a coherent
   sample. Call `fortress.wait` to reconcile any retained clock run before
   attempting a fresh original-goal sample. A running or unresolved clock
   returns pending evidence without a paused goal read. Each call is bounded;
   it never starts a run, retries an unpause or starts a background loop.
   Later samples must satisfy the original cadence and stability rules.
   The first original-goal sample after insertion starts durable goal history,
   including when acquired during run preparation or confirmation. Preserve
   the updated recovery object with `history_present: true` after that history
   exists. Repeated reads at the same game tick do not satisfy an advancing
   cadence.

6. **Inspect or close.** `fortress.query`, `fortress.explain` and
   `fortress.doctor` verify retained custody without contacting the game. They
   present evidence as of its recorded anchor. `fortress.cancel` with
   `scope: "session"` releases held sockets and locks without deleting evidence
   or cancelling game effects. It remains usable when ordinary operations are
   fenced, cancelled or no longer authorized.

`fortress.cancel` with `scope: "goal"` is the separate authorized operation
that retires a still-prepared insertion. It cannot delete an already inserted
manager order or erase an uncertain effect. `scope: "run"` instead retires a
prepared clock run or requests its native owner's safety pause after dispatch,
under current clock authority. It retains the original goal and manager order.
`fortress.checkpoint` and `fortress.restore` are explicitly unavailable; their names remain present to
preserve the eleven-tool interface.

### Advance the retained goal with a reviewed bounded run

After verified insertion, call `fortress.plan` with the same `session_id` and a
`request` string containing this closed JSON object:

```json
{
  "kind": "run",
  "original_paired_seal": "<result.review.paired_seal for the original goal>",
  "game_ticks": 10,
  "wall_millis": 1000
}
```

Run requests are bounded to 2 KiB. The caller chooses a finite time slice;
the runtime derives the exact native order from the original Created receipt.
It verifies the archived original plan, acquires fresh original-goal evidence
and prepares a distinct clock review without unpausing. The complete requested
tick slice must fit within the original absolute deadline and within the
session's negotiated tick allowance. A different original seal or a request
that would extend the deadline refuses.

Review `result.clock.active_run`, including the native order, tick and wall
bounds, stop predicate, purpose and separate `review_seal`. Confirm that exact
clock review through the existing tool:

```json
{
  "session_id": "<retained session>",
  "scope": "run",
  "paired_seal": "<result.clock.active_run.review_seal>",
  "confirmed": true
}
```

This is the input to `fortress.commit`. The original paired seal does not
substitute for the clock review seal. The runtime rechecks the original archive,
Created receipt and current clock authority before the native commit, after
durably recording dispatch intent. The native DFHack callback owns the bounded
run and stop after the request connection closes. There is no Rust background
worker and no automatic next run.

Call `fortress.wait` to query the same native run. Once it is safe to attempt a
fresh paused sample, the original goal monitor evaluates that sample under the
unchanged deadline, cadence and stability requirement. `clock.runs` retains
native receipts separately from `current_goal`. A native stop, pause receipt or
observed counter never by itself promotes the original goal to completion.
If more progress is needed, prepare and confirm another run explicitly. One
unsettled run blocks another, and one original goal retains at most 32 runs.

For an incomplete order, the native early-stop predicate is remaining units at
most zero. Once the exact order is already complete, validated and inactive,
the runtime can still advance a bounded slice for a second original stable
sample or for global job quiescence. It then uses `Active` as an early-stop
guard against unexpected reactivation. That guard is not the goal's success
predicate. For example, a `stable_samples: 2`, `poll_interval_ticks: 10` goal
still requires two qualifying original samples at least ten ticks apart;
neither an immediate repeated read nor a clock receipt supplies the second
sample. For a currently true goal still awaiting temporal proof, run
recommendations use the original polling cadence to leave room for the further
samples required by `stable_samples` greater than two. Once original completion
and physical quiescence are both proved, further runs for this goal are refused.

## Lost replies and restart

A failed commit can leave an insertion outcome indeterminate. Its error packet
does not claim that no mutation occurred and does not recommend retrying
insertion. Inspect and reconcile the **same original goal and paired review**.
Neither a new key nor a new semantic request can discharge the old uncertainty.
Failed calls retain the last complete response only as an explicitly unverified
historical reference; no cached prepared state authorizes another commit.

A lost clock commit reply similarly retains the exact native run and its
durable dispatch intent. `fortress.wait` queries that run; it never dispatches
another unpause. A running, indeterminate or source-lost clock blocks new runs
and paused goal sampling until the retained state permits them. Source-loss
evidence remains unresolved and does not prove that a replacement source is
paused. Preserve clock
recovery identity alongside the original plan even if an error contains only
the bounded recovery projection.

For restart, close the old session/process and set `initialize: false`. Add
the exact saved `recovery` object to operator configuration:

```json
{
  "plan_digest": "<original plan digest>",
  "object_digest": "<original archive object digest>",
  "anchor": {
    "fortress_id": "<canonical decimal fortress identity>",
    "epoch": 0,
    "sequence": 1,
    "tick": 10000,
    "state_hash": "<original anchor hash>"
  },
  "history_present": true,
  "clock_journal_id": "<exact clock journal identity when configured>"
}
```

All fields come from the exact prior response; the example numbers are not
replacement values. When clock custody is configured, `clock_journal_id` is
required in this recovery object and must equal `clock.journal_id` in the
operator configuration. Without clock custody, omit it as in prior handoffs;
an explicit null is also accepted. The archive cannot authorize itself by
presenting its own digest. A lost handoff, partial initialization or corrupt custody requires
operator investigation; the server does not repair, prune, replace or adopt it
automatically.

Reopen in `offline` to inspect without any native credentials or bridge
contact. Reopen in `reconcile` for Query/Observe-only receipt and goal polling;
this mode never acquires insertion or clock control authority. Reopening
existing clock custody itself needs no clock credential; native receipt
reconciliation subsequently needs its source credential. Exact recovery can
retain verified first achievement and historical terminal status. It clears the latest sample
and unfinished stability streak and never renews the original deadline.
Reopening a still-prepared plan for control requires reacquiring its exact
original canonical anchor before commit.

## Bounds and evidence

One process retains one session and one original goal. Concurrent requests use
nonblocking ownership acquisition; a busy session returns a bounded refusal.
Session identities are process-scoped and use a distinct semantic runtime
namespace. Each operation has at most 60 seconds, 4096 entities, one action and
the negotiated game-tick allowance of 0–1200, defaulting to zero. The default
call wall allowance is five seconds. An optional native run requests 1–1200
ticks and a separate 1–60000 millisecond stop bound. Those native limits are
checked at callback opportunities, so actual progress can overshoot; the
receipt reports observed tick overshoot. Other controllers can also affect the
game clock and are not globally fenced by this runtime.
Complete native acquisition is capped at 128 citizens, 1024 jobs, 1024
buildings and 2046 items; an oversized observation refuses instead of silently
truncating its domain.

The maximum byte-work allowance is 2 GiB. This is a logical upper bound covering
repeated complete custody verification and all potential history appends, not
an eager allocation or a network-frame size. The facade reserves 128 KiB for a
complete response before work begins, plus 32 MiB for its separate opening
inspection. The output allowance is 32768 four-byte proxy units. Smaller output
allowances refuse before I/O; JSON is never truncated. An oversized response
falls back to bounded recovery identifiers and explicit uncertainty.

Source policy is fixed in code: initial canonical pause/citizen identity,
receipt-linked selected-order counters/status and complete global job count
needed for physical quiescence. Operator JSON cannot add evidence fields or
capabilities. All native source pins are rechecked at acquisition and publication
boundaries. Prepared-plan, association, creation and durable-history formats
are the existing bounded formats.

The component tests execute the real original-plan compiler, semantic/native
and clock coordinators, durable goal monitor and private-file open modes with
injected native responses. They cover original-goal continuation, lost clock
replies, exact recovery and final-effect revalidation. They do not establish a
live DFHack campaign or compatibility admission. Current executed validation and its exact scope are recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md).
