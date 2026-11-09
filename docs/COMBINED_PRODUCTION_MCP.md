# Labor and bounded furniture production through MCP

`dwarf-fortress-mcp serve-gameplay-dev labor-order` connects the real
`live_production::Executor` and durable mutation dispatcher to the eleven-tool
MCP surface. One session owns one complete original plan: configure one labor
for selected citizens, then insert one dependent finite furniture order or an
explicit portfolio of up to four distinct furniture recipes.
An explicit `kind="existing_labor"` request instead inserts one finite order
using already enabled workers, with no labor assignment step.
`kind="existing_labor_portfolio"` retains up to four distinct orders under the
same observed worker prerequisite, also without assigning labor.
`kind="stock_target"` derives one finite furniture amount from a fresh complete
inventory census and uses already enabled workers by default. Explicit
`assign_labor=true` instead retains a separately reviewed labor enable before
the dependent order; an already-met target creates neither effect.
The source is explicitly unadmitted development work. Current executed checks,
qualification and real-game evidence belong in `IMPLEMENTATION_STATUS.md`.
The private custody implementation supports Linux x86_64/aarch64; unsupported
platforms refuse private-journal opening.

Owner: `df-dfhack-bridge-plane-c-pic.4.1`. The broader dispatcher and recovery
tasks `.4` and `.5` remain open. This host does not supply a complete fortress
controller or proven native material/workshop feasibility. It can diagnose
captured operations and allocate caller-declared inventory within the original
session. An optional acknowledged original Carpenter workshop can also be bound
as a fresh dispatch prerequisite for every original furniture order, as described
below. Separately configured bounded clock
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
Boolean `initialize`, and optional `recovery`, `clock`, `inventory_clock`,
`workshop_prerequisite` and `workshop_prerequisite_checkpoint`. All three custody directories
must be separate and nonnested. Existing private-file owners require real
owner-private directories and regular no-follow files with canonical modes;
the host neither repairs nor silently replaces custody.

The optional `workshop_prerequisite` imports one exact independently acknowledged
Workshop original. Its closed `workshop_prerequisite_checkpoint` is permitted
only before complete original recovery exists. The donor directory is a fourth
separate, nonnested private owner; neither object is an MCP argument.

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
- `DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN`

Each pair of clock environment names is accepted only when that clock is
explicitly configured in the parent and child. Dropping configuration cannot
hide an existing private clock journal.
The Workshop read credential is accepted only with `workshop_prerequisite`.
Standalone Workshop initialization, construction permission and configuration
variables are not accepted by this host.

Authority flags are absent or exactly `1`. Initializing in Control requires
both write flags. Opening authority can later be reduced; it cannot be
increased within that session. All three exact configuration strings are sealed
at opening and rechecked at native boundaries. A changed string, revoked opt-in,
other `DFMCP_` variable or production launch/admission state refuses execution.
Configuration, paths, credentials, pins and native methods are never MCP inputs.

## Require an exact completed original workshop

A furniture order can depend on the Carpenter workshop built by the separate
[Workshop owner](WORKSHOP_CONSTRUCTION_MCP.md). The operator selects its original
key and plan digest, exact private journal and matching Workshop acknowledgement.
The combined owner imports those files Offline and never modifies them. It can
consume a resolved `Placed` record without a historical completion sample:
current completion must still be established by a fresh native progress read.

Add this closed object to the parent configuration, using the actual donor
source pins and latest externally retained acknowledgement:

```json
{
  "schema": "dfmcp.combined-production-development/1",
  "directory": "/private/combined-production",
  "initialize": true,
  "workshop_prerequisite": {
    "directory": "/private/completed-workshop",
    "endpoint": "127.0.0.1:5001",
    "world_folder": "region1",
    "site_id": 7,
    "df_version": "<exact donor DF version>",
    "dfhack_version": "<exact donor DFHack version>",
    "generation": 30,
    "map_dimensions": [64, 64, 4],
    "acknowledged": {
      "journal_id": "<64 lowercase hexadecimal digits>",
      "head": "<64 lowercase hexadecimal digits>",
      "frames": 7
    },
    "original_key": "original-carpenters.1",
    "original_plan_digest": "<64 lowercase hexadecimal digits>"
  }
}
```

The example values are illustrative. The endpoint must be canonical numeric IPv4
loopback, and the directory must be an absolute normalized owner-private path.
The donor world, site and software must match both production families. Its
native generation, endpoint and map dimensions remain separately pinned; a
workforce or order incarnation cannot substitute for the Workshop incarnation.
The latest retained ACK must name the exact complete donor journal. No building
ID, input item ID, alternate file path, initialization flag or recovery
generation can be supplied inside this object.

Provide `DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN` for native qualification. It
is a separately issued Workshop read credential, 32–256 UTF-8 bytes without NUL.
The combined owner grants it no construction permission and pins it for each
connection. Opening or inspecting retained custody Offline requires no Workshop
token and performs no native contact. No new bridge protocol or public tool is
introduced; the native source remains Workshop/1.20.

### Bind the prerequisite to the complete production plan

The ordinary assignment request, `existing_labor`, `portfolio` and
`existing_labor_portfolio` retain their existing request fields. With the
operator prerequisite configured, original Plan first obtains fresh complete
progress for the exact original inserted building at the current paused citizen
tick. A still unfinished, absent, replaced or differently qualified workshop
cannot publish a production original. A historical completion sample or a
cached successful check cannot authorize planning.

The immutable imported origin witness participates in the complete original
request identity before the IntentId, step keys and retained order identities
are derived. A versioned requirement marker in the sealed original plan also
names that witness. The parent then stores `original.workshop`, a private
immutable association bounded at 64 KiB. Its bytes bind the complete production
plan digest, exact donor directory and complete original Workshop descriptor,
including the acknowledged pin and resolved receipt. Its digest is returned as
`workshop_prerequisite.association_digest` and retained in complete recovery.
An original without this optional prerequisite keeps its existing identity
bytes and custody layout.

