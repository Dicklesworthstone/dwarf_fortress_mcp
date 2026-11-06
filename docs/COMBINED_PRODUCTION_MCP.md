# One labor and finite-order plan through MCP

`dwarf-fortress-mcp serve-gameplay-dev labor-order` connects the real
`live_production::Executor` and durable mutation dispatcher to the eleven-tool
MCP surface. One session owns one complete original two-step plan: configure
one labor for selected citizens, then insert a dependent finite furniture order.
The source is explicitly unadmitted development work. Current executed checks,
qualification and real-game evidence belong in `IMPLEMENTATION_STATUS.md`.
The private custody implementation supports Linux x86_64/aarch64; unsupported
platforms refuse private-journal opening.

Owner: `df-dfhack-bridge-plane-c-pic.4.1`. The broader dispatcher and recovery
tasks `.4` and `.5` remain open. This host does not supply a complete fortress
controller or material/workshop planning. Separately configured bounded clock
runs can advance an original receipt-linked production order or finish an already
observed inventory stability interval after the order retires. Wait monitors the
original order's completion by default, or an explicitly selected inventory
objective. A separate fresh observation of that goal and current labor can
establish the complete original parent goal at its exact reported anchor.

## Operator configuration

Set all three opt-ins to exactly `1`:

- `DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION`
- `DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE`
- `DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS`

Supply the existing `DFMCP_SEMANTIC_WORKFORCE_CONFIG` and
`DFMCP_SEMANTIC_WORK_ORDER_CONFIG` described in
[the workforce guide](SEMANTIC_WORKFORCE_MCP.md) and
[the work-order guide](SEMANTIC_WORK_ORDER_MCP.md). Their original single-goal
`recovery` objects and workforce `clock` remain forbidden in this host. The order
`clock` and `inventory_clock` are optional; each must exactly match its separate
combined-parent configuration described below. Each clock's independent authority
is never inferred from permission to assign labor, insert orders or control the
other clock.
Both must use the same numeric loopback endpoint, initialization mode and exact
citizen world/site/DF/DFHack/bridge-version/generation pins. Native family
generations remain independently pinned; they need not equal each other.

`DFMCP_COMBINED_PRODUCTION_CONFIG` is a closed JSON object with schema
`dfmcp.combined-production-development/1`, an absolute normalized `directory`,
Boolean `initialize`, and optional `recovery`, `clock` and `inventory_clock`. All three custody directories
must be separate and nonnested. Existing private-file owners require real
owner-private directories and regular no-follow files with canonical modes;
the host neither repairs nor silently replaces custody.

Only the following additional family environment names are accepted:

- `DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR`
- `DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN`
- `DFMCP_SEMANTIC_WORKFORCE_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION`
- `DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK`
- `DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK`
- `DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN`

Each pair of clock environment names is accepted only when that clock is
explicitly configured in the parent and child. Dropping configuration cannot
hide an existing private clock journal.

Authority flags are absent or exactly `1`. Initializing in Control requires
both write flags. Opening authority can later be reduced; it cannot be
increased within that session. All three exact configuration strings are sealed
at opening and rechecked at native boundaries. A changed string, revoked opt-in,
other `DFMCP_` variable or production launch/admission state refuses execution.
Configuration, paths, credentials, pins and native methods are never MCP inputs.

## Public sequence

Use the exact names returned by `tools/list`. The logical operations below
retain the frozen eleven-tool waist; this profile's wire names are underscores.

1. Open `mode="control"`. Offline and Reconcile modes do not gain write authority.
2. Observe the paused canonical citizen roster. Retain its `observation.seal`
   and canonical citizen `entity_id` values; native unit IDs are not substitutes.
   Current source also returns `observation.workforce_choices`, described below.
   The complete roster is bounded at 200 citizens, acquired in native pages of
   at most 128. Oversized or inconsistent captures refuse as a whole; the
   selected workforce domain remains bounded at 32 citizens.
