# Native resource project MCP

## Scope and evidence posture

`dfmcp-native-resource-project-dev-server` owns an optional original Manager
1.23 appointment, an optional original Tree harvest, one original Carpenter
Workshop construction and either one fixed wooden-furniture stock target or a
fixed portfolio of one to four targets. It composes
the actual Manager, Tree, Workshop and combined-production runtimes inside one
modern MCP session. The public surface
retains the eleven existing tool names.

This is explicitly **unadmitted development source**. It adds no bridge method,
production runner, registry entry, canonical cross-family world adapter, or
automatic native effect. Every designation, construction, labor assignment,
finite order and clock interval still requires its child's own current review
and confirmation.

Set `harvest_tree: true` to start with the existing one-tree controller and
transfer its freshly verified supply to the Workshop Tree-material importer.
The default still starts with Workshop and can use an already completed Tree
donor. Set `appoint_manager: true` to appoint the configured eligible candidate
first, prove the Manager 1.23 production-responsibility cache, and retain that
exact original as the later production prerequisite. Without this optional
stage, configured external Manager and input-supply policies retain their
existing independent checks.

Owning work: `df-dfhack-bridge-plane-c-pic.4`,
`dfmcp-reality-wiring-proof-4bf.14` and `df-native-stock-target-3m9`. These remain
in progress. This increment
implements a concrete resource consumer; it does not complete generic
mixed-family planning or the full live acceptance campaign.

## What the parent owns

The parent fixes the original goal, host policy identities, custody directories,
stage order and absolute game-tick deadline before opening a child. Its journal
and independent acknowledgement retain:

`WorkshopOpening → Workshop → Transition → ProductionOpening → Production`

With `harvest_tree: true`, the original prefix is:

`TreeOpening → Tree → TreeTransition → WorkshopOpening`

With `appoint_manager: true`, the original prefix is:

`ManagerOpening → Manager → ManagerTransition → TreeOpening or WorkshopOpening`

The original `harvest_tree` selection fixes that successor. Manager appointment
does not select a tree, build a workshop or publish a production order. Each
later child still requires its own fresh review and explicit effect confirmation.

The original first child is part of the journal identity. Recovery cannot add,
remove, repeat or substitute either optional prefix. Older Workshop-first and
Tree-first journals
retain their exact serialized header, checkpoint and frame bytes.

`Cancelled` records its prior stage and retains all existing child references.
It cannot transition forward again.

The parent writes and acknowledges an opening marker before initializing each
child. It acknowledges each transition before releasing the donor writer.
Workshop opens the exact Tree donor Offline, and production similarly opens
the exact Workshop donor Offline using their existing importers. The parent
does not bypass any child's
journal, independently acknowledged origin, native permissions, source checks,
lease, current review or effect confirmation.

A marker left by an incomplete open is **not initialization permission**. Reopen
existing custody only. The controller does not recreate missing files, choose a
replacement key, silently repair a journal, or dispatch an effect from replay.

## Inherited game-tick authority

Every project-owned Tree and Workshop child receives the greatest game tick
already consumed by the parent before its first native read. Initialization
rejects an older capture before creating the child's journal or acknowledgement.
The same floor is retained before subsequent child operations and handoffs.
Raising it withdraws old offers and preparations; it does not rewrite the tick
or witness of an actual native observation. A new stage therefore cannot reset
expiry checks or recover authority from an older source response.

## Start the profile

Select the fixed profile with the standard executable:

```text
dwarf-fortress-mcp serve-gameplay-dev native-resource-project
```

The dedicated `dfmcp-native-resource-project-dev-server` binary selects the
same owner. Supply the operator configuration below before startup. The
separate `workshop-production` profile retains its broader staged production
requests and externally held parent checkpoint; this profile adds a fixed
root goal, absolute deadline and independently acknowledged parent journal.

## Operator configuration

Configuration, source endpoints, credentials, private paths, footprints and
native authority are host inputs. MCP requests cannot select them.

The dedicated profile requires:

| Setting | Purpose |
| --- | --- |
| `DFMCP_ALLOW_UNADMITTED_NATIVE_RESOURCE_PROJECT=1` | Explicit parent development opt-in |
| `DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG` | Closed parent configuration described below |
| `DFMCP_WORKSHOP_MCP_CONFIG` | Workshop source, footprint, material/template, authority and custody configuration |
| `DFMCP_COMBINED_PRODUCTION_CONFIG` | Existing production configuration without a caller-selected `workshop_prerequisite` |
| `DFMCP_SEMANTIC_WORKFORCE_CONFIG` | Existing workforce source and custody configuration |
| `DFMCP_SEMANTIC_WORK_ORDER_CONFIG` | Existing finite-order source and custody configuration |

All existing independent family opt-ins and permission flags remain required.
Configured Tree ownership/material, Tree clock, Workshop clock, production clock, inventory clock and
Manager credentials are accepted only for the corresponding configured feature.
Scalar and portfolio roots accept independently configured production and
inventory clocks. Each original order retains its own custody and current
review. Optional Tree and Workshop clocks keep their separate authority.
Game-time allowance still defaults to zero.
Production launch/admission variables and unrelated `DFMCP_` profiles refuse.

The parent and each child retain exact byte hashes of their live configuration
strings. A changed configuration during a session refuses further operation.
On restart, only the explicitly recognized initialization/recovery selectors
are omitted from the stable policy identities; their actual journal identities,
acknowledgements, original archive references and floors are bound separately
to the retained parent checkpoint.

The parent, optional Manager and Tree, Workshop, production, workforce and order directories must be
distinct canonical absolute paths, without ancestor/descendant overlap.
Existing private-file ownership, exact modes, no-follow checks, exclusive locks,
file synchronization and directory synchronization remain in force.

A fresh parent configuration has this shape:

```json
{
  "schema": "dfmcp.native-resource-project-development/1",
  "directory": "/private/fortress/resource-project",
  "initialize": true,
  "goal": {
    "recipe": "wooden_bed",
    "minimum_stack_units": 5,
    "max_order_amount": 20,
    "deadline_tick": 13000,
    "poll_interval_ticks": 10,
    "stable_samples": 2,
    "assign_labor": false
  }
}
```

The tick above is illustrative. Choose an absolute future deadline within the
original Workshop host authority and the production child's existing goal
horizon. A Tree-first project also fixes this same deadline inside Tree host
authority and the Tree original's 12,000-tick horizon. If a Workshop clock is configured, its fixed deadline must equal the
parent deadline. Future review, restart, handoff or source progress cannot
extend it. Query, original reconciliation and separately authorized clock safety
stops retain the original host authority expiry, so the parent goal deadline
does not itself revoke those safety paths.