Each original order requires another fresh completed-workshop check before
native preparation and after its durable `DispatchStarted` marker before native
insertion. This includes every portfolio leaf and dependency-free existing-labor
orders. The same original donor, its ACK and the immutable parent association are
reverified around each read. A changed donor, lost completion, changed paused
tick, revoked source policy or unavailable qualification prevents insertion.
A refusal after the dispatch marker retains that original frontier for
reconciliation; it cannot authorize a replacement key or automatic redispatch.

This is a host dispatch prerequisite for the original constructed workshop.
It adds no workshop facts to the canonical citizen projection and changes no
labor, inventory, cadence or stability predicate. It does not prove material
supply, path access, productivity, manager feasibility or attribution of an order
to that building. A later original-goal proof still means the exact sealed
labor and production terminal holds at its reported anchor. The workshop's
status does not itself prove that terminal or current whole-world state.

### Preserve source floors and exact recovery custody

`workshop_prerequisite` status reports the original witness, key, plan digest,
receipt and inserted building, together with the association digest, consumed
tick floor and sticky `source_fenced` flag. `last_qualified` is explicitly a
historical check with `fresh_for_current_call=false`; Query does not contact the
source or turn it into fresh dispatch evidence. Failed publication withdraws
that sample while retaining the latest complete checkpoint.

Every decoded native progress read can consume a newer tick, source generation
or building allocator floor even when its final qualification fails. A missing
original building, contradictory identity, source reset or ambiguous native
connection fences the same owner. Fencing cannot be cleared by retry, local
verification or lower-floor restoration within that retained owner. On reopening,
the caller must supply the latest external floors: the new owner rejects values
below the donor's retained seed, invalid generations and any regression it has
itself consumed. The immutable origin descriptor and association bytes remain
stable while these consumed floors advance.
An already decoded handshake generation is retained immediately, before a later
budget, authority or deadline refusal can prevent the progress read.

Before an original plan exists, copy the complete returned
`workshop_prerequisite.checkpoint` into the optional parent field
`workshop_prerequisite_checkpoint` when reopening:

```json
{
  "workshop_prerequisite_checkpoint": {
    "origin_witness": "<the exact returned origin witness>",
    "tick": 13212,
    "generation": 30,
    "next_building_id": 71,
    "fenced": false
  }
}
```

Use every returned value exactly; these numbers only illustrate the shape. The
checkpoint object is closed: missing or unknown fields, explicit null, malformed
witnesses, illegal native bounds or generation rewind refuse. An increased
generation must retain `fenced=true`. Opening verifies the witness against the
newly imported exact donor and restores monotone floors before native contact.
The consumed tick raises the opening authority floor; it does not manufacture a
new canonical observation. This field requires `workshop_prerequisite` and is
mutually exclusive with the complete original `recovery` object.

After original publication, retain the whole returned recovery reference. In
addition to its existing fields it requires
`recovery.workshop_prerequisite_digest` and the closed
`recovery.workshop_prerequisite_floor` containing `tick`, `generation`,
`next_building_id` and `fenced`. The frontier authority floor must cover the
Workshop tick. Reopening checks the marked original, exact donor descriptor,
full `original.workshop` bytes and external digest together. Deleting the donor
configuration or sidecar cannot downgrade that marked original into an
unconstrained legacy plan. An unmarked original also refuses an unexpected
sidecar. Offline recovery needs no Workshop token and restores no current
qualification or confirmation permit; Reconcile resolves original native keys
without another insertion.

These fresh-read floors are externally held recovery custody. The combined
reader does not append them to the donor journal or ACK. Before any original
archive or sidecar exists, local files cannot detect a caller omitting a
previously returned checkpoint on a later opening. Rolling back an entire
externally held checkpoint, or the consumed-source floor inside complete
original recovery, likewise cannot be independently detected when the supplied
values still satisfy the immutable donor seed. After original publication, the
marked plan and sidecar prevent dropping the workshop policy; they do not
durably acknowledge later read floors. Preserve the latest complete external
state across failures and close/reopen. The host does not claim independent
durable acknowledgement or complete native continuity for omitted or rolled-back
externally held state.

## Public sequence

Use the exact names returned by `tools/list`. The logical operations below
retain the frozen eleven-tool waist; this profile's wire names are underscores.

1. Open `mode="control"`. Offline and Reconcile modes do not gain write authority.
2. Observe the paused canonical citizen roster. Set `include_production=true`
   in the optional closed `request` to also inspect jobs, buildings and inventory
   before choosing production amounts, as described below. Retain its `observation.seal`
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
6. Repeat steps 4–5 for each dependent order only after the first step has a
   verified historical labor receipt. The actual original StepIds, keys, anchor,
   deadline and whole plan digest are retained throughout.

Wait performs one original-key reconciliation. Once all original effects are
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

## Inspect production before choosing the original plan

Before an original plan exists, the same Observe tool can acquire a complete
operations/1.3 capture alongside the paused citizen and selected-workforce view:

```json
{
  "session_id": "<the current combined session ID>",
  "request": "{\"include_production\":true}"
}
```

`include_production` is a Boolean and defaults to false. It can be combined with
the existing `units`, `option_offset` and `existing_labor_offset` selectors.
`include_operations` is an accepted alias for the same Boolean; supplying both
names is rejected as a duplicate field. Fresh workforce page requests preserve
an explicitly requested production capture using the canonical
`include_production` spelling. Workforce selection witnesses remain unchanged.
Explicit true acquires exactly one complete operations capture. It needs current
Query and Observe authority and the configured operations read credential; it
does not require Plan or grant any production authority. The host independently
checks the operator's world/site, software and operations-generation pins, and
requires the same paused game tick as the complete canonical citizen observation.
Unqualified or incomplete reads refuse the combined publication.

The returned `production_inspection_source` has
`scope="preplan_captured_operations"` and `order_step_id=null`. Its `anchor`
belongs to the operations snapshot; `canonical_anchor` and `parent_anchor` name
the citizen observation used for subsequent planning. These are distinct
source identities, even at the same tick. The source descriptor includes its
digest, ready `diagnosis_request`, `inventory_request` and
`inventory_totals_request` handles, and the twelve supported query kinds.
Use the descriptor's operations `anchor` for query
`expected_anchor`, and the current citizen `observation.seal` for original Plan.
Neither field substitutes for the other.

