# Finite native stock targets

Owner: `df-native-stock-target-3m9` (WP-05/WP-13/WP-21).

The native furniture path can calculate a finite order from independently
qualified current inventory. The reusable adapter entry is
`InventoryBaselineProjection::stock_target`. This is an explicitly bounded
stock-sizing policy, not a background stock-maintenance service or a native
recipe-feasibility proof. The combined MCP consumer exposes `kind="stock_target"` through the existing
Plan tool and additive `kind="stock_targets"` for a bounded complete target set;
see [COMBINED_PRODUCTION_MCP.md](COMBINED_PRODUCTION_MCP.md) for the
complete request, no-work result, native guards and recovery flow.

## Qualified sizing

The caller supplies a closed furniture recipe, a minimum of 1 through 1,000,000
observed stack units, and a maximum finite order of 1 through 100. The count
comes from the complete operations/1.3 Item census at the exact paused canonical
fortress tick. Query and Observe authority, a complete Item-domain policy and
independently accepted type/stack fields are required. An omitted field or
incomplete domain cannot become zero stock.

The deficit is `max(target - observed, 0)`. A zero deficit returns a typed
already-satisfied assessment with no action or positive-growth objective. A
positive deficit above the selected maximum refuses the whole proposal; it is
never silently reduced to the limit. An eligible deficit produces a one-unit
per-furniture-order sizing proposal and the existing typed inventory-increase
objective with that exact baseline and deficit. Its terminal target is the
requested minimum. The policy does not certify recipe output, inputs, material,
ownership, usability, access, available labor, workshop suitability or eventual
production. The actual inventory goal must still be observed independently.

For a positive deficit, a currently observed matching furniture job refuses
the proposal. A CustomReaction job or nonempty reaction also refuses because
its output is not classified by this bounded policy. Suspended and repeating
jobs are not ignored. None of these jobs is credited as future inventory.
Existing inventory that already meets the target needs no insertion and does
not require a no-competing-job claim.

## Preserve the original basis

The immutable assessment binds recipe, requested target, order limit, qualified
count, deficit, complete canonical anchor, operations source digest, native
generation, allocation counters and item-row count. Its canonical bytes and
witness are deterministic. Changing any of those inputs changes the assessment.
The existing inventory-increase predicate retains the source-qualified baseline
without changing its established encoding.

Revalidation independently qualifies a new complete projection and requires
the exact original paused census and assessment. This deliberately uses a
coarse witness: even an unrelated change in that operations capture can require
a new original plan. Revalidation cannot update the count, shrink or enlarge an
already sealed amount, move the target or renew the deadline.

The complete manager-order membership observation is a separate source.
Protocol 1.10 supplies no recipe or remaining-quantity detail for those rows.
The single-target combined path therefore requires its queue to be empty before
stock-target preparation and insertion. The vector path permits only the exact
receipt-backed prefix of earlier original orders from that same sealed plan.
Neither assumes that an unclassified existing order is unrelated, subtracts
expected future output, or changes protocol 1.10.

## Explicit labor prerequisite

The combined consumer defaults to observed already-enabled workers. A closed
`assign_labor=true` option composes the same qualified stock assessment with
the existing reviewed labor-enable and dependent-order executor. It uses a
separate `Stock target/2` original identity; the `Stock target/1` existing-worker
encoding remains unchanged. The assignment action and complete original
citizen preconditions are sealed with the inventory target, finite deficit and
timing. Each native step requires its own review and confirmation.

A met target never creates the assignment, even when requested. A positive
target requires current labor authority through the ordinary planner. Historical
assignment evidence releases the dependency but cannot replace current source
checks or the inventory goal. Stock and queue drift after labor changes refuses
the order under its original identity, without a new count or replacement key.
The complete parent monitor still checks current labor and stock together.

## A bounded original target portfolio

The `stock_targets` consumer assesses one to four distinct furniture recipes
against a single independently qualified census. It refuses the entire proposal
if any positive deficit is over its limit, creates no effect-bearing original
when every target is met, and emits original orders only for positive deficits. A
wholly met request instead retains the read-only original portfolio described
below. All requested minima remain explicit complete-parent requirements,
including already-met stock.
The selected eligible workers must already have the requested labor enabled by
default. Explicit `assign_labor=true` instead creates one reviewed enable at
StepId zero and one to four dependent orders at StepIds one through four. A wholly
met vector creates neither assignment nor order, even with the opt-in.