Supported recipes are `wooden_bed`, `wooden_door`, `wooden_table` and
`wooden_chair`. Each target is 1–1,000,000 observed type stack units; a single
finite order is bounded by `max_order_amount` in 1–100. Positive deficits beyond
that bound refuse instead of being silently truncated.

`assign_labor: false` uses already-enabled eligible Carpenter workers.
`assign_labor: true` adds an independently reviewed labor-enable step before the
dependent finite order or orders. This choice is fixed in the parent original. Worker IDs
come from the later production observation; labor remains `CARPENTER`.

The production template must not contain `workshop_prerequisite`: the parent
supplies the typed, independently verified original donor at the transition.
All three production configuration strings are validated before the first
child operation. Fresh initialization requires both existing production native family
authorities even when the selected later plan will not assign labor.

### Fixed portfolio goal

To retain several furniture minima in one project, replace the scalar `goal`
above with:

```json
{
  "targets": [
    {"recipe": "wooden_bed", "minimum_stack_units": 5, "max_order_amount": 20},
    {"recipe": "wooden_chair", "minimum_stack_units": 5, "max_order_amount": 20},
    {"recipe": "wooden_door", "minimum_stack_units": 1, "max_order_amount": 5},
    {"recipe": "wooden_table", "minimum_stack_units": 5, "max_order_amount": 20}
  ],
  "deadline_tick": 13000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "assign_labor": false
}
```

A portfolio contains one to four distinct supported recipes. Fresh operator
configuration sorts targets by recipe before retaining the original; duplicate
recipes refuse. Retained journals, intents and recovery references must already
have that exact canonical order. Do not combine `targets` with the scalar
`recipe`, `minimum_stack_units` or `max_order_amount` fields. Both forms are
closed, and existing scalar journals retain their original JSON bytes.

All targets share the same deadline, cadence, stability requirement and
labor-assignment choice. The complete vector remains mandatory even when the
initial census finds some targets already met. The compiler creates an order
only for a positive bounded deficit. Four orders plus one optional labor-enable
step fit the existing five-action ceiling; no per-target timing or extra action
authority is introduced.

Combined-production `clock` and `inventory_clock` remain independently optional.
When configured, each compiled original order receives its own fixed clock
custody; initially met minima do not create extra orders or clock targets.
The parent acknowledges every prospective clock identity before additional
clock files are created. All targets retain the same original deadline and
joint stock-proof requirement.

Existing-worker order StepIds start at zero; a portfolio with an original
labor-enable step starts its order StepIds at one. Use the StepIds published in
the child's `portfolio.orders`, not native manager-order IDs or custody-file
ordinals. Planning, confirming or cancelling a portfolio clock requires that
explicit StepId. Scalar clock actions reject it.

A configured owner reopened before its first original can publish a multi-order
portfolio under the same project goal and deadline. The publication derives the
exact identities of additional fixed clock slots before the parent ACK. After
that ACK succeeds, a separate first-publication path exclusively creates those
preannounced headers. Each configured family requires its independent opening
and current clock authority. Known additional paths must be absent before the
ACK, and a later collision fails publication without adopting existing bytes.

This applies only to a new original being published in the current controlled
call. Reopening a retained original still requires every acknowledged journal
to exist and match its exact identity; it cannot recreate missing or partial
custody. A one-order portfolio uses the retained first clock owner without
requiring authority to create additional owners. An all-met stock watch has no
original order or per-order clock target. The optional parent stock-watch clock
below owns separate custody and still requires independent clock authority.

### Optional Manager stage

Add `"appoint_manager": true` to the parent configuration and supply the existing
closed `DFMCP_MANAGER_V1_23_MCP_CONFIG` with schema
`dfmcp.manager-appointment-mcp-development/2`. Keep its exact candidate unit and
historical figure, source binding, map dimensions, authority, checkpoint and
private custody directory. Require `DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23=1`,
`DFMCP_MANAGER_V1_23_TOKEN` and `DFMCP_MANAGER_V1_23_ALLOW_APPOINT=1` for fresh
control. The later independent production guard also requires its existing
`DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN` credential. Legacy Manager 1.22
configuration and credentials do not select this stage.

Manager and Workshop must name the same fortress, software versions, endpoint
and dimensions; independent native generations remain distinct. Every selected
child policy and private directory is checked before the first child opens.
The root deadline must fit inside the Manager host authority and its existing
12,000-tick appointment horizon. It cannot be replaced with a new child deadline.

With this stage selected, omit `manager_prerequisite` from the combined
production template. The actual completed appointment supplies it later, along
with its consumed source checkpoint. Configuration preflight creates no donor,
placeholder original, acknowledgement or native authority. An external Manager
donor remains supported when `appoint_manager` is absent or false.

### Optional Tree stage

Add `"harvest_tree": true` to the parent configuration and supply the existing
closed `DFMCP_TREE_HARVEST_MCP_CONFIG`, including the fixed root, source binding,
visible/protected domain, authority and private directory. Require the existing
`DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21=1` opt-in, Tree token and designation
permission. The optional Tree clock retains its own configuration and
`DFMCP_ALLOW_UNADMITTED_TREE_CLOCK=1` opt-in. Shared Run credentials are accepted
only if a Tree or Workshop clock is selected; the other clock receives no
authority just because those credentials exist.

For this mode, Workshop uses the closed schema
`dfmcp.workshop-project-tree-material-mcp-development/1`. Keep the existing
Workshop source, footprint, safety, authority, clock and custody fields, and
omit `item_id` and `tree_material_source`. The template deliberately has no log
or donor acknowledgement until the parent transfers its actual original Tree.
`DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL=1` remains required. A standalone
Workshop owner cannot activate this unresolved template. The completed typed
handoff supplies the original donor; the importer independently reopens its
exact files and freshly qualifies available wood before construction planning.

All selected child configurations are validated before initializing Tree.
Recovery retains the same root, source policy and deadline. This mode harvests
one configured visible tree; it does not infer additional trees, assert that
one harvest is sufficient for every stock target, or change Manager policy.

## One-session workflow

### Optional first step: appoint the original Manager