Call `fortress_query` with the same session ID and the JSON encoding of
`diagnosis_request`, `inventory_request` or `inventory_totals_request` as its
`request` string. No
`order_step_id` is accepted before an original exists. Query uses only the
retained capture, rechecking current read authority and source pins without
native contact. It supports the same `production_diagnosis`, `inventory_plan`,
`entities`, `inspect`, `traverse`, `dependencies` and `aggregate` operations described below.
`search` finds captured text; `spatial_entities`, `related_entities` and
`ranked_entities` select bounded pages using observed positions, witnessed
relationships or exact numeric fields. `events` reports the projection's
available event coverage, which is not a complete native event history. Each
route uses the same advertised operations anchor, current read authority and
source-bound continuations. Watches and effect-bearing artifacts are excluded.
The diagnosis can identify observed suspension, worker, holder and attachment
conditions. The inventory query exposes individual item types and stack quantities,
while the totals request groups complete observed stock by item type in one bounded
result. Declared-demand allocation can then show competing demand and conservative
shortage without counting the same stack units twice.

Agent Turn guidance offers this explicit read before original planning when
production inspection is missing, preserving the current workforce selectors.
After capture it offers the exact source-bound diagnosis. Recommendations carry
the citizen anchor and, when available, operations anchor and digest. The read
uses zero game ticks; diagnosis also uses zero bridge bytes. These are
information-gathering suggestions. Captured supply does not establish native
recipe costs, workshop suitability, path access, future labor productivity,
reservations or goal completion. The source remains
`attribution="not_order_attributed"` and grants no mutation authority.

Source state retains generation history across explicit refreshes, so a removed
and reappearing native identity cannot reuse its previous canonical generation.
The complete optional capture is reserved before any native source is contacted;
projection admits at most 4096 current entities and 8192 total edges. Accepted
canonical parent cursors remain a floor after failed publication, so a retry
cannot reuse a provisional cursor with a different canonical state.
Every new Observe withdraws the previously exposed inspection; omitting the
flag does not preserve it. A failed call, consumed newer source tick, original
plan creation, session close or restart also withdraws current inspection.
Offline recovery restores no such capture. Native ticks consumed by refused
reads remain an authority floor and cannot revive older read grants.
Continuations and drill-down handles bind the exact source and session; use
new handles after a changed capture.

The normal native preparation and commit paths independently revalidate their
required evidence. In particular, choosing an `observed_inventory_increase`
objective still acquires a fresh complete census during original Plan. It does
not reuse this advisory inspection as the immutable baseline. After planning,
`include_production=true` is refused before native contact; the original-goal
Wait path supplies subsequent production inspection.

## Use workers whose labor is already enabled

The assignment workflow requires a real supported change to a single-labor
work detail. A fortress may already have suitable labor enabled through broad
details or manual labor settings. Use `kind="existing_labor"` for that case:

```json
{
  "kind": "existing_labor",
  "expected_observation": "<latest observation.seal>",
  "units": ["<observed canonical citizen ID>"],
  "labor": "CARPENTER",
  "recipe": "wooden_bed",
  "amount": 4,
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 2
}
```

The labor key and citizen IDs must come from the actual observation. The
request accepts the same finite recipes, amounts, original deadline, cadence,
stability and optional `observed_inventory_minimum` or
`observed_inventory_increase` objective as the existing single-order request. It has no `enabled` flag: every selected citizen must be
currently eligible with that labor positively observed as enabled. Callers
cannot supply historical identities or assert eligibility.

Observe exposes `workforce_choices.existing_labor_options` independently of
assignment candidates. Each option names its labor and `eligible_enabled_ids`.
These options require neither automatic professions nor a uniquely assignable
single-labor detail. They are advisory and do not prove workshop access,
productivity or recipe feasibility. At most eight options are returned in labor
key order. The optional closed Observe selector `existing_labor_offset`, from
zero through 128, pages this list independently of `option_offset`. Explicit
coverage records returned and omitted options. Follow
`next_existing_labor_observation_request` for a fresh observation of the same
citizens and use that new observation seal when planning. A nonzero existing
offset uses a version-three choices witness; the earlier default and assignment
page witnesses retain their bytes.

Compilation uses the retained raw capture and an independently issued workforce
policy. It seals the selected citizens' historical identities, eligibility and
positive labor requirements into the complete original parent terminal. The
original has exactly one `CreateWorkOrder`, StepId 0, with no predecessor. Its
idempotency key is in a separate request identity domain, and its order goal
keeps the original temporal requirement. There is no assignment preparation,
Applied workforce receipt or fabricated dependency.

Plan `kind="step"` with the original digest, review the returned order, then
Commit its paired seal. The executor obtains fresh source-qualified labor
before native preparation and again after the durable dispatch marker before
native insertion. Disabled labor, ineligibility, historical-identity replacement,
source drift or unavailable qualification refuses the effect. The order uses
production authority; checking current workers uses Query and Observe. The
current development host still requires both family write opt-ins when creating
its initial private custodians. That initialization requirement does not create
a labor effect, and a retained owner can reopen without assignment authority.

After the insertion settles, Wait brackets the original selected order or
inventory sample with matching current workforce captures. The full parent
requires both the original production goal and the sealed existing-worker
prerequisite. Later labor loss or ineligibility prevents parent success even
when the order's own goal is fulfilled. Source or identity replacement fences
the original lineage. The optional order and inventory clocks use the same
independent authority, original objective and fresh parent checks as other
single-order plans.

Retain the usual complete recovery reference and all private owners. The
single order uses `goal.history` and the existing single-order clock paths;
the workforce journal remains an empty read custodian. Recovery verifies the
sealed parent and exact source identities, restores no assignment receipt or
current parent proof, and requires a fresh Wait for current evidence. Existing
assignment, single existing-labor and assignment-portfolio request identities
remain unchanged. For several goals using the same existing workers, use the
explicit existing-labor portfolio described below.

