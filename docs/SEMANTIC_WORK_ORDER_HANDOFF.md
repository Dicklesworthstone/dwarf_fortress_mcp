# Semantic CreateWorkOrder handoff

`dfmcp_adapter::semantic_work_orders` connects one original sealed
`PreparedPlan` containing `CreateWorkOrder` to the existing native
`WorkOrderSession`. It preserves the original semantic action key, full plan
identity and obligation through review, durable preparation, one native insertion
attempt, inspection and restart recovery.

The handoff uses the existing work-orders/1.10 protocol and coordinator. It is
an embedding API for the trusted operator-owned acquisition shell. The generic
MCP route remains advisory, and this module supplies no compatibility admission,
source-policy issuer, bridge method or new production runner.

## Supported original plans

The original plan contains exactly one unconditional work-order action, for
one of the existing wooden furniture recipes, with an amount from 1 through 100:

| Native recipe | Accepted semantic job tokens |
|---|---|
| Wooden bed | `CONSTRUCT_BED`, `wooden_bed` |
| Wooden door | `CONSTRUCT_DOOR`, `wooden_door` |
| Wooden table | `CONSTRUCT_TABLE`, `wooden_table` |
| Wooden chair | `CONSTRUCT_CHAIR`, `CONSTRUCT_THRONE`, `wooden_chair` |

The original name remains sealed semantic metadata. This native protocol does
not add a manager-order name or support named-order conditions. Brewing and
cooking are not native recipes in this generation. Conditions, dependencies,
compensations and mandatory checkpoints are refused rather than discarded.
Only `True`, `Paused(true)` and bounded conjunctions of those preconditions are
protected by this exact queue capture. Other original postconditions and goal
predicates are retained for later proof, with bounded shape and size checks.

The entire original `PreparedPlan` is validated, including its digest, action
metadata, stable key and obligation. The plan and obligation must still permit
control at the exact original canonical anchor. Control requires current Query,
Plan and ConfigureProduction grants; Plan and ConfigureProduction must cover
the fortress-wide operation at Guarded risk. Narrow entity/map grants and
use-limited grants are not widened by the wrapper. Inspection and reconciliation
have separate historical authority checks; they do not renew those deadlines.

## Acquire and review

Open the existing native work-order journal/session under current authority.
Use its verified journal ID to open a separate semantic `AssociationStore`.
The operator supplies the private file location; MCP arguments cannot supply
it. Then construct `SemanticWorkOrderSession::new(native, associations, context)`.
Every native key already present must have the corresponding exact semantic
association. An empty or replacement store cannot adopt old native work.

Supply a `WorkOrderEvidenceOwner` whose observing shell owns canonical
acquisition and independently authorized routing evidence. The existing concrete
`CitizenWorkforceEvidenceOwner` implements this interface: it reacquires the
complete citizen capsule and checks its separately supplied policy issuer.
The queue capture alone cannot grant source authority or invent a canonical
observation cursor.

`observe(original_plan, evidence_owner, context)` first obtains the complete
native queue, then refreshes independent canonical evidence. The exact fortress,
world/site, bridge generation, paused tick and canonical anchor must agree.
`resolve_work_order_step` constructs the native candidate using the original
semantic key and closed recipe. The returned `SemanticWorkOrderReview` exposes
`original_plan()`, `native_plan()` and the paired `seal()`.

That review seal covers the semantic digest, native digest, original anchor,
source digest, native queue witness, step, original key and native journal
identity. The operator confirms this paired seal; the native digest alone is
not the semantic review.

## Prepare, commit and read the result

`prepare(seal, evidence_owner, context)` publishes the immutable association
before contacting native preparation. Publication appends the frame, syncs it
and verifies every stored byte under exclusive custody. The coordinator then
refreshes and revalidates independent source evidence immediately before native
preparation. An existing unresolved native creation blocks a new association
before it is written.

`commit(seal, confirmed, evidence_owner, context)` checks the exact review and
current authority. The native coordinator durably publishes `DispatchStarted`
first. Only after that sync does the final guard refresh independent canonical
evidence and recheck original source, paired custody, native manifest, deadline
and authority. The sole insertion call follows that guard. A failure after
`DispatchStarted` retains uncertainty and fences the native source; it cannot
return the record to Prepared for another insertion attempt.