With `appoint_manager: true`, opening retains the real Manager 1.23 owner.
Observe the configured candidate, then request `appoint_vacant_manager` through
`fortress_plan` using the exact current observation witness and the parent
`deadline_tick`. The closed request rejects changed deadlines, duplicate fields
and caller-selected candidates. Review the displayed appointment terms and
explicitly confirm the current seal through `fortress_commit` with `scope="goal"`.
Use Wait to reconcile the original receipt after a lost reply; do not create a
replacement appointment. Manager owns no clock action.

After both the original appointment and production cache are proved, request:

```json
{"kind":"advance_from_manager","expected_head":"<current parent head>"}
```

The transition uses the typed existing Manager exporter to verify the original
journal and independent acknowledgement and acquire a fresh paused cache-bearing
observation. It retains all consumed source floors before acknowledging
`ManagerTransition`. Only that acknowledgement permits releasing the original
writer. The next child's opening marker is acknowledged before initialization.
The frozen Manager checkpoint and handoff remain in every later parent frame.

The later production owner imports this exact Manager donor Offline, verifies
the complete original descriptor, and independently requalifies current Manager
readiness before planning and native production boundaries. The historical
appointment alone never proves current production readiness. A handoff itself
dispatches no native effect and does not advance game time.

The Agent Turn recommends only the current stage's legal next action. A current
Manager review includes the exact semantic terms and a named commitment to its
bounded native capsule. Stale, fenced, expired, stopped, uncertain or failed
publication views withhold confirmation. Later stages retain Manager history
without presenting its old review as a new approval.

### Optional first step: harvest the original Tree

For a Tree-first project, opening starts the real Tree owner. Observe its
current eligible offer and plan:

```json
{
  "kind": "fell_one_visible_tree",
  "expected_observation": "<published Tree witness>",
  "deadline_tick": 13000
}
```

The deadline must equal the parent original. Review and explicitly confirm its
designation seal with `scope: "goal"`. Reconcile the original with
`fortress_wait`. If the independently configured Tree clock is needed, plan
`run_tree` using its published target, review and confirm with `scope: "clock"`,
then reconcile that clock. Historical designation or a clock receipt is not
proof that the Tree supply predicate has been observed.

Once the original has timely historical supply evidence and its clock is
settled without preparation, uncertainty or advancement fence, request:

```json
{
  "kind": "advance_to_workshop",
  "expected_head": "<exact current parent head>"
}
```

The parent verifies the original Tree journal and independent ACK and obtains
a **fresh paused, same-incarnation supply read**. A cached achieved tick cannot
satisfy this transfer. Source tick, sequence, generation and item allocator
floors are retained even when a later check refuses. The parent acknowledges
the updated Tree checkpoint and immutable handoff, releases the Tree writer,
then acknowledges `WorkshopOpening` before constructing the real Workshop
owner. This performs no designation, construction or clock effect. Continue
with the normal Workshop workflow below using its freshly published Tree
material choices. Neither Tree supply nor completed Workshop alone proves the
final furniture stock goal.

### 1. Open and inspect the Workshop offer

Open with `mode: "control"` and the intended per-request budget. The default
mode is Offline. Opening creates custody and can acquire bounded native
observations; it never constructs, inserts an order or advances game time.

Call `fortress_observe`. For operator-pinned material, the existing closed
Workshop request is:

```json
{
  "kind": "carpenters",
  "expected_observation": "<published Workshop witness>",
  "material": "<published material_handle>"
}
```

For a configured completed Tree donor, use the existing
`carpenters_from_tree_supply` request and its exact published donor witness and
material handle. The importer freshly qualifies supply and the Workshop owner
independently verifies the selected log and footprint.

Pass the JSON as the `request` string to `fortress_plan`. Read the complete
child review. Confirm that exact seal with `fortress_commit`; then reconcile
and sample the original with `fortress_wait`. A placement receipt is not
completed construction.

### 2. Use only independently reviewed time

Time allowance defaults to zero. If the host configured a Workshop clock,
`kind: "run_workshop"` uses the existing original target ID, bounded game ticks
and wall milliseconds. Review and confirm that interval, then call
`fortress_wait(scope: "clock")` to reconcile it.

The same project deadline remains in force. A clock receipt establishes
historical clock-control custody, not Workshop completion. Pending, unresolved,
prepared or advancement-fenced Workshop clocks prevent the handoff.

### 3. Transfer the completed original

Use the latest parent head returned under
`retained_reference.parent_acknowledged.head`:

```json
{
  "kind": "advance_to_production",
  "expected_head": "<exact current parent head>"
}
```

Send it through `fortress_plan`. The transition:

1. Verifies the original Workshop journal and independent ACK.
2. Requires settled original clock custody without a live clock preparation.
3. Reads fresh progress for the exact original building and material.
4. Consumes source generation, tick and building allocator floors even on a
   later refusal, and checks the final native reply manifest.
5. Rechecks complete custody and original completion.
6. Retains updated Workshop recovery floors, then acknowledges `Transition`.
7. Releases the exclusive Workshop owner.
8. Acknowledges `ProductionOpening` before opening the actual production owner.
9. Independently imports the exact donor Offline and retains production custody.

This transition dispatches no construction, labor, order or clock effect. The
fresh handoff sample is evidence at its export anchor; production still freshly
qualifies the original donor before positive planning and protected native
order boundaries.

### Inspect production without leaving the project

In the Production stage, `fortress_query` accepts optional `order_step_id`
and `request` arguments. The selector is an original plan StepId, not a native
manager-order ID. Existing-worker portfolios start their order StepIds at zero;
portfolios with a separately reviewed labor-enable step start at one. Scalar
goals and non-production stages reject order selectors.

With no `request`, selecting an order inspects that exact original's local
custody, goal and independently configured clock state. It does not sample the
game or change the original portfolio. A pending or uncertain clock still
prevents an incompatible selection.

A closed `dfmcp.query/1` request uses the same bounded inspector as the combined
production profile. Before planning, call `fortress_observe` with
`include_production: true` in its existing observation request and omit an order
selector. After an original exists, `fortress_wait` must first publish the
selected order's current operations sample. Use the exact
`child.production_inspection_source.anchor` as `expected_anchor`; the parent
or citizen anchor cannot substitute for it.

For example, pass the child's published `diagnosis_request` or
`inventory_totals_request` as the Query `request` string, with the corresponding
original `order_step_id` for a portfolio. Supported queries include observed
production diagnoses, inventory totals, declared supply-chain models and
bounded entity, graph and event inspection. The existing child validates
coverage, freshness, source identity, current authority and query budgets.
Inspection performs no native read or mutation, does not advance time, and
cannot restore a source after a failed observation or restart. Stock totals
include ineligible items; declared supply-chain results are models and do not
prove native recipe feasibility, reservations or goal completion.