The separate `Inventory targets/1` archive identity binds the common census,
all target assessments, workers and unchanged timing. The assignment variant
uses `Inventory targets/2` and a separate identity domain, binding the exact
assignment action and original citizen identity guards. Omitted/false assignment
keeps `/1` bytes unchanged. Positive leaves retain
typed inventory-increase objectives; distinct complete-parent minimum predicates
preserve every target without fabricating an order. Before each real insertion,
the complete census must still match and the native queue must contain exactly
the prior original receipt-backed insertion prefix.

Current complete-parent proof requires independently qualified quantities for
all targets and the original cadence/stability across complete cohorts. A leaf's
historical success cannot supply the missing stability of an initially met target
that was later consumed. An unfinished cohort streak resets on False, Unknown or
failure; a completed in-process achievement remains historical while current
truth determines current proof. Reopening restores no cohort achievement or new
deadline. Original per-order recovery, read-only custody and independently
authorized clocks retain their normal boundaries. Assignment is not a stability
sample: the complete-parent tracker reads timing only from the original positive
order obligations, and clock requests use the actual order StepIds.

### First combined or staged portfolio after restarting before publication

A `labor-order` owner, including the production child of
`workshop-production`, may reopen native custody before any effect-bearing
original exists and then publish its first positive stock portfolio. The
compiler retains a nonserializable witness for the complete original digest,
anchor, compiling session/request and every original order's clock identity.
It is available only to that same first publication and is never reconstructed
from retained recovery metadata.

For multiple positive orders, additional fixed clock paths must be absent.
Each configured family needs its own opening and current permission and current
`ControlClock` authority before any original archive or frontier is written.
After those checks, publication creates the original and exclusively creates
the predicted additional headers. Existing first clock owners remain unchanged.
A single positive order needs no additional header or new-slot authority.
The same boundary also serves manually sized combined portfolios.

This ordinary path has no resource-project parent ACK. The fixed-project
contract below keeps that separate requirement. Neither publication path
prepares or commits a native effect or advances game time. Once writing has
begun, a failure preserves partial custody and prevents another original in that
owner. Recovery of an existing original remains strict: all original journal
identities and files must be present, with no recreation of missing, corrupt,
substituted or omitted slots. See
[COMBINED_PRODUCTION_MCP.md](COMBINED_PRODUCTION_MCP.md) and
[WORKSHOP_PRODUCTION_MCP.md](WORKSHOP_PRODUCTION_MCP.md) for reopening details.

The ordinary Runtime/native-TCP regression cases, including the imported-Workshop
child, are authored but unexecuted for this change. They add no native-plugin,
full staged-profile or live-game qualification.

### First project portfolio after restarting before publication

The fixed resource-project owner can reopen its pre-plan custody and then
publish its first stock portfolio with either or both configured clock families.
Its immutable production intent names every additional fixed clock identity
before the parent acknowledges it. Only that successful current publication
can create those exact new headers, using independent current and opening
clock authority and exclusive file creation. Refused acknowledgment writes no
original or additional clock; an existing or partially created slot cannot be
adopted or repaired.

Retained-original recovery continues to require every previously acknowledged
journal and exact identity. It receives no new-slot creation path. A one-order
portfolio reuses the existing first clock owner. See
[NATIVE_RESOURCE_PROJECT_MCP.md](NATIVE_RESOURCE_PROJECT_MCP.md) for the
publication and recovery contract and the current validation limits.

## Read-only monitoring when no order is needed

The reusable `StockTargetWatch` adapter accepts only an independently qualified
already-satisfied `StockTargetAssessment`. It fixes the original target,
source lineage, absolute deadline, cadence and required sample count. It
creates no order, labor assignment, preparation, clock run or effect journal.

The planning census is historical sizing evidence and contributes zero
stability samples. Later `InventoryBaselineProjection` captures require fresh
independent Item-domain and field policy. Complete empty stock is zero; partial
or unqualified stock remains Unknown. Changed stock is evaluated as a minimum,
without applying order-size limits or rejecting unrelated production jobs.