3. Plan a closed JSON request containing `expected_observation`, `units`,
   `labor`, `enabled`, `recipe`, `amount` and `deadline_tick`. Optional
   `poll_interval_ticks` and `stable_samples` default to 10 and 1. An optional
   explicit `objective` is described below. Select
   1–32 citizens, an existing single-labor detail supported by workforce/1.17,
   and 1–100 units of `wooden_bed`, `wooden_door`, `wooden_table` or
   `wooden_chair`. The future deadline is at most one game year away and must
   contain the requested cadence/stability span. Compilation archives one
   complete original plan; it does not prepare or dispatch either native effect.
4. Plan again with the closed JSON object containing `kind="step"` and the exact
   `original_plan_digest`. This prepares the next eligible original step.
5. Review that response and immediately Commit its exact `paired_seal` with
   `confirmed=true`. Native preparations expire; an extra Explain call is
   unnecessary before confirmation. Explain remains available afterward.
6. Repeat steps 4–5 for the dependent order only after the first step has a
   verified historical labor receipt. The actual original StepIds, keys, anchor,
   deadline and whole plan digest are retained throughout.

Wait performs one original-key reconciliation. Once both original effects are
settled, it can acquire one qualified sample for the original order-completion or
explicit inventory objective, bracketed by matching current selected-labor captures. It never
dispatches, sleeps, polls indefinitely or advances game time. An uncertain effect prohibits a
replacement key or a fresh original plan. Cancel with `scope="step"` retires
a proven preparation; it does not undo an Applied effect. `scope="session"`
releases local ownership while preserving custody and makes no native
cancellation claim. Checkpoint and Restore return explicit capability refusals.

If clocks are configured, Wait first reconciles their retained original run keys.
Either unresolved run blocks paused goal acquisition. Explicit `scope="run"` or
`scope="inventory_run"` cancellation retires that owner's prepared run or requests
its native safety pause;
it does not change the labor assignment or remove the manager order.

## Advance the original production order with a reviewed clock

Clock control uses the existing `order-run/1.14` native owner and a separate
private `clock.journal` under the order custody directory. It applies only after
both original effects settle and the original request explicitly selected an
inventory objective. The complete parent archive, selected StepId and native
key, paired review, Created receipt, allocation identity and predecessor custody
remain bound to every run. The original deadline, cadence and sampled stability
are unchanged.

To enable it, put the same closed object in the parent combined configuration
and the order child configuration:

```json
{"clock":{"generation":77}}
```

Here `77` is only an example; use the exact operator-pinned native clock
generation. Set `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK=1` and supply its separate
`DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN`. Open Control mode with an explicit
`max_game_ticks` between 1 and 1200. The default is zero. Neither configuring nor
opening a clock advances game time, and a reconcile/offline owner cannot become
a control owner within its session.

After the original order is created, obtain `clock.original_paired_seal` from
the current response. Prepare through the existing Plan tool:

```json
{
  "kind": "run",
  "original_paired_seal": "<clock.original_paired_seal>",
  "game_ticks": 5,
  "wall_millis": 1000
}
```

Preparation acquires fresh joined labor/inventory evidence and a paused native
order capture. It refuses a completed inventory obligation, a retired/unlinked
order, expired original time, unresolved clock, replaced custody or missing
independent clock authority. Both native prepare and commit recheck the exact
original custody after the durable clock marker. False stock can advance toward
the requested goods; true stock can advance unfinished sampled stability while
the original native order remains eligible.

Review `clock.active_run`, then Commit with `scope="run"`, its `review_seal` in
`paired_seal`, and `confirmed=true`. That confirms only this bounded run. The
clock has at most 32 retained runs; each is limited by the session budget and
the remaining original deadline. A native stopping receipt is historical clock
evidence. Commit withdraws current goal proof, and a subsequent Wait must obtain
fresh coherent labor and inventory evidence before reporting success.

Lost replies retain the original run key. Wait queries that key without another
unpause. While the native owner may still be running, goal reads and new runs
remain blocked. A failed response or restart withdraws the process-local clock
review. Inspecting a historical Prepared record does not restore confirmation
authority; an explicit fresh preparation is required. `scope="run"` cancellation
uses current Query and clock authority and remains available after the original
goal deadline for the retained native safety pause.

