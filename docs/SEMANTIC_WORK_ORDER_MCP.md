# Original native work-order goals through MCP

`dfmcp-live-semantic-work-orders-dev-server` connects one original semantic
furniture-order goal to native preparation, a single confirmed insertion,
optional reviewed bounded simulation runs, foreground completion checks and
durable recovery. It composes the existing canonical citizen, work-orders/1.10,
selected-order progress/1.12 and
operations/1.3 clients with `SemanticWorkOrderSession` and its original-goal
history. Optional order-run/1.14 control advances that same receipt-linked
order through the frozen eleven MCP tools. Independently configured run/1.13
control can finish an already true inventory objective's remaining stability
after the native order has retired.

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

An optional, explicitly selected `observed_inventory_minimum` objective instead
asks whether the complete operations item roster contains a minimum number of
stack units of the recipe's item type. Its entire original terminal,
postconditions and temporal obligation use that inventory predicate. It has a
distinct sealed request identity; an existing order-completion goal cannot be
converted after the order disappears. Omitting the objective preserves the old
request identity, original predicates and durable history policy exactly.

Inventory proof can remain available after normal order retirement. It includes
preexisting and unrelated matching items, and items of any material or usability.
It proves neither new production nor which order caused an item. A missing
original order still has unknown physical work, `receipt_linked: false` and
`semantic_completion_proven: false`, even when the inventory objective is true.

A separately selected `observed_inventory_increase` objective requires a net
increase above the complete, independently qualified inventory census captured
at the original paused planning tick. The server seals the baseline quantity,
source witness and allocation frontier before native preparation. Preexisting
stock alone cannot satisfy the requested increase; newly produced output and
order attribution still remain unproved.

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
| `original_plan.objective` | The immutable selected objective: original order completion, observed inventory minimum, or observed inventory increase. |
| `inventory_objective.proven_at_anchor` | The explicitly selected inventory predicate satisfies its original temporal requirement at the named sample. |
| `inventory_objective.first_satisfied_anchor` | First qualified inventory achievement, preserved independently of later consumption or restart. |
| `clock.active_run` | The separate reviewed native run, if one remains unsettled. |
| `clock.may_be_running` | Retained order-clock uncertainty still prevents a fresh paused original-goal sample. |
| `inventory_clock.active_run` | The independently reviewed run for unfinished original inventory stability. |
| `inventory_clock.may_be_running` | Retained inventory-clock uncertainty; it also blocks a fresh goal sample and another clock run. |
| `clock.runs[].native.historical_pause_verified` | That native receipt verified a pause at its recorded tick; it does not establish the game's current pause state. |

The default goal proves finite native order completion. The optional inventory
goal proves only its declared observed stock minimum or net increase; no mode proves
order-attributed output quantities, material availability or workshop feasibility. Brewing,
cooking, order conditions, multiple goals and automatic chaining of runs are
outside this runtime. Clock receipts retain their own source anchors and never
become original-goal proof or replace its canonical sample anchor.

## Operator configuration

### Configure the game process separately

The native plugins and the Rust MCP process have different closed environments.
Enabling the semantic MCP runtime does not enable a native plugin in the game.
The game process must receive the following configuration before it starts:

| Native source | Game-process credential | Additional game-process gate |
|---|---|---|
| Citizen 1.0 | `DFMCP_BRIDGE_TOKEN` | None |
| Work orders 1.10 | `DFMCP_WORK_ORDERS_TOKEN` | `DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10=1`; prepare/commit also require `DFMCP_WORK_ORDERS_ALLOW_PRODUCTION=1` |
| Selected-order progress 1.12 | `DFMCP_WORK_ORDER_PROGRESS_TOKEN` | `DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12=1` |
| Operations 1.3 | `DFMCP_OPERATIONS_TOKEN` | None |
| Optional order-run 1.14 | `DFMCP_ORDER_RUN_TOKEN` | `DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14=1` |
| Optional inventory clock, run 1.13 | `DFMCP_RUN_TOKEN` | `DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1` |