`SemanticWorkOrderResult::creation_result()` is either `Unverified` or
`NativeOrderCreation::Inserted { native_order_id, receipt }`. Inserted requires
strict native Created readback for the original plan. It establishes insertion
of that manager order only. It does not prove workshop execution, produced
items, the original postconditions or stock goal. `original_goal_proven()`
therefore remains false, and `pending_obligation()` returns the unchanged
original obligation. No background goal monitor or production replenishment is
started by this handoff.

## Restart and uncertain effects

The embedding owner must retain the exact original semantic plan. The separate
association journal retains its identity and source binding; it is not a
PreparedPlan decoder and does not reconstruct permissions.

For historical recovery, reopen the native coordinator in Reconcile/Offline
mode and the semantic store read-only under current Query authority, then call
`reattach(original_plan, context)`. A Control-mode reopen still requires current
control authority. A changed original goal, key, source anchor
or digest cannot attach to old native work. `inspect(seal, context)` returns
historical native state with the original semantic review attached.
`reconcile(seal, context)` can query the original native receipt after an
uncertain dispatch; it never inserts again. Missing native retention and Unknown
remain unresolved and block fresh creation.

An undispatched Prepared record may call
`refresh_prepared(seal, evidence_owner, context)` to reacquire the exact unchanged
queue and source before control. This neither renews preparation lifetime nor
changes the review seal. If semantic intent survived but native preparation was
never recorded, observing the exact original plan is required again.

`cancel(seal, context)` retires only a local, undispatched Prepared record under
current control authority, including after original plan expiry. It cannot
remove a Created manager order or cancel an indeterminate effect. Read-only
association recovery and Offline/Reconcile native modes cannot dispatch.

## Chronology and storage bounds

The owner keeps canonical cursor/epoch and native intervention sequence/tick
separate. Every view reconstructs each bounded retained history in sorted order
and validates it before merging the current observation maximum. Thus a newer
observation cannot hide a fork in older stored history. A new evidence owner or
process cannot use a source older than another retained goal to authorize new
control. Native Created readback advances the retained native sequence; it does
not advance a canonical cursor. Restore epochs and bridge incarnations must be
established by their independent owners, not by this handoff. Retained control
custody covers one restore epoch and native incarnation; this module does not
migrate retained control across either boundary.

Historical queries remain available without requiring the old anchor to become
current. Within the retained epoch, grant expiry is evaluated no earlier than
the latest retained canonical/native tick. Old authority cannot be revived by
passing an older context.

The semantic journal uses the distinct `DFMSOJ01` codec, at most 64 immutable
associations and 64 KiB of stored bytes. The existing private-file shell requires
an absolute normalized operator path, a real 0700 parent and a 0600 regular file
with exclusive locked custody. It rejects symlinks, aliasing, wrong family
bytes, torn frames and replaced storage. There is no repair or history pruning;
an ambiguous append or sync fences the handle until verified reopen.

Acquisition, full queue views, source verification, storage verification and
native calls consume explicit reservations from one shrinking operation budget.
Each evidence refresh reserves 16 MiB, and the whole call reserves all required
local, view and native work before effectful boundaries. The source plan is
bounded to 64 KiB of predicate/text content and 512 predicate nodes. Every
session operation also retains a maximum 60-second wall deadline. Direct store
open/verify operations use their explicitly supplied operation budgets. A small
budget refuses before new semantic persistence or native preparation/insertion.

## Validation scope

The public integration target `semantic_work_orders_isolated` exercises the
actual adapter library, strict native wire/effect codecs, independent evidence
owners, both real and injected storage, guarded coordinator and restart paths.
Its cases cover all supported recipes, complete original-plan retention,
lost Created replies, Unknown preservation, source loss after dispatch sync,
association/native sync failures, original preparation recovery, current grant
expiry, cross-goal chronology, forked cursors, changed incarnations, real private
file reopen and malformed custody. The complete citizen acquisition owner is
exercised through preparation and commit.

All 21 integration cases pass. All 18 existing work-order coordinator cases,
all 43 workforce integration tests and all 93 production module tests also pass,
with warning-denied adapter-library Clippy and 644 unchanged inventoried inputs.

This evidence uses synthetic native responses and real local storage. It does
not establish a DFHack plugin build, live-fort campaign, native mutation
admission or full-workspace qualification.