For recovery, retain the returned parent `recovery.clock_journal_id`. Copy that
same identity into `clock.journal_id` in both parent and order child configurations,
set all `initialize` values to false, and supply the complete original parent
recovery reference. Missing, omitted or replaced clock custody refuses; recovery
never creates a replacement journal. Offline opening and inspection perform no
native contact and restore no current goal proof or local clock review.

The order clock requires the original native order to remain eligible. The
independent inventory clock below handles unfinished sampled stability after
normal engine retirement.

## Finish original inventory stability after order retirement

The separate `run/1.13` owner can advance an already True original inventory goal
through its remaining stability interval. It retains the complete combined plan,
selected StepId/key, verified Created receipt and predecessor custody. It does
not require a present manager-order row or reinterpret inventory as output
attributed to that order. Both original effects must be settled, and fresh
qualified evidence must show the requested labor settings and inventory minimum
are True. Fulfilled, failed, cancelled or expired original obligations refuse
new runs; the original deadline and cadence are never renewed.

Configure the same independent source incarnation in the parent and order child:

```json
{"inventory_clock":{"generation":78}}
```

Use the actual operator-pinned generation, set
`DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK=1`, and provide
`DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN`. Open Control with an explicit
`max_game_ticks` between 1 and 1200. The native Run/1.13 protocol identifies a
source incarnation, not a named fortress lineage. Its native authority is issued
independently; canonical fortress grants are never converted into native scope.

First call Wait to acquire the current original labor-and-inventory sample.
When both are True and the selected stability requirement is unfinished, Plan:

```json
{
  "kind": "inventory_run",
  "original_paired_seal": "<inventory_clock.original_paired_seal>",
  "game_ticks": 5,
  "wall_millis": 1000
}
```

The native observation must match that sample's tick. Before each native prepare
or commit, the host rechecks exact original custody and obtains a complete fresh
joined labor/inventory sample **after** the durable clock marker. Changed labor,
insufficient or unqualified inventory, a changed tick, revoked authority or
failed publication prevents the native effect. Re-reviewing an existing prepared
run also refreshes evidence; reading its journal alone never restores a review.

Review `inventory_clock.active_run`. Commit with `scope="inventory_run"`, that
run's `review_seal` in `paired_seal`, and `confirmed=true`. Each bounded run retains
the original deadline and has one unpause attempt. At most 64 runs are retained
by this owner. Commit immediately withdraws current selected and parent proof;
Wait must obtain fresh evidence after the run has settled before reporting
stability or complete-parent success. A same-tick sample does not advance the
stability count.

Both clock owners remain visible. A pending or unresolved run in either prevents
starting the other, and either unresolved owner blocks paused goal sampling.
Lost replies retain the original key for Wait reconciliation without another
unpause. Cancel with `scope="inventory_run"` requests that owner's safety pause
or retires its preparation; it needs Query and its independent clock authority,
including after the original goal deadline. It does not require current goal
truth or permission to plan another effect.

Retain `recovery.inventory_clock_journal_id` along with the complete original
recovery reference. On reopening, copy that exact ID into
`inventory_clock.journal_id` in both parent and child configurations and set all
`initialize` values to false. The existing order-child `inventory-clock.journal`
must be present and match. Offline opening reads retained custody without native
contact, current proof or restored confirmation. Missing, replaced or omitted
inventory clock custody refuses instead of creating a replacement.

The newer `.4.2` source exposes pre-plan advisory workforce choices; its current
test and live evidence remains explicit in `IMPLEMENTATION_STATUS.md`. BB's
earlier real-game trial used a separate operator scout. That scout does not
count as the MCP agent discovering a valid intent.