The native production gate is checked for both preparation and commit; a passing
handshake alone does not establish permission to insert an order. Order-run also
refuses `DFMCP_ADMITTED_BRIDGE_PROTOCOL`. Use separate credentials
and exact matching plugins. After loading the intended disposable save, verify
each configured source's authenticated handshake and retain its actual version
and generation before preparing workshop/order fixtures. A refused handshake is
not an accepted source pin. Keep the raw reply and transport stderr before
decoding or asserting success, so a setup failure retains its diagnosis.

These game-process names must not leak into the Rust server environment below:
the semantic runtime rejects unrelated `DFMCP_*` variables. Its production and
clock gates independently authorize the corresponding MCP operations. Neither
set of gates creates compatibility admission. This separation addresses an
actual pre-effect trial failure where a valid semantic-runtime configuration
was paired with a game process missing the native work-order opt-in.

### Configure the MCP process

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
| `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK` | Absent, or exactly `1` to permit independently reviewed inventory stability advancement. |
| `DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN` | Separate credential matching the optional native run/1.13 source. |

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
directory. Optional independent inventory-clock custody adds
`inventory-clock.journal`; it never replaces the original order-clock file.
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

### Optional independent inventory clock

To finish the original inventory stability requirement after the selected order
retires, explicitly configure the independent run/1.13 owner:

```json
{
  "inventory_clock": {
    "generation": 1101
  }
}
```

Replace the example with that native source's exact incarnation. Set
`DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK=1` and supply its separate
`DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN`. The game-side run/1.13 plugin
uses its own gate and `DFMCP_RUN_TOKEN` above. Existing order-clock configuration
and credentials confer no inventory-clock authority.

This native profile identifies a source incarnation, rather than a named
fortress lineage. The operator explicitly accepts that narrower identity under
the pinned endpoint and DF/DFHack versions. The host separately requires the
original named-fortress Query/Observe/Plan/ControlClock authority and qualified
inventory evidence at the same paused tick. It never rewrites a fortress grant
into a source-scoped clock grant. Other controllers are not globally fenced.

Initialization creates `inventory-clock.journal` and returns
`inventory_clock.journal_id`. On every reopen, set `initialize=false`, put that
exact ID in `inventory_clock.journal_id`, and, once the original plan exists,
retain the same value as `recovery.inventory_clock_journal_id`. Omit the ID
entirely for initialization. Missing custody, wrong identity or omission of
configuration while the file exists refuses; the host does not replace it.
Offline inspection needs no native token or connection. A journal opened with
only reconciliation authority cannot later be promoted within that session.

Both clocks may be configured. Each keeps its independent journal, exact target
and authority. Prepared or unresolved work in either prevents starting work in
the other; Wait consults both before a fresh goal sample. The combined labor/order
profile rejects this standalone configuration and its environment names.

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

### Select an observed inventory minimum before preparation

To request useful stock guidance after a finite order may retire, include this
optional objective in the original `fortress.plan` request string:

```json
{
  "expected_observation": "<64-character observation seal>",
  "recipe": "wooden_bed",
  "amount": 1,
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 1,
  "objective": {
    "kind": "observed_inventory_minimum",
    "item_type": "BED",
    "minimum_stack_units": 1
  }
}
```

The recipe and item type must match exactly: wooden bed → `BED`, wooden door →
`DOOR`, wooden table → `TABLE`, wooden chair → `CHAIR`. The minimum is an integer
from 1 through 1,000,000 and is independent of the explicitly requested order
amount. Unknown objective fields, other kinds, lowercase aliases and mismatched
item types are refused before preparation. Changing the minimum, type, cadence
or deadline cannot replace a retained original plan.

Proof sums `stack_size` over every matching `type_key` in the complete qualified
operations roster. It never counts only the sixteen presentation examples.
The source policy adds complete Item coverage and exactly the type/stack source
fields only for this recognized original objective. Incomplete coverage or
unqualified data remains Unknown, even if a partial subtotal is large enough.
The same conditional policy is reissued during exact durable replay; legacy
order-completion policy fingerprints remain unchanged.