The first eligible sample is at least one original cadence after the initial
census. Repeated ticks and exact replays cannot increase stability. False,
Unknown, acquisition/publication failure and cursor gaps interrupt an
unfinished streak. A missed absolute deadline cannot be renewed. Source
generation, fortress/restore identity, cursor, tick or allocation regression
fences the owner. Decoded rejection floors survive later policy refusal.

Each result separates current truth from the first timely achievement, names
its complete anchor and source digest, and carries a deterministic witness.
Historical achievement cannot make a later false or missing current count
true. Reopening restores neither an unfinished streak nor current proof.
Authenticated historical checkpoints are described below. The monitor provides no recurring maintenance, material or
usability guarantee, and no causal attribution to production.

The combined single-target consumer now retains this watch after a no-work
Plan. `fortress_wait` or a default `fortress_observe` performs one bounded fresh
citizen/operations observation and independent qualification. The returned
`no_work_stock_goal` reports the exact source, anchor, count, cadence progress,
timely achievement and current proof separately from the unchanged historical
`stock_target` assessment. Query does not acquire another sample. A repeat of
the same Plan retains its timing and progress; changed planning is refused until
the original watch is explicitly cancelled. The explicit continuation below can
pursue the unchanged unfinished goal after a fresh deficit.

This path never dispatches an order, labor change or clock run. The game must
reach later observed ticks for spaced samples to accumulate. Current proof
withdraws after failed publication, cancellation, an unsettled clock, or
contradictory/missing evidence. A later qualified count can restore current
truth after a timely historical achievement. Recovery output carries the
original goal identity with no current sample or proof. The resource-project
parent consumes this typed result, so an initially met single stock target can
complete under its original deadline without inventing production.

### Historical watch checkpoints

`StockTargetWatch` and `StockTargetsWatch` provide bounded canonical checkpoints
for an embedding owner with authenticated private journal custody. Their
separate version-one domains preserve the existing assessment and watch identity
bytes. The complete original target set, deadline, cadence and sample requirement
are immutable. Native tick/allocation limits, canonical cursor, last counted tick,
first timely achievement, expiry and source fencing cannot move backwards across
successive checkpoints. Scalar and portfolio encodings cannot be substituted.

Recovery reads historical custody under current Query authority at the consumed
tick floor. It never restores a current sample or unfinished stability streak.
Historical achievement remains explicit so that a completed goal cannot become
new production merely because its owner restarted. A fresh complete census and
ordinary Observe/Query authority are still required to prove current stock.
Portfolio proof continues to require every target together in one coherent
observation.

The decoder bounds the entire record to 2,048 bytes before allocation and refuses
malformed, noncanonical, truncated, trailing or inconsistent fields. Historical
term inspection returns primitive request data, not a qualified current
assessment. The embedding owner must authenticate the checkpoint and its complete
request against the original private journal; a caller-supplied JSON object or
matching content digest is insufficient. These adapter APIs alone do not make a
standalone combined session durable.

### Resume an authenticated resource-project watch

The resource-project parent retains the complete original request and canonical
watch checkpoint in its existing acknowledged child journal reference. Only that
private parent-custody path can supply a watch to a reopened production child.
The original workers, labor choice, assignment option, recipes, minima, finite
caps, observation selector and timing stay fixed. First watch publication requires
current Control and Plan authority before the parent writes or acknowledges it.

Recovery verifies the original native and prerequisite custodians before restoring
historical watch ownership. Missing, substituted, noncanonical or regressed custody
refuses; an operator-supplied count or recovery JSON does not reconstruct a lost
watch. Legacy references without watch custody remain legacy references. No new
sidecar, public tool or native protocol is introduced.

Wait and default Observe can then gather fresh complete scalar or portfolio
samples under the unchanged deadline. Unfinished stability starts again; past
timely achievement, cancellation, expiry and source fences remain historical.
A still unfinished eligible watch can use the existing explicit
`continue_stock_goal` request after fresh stock loss, preserving the same
workers, target set and timing. Completed or cancelled watches cannot become
maintenance. Pending first-production recovery retains whichever authenticated
watch checkpoint has the newer comparable source history; incompatible histories
are rejected rather than merged into invented evidence.