Before an original plan exists, Observe reads workforce/1.17 for at most
32 canonical citizens. Its default remains the first 32 in numeric order.
The optional `request` argument is a closed JSON string containing `units`,
`option_offset`, or both. `units` selects 1–32 distinct canonical citizen IDs
from `observation.citizens`, encoded as nonzero decimal strings. IDs are
normalized numerically; duplicates, noncanonical spellings, unknown fields
and absent citizens are refused. Explicit selection makes citizens beyond the
default first 32 accessible without widening the native selection limit.

This additionally needs the operator's workforce read credential. It checks
the same paused tick, world, site, source generation and exact selected native
identities against the fresh complete canonical capsule.
`workforce_choices` carries its canonical anchor/seal, native capture witness,
source version/generation/sequence, automatic-professions state, labor schema,
selected citizen identity/eligibility and bounded `options`.

Each option identifies a labor and observed single-labor selected-only detail,
current `member_ids` and `labor_enabled_ids`, and advisory `enable_candidates`
and `disable_candidates`. Copy a canonical ID from the desired candidate list
to the original request's `units`, and the option's `labor` to its `labor` field.
Only eligible citizens needing the corresponding membership/labor change are
suggested. Ambiguous and multi-labor details are omitted; disabling is omitted
when another detail can keep granting that labor. Automatic professions must be
enabled. At most eight options appear, ordered by labor key. `option_offset`
defaults to zero and selects a window after all candidate checks and sorting.
It is bounded at 128 and must not exceed the current eligible option count.
An offset equal to that count returns an explicitly empty final page; a larger
offset is an error. Coverage states the available, returned and omitted option
counts, including omissions before and after the requested window.

For example, using canonical citizen IDs obtained from an earlier observation,
the existing Observe tool accepts:

```json
{
  "session_id": "<the current combined session ID>",
  "request": "{\"units\":[\"43\",\"252\"],\"option_offset\":8}"
}
```

The response records the normalized `selection_request`. When another window
exists, `next_observation_request` contains its offset and the exact resolved
canonical citizens. The Agent Turn also exposes that read as a structured
`fortress_observe` recommendation. Keeping those explicit identities prevents
a changed roster from silently replacing citizens with a new default first 32.

Every window acquires a new full canonical observation and native capture.
These are independent fresh observations: their pages cannot be combined into
one canonical snapshot. Use the latest successful response's observation seal
when planning. The default attachment retains its original version-one witness;
explicit selections or nonzero offsets use a version-two witness that also
binds normalized selectors and offset. `witness_version` makes that distinction
explicit. Neither witness grants mutation authority.

Coverage distinguishes the complete native capture from its partial public
projection and states unrepresented details and omitted citizens/options.
Empty results never establish absence outside the covered selection. With no
citizens, no native request occurs, the native work-detail domain and available
option count remain unknown, and no later-page request is offered.

These are observed attachments and derived candidates, not canonical mutation
preconditions or authority. Other recomputed labor effects remain unknown.
The original compiler still binds canonical identity, and existing native
preparation and commit independently re-observe and validate their full witness.
The citizen observation seal remains a citizen identity seal; the additional
choice witness separately binds that seal and native capture. Choices are
process-local, disappear after original planning, and are not restored from
historical custody. Query inspects them without acquiring another native read.
Any failed call bound to this session withdraws choices; another Observe is
needed to obtain them.
Within the session, consumed authorized observation ticks remain an explicit monotonic floor even
when the combined read fails, so retry cannot publish an older canonical tick.

The returned `frontier.all_applied` means both historical native effects were
proved at their respective boundaries. It does not mean current labor, produced
goods, workshop/material feasibility or the original terminal predicate is true.
`current_state_proven` remains false because the host does not prove the whole
world. `original_goal_proven` requires the separate qualified current-parent
sample described below; native receipts alone always leave it false.
After an original exists, default Observe reports retained original custody.
A nondefault selection or page request is refused before native contact; the
host cannot acquire a replacement canonical world or rebase the original
anchor through these selectors.

## Monitor the original finite order's completion

When `objective` is omitted or null, Wait now monitors the completion predicate
already sealed into the original plan. The request identity, original steps,
idempotency keys and terminal predicate remain unchanged. After both native
effects settle, Wait starts the same durable `goal.history` machinery used by
explicit inventory objectives and samples the exact receipt-linked manager
order with the existing qualified progress and operations sources.