### 4. Plan the fixed stock target or portfolio

Call `fortress_observe` for current production worker choices. The existing
closed `stock_target` request names the published observation seal, selected
eligible worker IDs and all original goal terms:

```json
{
  "kind": "stock_target",
  "expected_observation": "<published production observation seal>",
  "units": ["<published eligible worker ID>"],
  "labor": "CARPENTER",
  "recipe": "wooden_bed",
  "minimum_stack_units": 5,
  "max_order_amount": 20,
  "deadline_tick": 13000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "assign_labor": false
}
```

Recipe, minimum, order bound, deadline, cadence, stability and assignment choice
must equal the original parent goal. A scalar root requires `stock_target`;
a portfolio root requires `stock_targets` with every original target. Neither
form can change into the other.

The child obtains its real qualified complete stock census and computes the
finite deficit. A met initial target returns `already_satisfied` with its exact
anchor, source identity, counters and assessment witness, without creating an
order. That result explicitly retains `fresh_for_current_call: false`,
`current_inventory_proven: false` and `stability_proven: false`. It is not a
current temporal completion proof or a causal production claim.

The child now retains a separate `no_work_stock_goal`. Use `fortress_wait`
(or default `fortress_observe`) for one fresh, independently qualified stock
sample. The planning census contributes zero stability. Only distinct suitably
spaced observed ticks can prove the original minimum within the fixed deadline.
When that requirement is met and the current count is still sufficient, the
typed child result can establish the parent's `original_goal_proven` at its
exact observation anchor without creating any order or labor effect.

Ordinary no-work sampling does not advance game time. An explicitly configured
parent stock-watch clock can advance one separately reviewed interval as
described below. Repeated Plan and Query calls cannot renew the deadline or
manufacture samples. Failed observations or
publication withdraw current proof; consumption makes current truth false.
Explicit goal/project cancellation withdraws the watch. Its sample history is
process-local and is never restored from the retained census or recovery JSON.
The parent goal and deadline remain immutable across recovery.

For the four-target configuration above, use this production request:

```json
{
  "kind": "stock_targets",
  "expected_observation": "<published production observation seal>",
  "units": ["<published eligible worker ID>"],
  "labor": "CARPENTER",
  "targets": [
    {"recipe": "wooden_bed", "minimum_stack_units": 5, "max_order_amount": 20},
    {"recipe": "wooden_chair", "minimum_stack_units": 5, "max_order_amount": 20},
    {"recipe": "wooden_door", "minimum_stack_units": 1, "max_order_amount": 5},
    {"recipe": "wooden_table", "minimum_stack_units": 5, "max_order_amount": 20}
  ],
  "deadline_tick": 13000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "assign_labor": false
}
```

The Agent Turn supplies the complete canonical vector in its planning
recommendation. Each requested minimum and order limit must match the root,
including already-met members. One positive deficit plus several met targets
still remains a portfolio. If every minimum is met, the child retains a
`dfmcp.stock-targets-watch/1` no-work watch instead of an original plan or
publication intent. Planning contributes no stability sample. Wait and default
Observe require the entire cohort to hold in the same fresh qualified census
at the required spaced ticks. Separate historical member successes cannot
combine into project success. Stock loss, Unknown coverage, source rejection,
failed publication or cancellation withdraws current proof.

For a positive deficit, the compiler fixes the complete original before any
child publication. The parent then writes and independently acknowledges a
`dfmcp.project-production-intent/1` record containing the exact goal, plan and
archive digests, citizen capsule identity, initial execution-journal identity,
Workshop/Manager associations, input binding and native custody identities.
Only after that acknowledgement succeeds can the child create its original
archive, capsule, associations and execution journal. This publication creates
no native review, preparation, order or labor effect. The no-work census path
creates neither this intent nor an order.

For a positive deficit, plan each dependency-ready original step with:

```json
{
  "kind": "step",
  "original_plan_digest": "<the retained original production digest>"
}
```

Read and confirm each exact current review. The existing
`review_prepared` request can obtain an eligible fresh review of the same
undispatched original step; it never repeats native preparation or restores an
old confirmation. An acknowledged labor step and dependent order remain
separate native effects.

The parent recommends this recovery request before another ready step or a
goal wait when exactly one eligible original preparation remains. It checks
the acknowledged child original, current Control view and source floor,
unexpired original lifetime, settled clocks and independent effect authority.
Its affordance is `review_prepared_original_step` and names the actual original
StepId; the returned request fixes the retained original digest. An existing
current review instead keeps its exact confirmation recommendation. Retained,
offline, uncertain, fenced, expired or partial views supply no fresh review
offer. Calling Plan obtains a review only; the later Commit requires a separate
fresh confirmation.

### Continue an unfinished no-work goal after consumption

A scalar or portfolio no-work watch may lose stock before ever satisfying its
required stability. The parent accepts the child's exact closed continuation
request through `fortress_plan`:

```json
{"kind":"continue_stock_goal","goal_witness":"<the child's retained watch witness>"}
```

The watch must still belong to the current Production stage and the unchanged
root goal. The request cannot replace any target, worker selection, order cap,
labor-assignment policy or original timing. Current production authority and,
when selected originally, labor authority are checked before source reads.
The child freshly qualifies the complete original citizen selection, inventory
and empty paused native queue; only a fresh bounded deficit can publish work.
A completed, cancelled, expired or fenced watch cannot continue.

This uses the existing first-production publication boundary: the parent
acknowledges the exact immutable production intent before the child writes its
original archive, citizen capsule, associations or execution journal. The new
original binds the watch's witness as historical predecessor identity while
preserving all root minima and the absolute deadline. The ordinary separate
step review, Commit and clock confirmation rules then apply.

A refused read or prepublication check retains the watch and consumed source
floors while withdrawing current proof. Partial publication remains fenced.
Neither recovery JSON nor the marker in a later archive recreates a watch or
its prior stability samples. After timely watch achievement, later stock loss
is current evidence loss and does not authorize another maintenance cycle.

### 5. Verify the original outcome

`fortress_wait` performs one foreground reconciliation or goal sample. It
does not run a background loop. The existing full-parent labor/inventory
monitor, original deadline, cadence and stability rules determine
`original_goal_proven`. For a portfolio, every original minimum must hold
under the existing joint portfolio monitor; selected-order or per-leaf success
is insufficient. Settled clocks are required. An order insertion
receipt or a retired order cannot alone produce this flag.