## Plan a finite stock target from current inventory

Use `kind="stock_target"` when the desired result is an observed minimum stock
and the server should calculate the finite insertion amount:

```json
{
  "kind": "stock_target",
  "expected_observation": "<latest observation.seal>",
  "units": ["<observed canonical citizen ID>"],
  "labor": "CARPENTER",
  "recipe": "wooden_bed",
  "minimum_stack_units": 7,
  "max_order_amount": 10,
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 2
}
```

Use actual observed citizen IDs and labor keys. The request is closed and
bounded to 4,096 bytes. Its minimum is 1–1,000,000 stack units; the optional
`max_order_amount` is 1–100 and defaults to 100. Recipe, selected-citizen,
future-deadline, cadence and stability bounds are the same as the existing
finite-order path. Optional `assign_labor` is a Boolean and defaults to false;
null, text and numeric substitutes are refused. Clients cannot supply an `amount`, a baseline, a source
witness, an `enabled` flag, an order condition or a native selector.

Plan obtains its own complete operations/1.3 census at the exact paused citizen
tick under independently checked source pins and current read authority. It
does not reuse a previously displayed advisory inventory page. If the census
contains four BED stack units in this example, the one original order has
amount three. The original typed baseline and increase retain the requested
terminal target of seven. The request does not mean "produce seven more".

The successful response includes `stock_target`, with the qualified baseline,
requested target, calculated deficit, finite amount, chosen limit, canonical
anchor, source digest/generation/counters and immutable assessment witness.
The following outcomes are distinct:

- **Already satisfied:** `status="already_satisfied"`, `amount=0`,
  `work_required=false` and `mutation_planned=false`. No original plan, keys,
  archive, frontier or native preparation/insertion is created. The result
  establishes only the stock minimum at its reported source anchor. Since no
  work is needed, existing labor is not qualified and
  `worker_prerequisite_checked=false`; this is not a complete labor/production
  goal proof. Observe again before a later goal.
- **Within the limit:** `status="planned"` and the exact positive deficit is
  sealed into one finite order. With the default `assign_labor=false`, this is
  StepId 0. Current selected labor and citizen eligibility are independently
  established before original publication, and `worker_prerequisite_checked=true`.
  There is no labor assignment action. With explicit `assign_labor=true`, the
  original instead retains the two-step assignment workflow below.
- **Above the limit:** Plan refuses the complete proposal before original
  publication. It never silently caps the amount or treats a partial order as
  satisfying the requested target.

For a positive deficit, a matching native furniture job blocks planning even
when suspended or repeating. CustomReaction and nonempty-reaction jobs also
block because their output is unknown to this bounded policy. Future output
is not credited as inventory. A met target needs no insertion and can return
its no-work observation despite outstanding jobs.

### Enable selected labor before the stock-target order

When the desired worker appears in the observed labor option's
`enable_candidates`, add `"assign_labor": true` to the stock-target request.
The server still obtains its own qualified inventory census and calculates the
same complete finite deficit. A positive deficit then compiles:

1. StepId 0: enable exactly the requested labor for the selected canonical citizens.
2. StepId 1: insert the original finite furniture order, dependent on the verified
   historical result of StepId 0.

This requires the ordinary current ConfigureLabor, ConfigureProduction and Plan
authority for the generated plan. The option authorizes only planning an enable;
it does not confirm either effect or grant a capability. Use the existing
`kind="step"` request with the returned original digest, review and confirm the
labor step's paired seal, then request and separately confirm the dependent
order's review. Wait never dispatches the second step. No labor-disable option
is accepted by this stock-target mode.

The selected citizens and supported labor must come from the observed assignment
choices. Planning binds their canonical identities; the existing workforce/1.17
native prepare and commit paths independently recheck the exact supported change,
eligibility, source identity and current authority. A plan is not proof that
labor has already been enabled. For this path, `worker_prerequisite_checked=false`
describes original stock planning; the separate workforce review and receipt
carry the assignment evidence.

The `stock_target` object adds `labor_assignment_requested`,
`labor_assignment_planned` and the actual `order_step_id`. For an assigned
positive target these are true, true and 1. A satisfied target retains the
requested option but returns `labor_assignment_planned=false`,
`order_step_id=null` and `mutation_planned=false`: it does not change labor
merely because the caller opted in, and does not require ConfigureLabor for a
nonexistent effect. Omission and explicit false preserve the existing
single-order request identity.

The two-step original uses the separate `Stock target/2` identity domain.
Its exact enabled assignment and canonical citizen preconditions, original
inventory assessment, finite amount, deadline, cadence and stability participate
before any intent or step key is derived. Recovery checks the complete assignment
and dependent-order shape. The existing `Stock target/1` encoding remains
unchanged. Both modes use the same native protocols, optional original Workshop
prerequisite, private stores, inventory-growth monitor and separately reviewed
clocks.

Enabling labor does not permit a later inventory adjustment: before the order
can be prepared or inserted, its fresh complete operations census and empty
manager queue must still match the original. A job or inventory change caused
by the interval between the two reviews can therefore refuse the order while
retaining the completed labor receipt. The owner does not resize the order,
undo the assignment, renew the deadline or authorize a replacement key.

### Recheck the original amount before native insertion

After a positive Plan, use the ordinary `kind="step"` preparation and confirm
its exact current `paired_seal`. Every protected order-evidence boundary obtains
a new complete inventory census, including the refresh after the durable
`DispatchStarted` marker and the additional checks after existing-worker reads.
The new capture must equal the exact original operations digest and paused tick.
The assessment and typed baseline must also agree with the sealed original.

This is a conservative whole-census witness: unrelated changes to the captured
jobs, buildings, items, attachments or allocator counters can also refuse the
original. The server never recounts into a replacement amount, changes the
target, renews the deadline or substitutes another key. The original must be
retained for reconciliation when any effect may have been attempted.