The `selected_order_goal` object has `kind="retained_order_completion"` and the
original `step_id`. Its canonical `order_entity_id` is derived from the original
order key; `required_order_state` exposes the exact required fields:

- `amount_total` equals the original requested finite amount.
- `amount_remaining` equals zero.
- `validated` is true and `active` is false.

All four fields must be qualified at the same sample. A still-outstanding order
makes the selected goal false. A missing or retired order leaves completion
unknown: disappearance, a Created receipt, and existing inventory cannot prove
this predicate. `disappearance_proves_completion` is always false. This goal
proves the observed retained order state, without attributing produced goods.

`current_truth`, `anchor`, `proven_at_anchor`, `first_satisfied_anchor` and
`history` have the same current-versus-historical meaning described below for
inventory goals. The original deadline, cadence and sampled stability apply;
repeated reads at one tick cannot complete advancing stability. Agent Turn
recommends bounded Wait calls while this requirement remains unfinished.
Current game-time progression must be managed separately for this default
objective; the optional reviewed clocks retain their explicit inventory-objective
requirement.

The selected object alone keeps `original_goal_proven=false`. A fresh current
labor bracket can establish the whole parent under
`current_parent_goal.scope="complete_original_labor_and_order_goal"`, provided
the selected temporal requirement is also fulfilled. The other selected-goal
field is null: explicit inventory requests use `selected_inventory_goal`, while
default requests use `selected_order_goal`. Query, Explain, failure withdrawal
and offline recovery retain this distinction.

## Select and monitor an original inventory objective

Include an `objective` in the first closed request to ask whether the original
order step's requested inventory minimum is present. For example:

```json
{
  "expected_observation": "<latest complete citizen observation seal>",
  "units": ["<selected canonical citizen ID>"],
  "labor": "CARPENTER",
  "enabled": true,
  "recipe": "wooden_bed",
  "amount": 1,
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "objective": {
    "kind": "observed_inventory_minimum",
    "item_type": "BED",
    "minimum_stack_units": 1
  }
}
```

Use the observed supported labor key, exact canonical IDs and a future deadline
for the actual fortress. BED/DOOR/TABLE/CHAIR must match the chosen wooden
furniture recipe, and `minimum_stack_units` must be 1–1,000,000. The minimum is
independent of the finite order amount. Unknown fields and mismatched types
refuse. This request has a distinct version-two identity; omitted or null
objectives preserve the previous request identity and predicates. An existing
plan cannot acquire or change an objective after preparation or retirement.

Compilation retains both original steps, their exact keys and dependency. The
order step's original postconditions and obligation use the inventory predicate.
The whole parent's terminal still contains both labor and inventory requirements.
Wait starts `goal.history` only after both original native effects are settled
and the exact insertion is verified. It reads complete qualified operations
inventory with the original progress source, using the original deadline,
cadence and stable-sample requirement. It does not derive a replacement plan or
an item subset from presentation examples.

The response's `selected_inventory_goal` separates current and historical proof:

| Field | Meaning |
|---|---|
| `scope`, `step_id` | The actual original order step within the complete parent. |
| `current_truth`, `anchor` | The selected inventory predicate at a complete published sample. |
| `proven_at_anchor` | That selected predicate satisfies its original temporal requirement at the sample. |
| `parent_terminal_truth` | Evaluation using only the selected inventory projection; the accompanying `parent_terminal_evidence_scope` states that current labor is not supplied here. |
| `first_satisfied_anchor` | First qualified selected achievement, retained across later consumption, failure and recovery. |
| `history` | Whether exact selected history exists, its retained size and whether it is fenced. |

The selected object keeps its own `original_goal_proven` false. Qualified inventory can be
true after the order retires even while receipt linkage or physical work is
unknown. Preexisting and unrelated matching stock counts. Material, usability,
new production and order attribution remain unproved. Inventory truth cannot
reconcile uncertain insertion, release another mutation or authorize redispatch.