Independently configured production clocks use these requests for scalar and
portfolio roots:

| Plan kind | Commit/cancel scope | Purpose |
| --- | --- | --- |
| `run` | `clock` | Advance the exact retained original order/goal |
| `inventory_run` | `inventory_clock` | Advance an already observed inventory stability window |

Each interval needs a separate current review and explicit confirmation.
Neither changes the parent deadline or grants another native mutation.
A portfolio Plan request includes `order_step_id` alongside the unchanged
`original_paired_seal`, `game_ticks` and `wall_millis` fields. For example,
after the original effects have settled, the request string for a published
original order with StepId 1 has this shape:

```json
{
  "kind": "run",
  "original_paired_seal": "<that original order's published paired seal>",
  "game_ticks": 10,
  "wall_millis": 1000,
  "order_step_id": 1
}
```

Use the new clock review's `review_seal` for `fortress_commit`, with
`scope: "clock"`, `confirmed: true` and the same `order_step_id`.
For `inventory_run`, use `scope: "inventory_clock"`. The independent child
owner still checks the exact target, settled original effects, observed goal,
current authority, budgets and current review; a selector grants none of them.
Missing, null, duplicate, foreign or out-of-range Plan selectors refuse.
Goal-effect confirmation has no selector.

The Agent Turn includes the exact selected StepId in clock confirmations and
commit recommendations. It lists every original clock owner, including
inactive and unresolved owners. A retained recovery pin never becomes a live
review. A pending clock prevents incompatible order selection.
`fortress_wait` reconciles the original clock owners without a selector before
qualified goal sampling. Fresh stock samples require actual observed
game-tick progress; repeated foreground calls do not advance time.

### Choose the next production interval

The child publishes `clock_guidance` with schema
`dfmcp.production-clock-guidance/1`. The parent's briefing summarizes its
validated source anchor, original digest, selected StepId, offer count and
explicit blockers. Its enabled clock affordances and next Plan recommendation
copy the child's complete closed request. A portfolio request names the actual
original order StepId; scalar requests omit that selector.

A `new_interval` offer is bounded by the remaining original deadline,
session tick allowance and current independent clock authority. When stock or
order truth is already True, its proposed tick count fits the original
stability cadence. Inventory clocks require qualified True inventory and labor;
an inventory-only configuration cannot advance a False stock goal. All original
effects and incompatible clock owners must first settle.

A `review_prepared` clock offer uses the same `run` or `inventory_run`
request kind with the exact retained original seal, before tick and tick/wall
interval. It neither renews the original nor repeats native preparation.
A current confirmation remains the next action when already available; reading
a retained journal cannot recreate it. Planning advances zero game ticks.
Commit requires its own explicit current confirmation.

Without an eligible offer, guidance requests one reconciliation/fresh sample,
queries the exact portfolio owner holding pending work, or explains an operator
blocker such as zero allowance, missing independent authority or an expired
original. A consumed newer source floor can therefore request a fresh sample
without suggesting a stale interval. Source loss, Unknown truth and full
retained history remain explicit limitations. No-work watches have no original
Created order target; their optional parent clock has its own binding and
observation lifecycle.

### Advance a no-order stock watch

Set `stock_watch_clock: true` in the closed parent configuration before
initialization to enable this path. The field defaults to false; null and
caller-selected clock paths are rejected. It requires the existing independent
production `inventory_clock` configuration, source generation, credential
and control permission. A positive session game-tick allowance is also needed.
The immutable original retains this opt-in, so an existing project cannot
acquire it by editing restart configuration.

After an explicit `stock_target` or `stock_targets` Plan finds every minimum
already met, the parent acknowledges a separate clock target and exact initial
journal identity before creating `stock-watch-clock.journal` in its private
directory. This creates no order, production publication intent or native time
advance. Per-order clock slots remain available if consumption later requires
the first production original.

The target binds the complete original goal, parent journal and acknowledged
production source custody, including the original fortress, source epoch and
absolute deadline. It does not use a process-local watch witness as a durable
identity. Scalar and one-to-four-target portfolio goals use the same parent
clock, and every original minimum must hold in the fresh joint census.

1. Use `fortress_wait` with `scope: "goal"` for a fresh paused stock sample.
   A True sample at the initial tick can justify the first cadence interval
   even though it has contributed zero stable samples.
2. Copy the exact `stock_clock_guidance.plan_request` into
   `fortress_plan`. Its closed request has exactly four fields:

   ```json
   {
     "kind": "stock_watch_run",
     "original_paired_seal": "<exact published stock-clock original seal>",
     "game_ticks": 10,
     "wall_millis": 1000
   }
   ```

3. Confirm the newly published `review_seal` with `fortress_commit`,
   `scope: "stock_clock"` and `confirmed: true`. There is no
   `order_step_id`, including for portfolios.
4. Reconcile retained time-control work with `fortress_wait`,
   `scope: "stock_clock"`. Once it settles, use a fresh
   `fortress_wait(scope="goal")` to qualify the next stock sample.
   Repeat only through another separately reviewed interval if the original
   stability requirement remains unfinished.

Prepare and Commit each requalify the complete unchanged stock watch. The final
guard verifies the parent custody and a fresh citizen/operations census after
the durable native marker; its paused tick must equal the independently
observed Run/1.13 tick. Full census work, native work and the closing parent
checkpoint are reserved before native contact. One original foreground wall
deadline bounds the entire call.

Pending or uncertain time control withholds stock proof and production
continuation. A successful intentional interval hides the old current sample
while preserving its authentic counted history; it does not create another
sample. Same-tick reads never count twice. Failed observations, source gaps,
missing members and failed publication retain the existing interruption rules.
False stock follows the original `continue_stock_goal` path after clocks
settle. Achieved goals cannot restart maintenance through this clock.

Each original clock retains at most 64 runs. Native receipts report historical
advancement and pause verification; they are not current pause, inventory or
stability proof. A changed source, missing authority, exhausted history,
insufficient allowance or expired original refuses a new interval.