The protected native manager-order queue must be empty before preparation and
insertion. After each fresh inventory census, an independent ReadOrders capture
must equal the original native queue witness, including its sequence and
allocator. A queue inserted and then removed cannot silently restore that
witness. This check occurs at those real native boundaries, not during the
initial stock-sizing Plan. Protocol 1.10 exposes complete membership but no
recipe or remaining quantities for arbitrary existing rows; therefore even an
unrelated-looking row blocks this first bounded stock-target mode. The server
neither assumes that it is harmless nor subtracts its possible future output.
Existing manual-amount request modes preserve their prior queue behavior.

The optional original Workshop prerequisite composes with these checks. Its
exact donor identity remains part of the original, and fresh completion is
required independently of stock sizing. Worker evidence, inventory evidence,
Workshop evidence and the native queue retain their separate source pins.

### Keep the target through progress and recovery

The stock-target identity domain binds the complete original inventory basis,
worker prerequisite or explicit enabled assignment, selected maximum and temporal
requirements before deriving the intent and step keys. A strict versioned marker in `original.plan` identifies
the policy; the existing archive seal covers it. Recovery rebuilds the required
guard from that original, not from optional presentation fields. It needs no
new sidecar or credentials to recover the stock target Offline. The externally
retained frontier authority floor also constrains the reconstructed owner.

Before insertion, failed reads preserve consumed native ticks and withdraw the
last check. `revalidation_failed` describes the latest attempted check in the
retained process; it is not an independently durable source fence or a claim of
complete source continuity across reopening. A recovered owner still needs the
exact original census at every possible future insertion boundary. An uncertain
original insertion can only reconcile under its original key, without another
insertion or a newly sized plan. Reopening restores no local review permit.

After insertion settles, ordinary Wait observes the sealed inventory-increase
objective and current labor under the original cadence, stability and deadline.
The target remains seven in the example, including after native order
retirement. Configured order and inventory clocks retain their existing separate
review and authority requirements. Goal sampling may observe changed inventory;
the strict original-census guard protects insertion, not later progress reads.

Counts mean all observed stack units of the exact furniture type. They do not
establish wood material, usability, ownership, availability for furnishing,
recipe inputs, workshop access or causal output from this order. An existing
stone table can contribute to a TABLE minimum even when the selected finite
recipe is wooden_table. This is one finite stock goal, with no recurring
maintenance, automatic order renewal or background time advancement.
See [NATIVE_STOCK_TARGET.md](NATIVE_STOCK_TARGET.md) for the qualified adapter.

## Inspect observed production conditions and declared inventory

After an original exists and Wait successfully publishes a joined goal sample, Query and
Explain expose `production_inspection_source`. Its `anchor` and `source_digest`
identify the exact selected operations projection collected with that goal.
This descriptor uses `scope="captured_operations"` and the actual selected
original `order_step_id`, distinct from the pre-plan read described above.
The enclosing labor/order or portfolio anchor remains separately named
`parent_anchor`. Entity IDs, generations and query continuations belong to the
selected projection.

Call `fortress_query` with the session ID and an optional `request` string
containing one closed `dfmcp.query/1` envelope. The descriptor supplies ready
`diagnosis_request` and `inventory_totals_request` handles, including the correct
`expected_anchor`. For a portfolio,
`order_step_id` optionally selects the exact original leaf. A leaf is inspectable
only when its source matches the current selected sample or an exact member of
the current complete portfolio cohort. A different leaf's same-tick observation
is insufficient.

For example, a diagnosis request without the optional expected anchor is:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "production_diagnosis",
    "limit": 4
  }
}
```

The `production_inspection` result reuses the existing complete-row diagnosis
renderer. It can report suspended jobs, absent worker assignments, unfinished
holder buildings, unattached input filters, direct and inherited item flags,
and shared or zero-sized attachments. Job and holder filters use exact canonical
`entity_id` and `generation` handles. Its inspection and relationship requests can
be passed back to the same Query tool.

These are observed conditions in the complete captured operations roster.
operations/1.3 supplies no job-to-manager-order provenance, workshop subtype,
full native recipe requirements, path access or labor-eligibility proof.
The result explicitly carries `attribution="not_order_attributed"`. It neither
identifies a proven causal blocker for the selected order nor changes the
original goal's proof, deadline, plan or clock authority.

### Count the complete captured stock

Copy `production_inspection_source.inventory_totals_request` into the Query
request to obtain item records and stack-unit totals by `type_key`. The same
handle is available before original planning and after a successful original-goal
Wait. It names the source's exact public `anchor`, so the result stays attached to
the operations census the caller inspected. Query uses the retained source and
performs the existing current-authority and whole-source budget checks.

The ready request has this query body:

```json
{
  "kind": "aggregate",
  "kinds": ["item"],
  "group_by": {"kind": "field", "field": "type_key"},
  "metrics": [
    {"name": "stack_units", "field": "stack_size", "numeric_type": "u64"}
  ],
  "max_groups": 32
}
```

Each `groups` entry contains its typed `key`, a `count` of matching item records,
and `metrics.stack_units`. The metric reports numeric samples and excluded
values, source categories, exact decimal-string `sum`, `minimum` and `maximum`,
and an exact rational `mean`. Record counts and stack quantities are separate:
three WOOD records with stack sizes 1, 7 and 99 produce `count=3` and `sum="107"`.
If the first record is rotten and the last removed, the conservative inventory
allocator can still expose only seven eligible units from the same source.

The descriptor labels these totals with
`inventory_totals_semantics="observed_stock_not_usable_supply"`. The report repeats
that value as `stock_semantics`, with `eligibility="not_assessed"`,
`reservation_created=false` and `original_goal_evidence=false`. This is an inferred
summary of recorded quantities. Access, usable materials, native recipe
feasibility, and production attribution remain separate questions. A large
captured total cannot establish completion of the original goal or replace the
fresh baseline required by an inventory-increase Plan.

Use the existing typed `where` filter to select a narrower total. For example,
this filter selects BED records:

```json
{
  "op": "compare",
  "field": "type_key",
  "comparison": "eq",
  "value": {"type": "text", "value": "BED"}
}
```

Omit `group_by` to obtain one total for that selection. The `selection` object
distinguishes `matched`, `excluded_false` and `excluded_unknown`; a missing or
unavailable field remains Unknown even under negation. An empty selection still
has one all-group with `count=0`, but its `sum`, `minimum`, `maximum` and `mean`
are null when there are no numeric samples. Check sample and exclusion counts
before interpreting a total. `coverage.absence_proven` remains false.

`max_groups` defaults to 32 and accepts 1–128; at most 16 uniquely named metrics
are accepted. Metric numeric types are `u64`, `i64` or `fixed`; fixed metrics
require an explicit scale, while integer metrics omit it. Exceeding the group
bound refuses the complete result, so raise the bound within its limit or narrow
the filter before requesting a new source. Aggregate has no `limit` or
continuation fields and never publishes partial totals. A refused aggregate
withdraws all current inspection handles: acquire another explicit production
Observe before planning, or another successful original-goal Wait afterward.

The `inventory_plan` query allocates caller-declared stack-unit requirements
against conservatively eligible observed inventory. For example:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "inventory_plan",
    "quantity_unit": "stack_units",
    "demands": [
      {"key": "first_batch", "units": 4, "item_types": ["WOOD"]},
      {"key": "second_batch", "units": 4, "item_types": ["WOOD"]}
    ],
    "limit": 4
  }
}
```