The original deadline, cadence and stable-sample count still apply. Repeated
reads at the same game tick do not advance stability. A failed Wait withdraws
the current sample and unfinished streak even when failure occurs before native
sampling; that withdrawal performs no extra I/O under revoked authority. The
next authorized poll persists the gap. First achievement remains historical
evidence if later stock falls below the minimum, a later call fails or the
process reopens. Offline recovery exposes no current inventory proof.

After inventory proof succeeds, Agent Turn marks the selected objective as
satisfied and stops proposing additional clock runs or required polling for
that objective. Original insertion uncertainty still requires reconciliation;
inventory truth cannot resolve it or authorize another insertion. The original
order and any other game work may still be active.

The existing `kind="run"` clock remains tied to the exact receipt-linked order.
After retirement, the separately configured `kind="inventory_run"` path below
can advance unfinished stability only when qualified inventory is already true.
Without that explicit configuration, game time must progress through a separately
managed control path. The one-sample default can finish at the first qualified
post-retirement sample.
The minimum mode makes no delta-inventory, newly-produced, wood-classification,
available-for-use or causal-output claim.

### Require a net inventory increase from the original census

Use this objective in the first request when existing stock should not satisfy
the goal:

```json
{
  "expected_observation": "<latest complete citizen observation seal>",
  "recipe": "wooden_bed",
  "amount": 4,
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "objective": {
    "kind": "observed_inventory_increase",
    "item_type": "BED",
    "increase_stack_units": 4
  }
}
```

The item type must match the finite furniture recipe. `increase_stack_units`
is an integer from 1 through 1,000,000 and is independent of the order amount.
The closed request accepts no baseline quantity, target, source digest,
generation or allocation frontier from the caller.

Before publishing an original archive or preparing an order, Plan acquires one
complete operations/1.3 census at the exact paused canonical observation tick.
The independently pinned operations source must match the accepted world,
site, DF/DFHack versions and its own incarnation. Every Item and its type/stack
facts must qualify; a partial roster or an unknown count cannot become a zero
baseline. Existing acquisition, entity, byte and deadline bounds apply. A newer
consumed native tick requires a fresh canonical Observe before any new original
plan, including a later request that switches to minimum or order completion.
Query and Observe authority are checked at that consumed time without changing
the original canonical anchor.

The immutable typed predicate contains the starting stack quantity, requested
increase, original canonical anchor, exact operations digest, source generation
and native allocation floors. Its target is the checked sum of baseline plus
increase. For example, a baseline of three beds and an increase of two requires
at least five observed bed stack units. All complete later samples must retain
the original fortress/restore epoch, native source incarnation and nonregressing
job, building and item allocation frontiers. First sampling and durable replay
both enforce those original floors.

`original_plan.objective` and `inventory_objective` expose
`baseline_stack_units`, `increase_stack_units`, `target_stack_units`,
`baseline_anchor`, `baseline_source_digest`,
`baseline_source_generation` and `baseline_source_counters`.
The three counters are ordered as next job ID, next building ID and next item ID.
They preserve source continuity; they do not claim that counted items have new IDs.

The original plan archive and its external object seal bind the full baseline.
Repeating the same request uses that retained value without a new baseline
read. Changing its objective cannot replace the original. Offline recovery
restores the sealed baseline and historical achievement, with no source contact,
current inventory proof or unfinished stability. No separate raw baseline
census is retained for offline recounting. Existing minimum/default request
identities, native wires and custody filenames remain unchanged.

Wait evaluates current complete stock against the sealed target under the
original deadline, cadence and stable-sample requirement. An unchanged baseline
is False. Subsequent consumption can make a previously achieved increase False;
historical first achievement remains separately recorded. The receipt-linked
order clock can advance a pending increase while its original order is eligible.
Once inventory is True, the independent inventory clock can finish the original
stability interval after order retirement. Neither clock renews the baseline,
deadline or insertion key.