An acknowledged parent cancellation dominates its no-order watch even when the
process died before the child recorded cancellation. The reopened child withdraws
proof and continuation guidance without native I/O. Independently retained stock
clocks remain available for their existing pause/reconciliation and cancellation
paths; no new run authority is restored. Positive production originals retain
their own unchanged execution and predecessor history.

Standalone combined sessions remain process-local. The parent-journal integration
and its actual Runtime/private-file/TCP regression sources have separate evidence
status in `IMPLEMENTATION_STATUS.md`; adapter checkpoint tests do not qualify
the complete MCP or native recovery flow.

### Keep every already-met target stable together

The reusable `StockTargetsWatch` accepts one through four distinct, already-met
assessments from the same complete census. Recipe ordering is canonical. Its
immutable witness binds every original assessment and the one shared deadline,
cadence and required sample count. Each sample qualifies every requested minimum
against one `InventoryBaselineProjection`, with one exact anchor, source digest,
generation and allocation horizon. A missing or failed qualification cannot
publish a partially successful target set.

The portfolio owns one joint stability streak. All targets must be True together
in each eligible cohort; independent achievements at different ticks cannot be
combined. For example, two good bed samples followed by two good door samples
do not establish a two-sample portfolio while either stock was missing from the
other samples. The original planning census contributes zero samples. False,
Unknown, cursor gaps and failed acquisition or publication interrupt unfinished
joint stability. Repeated ticks do not advance it. A timely achievement remains
historical, but current proof additionally requires a complete current cohort
in which every original minimum is True. Missing the original deadline cannot
be repaired by repeating Plan or replacing an individual target.

An all-met `kind="stock_targets"` Plan retains this monitor through the existing
`no_work_stock_goal` field, tagged with schema `dfmcp.stock-targets-watch/1` and
scope `original_no_work_inventory_portfolio`. Its `targets` preserve every
original minimum; `sample.targets` carry the independently qualified current
counts or null and each comparison truth. The sample also carries the joint
truth, stability count, exact source and timely achievement. The separate
`stock_targets` assessment remains historical and does not itself prove current
inventory. This inventory-only goal does not establish a workforce prerequisite,
including when the original request used `assign_labor=true`.

Wait and default Observe acquire one complete current portfolio cohort per call.
Query inspects the retained result without native I/O. Repeating the identical
Plan preserves the original target set, deadline and progress; changing a target,
timing field or request mode requires explicit cancellation first. Use the
existing Cancel with its default scope or `scope="step"`; a replacement request
must name a valid retained observation, so Observe again when that selection
has been consumed by sampling. Cancellation and failed publication
withdraw the complete current proof. Decoded native and canonical rejection
floors survive later authority, policy or budget failures, and source or identity
regression fences the complete watch. Recovery output preserves original target
identity without a current sample. Standalone reopening restores no joint
achievement; authenticated resource-project reopening retains historical joint
achievement, but never an unfinished streak or current proof.

This path creates no insertion keys, effect-bearing original, native preparation,
labor assignment, clock run or goal-history sidecar. Sampling reports stock loss
without automatically creating an order. Scalar and one-to-four-target native
resource-project roots can consume the typed watch proof. An explicit
continuation can pursue an unfinished watch as described below.
The adapter sources contain eighteen portfolio regression cases alongside the
fourteen single-target cases. Runtime/native-peer and presentation coverage
address the public sampling, cancellation and proof-withdrawal boundaries.
Execution evidence for these bytes belongs in `IMPLEMENTATION_STATUS.md`;
source presence and authored tests add no qualification or admission.

## Continue an unfinished watch after stock loss

An initially satisfied goal can lose stock before its required stability
interval finishes. In a current Control session, use the existing Plan tool
with this closed request:

```json
{
  "kind": "continue_stock_goal",
  "goal_witness": "<exact retained no_work_stock_goal.goal_witness>"
}
```

The selector names the retained scalar or complete portfolio watch. It accepts
no replacement targets, workers, observation seal, order amount or timing.
The original recipe set, minima, finite order caps, selected citizens, labor,
assignment choice, absolute deadline, cadence and sample requirement remain
fixed. A current False sample can expose this request in the Agent Turn; the
request itself always reacquires and qualifies a complete fresh citizen and
inventory basis before computing any deficit.