The existing joint allocation solver prevents the same stack units from serving
both demands and reports a shortage certificate when the joint requirements
cannot be met. Demand names and quantities are supplied by the caller; they are
not inferred native recipe costs. This query creates no reservation, mutation
permit or commit-compatible artifact.

`entities`, `inspect`, `traverse` and `dependencies` provide canonical drilldown
over the same source. Unknown envelope/query fields, unsupported kinds,
mismatched expected anchors, stale generations and continuations from a different
source/session are refused by the existing query engines. The entire source
must fit the entity and byte-work budgets. Inner query output is capped at the
smaller of the negotiated allowance and 4096 tokens; the complete Agent Turn
still has to fit the host's final response bounds. Pages retain complete rows
and their original continuation; an oversized final packet is refused.

Inspection of an original goal performs current authority, operator-pin and private-custody checks
before and after analysis. It does not contact native sources or advance time.
A failed call, new goal observation, owned clock run or restart withdraws the
current inspection source. Offline recovery restores no operations snapshot;
a new successful Wait is required. While an original goal is pending and this
source is available, Agent Turn guidance offers diagnosis before another bounded
run. A diagnostic summary is never goal-completion evidence.

## One original production portfolio

Use `kind="portfolio"` to configure the shared labor once and retain several
independent furniture goals in the same original plan. Use
`kind="existing_labor_portfolio"` when the selected citizens already have the
labor enabled. Both accept one to four distinct recipes and normalize them by
recipe name. The existing single-order requests retain their original identities.
Neither portfolio adds native order conditions, workshop construction or
material procurement.

```json
{
  "kind": "portfolio",
  "expected_observation": "<observation.seal>",
  "units": ["43"],
  "labor": "CARPENTER",
  "enabled": true,
  "orders": [
    {"recipe": "wooden_bed", "amount": 4},
    {"recipe": "wooden_door", "amount": 2},
    {
      "recipe": "wooden_table",
      "amount": 2,
      "objective": {
        "kind": "observed_inventory_minimum",
        "item_type": "TABLE",
        "minimum_stack_units": 2
      }
    }
  ],
  "deadline_tick": 300,
  "poll_interval_ticks": 5,
  "stable_samples": 2
}
```

Use a citizen and supported labor from the actual observation choices. Every
amount is between 1 and 100. An optional objective belongs to its own leaf and
must match that recipe; omitted objectives require that leaf's exact retained
order to finish. The portfolio has a separate request identity. All normalized
orders, objectives, citizens, labor and timing fields bind it; reordering the
input cannot create another original key. The whole parent terminal retains the
labor requirements and every requested production goal.

### Use existing workers for the entire portfolio

An existing-labor portfolio takes the same `orders` array, original timing and
observed citizen selection, with `kind="existing_labor_portfolio"` and no
`enabled` field:

```json
{
  "kind": "existing_labor_portfolio",
  "expected_observation": "<latest observation.seal>",
  "units": ["<eligible_enabled_ids from the observed labor option>"],
  "labor": "CARPENTER",
  "orders": [
    {"recipe": "wooden_bed", "amount": 4},
    {"recipe": "wooden_door", "amount": 2},
    {
      "recipe": "wooden_table",
      "amount": 2,
      "objective": {
        "kind": "observed_inventory_minimum",
        "item_type": "TABLE",
        "minimum_stack_units": 2
      }
    }
  ],
  "deadline_tick": 12000,
  "poll_interval_ticks": 10,
  "stable_samples": 2
}
```

The compiler qualifies the selected subset from the latest retained native
workforce capture. One complete positive requirement seals their canonical IDs,
historical figure IDs, eligibility and enabled labor into the parent. Every
order rechecks that requirement before native preparation and after its durable
dispatch marker. A fulfilled earlier order or another order's review cannot
replace those current checks. No labor assignment, assignment receipt or order
dependency is created. The request's separate identity domain binds every
normalized leaf and the complete prerequisite; a one-order portfolio remains
distinct from the existing single `existing_labor` request.

| Original request | Assignment | Actual original order StepIds | First private storage slot |
| --- | --- | --- | --- |
| `portfolio` | StepId 0, with a real verified receipt | 1 through N; each depends on StepId 0 | First order, StepId 1 |
| `existing_labor_portfolio` | None; current observed prerequisite | 0 through N−1; no dependencies | First order, StepId 0 |

The adapter and MCP host share one validated production layout. Original StepIds
remain the selectors and idempotency identities; a separate zero-based order
ordinal chooses the fixed custody slots. The maximum remains four order leaves
and 32 selected citizens. The same coherent cohort, independent clock authority,
original stability and exact recovery rules below apply to both layouts.