Agent Turn recommends a bounded Wait while the selected temporal requirement is
unfinished. Additional stability
samples need separately managed game-time progress and coherent paused reads;
repeated reads at one tick cannot supply advancing samples. A failed call clears
current proof and unfinished continuity without extra I/O. The next admitted
sample persists that interruption; historical first achievement remains intact.

## Prove the complete original parent

Each fresh goal Wait reads the selected citizens' current workforce state,
publishes the original selected order-completion or inventory sample, and reads the same workforce
state again. The workforce captures must agree exactly, including their source
incarnation, sequence, original citizen identities, labor schema and paused tick.
The selected goal and labor observations must also share the same fortress, restore
epoch and tick. Each source receives its own independently checked operator
policy. Historical Applied receipts establish custody and identity only.

`current_parent_goal` is a separate object containing `current_truth`,
`original_goal_proven`, a combined `anchor`, the original `selected_anchor`, and
the composite and workforce source digests. Its truth evaluates the unchanged
complete parent terminal. Its proof additionally requires the selected goal
obligation's original cadence and stability. The top-level `original_goal_proven`
and Agent Turn briefing reflect that same scoped result. The combined anchor has
its own state hash; the selected goal anchor remains distinct and visible.

The object's bounded `labor` array identifies the original selected citizens in
canonical ID order. Each row names `unit_id`, `labor`, `requested_enabled`,
`observed_enabled` and `current_truth`. The observed Boolean is exposed only when
the independent workforce evidence policy qualifies it; otherwise it is null and
the comparison is unknown. These rows share the parent's anchor and workforce
source digest. They do not claim complete-roster coverage or describe unselected
citizens. Agent Turn summarizes comparison counts and at most eight false or
unknown labor blockers, with an explicit omitted count and the full bounded
array available in `active_work.current_parent_goal`.

Current labor reversal makes the parent false even when the selected goal remains proved.
Missing qualified labor leaves it unknown. A changed capture during the bracket,
lost custody, revoked source policy or failed response publication withdraws both
current parent proof and the selected current sample. A proved parent stops
completion polling. If the selected goal is proved but current labor contradicts the
original goal, guidance exposes the contradiction without proposing another
dispatch under the already-used original keys.

Parent evidence is process-local and valid at its reported anchor. Query and
Explain inspect that retained sample without new native reads. Reopening starts
with `current_parent_goal=null` and `original_goal_proven=false`, even when durable
selected goal history records a first achievement. A fresh coherent Wait is required
to obtain parent proof again. No parent achievement is reconstructed from the
selected history. Completion still makes no causal claim that this order
produced any counted items, and no claim about material, usability or workshop
feasibility.

`parent_goal_observation_fenced=true` means a source/identity regression prevents
another parent acquisition in this process. It remains visible in compact error
and recovery output. Retain the original custody, inspect the failure and resolve
the source lineage before reopening; ordinary repeated Wait calls cannot clear
the fence.

## Recovery and failure

Retain all native journals and semantic associations in both child directories,
plus parent `original.plan`, `original.citizens`, `execution.journal` and
`goal.history` if selected goal monitoring has started.
Persist the latest complete `recovery` object outside those files. It includes
the original plan/object/citizen digests and anchor, and frontier journal/head/
authority-tick-floor. The additive Boolean `goal_history_present` records
selected history; older recovery objects default to false. Preserve its latest
value even when opening or publication fails. On reopen set all three configurations to
`initialize=false`, supply that parent object, and retain the original child
native journal identities required by their existing configurations.

Offline opens retain historical custody without contacting DFHack. Reconcile
mode can consult the original keys under Query authority. Neither reconstructs
a process-local review or native dispatch permit. A resumed Control owner must
prepare an eligible Pending step; it cannot reuse an old paired seal. A retained
Prepared step without its process-local review can be reconciled or cancelled,
but this host cannot renew its commit permit. Lost custody,
changed source or an inconsistent native/frontier record refuses rather than
inventing success or replacing a journal.
An uncertain Offline response recommends `fortress_cancel` with
`scope="session"`: retain its recovery object, close that local owner, then reopen
in Reconcile mode with the operator's native configuration. Offline Wait only
inspects custody and cannot resolve live uncertainty. Closing does not cancel
or settle the native effect.