This is observed net inventory growth, not an item-production receipt. Matching
items of any material or usability count, and unrelated acquisitions can increase
the stock. Consumption can offset new production. A disappeared manager order
still has Unknown original-order completion and unproved causal attribution.

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
samples required by `stable_samples` greater than two. Once original order
completion and physical quiescence are both proved, further runs for that goal
are refused. A selected inventory objective also refuses further runs once its
original temporal proof is satisfied, independently of remaining order work.

### Advance already true inventory through its original stability window

This path is available only for an explicit original inventory objective with a
verified historical Created receipt, qualified current inventory truth and an
unfinished original temporal requirement. It refuses False, Unknown, failed,
already satisfied or legacy order-completion goals. It may be used when the
original order has retired; it does not reconstruct that order or relax its
existing clock guards.

Call `fortress.plan` with a request string containing:

```json
{
  "kind": "inventory_run",
  "original_paired_seal": "<result.review.paired_seal for the original goal>",
  "game_ticks": 10,
  "wall_millis": 1000
}
```

Review `result.inventory_clock.active_run` and confirm its distinct
`review_seal` with `fortress.commit`, `scope="inventory_run"` and
`confirmed=true`. Preparation and commit independently reacquire qualified
inventory and verify the complete original archive, exact review, insertion
receipt and clock custody. A further durable inventory sample follows the
clock's durable marker and precedes its native prepare or sole unpause attempt.
Changed source, custody, inventory truth, original timing or either clock's
settlement refuses the effect. Every requested tick must fit the original
absolute deadline and the session's allowance; neither is extended.

After commit, Wait reconciles the original run and then obtains one fresh
qualified sample when both clock owners permit it. A clock receipt is historical
coordination evidence, not proof of present pause or inventory. Each additional
slice requires a new explicit review and confirmation. There is no background
loop, automatic unpause retry or silent switch from the original order clock.
Agent Turn exposes the separate inventory-clock action only when its precise
conditions and authority are available, and stops proposing it after original
inventory proof.

`fortress.cancel` with `scope="inventory_run"` retires a prepared inventory run
or requests its native safety pause after dispatch. Current Query and clock
authority suffice for that safety path even after the goal deadline or loss of
Plan/Observe authority. Cancellation retains the original goal, insertion and
both clock histories; elapsed game time is not reversed.

The inventory owner retains at most 64 runs. This is a retention ceiling rather
than a guarantee that every 64-sample objective fits: the existing 16 MiB,
2,048-event goal history and 2 GiB per-call work allowance still apply. Repeated
complete preflight and final-edge samples consume history. No history is pruned
and no proof or budget is relaxed to force a longer sequence through.

## Lost replies and restart

A failed commit can leave an insertion outcome indeterminate. Its error packet
does not claim that no mutation occurred and does not recommend retrying
insertion. Inspect and reconcile the **same original goal and paired review**.
Neither a new key nor a new semantic request can discharge the old uncertainty.
Failed calls retain the last complete response only as an explicitly unverified
historical reference; no cached prepared state authorizes another commit.