Recovery reopens the exact retained file and validates its ancestry. It never
recreates a missing journal, restores a current review, or restores stock-watch
sample history. Retain the separate `stock_clock_recovery` value returned in
the recovery reference, including after failure, and supply it in the parent
configuration when reopening. It is an additional exact clock pin, not a new
original: both its head and the acknowledged parent clock head must occur in
the surviving journal, and their rejection floors combine monotonically. It
cannot initialize a missing file or enable the feature for an older project.
A retained Prepared interval without its process-local watch
must be explicitly cancelled in Control mode before rebuilding the unchanged
watch. Recover mode can inspect and reconcile; it cannot promote itself to
Control. Cancellation uses independent Query and ControlClock authority and
remains available after the goal deadline or parent stop, without requiring
Plan, Observe or current stock truth. Project cancellation still retires a
later ordinary production preparation after its separate stock clock settles.

## Stop, close and recover

Default `fortress_cancel` uses `scope: "project"`; `goal` is an alias.
It first acknowledges the terminal parent stop, then retires the child's
undispatched preparation. It cannot undo a designation or construction or remove an already
inserted order. Explicit `clock`/`inventory_clock`/`stock_clock` cancellation delegates to
the existing independently authorized safety-stop owner. Portfolio clock
cancellation also requires the exact original `order_step_id`; project, goal
and session cancellation reject selectors. Continue original reconciliation
until the relevant child state is known.

`scope: "session"` releases local sockets and locks while retaining files. It
does not claim that an external native effect stopped or that the game paused.

Retain the complete recovery reference after every successful or failed call:

- Parent `resource-project.journal`, `resource-project.ack` and the external
  `parent_acknowledged` pin.
- Exact child custody, ACKs, original archive/frontier identities and clock pins.
- Each immutable handoff from the parent checkpoint.
- The separate `stock_clock` target and journal pin when configured, plus
  `stock-watch-clock.journal`. Its floors remain binding after later production.
- Any immutable `production_intent` from the parent checkpoint, including when
  the child original was not yet acknowledged in the next parent checkpoint.
- Consumed authority/source floors and sticky discontinuity fences, including
  newer in-memory rejection evidence returned after a failed append.

Reopen the parent with `initialize: false`, the same original goal/policies,
and `acknowledged` equal to the externally retained parent pin. Reopen an
active child with its existing custody selectors. Production recovery also
repeats the exact retained handoff in the parent configuration's `handoff`
field so configuration can be preflighted before custody I/O. The runtime then
compares that value against the actual parent journal.

A Manager-first restart keeps `appoint_manager: true`, the original Manager
policy and its existing independent acknowledgement. After transfer, supply
the exact parent-held `manager_handoff` for configuration preflight; it must
equal the frozen journal value. The parent retains the original appointment key,
plan, absolute deadline, native dispatch history, and every consumed source
counter or sticky fence. A newer parent tick narrows rejection authority before
native reads. A cancelled Manager opening stays local, and a recovered Manager
never regains a preparation or automatically repeats the appointment.

A Tree-first restart also keeps `harvest_tree: true` and the original Tree
policy. Its retained Tree checkpoint binds the exact journal/ACK, original
plan, deadline and clock recovery. An optional `tree_handoff` configuration
must equal the actual handoff in the parent journal; it cannot select a donor.
An authenticated newer child original must still match the fixed parent
deadline, including when the older checkpoint had no original. Neither opening
nor later inspection can acknowledge a rejected original. A newer externally
retained parent authority tick also raises the child's rejection floor before
any fresh progress read; it does not become an observation or canonical anchor.
After Tree transfer the frozen checkpoint remains in every later frame, and
the parent does not reopen Tree as an active writer. A cancelled Tree opening
stays local; it does not initialize or activate Tree custody.

Recover mode cannot start a new stage. Explicit parent Control can continue
from an acknowledged transition under current host permissions; recovering a
Workshop child still grants no fresh construction authority. Child checkpoint
selectors must match the parent's retained originals before opening, and
subsequent child validation accepts only authenticated descendants. A
cancelled opening remains a local terminal marker and never activates a child.

If the exact production intent was acknowledged but the child reply or next
parent checkpoint was lost, retain the intent and reopen the existing files.
The parent can derive the original recovery selectors from that intent, so a
missing child reply does not require guessing a plan digest or journal head.
Leave the combined configuration's `recovery` absent when those selectors were
lost; supplied selectors must match the parent-held original. Keep the native
workforce/order custody identities and configured clock journal identities.
Their files still undergo the ordinary child validation.

For a portfolio, the intent and recovery reference retain the exact ordered
original order slots, including the assignment-dependent first StepId. The
complete goal vector also retains minima that created no order. Each leaf's
historical goal marker is monotone; changing selection cannot drop or roll back
an inactive leaf. The top-level historical marker remains tied to the first
original order. Adding, removing, reordering or replacing slots or configured
clock families refuses. Every order and inventory clock identity must equal
its originally acknowledged slot. Fresh and recovered scalar originals cannot
be adopted by portfolio roots, or vice versa.

This pending-publication recovery requires the exact original archive, citizen
capsule, prerequisite associations and initial execution journal. Native effect
journals must remain empty; goal history in any portfolio slot, execution
progress or clock runs cannot be adopted through the pending intent. If the child checkpoint was
already acknowledged, ordinary authenticated descendant recovery applies.
Both paths retain newer rejection floors and sticky source fences. The intent
is immutable after publication and remains present through later checkpoints
and cancellation; it never supplies a replacement key or resets a deadline.

The Agent Turn exposes a pending publication under `active_work.publications`
and withholds plan/commit recommendations until the original has been verified.
An intent alone does not prove that the child publication finished. Missing,
partial, foreign or changed files refuse without repair or initialization.
Legacy parent journals remain readable, but an original created without any
prior acknowledged intent cannot be automatically adopted. General multi-family
planning and the complete live acceptance campaign remain open.

A parent append/readback/synchronization failure fences the owner. Partial or
rolled-back journal/ACK data is not repaired in place. A failed response fit or
expired final publication budget withdraws live reviews before the session lock
is released. Recovery data never restores a native permit or current goal proof.

### Complete portfolio clock recovery custody

Production recovery can retain `portfolio_clocks`, an ordered vector matching
the complete original order slots. Every entry carries `step_id` plus the
configured `clock` and/or `inventory_clock` checkpoint: `journal_id`, `head`,
`authority_tick_floor` and `high_source_sequence`. The top-level compatibility
summaries always name the first original order, independently of inspection
selection. Scalar and clockless references keep their prior shape.

Every inactive owner contributes to the parent rejection floor. Recovery
rejects missing/reordered slots, substituted or duplicated journal identities,
changed clock families, regressed counters and lost per-order goal history.
The child proves each actual private journal is descended from the retained
head before restoring targets; parent metadata never substitutes for that
proof. An unacknowledged original publication can adopt only its exact
untouched clock headers. None of these checkpoints restores a review,
permission to unpause, current pause proof or current stock proof.