Plan `kind="step"` and confirm each returned original review in sequence, as for
the single-order request. An uncertain insertion blocks subsequent work until
Wait reconciles that original key. It never recreates already inserted leaves
or replaces the complete original plan.

### Observe and advance the original leaves

Portfolio Wait initializes or replays at most one missing goal history, then
samples one original order per bounded call. A complete portfolio cohort uses a
single paused all-order progress bracket around those samples. All leaves must
agree on the exact current operations/inventory capture, workforce capture,
source generations and paused tick. A changed leaf or source refuses the whole
join. `portfolio.sampled_orders` reports progress through the current cohort.
`current_parent_goal.scope="complete_original_production_portfolio"` is published
only after all original leaves join. Its `selected_anchors` preserve each leaf's
separately qualified source anchor and original StepId. The top-level and Agent
Turn `original_goal_proven` require every original temporal obligation and
current labor requirement to hold together.

Each exact selected review owns its own sequential source cursor and durable
history. The parent has a separate monotonically increasing publication cursor.
Observing another leaf cannot create a fictitious observation gap or advance
this leaf's stability. A successful bounded clock withdraws every current
certificate while retaining each unfinished streak; a fresh complete cohort at
the next eligible tick can advance all original streaks. Failed observations,
failed response publication and restart still interrupt unfinished stability.
No deadline, cadence or required sample count is renewed. Repeated same-tick
reads cannot supply another temporal sample.

Use Query with `order_step_id` to select an original leaf and inspect its exact
retained clock seals. For portfolios, every Plan `kind="run"` or
`kind="inventory_run"` request and every Commit/Cancel with either clock scope
must include that original `order_step_id`. The first order is StepId 1 for
`portfolio` and StepId 0 for `existing_labor_portfolio`; a native manager-order
ID is not a selector. Non-portfolio single-order sessions reject selectors.
For example, a reviewed order-clock request adds `"order_step_id":2` to the run
request below, and its confirmation adds `order_step_id=2` beside `scope="run"`.
The separate run and inventory authority requirements remain unchanged. Any
pending or uncertain clock in any leaf blocks another clock's dispatch.
Guidance selects an inactive leaf that owns retained prepared work before
suggesting a new run. Clock advancement reports all retained portfolio clocks.

### Retain the complete portfolio recovery

The response adds ordered `recovery.portfolio` entries for every original
StepId, each containing `goal_history_present`, `clock_journal_id` and
`inventory_clock_journal_id`. Preserve the complete array. The legacy top-level
history/clock fields always mirror the first order's storage slot, regardless of
its actual StepId or the current selection.
The first leaf keeps the existing filenames. Further histories are
`goal-2.history` through `goal-4.history` in the parent directory. Additional
clock files are `portfolio-2-clock.journal` and
`portfolio-2-inventory-clock.journal` through the corresponding fourth-slot files
in the order directory. Each initialized slot has its own journal identity.
These are fixed host paths, never tool arguments.
Recovery retains every actual original order StepId in canonical order. The
host compares the full array with the sealed original layout before opening
additional child slots; shifting zero-based IDs into the legacy range refuses.
The old assignment layout keeps every existing private filename and journal
identity. Existing-labor portfolios keep the workforce owner as the same empty
read custodian, including after uncertain order insertion and reopening.

Reopening verifies every original selected association and configured clock
journal under the complete parent custody. Both clock families reuse only the
same immutable selected identity; they retain independent journals, authority
and review. Parent archive/identity reads share one prepaid allowance so the
four-leaf, two-clock-family shape fits the default public byte-work budget.
Missing, replaced, omitted or extra fixed custody refuses. Offline opening
performs no native contact or writes and restores no current proof or review.
Lost replies reconcile the original leaf/run key without another insertion or
unpause. A partial cohort is process-local and never restored from history.

## Advance the original production order with a reviewed clock

Clock control uses the existing `order-run/1.14` native owner and a separate
private `clock.journal` under the order custody directory. It applies after
all original effects settle, for either the default retained-order completion
goal or an explicitly selected inventory objective. The complete parent archive, selected StepId and native
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

Preparation acquires fresh joined labor and selected-goal evidence and a paused native
order capture. It refuses a completed original obligation, a retired/unlinked
order, expired original time, unresolved clock, replaced custody or missing
independent clock authority. Both native prepare and commit recheck the exact
original custody after the durable clock marker. A pending finite order can
advance toward completion. A retained validated, inactive order with zero
remaining work can advance the original unfinished sampled stability interval;
unexpected reactivation stops that run and does not prove the goal. For an
explicit inventory objective, the same owner can advance toward the requested
stock or finish sampled stability while the original native order remains eligible.

Review `clock.active_run`, then Commit with `scope="run"`, its `review_seal` in
`paired_seal`, and `confirmed=true`. That confirms only this bounded run. The
clock has at most 32 retained runs; each is limited by the session budget and
the remaining original deadline. A native stopping receipt is historical clock
evidence. Commit withdraws current goal proof, and a subsequent Wait must obtain
fresh coherent labor and selected-goal evidence before reporting success.

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

The order clock requires the original native order to remain eligible. A
retired default-order goal remains unknown, and no clock receipt can turn its
disappearance into completion. The independent inventory clock below handles
unfinished sampled stability after normal engine retirement only when the
original request explicitly selected the inventory objective.

## Finish original inventory stability after order retirement

The separate `run/1.13` owner can advance an already True original inventory goal
through its remaining stability interval. It retains the complete combined plan,
selected StepId/key, verified Created receipt and predecessor custody. It does
not require a present manager-order row or reinterpret inventory as output
attributed to that order. Both original effects must be settled, and fresh
qualified evidence must show the requested labor settings and selected inventory
predicate are True. Fulfilled, failed, cancelled or expired original obligations refuse
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
`option_offset`, `existing_labor_offset` and/or the Boolean `include_production`.
The latter adds the pre-plan operations read described above; its default false
does not add native operations I/O. `units` selects 1–32 distinct canonical citizen IDs
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
A nondefault selection, page or production-capture request is refused before native contact; the
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
recommends Wait for a fresh joined sample and, when the separately authorized
order clock is configured, a bounded `kind="run"` preparation for the original
order. That review and explicit confirmation advance time toward completion or
the remaining stability interval within the unchanged deadline. Without clock
authority or a positive session game-tick budget, game-time progression must be
managed separately. The independent `inventory_run` clock continues to require
an explicitly selected original inventory objective.