A lost clock commit reply similarly retains the exact native run and its
durable dispatch intent. `fortress.wait` queries that run; it never dispatches
another unpause. A running, indeterminate or source-lost clock in either owner blocks new runs
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
  "clock_journal_id": "<exact order clock journal identity when configured>",
  "inventory_clock_journal_id": "<exact inventory clock journal identity when configured>"
}
```

All fields come from the exact prior response; the example numbers are not
replacement values. When clock custody is configured, `clock_journal_id` is
required in this recovery object and must equal `clock.journal_id` in the
operator configuration. Independently configured inventory-clock custody
requires `inventory_clock_journal_id` to equal `inventory_clock.journal_id`.
Both identities survive bounded overflow and error projections. Without the
corresponding clock custody, omit its field as in prior handoffs;
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
Complete native acquisition is capped at 200 citizens, 1024 jobs, 1024
buildings and 2046 items; an oversized observation refuses instead of silently
truncating its domain. Citizen acquisition uses native pages of at most 128 and
publishes only a complete coherent paused roster. Original-roster revalidation
retains its existing 16 MiB allowance.

The maximum byte-work allowance is 2 GiB. This is a logical upper bound covering
repeated complete custody verification and all potential history appends, not
an eager allocation or a network-frame size. The facade reserves 128 KiB for a
complete response before work begins, plus 32 MiB for its separate opening
inspection. The output allowance is 32768 four-byte proxy units. Smaller output
allowances refuse before I/O; JSON is never truncated. An oversized response
falls back to bounded recovery identifiers and explicit uncertainty.

Default source policy is fixed in code: initial canonical pause/citizen identity,
receipt-linked selected-order counters/status and complete global job count
needed for physical quiescence. Only the explicit sealed inventory objective
adds complete Item coverage and the observed type/stack fields to its proof
policy. A separate read-only presentation policy accepts
the existing operations/1.3 item identity, type, subtype, numeric material and
stack fields. It does not change the original goal policy or its durable history
fingerprint. Operator JSON cannot add evidence fields or capabilities. All native
source pins are rechecked at acquisition and publication boundaries. Prepared-plan,
association, creation and durable-history formats are the existing bounded formats.

`result.item_observations` presents up to sixteen observed `BED`, `DOOR`, `TABLE`,
`CHAIR` or `THRONE` candidates from a complete paused operations item roster. It
binds the exact sample anchor, operations protocol/generation/source digest,
world/site and DF/DFHack versions. `total_items`, `matching_items`, `returned`
and `omitted` distinguish roster coverage from the bounded selection. Native
item IDs are observation data, not authorized handles. Numeric material pairs
do not establish wood, usable supply, new production or which order caused an
item. Material classification, eligibility and production attribution remain
unknown. Empty results prove no matching candidates only when coverage is
complete for that recorded anchor.

The item view may remain useful when the original order disappears and its
order-completion goal becomes Unknown. It never promotes original completion or mutation
preconditions. Failed/incomplete sampling, attempted unpause, changed source,
presentation-authorization loss and recovery withhold the previous rows;
unavailable coverage is explicitly unknown. Inspecting a retained sample does
not prove current game freshness. These are additional observations under the
same eleven tools and existing bridge fields. The separately selected inventory
objective uses its own original predicate and qualified full-roster proof.

A denied Query or failed canonical Observe also retires the optional cached
rows. Regranting authority or issuing another Query cannot revive them; a new
qualified goal sample is required. Error and close packets preserve recovery
identities while withholding copied item rows. The later canonical-observation
floor guard rejects a reacquisition below a published anchor, a consumed goal
cursor or native tick before rebinding planning evidence. Three `.23`
regressions fail on the original source; all 2,825 R27 MCP component cases pass,
including prepared-runtime reconstruction over memory-backed codecs and fresh
forward observations. Its newer-nightly qualifier subsequently fails Clippy
before full Rust tests or a server build. The next repaired source still needs
its own full qualification; the R26 evidence below does not cover it.

The independent inventory-clock contract is recorded in
[`inventory_goal_clock_v1.json`](../architecture/inventory_goal_clock_v1.json).

The component tests execute the real original-plan compiler, semantic/native
and clock coordinators, durable goal monitor and private-file open modes with
injected native responses. They cover original-goal continuation, lost clock
replies, exact recovery and final-effect revalidation. They do not establish a
live DFHack campaign or compatibility admission. Current executed validation and its exact scope are recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md).

### Executed public item-view proof

The new E trial at exact qualified R28 proves the positive and withdrawal path:
fresh Wait and same-anchor Query return BED 540 in a complete 535-item roster;
the complete 536-item baseline had no bed. Two separately reviewed runs advance
1,200 and 743 ticks with zero overshoot. After changed-source Observe refuses,
Query and Doctor preserve the original goal and anchor while returning unknown
item coverage with no rows. The removed original order remains Unknown, and
public wood classification/eligibility/attribution remain unproved. Parent
replay checks 139 exports, all 23 modern exchanges and native receipt/history
hashes. Six actual-file offline exchanges preserve all five custody hashes;
fresh host checks prove complete cleanup. The timed client retains full raw
frames and makes review practical without changing native TTL or automating
confirmation. Pre-effect configuration-display and UI-timeout diagnostic
limitations remain explicitly recorded in `IMPLEMENTATION_STATUS.md`.
Independent CC review accepts the original `.5.2` criteria at this exact R28
source; this does not close the original-order or broader production goals.
The export records the server hash but no raw launched-executable path/hash
receipt or opened process executable identity. Operator-reported launch and
preflight records remain outside the export. The coordinator accepts that
limited binding for the narrow item-view criteria; exported bytes alone do not
establish source-bound process or runtime qualification.

The separate later laboratory entity consumer is fully Rust-qualified at private
R29 `b6d0e626`: both required commands each pass 5,145 debug and 5,145 release
cases across 123 targets per profile, zero failed or ignored, with all 43 gates.
Its default CLI artifact and current-entity resource route are distinct from
E's live semantic work-order executable. These source checks do not transfer E
into later-source live evidence or close the broader production/resource goals.

The earlier item-view source repair is Rust-qualified at clean private R28
`e7b93a40e6a74bcc425c0c6fec5729296907d2af`, on the November 1 nightly.
Both required commands pass 5,140 debug and 5,140 release cases each, zero
failed or ignored across 123 targets per profile. Parent review verifies the
exact receipt, source and development executable. Actual live D's sixteen modern
exchanges prove the early anchor-floor refusal preserves the anchor and prepared
run; the first run advances exactly 1,200 ticks without overshoot. Its second
reviewed run is refused as stale before unpause, with the precise cause unresolved.
No bed appears in either the complete native roster or independent Lua (538 items
before, 532 after); the retained original order has one remaining and its goal is
false. The positive nonempty item and late changed-source probes remain unachieved.
Six actual-file offline exchanges preserve all five custody hashes and withhold
current goal/items. Parent replay checks all 105 exports and fresh host cleanup
confirms owned processes stopped, port closed, five plugins restored and both
30-file saves exact. D did not complete `.5.2`; the later E proof above does.
The earlier C result below is historical;
neither operator setup nor scoped safety proof establishes fortress competence.

Fresh review on November 1 found failed-call invalidation omissions in R25;
the positive trial below did not exercise those paths. Current source withdraws
item rows after failed Observe/Query or facade publication checks, and copied
error/close state exposes no old item rows. Five regressions fail against the
old production code. The repaired source passes clean private R26 qualification
and its separate verifier, each with 5,137 debug and 5,137 release cases and
zero failed or ignored across 123 targets per profile. Parent review checks
the exact source, official receipt and development executable. R28 subsequently
qualifies the anchor guard, and D exercises the early refusal and actual offline
recovery. Revocation of a nonempty live item view still lacks a successful current
trial; original-goal proof and native authority have not changed.

On exact qualified private R25 `bc5bcdfe28a895f9334fd8a116270be36650ee3a`,
Terra uses the actual development executable
`72f29e709a2fe40219e571390070e6e2918e6c19fc7cc52d6c3193a31ea4ba36`
for thirteen modern public exchanges. One reviewed bed insertion and one
reviewed 1,200-tick/30-second run advance 1,031 ticks without overshoot. Fresh
Wait and same-anchor Query expose BED 540 within a complete 535-item roster,
with subtype -1, numeric material (420, 163) and stack 1. Independent native
replay verifies that tuple; Lua separately corroborates cherry wood, without
independently supplying subtype or stack. The public view correctly leaves
material classification, eligibility and production attribution unknown.

The order has disappeared, so its original goal remains Unknown and semantic
completion is unproved. Operator workshop and manager setup are prerequisites,
not MCP construction or autonomous-fortress proof. Both the earlier B and new C
custody reopen offline without changing their five files or exposing stale item
rows; historical identities survive while current goal is unavailable. Raw
captures, independent replay and fresh cleanup checks are recorded in
`target/bang-for-buck-20261031/production-c-parent-review.json` and the
reality-check report. This is bounded development evidence, not native R1,
live-campaign qualification or compatibility/runtime admission.