The prospective identity calculation shares the actual clock journal header
codec and fixed-slot request mapping. It creates no files or native contact.
After original publication, the child must match every acknowledged initial
header exactly before the parent can retain the completed publication. A lost
reply can recover only complete existing custody, and pending publication
rejects advanced clocks, goal history or native effects in every original slot.

This custody and routing support is independently source-reviewed. Regression
sources cover child validation, actual parent journal plus independent ACK
replay, prospective header identity, explicit selectors, presentation and the
actual combined-production Runtime/native-TCP publication campaign. The new
tests have not executed because the execution environment is unavailable.

## Shared request and output bounds

| Bound | Value |
| --- | --- |
| Wall time | Default and maximum 60,000 ms per foreground call |
| Parent logical byte-work | Default 16 GiB, selectable 4–32 GiB |
| Output | Exactly 65,536 admitted tokens under the existing four-byte convention; complete encoded response capped at 256 KiB |
| Entities | 4,096 ceiling, narrowed by the child |
| Actions | Five ceiling; Manager, Tree and Workshop children narrowed to one |
| Game-time allowance | Default zero, maximum 1,200 per explicitly reviewed interval |
| Parent journal | 8 MiB, at most 4,096 frames |
| Complete parent frame | 272 KiB, including all retained child checkpoints, handoffs and production intent |
| Immutable original/handoff | 64 KiB each |
| Tree handoff | 16 KiB |
| Manager handoff | 16 KiB |
| Child checkpoint | 32 KiB each |
| Exact production publication intent | 32 KiB |
| Parent configuration | 128 KiB |
| `stock_targets` planning request | 8 KiB |
| Other parent planning requests | 4 KiB |
| Portfolio | One to four distinct recipes; at most four finite orders |

Byte-work is a conservative bound on repeated reads, replay, hashing and
verification; it is not resident memory or output size. Ledger operations
prepay 128 MiB each. Child work is prepaid at a narrowed ceiling, and unused
reservations are not recycled into another stage. Positive stock-target
planning prepays separate intent and closing-checkpoint reservations before
child source reads. A closing checkpoint and
complete output reserve remain available before child I/O. Final publication
rechecks the same original wall interval.

The first MCP text block contains the canonical Agent Turn. Large evidence
fields or complete recovery metadata may use
`dfmcp.native-resource-evidence/1` descriptors. Each descriptor names the exact
JSON field path, encoding, total byte length, SHA-256 and ordered content blocks
with byte offsets and chunk hashes. Hex evidence reconstructs the original
bytes; `utf-8-json` blocks reconstruct the complete serialized JSON subtree.
Descriptors can refer to evidence descriptors inside that subtree. Reassemble
all referenced data before reviewing a native confirmation or retaining restart
configuration. No review is published as a truncated prefix.

Project Tree views publish the full semantic approval information, including
root, species, dimensions, computed designation, effects/protected domain,
visible coverage, eligible worker/tool offers, lease, checkpoint and original
deadline. The raw native capture is represented by an explicitly named
`native_capture_commitment` with byte length and SHA-256; its exact bytes remain
in the current Tree owner or original Tree journal. This commitment is **not**
an inline evidence chunk and does not claim to reproduce raw capsule bytes.
The standalone Tree profile retains its existing forensic rendering.

There is no synthesized cross-family canonical anchor. Native child anchors,
historical custody and current production goal anchors remain explicit and
separate.

## Validation record

### No-order stock-watch time

The independent owner, parent journal replay, fixed native header creation,
scalar/vector monitor behavior, typed Runtime sampling and public guidance
have focused regression source. Independent source review covered initial
cadence, preserving authentic sample history across successful intervals,
closing-checkpoint budget, cancellation after later production, recovery-mode
authority and complete response evidence. Current execution results are
recorded in `IMPLEMENTATION_STATUS.md`; historical component receipts below
do not qualify this increment. Full MCP/stdio and live DFHack acceptance
remain separate requirements.

### First portfolio publication after reopening

The actual Runtime/TCP campaign now exercises successful first publication
from a reopened pre-plan owner with order clocks, inventory clocks, or both,
including both original StepId layouts. It also covers a one-order reopened
portfolio without additional clock effect authority, missing independent
creation authority, refused parent acknowledgment, a foreign path before
acknowledgment, and a collision after one valid additional header was created.
Strict offline recovery preserves the bytes and refuses incomplete custody.
The native owner regressions preserve the ordinary reopen-mode checks and
exercise the distinct combined-project publication path.

Independent source review checked the acknowledged descriptor, exclusive native
initializers, all changed callsites and the partial-publication failure paths.
These new Rust/Python sources have not been compiled, formatted or executed:
the execution environment is unavailable. Earlier component receipts do not
qualify this increment; full MCP and live DFHack acceptance remain open.

### Current per-order inspection and portfolio-clock increment

The existing Query now exposes bounded source-anchored production inspection
and explicit original order inspection. The complete clock extension adds
per-slot real-owner custody, prospective identities acknowledged before child
writes, explicit Plan/Commit/Cancel selection and selected-order confirmations.

Independent source reviews cover the child recovery and header calculation,
parent routing, actual journal/ACK replay regressions, presentation and the
native-TCP fixture extensions. The five initial inspection Rust files passed a
parser check using the available September nightly rustfmt during a brief host
recovery. No new Cargo build, Rust test execution, formatter pass, integrated
MCP/stdio or live DFHack result is claimed. The execution environment repeatedly
failed during latest-nightly provisioning and is unavailable. Earlier receipts
below concern their exact earlier source revisions and do not qualify these
changes.

### Current parent-portfolio increment

A focused component passes **162 tests, zero failures**, using the official
2026-11-23 nightly (`rustc 1.102.0-nightly (137d1d494 2026-11-22)`) and exact
locked dependencies. Its included source hashes match the formatted repository
files. It compiles the complete actual core/world/intent/adapter graph and
semantic workforce/order production owners without changing repository
dependencies or substituting native implementations.

The executed cases include six new goal/compiler groups, fifteen recovery
groups (seven new portfolio groups), nine continuity groups (five new portfolio
groups), four preserved scalar project groups, two parent request-routing
groups, one parent clock-policy group and the actual NativeShell clockless
reopening regression. The other 124 cases cover existing semantic compilers,
bindings, clocks and rosters. The new native regression runs all four fresh/
reopened and configured/unconfigured subprocess cases with no native contact.
The six new portfolio presentation groups are authored and source-reviewed but
were not included in this component.