The selected object alone keeps `original_goal_proven=false`. A fresh current
labor bracket can establish the whole parent under
`current_parent_goal.scope="complete_original_labor_and_order_goal"`, provided
the selected temporal requirement is also fulfilled. The other selected-goal
field is null: explicit inventory requests use `selected_inventory_goal`, while
default requests use `selected_order_goal`. Query, Explain, failure withdrawal
and offline recovery retain this distinction.

## Select and monitor an original inventory objective

Include an `objective` in the first closed request to ask whether the original
order step's requested inventory minimum is present. This minimum can already be
satisfied by preexisting stock. To require growth from a qualified original
census, select `observed_inventory_increase` as described below. For example:

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

### Require growth above the original inventory

Every original request shape supports a separate
`observed_inventory_increase` objective: the ordinary assignment/order request,
`existing_labor`, and each leaf of `portfolio` or
`existing_labor_portfolio`. For a bed order, replace its objective with:

```json
{
  "kind": "observed_inventory_increase",
  "item_type": "BED",
  "increase_stack_units": 4
}
```

The increase is 1–1,000,000 observed stack units, independent of the requested
finite order amount. The type must match that leaf's furniture recipe. Baseline
quantities, targets, source witnesses, generations and counters are server-owned
and are rejected as request fields.

Before any original parent custody is published, the host acquires one complete
operations/1.3 census at the same paused tick as the retained canonical citizens.
The order family's independent source acceptance checks the exact world/site,
software and operations incarnation, complete Item coverage, and every required
type/stack source field. It qualifies the whole count, including a complete
empty roster; missing evidence cannot become zero. A portfolio resolves all
growth leaves against this single census. Default and minimum leaves retain
their existing predicates, and an original with no growth objective performs
no new baseline acquisition.

The resulting typed predicates seal each starting quantity and requested
increase together with the exact original canonical anchor, operations digest,
source generation and three allocation floors. A baseline of three beds plus
four requested stack units requires at least seven observed bed stack units.
A source, tick or allocation regression refuses both initial monitoring and
durable replay. The entire original parent still includes every requested
current labor prerequisite and every original production goal.

`selected_inventory_goal.kind` identifies the increase. The selected object
and portfolio leaf retain `baseline_stack_units`, `increase_stack_units`,
`target_stack_units`, `baseline_anchor`,
`baseline_source_digest`, `baseline_source_generation` and
`baseline_source_counters` (next job, building and item IDs, in that order).
The selected source anchor and complete-parent anchor keep their separate
identities. The normal current-truth, temporal proof, first-achievement and
labor-bracket rules apply without changing the original deadline or cadence.

Prepared plan IDs and step keys bind the full baseline; a later request cannot
change it or reinterpret the original as a minimum. Recovery loads its exact
typed predicate from `original.plan` and restores no current inventory or
parent certificate. It does not acquire a replacement baseline, introduce a new
custody file, or independently recount a retained raw baseline census. Existing
request modes keep their original identity bytes.

Both clocks recognize the increase. The order clock can advance its unfinished
goal while the exact original manager order remains eligible. The independent
inventory clock requires current qualified stock at or above the sealed target
and current labor to be True; it can then finish the original stability interval
after retirement. A clock receipt still supplies no inventory or parent proof,
and no run changes the baseline or permits another insertion.

Unchanged preexisting stock cannot satisfy a positive increase. The result
nevertheless measures net stock, including unrelated matching acquisitions and
items of any material or usability. It neither identifies newly created items
nor attributes output to the original order. Consumption can offset production
or reverse current truth after historical achievement. Retired order completion
remains Unknown.

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
`goal.history` if selected goal monitoring has started. A configured original
Workshop prerequisite also retains its immutable parent `original.workshop` and
the exact donor `workshop.journal`/`workshop.ack` pair in their separate directory.
Persist the latest complete `recovery` object outside those files. It includes
the original plan/object/citizen digests and anchor, and frontier journal/head/
authority-tick-floor. The additive Boolean `goal_history_present` records
selected history; older recovery objects default to false. Preserve its latest
value even when opening or publication fails. On reopen set all three configurations to
`initialize=false`, supply that parent object, and retain the original child
native journal identities required by their existing configurations.
Configured workshop originals additionally retain the association digest and
complete consumed-source floor described above. Pre-plan refusals instead
return their separately witnessed `workshop_prerequisite.checkpoint` even when
`recovery` is still null.

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

Each call admits at most 60 seconds, five original actions, zero game ticks by default and 32 GiB of
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

The optional Workshop owner prepays 200 MiB for bounded Offline import, 80 MiB
for each fresh qualification and 32 MiB for local custody verification from the
same shrinking allowance. The complete read reservation is made before token
access or native contact. With this prerequisite configured, read-only original
custody reserves at most 768 MiB per operation; native prepare, commit and cancel
delegation can reserve up to 2 GiB for repeated boundary checks. This distinction
lets a bounded Wait pay for reconciliation, history initialization and fresh
parent sampling from one unchanged public allowance. Owners without the optional
prerequisite keep the existing 512 MiB custody ceiling, and the native frontier
ceiling remains 2 GiB. Stock-target planning reserves 177 MiB for one complete
census, projection and qualification before native contact. Each fresh dispatch
check reserves 182 MiB for that census and a subsequent independent native queue
read. The queue must still match the original native observation and be empty.
A stock-target dispatch reserves up to 3,140 MiB of parent custody, covering six
such checks plus the existing 2 GiB prerequisite ceiling, within the unchanged
8 GiB default public allowance. Read-only goal and Offline custody retain their
earlier ceilings.
These are logical work charges, not native frame sizes or eager allocations;
the immutable association itself remains at most 64 KiB.

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