The watch must still be unfinished, uncancelled, unfenced and inside its
original deadline. A previous timely achievement makes it ineligible even if
the latest sample or current proof was later withdrawn. Completed goals do not
become recurring maintenance. A resource-project parent can restore the exact
original watch from authenticated journal custody. Historical recovery JSON alone
cannot reconstruct a lost standalone process-local watch.

Continuation requires current Plan and production authority, plus labor
authority when the original request selected assignment. It rechecks eligible
original workers, complete current inventory, all required Workshop/Manager/input
prerequisites and a fresh empty paused native order queue. Only positive
deficits create finite orders; every original portfolio minimum remains in the
complete goal. An oversized deficit or a still wholly met census refuses
publication. A stale False sample cannot determine an order's amount.

The resulting first effect-bearing original includes the predecessor witness
in a closed `dfmcp-stock-watch-continuation/1` summary marker and a separate
identity domain before deriving intent IDs, idempotency keys and obligations.
Legacy plans without that marker keep their exact prior identity bytes.
Archive recovery checks the complete marked original; its predecessor marker
is historical identity and restores no watch, proof, permit or authority.

Before durable publication starts, refusal retains the same watch and original
deadline while withdrawing current proof and unfinished stability. Decoded
source, tick and allocation floors are retained, and source regression remains
a fence. Once first publication starts, partial custody remains fenced and
cannot become permission to create another original. A resource-project parent
acknowledges the exact production intent before child files are created, using
the existing first-publication and strict recovery path.

Plan publishes a finite original only. Each labor/order effect still needs its
own subsequent review and explicit Commit; production and inventory clocks
retain their separate review/confirmation. No elapsed watch samples are credited
to the new production obligation. The new stock-goal proof must satisfy the
unchanged deadline and stability requirement from qualified current evidence.

## Evidence boundary

The assessment itself observes no source, persists no effect, inserts no order
and grants no mutation or clock authority. Its positive result is source-bound
planning data. The host must preserve it in the original identity and perform
fresh checks at the real native boundaries. A queue insertion receipt remains
distinct from current inventory truth and from a complete original goal proof.
The existing registry and production runner map are unchanged.

The machine contract is
[native_stock_target_v1.json](../architecture/native_stock_target_v1.json).
Executed checks and current-source limitations are recorded in
`IMPLEMENTATION_STATUS.md`; test-source presence does not imply execution.

## Focused acceptance source

The `stock_target_request` and `stock_target_binding` test modules cover
explicit opt-in, exact assignment/order layout, original identities, archive
round trips, bounded deficits and rejection of weakened recovery predicates.
The vector request/binding/runtime and presentation sources additionally cover
whole-proposal bounds, no-action minima, exact original queue prefixes, lost-reply
recovery and whole-parent temporal stability. The assignment vector adds exact
StepId offsets, explicit authority and full five-action budget coverage.

The runtime suite uses the real combined Runtime and native clients against
the existing TCP fixtures. The single-target assignment campaigns cover eleven cases,
including independent step confirmation, zero work, missing current labor
authority, action budgets, source drift after assignment and original-key
lost-reply recovery without redispatch. Presentation tests retain assignment
intent and actual StepId through compaction and overflow while withdrawing
current proof and review. The vector campaigns cover eleven existing-worker
cases and fourteen assigned-worker cases, including two and four actual orders,
exact prior-order queue prefixes, no-action target loss and independent clocks.

Run the focused suite in a working latest-nightly environment with the locked
dependencies:

```bash
cargo +nightly test --locked --offline -p dfmcp-mcp --lib stock_target -- --nocapture
```

The previously recorded focused component run passed 78 live-production
adapter tests and 418 Runtime/presentation tests, including 66 presentation tests
and all 25 vector Runtime/native-peer cases. Those historical results do not
validate the subsequently added no-work watch and consumer. Full MCP build/check is still
memory-blocked in the unchanged pinned runtime dependency; component execution
excludes MCP transport/supervision and admission. Execution details and full
qualification limits are recorded in `IMPLEMENTATION_STATUS.md`. Deterministic native-peer results do not establish
real-game production or production admission.