The component excludes full native resource-project and combined-production
Runtime orchestration, MCP transport and real Asupersync scheduling. It does
not establish integrated or live qualification. Independent source review covers
the acknowledged publication sequence, complete target proof and per-slot
recovery. Scoped formatting and patch whitespace checks pass. Repository,
dependency, source and agent contracts pass; Beads coverage passes 22/22 after
locating the existing CLI. Workspace formatting still reports drift in 117
unchanged files. Full `verify.sh`, `qualify_local.sh`, integrated MCP/stdio and
live DFHack qualification have not passed for this increment. Earlier receipts
below describe their stated checkpoints and do not qualify this extension.

### Expanded portfolio Runtime/TCP campaign

The existing real Runtime and native-protocol test peer now cover seven scalar
and eight portfolio scenario families. The mixed portfolio fixes BED, CHAIR and
DOOR minima while compiling only the BED and CHAIR deficits. The campaign checks
the parent ACK before original writes, unchanged custody on ACK refusal,
interrupted publication, prepared/advanced-original refusal, lost-selector
recovery and slot-selection continuity. Missing slots and changed or removed
already-met minima refuse. All-met watches require fresh joint samples and
withdraw proof when the parent floor advances.

A restart case reopens actual native custody before any original exists, then
publishes deficits under the same retained root and deadline. Recovery asserts
no native contact, custody writes, restored review or current goal proof.

These expanded Rust/Python fixtures are formatted, syntax-checked and
independently reviewed but **not executed**. A separate full-owner metadata
check was killed under memory pressure without a successful compilation.
The 162-test component receipt above excludes this campaign and the six new
parent presentation groups. No integrated or live qualification is inferred.

### Current Tree integration evidence

The merged Tree and interrupted-production publication ledger passes 41 exact
component tests: 38 ledger cases and three child-continuity cases. This includes
three actual Linux private-storage tests, independent pre-Tree canonical replay,
every Tree/Workshop/production cancellation stage, and a Tree-prefixed project
that recovers the existing exact production intent without changing either donor.
The component uses the complete actual core/world/intent/adapter storage graph
and the actual production-intent parsing/validation closure without replacement
behavior.

The complete selected Tree, Workshop, combined-production, semantic workforce/
order and parent owner bodies pass library and selected-test metadata checking
with the real core/world/intent/adapter/laboratory dependencies. The three
unavailable Asupersync context wrappers are declaration-only external hooks in
the scratch component: they supply no implementation or fabricated authority.
Thirty actual policy, custody, deadline, template, routing and floor tests pass;
none of those executed cases calls an excluded hook. They include real private
journal/ACK descendant recovery with both an exact and a conflicting deadline.

All 26 presentation tests pass: 12 Tree cases, eight preserved parent cases and
six actual AgentTurn cases. All six encoding cases execute the actual modern
`tools/call` content types, raw admission, result codec and final encoder, with
unrelated dispatch variants excluded. Complete Unicode recovery chunks, semantic
Tree reviews, oversized-review refusal and retained recovery are checked without
substitute serialization.

All 12 Tree-material adapter tests also pass, including two new original
checkpoint cases. That component retains the entire actual adapter production
source and the exact native-vector and private-file helpers; only unrelated
tests are excluded. The three continuity cases also appear in the owner suite,
so the four Rust components together cover **106 distinct passing tests**.

The Tree and Workshop source-inventory suites pass 3/3 and 11/11 respectively.
The full MCP dependency build still ends with SIGKILL in unchanged Asupersync
before repository diagnostics, including a serial frontend/backend attempt.
Mandatory verification reaches the missing `br` executable, and release
qualification refuses the changing worktree. Full MCP/stdio, real Asupersync
scheduling and live DFHack qualification remain separate and unclaimed. The
complete adapter test binary also exceeded the host limit before execution;
its selected material tests passed in the component described above.

### Preserved production-publication evidence at `5643fa5`

The current recovery increment passes exact-source component library and
test-target metadata checking with locked dependencies on
`rustc 1.102.0-nightly (bdf712ee8 2026-11-21)`. The component retains the complete
actual core, world, intent, adapter and laboratory crates and selected MCP
owner/test bodies. Its eleven external dependency identities match the original
lockfile. FastMCP transport/codec, unrelated module surfaces and real Asupersync
scheduling are excluded; explicit synchronous I/O-capable context glue is used.
All 82 selected tests pass: 41 parent, eight child recovery, five project
configuration, nineteen combined configuration, four fixed-goal, four ordinary
stock-target runtime campaigns and one new publication campaign. The new actual
Runtime/native-TCP/private-file campaign checks no original writes before the
parent ACK, zero writes on no-work or refused ACK, exact lost-selector recovery,
missing/corrupt/foreign custody, incomplete publication and advanced native
Prepare refusal. Recovery performs no native calls or custody writes. The
component's 2,282 source hashes and seventy selected recovery paths match;
proof SHA-256 is
`f195c238253e6dc8acbfb7bbdf6b81aa88ef0ae96ebb91a031f911d5c966354e`.

The complete actual adapter library also passes metadata checking. Full MCP
checking was killed in unchanged Asupersync 0.5.0 before repository diagnostics,
despite serial frontend/backend compilation. Current agent/dependency/status
and Workshop source gates, eleven Workshop inventory negatives and scoped
formatting pass. Mandatory verification reaches missing `br`; release
qualification refuses the changing worktree. These are development checks,
not a full MCP transport, live-game or production-admission qualification.

### Earlier parent implementation evidence

Before the execution host disconnected, a focused ledger harness passed
17/17 tests, including two cases using the actual Linux private-file storage.
It used the actual core crate, complete new ledger/test logic, exact journal
trait/mode excerpts and the real private-file/locking implementation; unrelated
adapter decoding factories were excluded.

That host became unavailable before the integrated controller could compile.
Source was reconstructed through GitHub against the then-current main,
preserving the concurrent completed-Workshop export. The reconstructed
formatted bytes had not been replayed at that checkpoint. Parent facade/output/
budget tests, Workshop handoff/recovery tests and production goal/custody tests
were authored then. The component checks above supersede the earlier
uncompiled-source status; execution, full verification, modern stdio and live
DFHack qualification remain separately recorded.

The independent source reviews and GitHub blob readbacks are not Rust,
native, live-campaign or admission evidence. Existing registry and production
protocol mappings remain unchanged.