The parent archive and citizen identity are verified before and after each
child evidence refresh, including the order's post-dispatch-marker refresh.
Before first-step labor preparation, a separately prepaid canonical citizen
read primes the resolver under current Query/Observe authority and the same
shrinking deadline. It retains the exact original anchor and roster; it neither
revives a cached routing generation nor replaces the later native-capture checks.
The existing executor also verifies the predecessor's exact historical labor
receipt and native/semantic custody before permitting the dependent order.
Partial initial publication is retained and prevents another plan in that
owner; it is not repaired automatically. No native preparation or mutation has
occurred if initial publication has not reached the executor.

Errors withdraw local reviews. Recovery references preserve the newest known
frontier head and authority floor, but any copied frontier is explicitly stale.
Failure to publish a successful response does not erase a possible effect.
Reconcile the original key before deciding what is safe next.
Successful responses receive a final owned cancellation and wall-deadline check
after rendering. A failed final check discards that success, withdraws the
review and returns a failure with retained recovery; it does not undo a possible
native effect.
Before the original executor and frontier exist, lifecycle refusals cannot
dispatch a native effect. They report no dispatch and do not create a fictional
uncertain key requiring reconciliation. The caller can inspect and acquire its
canonical observation before compiling the original plan.
Query verifies the parent archive/identity and durable frontier. Native family
custody and effects are rechecked through their owners at preparation, commit
and reconciliation boundaries; Query is not a new native effect observation. Selected history replays under
newly reissued exact source policy and retained predecessor custody. Offline
replay retains first achievement and terminal history but restores neither
current inventory/parent proof nor unfinished stability. Recovery never renews the deadline or
reconstructs a native commit permit.

## Bounds and evidence

Each call admits at most 60 seconds, two actions, zero game ticks and 32 GiB of
logical byte-work, defaulting to five seconds and 8 GiB. This accounts for
repeated bounded custody validation; it is not an eager memory allocation or
frame size. One shrinking deadline covers the nested work. Existing-history
replay can use the remaining configured allowance up to the existing 32 GiB
ceiling, so a larger caller allowance is not trapped behind the empty-history
initialization bound. A retried history opening through Wait publishes recovered
history first; the following Wait acquires a fresh sample under its own budget.
History size/event and source limits still apply; a large allowance is not a
promise that every possible sample history will fit. The host reserves
128 KiB for response publication and an additional 512 MiB for opening
inspection. Complete escaped response text is limited to 60,000 bytes; overflow
withdraws the local review and returns bounded recovery/uncertainty instead of
truncating JSON. Smaller output budgets refuse before I/O.

The machine description is
[`live_production_executor_v1.json`](../architecture/live_production_executor_v1.json),
under `development_mcp_host`. Standard workspace checks compile and exercise the
host. Injected-source and loopback-peer tests are component evidence, even when
they run the actual executable and real private stores. A source-bound actual
DFHack/game trial, clean full-source qualification and production admission
are separate evidence requirements. The registry and production runner map
remain unchanged.

### Selected workforce discovery increment (2026-11-05)

Source now includes canonical citizen selection, option paging, versioned
projection witnesses and fresh-read recommendations. The choices module has
13 component regressions; the actual CLI/stdio suite contains 14 scenarios,
including a two-citizen assignment using the 33rd roster citizen and a labor
option beyond the first eight, followed by its original dependent order.
Malformed selectors, unavailable identities, later-page range checks and
post-plan selection refusal retain the existing no-redispatch behavior.

These new cases and their integration were independently reviewed in source.
They were not executed: this editing session had no usable execution environment.
Compilation, rustfmt, focused and full tests, verification and qualification
remain pending for these bytes. Existing recorded game trials do not qualify
this increment, and bead `df-dfhack-bridge-plane-c-pic.4.2` remains in progress.
