# Continue an interrupted furnished-room project

The laboratory can resume a retained `provisioned_room` request through the
existing `continue_goal` blueprint. A new reviewed pursuit keeps valid completed
furnishings, reuses eligible loose or released items, and plans the missing work.
The original layout, every furnishing slot and all supplied setup choices remain
part of the request. This is reference-simulator behavior; native mutation and
live compatibility admission are unchanged.

## Request and review

Pass this object as the JSON string in the existing `fortress.plan` tool's
`blueprint` argument:

```json
{
  "template": "continue_goal",
  "plan_digest": "<retained committed provisioned-room plan digest>"
}
```

The digest can name the original room plan or an earlier room continuation.
When eligible, the original-goal observation provides a `continuation_request`
with the session and digest already filled in. The server retrieves the complete
original request. The caller cannot supply a replacement layout, remove a room,
change setup permissions, inject lineage or assert that earlier work is quiet.

Planning performs no game effects. Review the new plan, its forecast, exact
furniture allocations, `retained_placements`, `new_placements`, and continuation
lineage before explicitly submitting its new seal to `fortress.commit`.
Production, excavation and placement retain their ordinary action authority,
verified-completion dependencies and checkpoint requirements.

For example, a dining-hall plan may manufacture a table and a chair, complete
the table installation, and cancel the unfinished chair placement. After the
earlier work is proven quiet, continuation preserves the table's exact building
and item, reuses the released chair, and proposes one new chair placement. It
neither manufactures a duplicate table nor debits stock for the retained table.

## What is retained and what is replanned

A retained furnishing must occupy the exact singleton tile and have the exact
kind required by an original slot. Its construction must be complete and its
current evidence must establish the original building, item and producer
generations, eligible material and production ordinal, and reciprocal exclusive
item/building custody. Its tile must still have the room's required terrain.
An observed name or `construction_stage="complete"` field is insufficient.

Only remaining slots participate in inventory allocation and manufacturing.
Eligible loose items are selected in canonical identity order, including items
released by properly cancelled placements. If more are needed, the existing
finite-WOOD production compiler creates the deficit. A fully retained furniture
kind requires no new stock evidence for that kind. Retention itself creates no
item, stock credit, allocation or refund.

Installed tiles are excluded from new excavation effects. Affected excavation
rectangles are split into bounded disjoint pieces; already satisfied pieces can
be omitted. Missing placements depend on all remaining excavation pieces for
their original room. The final goal still includes **every original terrain
region**, including installed tiles, corridors, doorways and already-dug areas.
No part of the complete room goal is dropped because it happened to be satisfied
when continuation was planned.

Unrelated occupancy, unfinished or unknown installation custody, a duplicate
slot or item, changed generations, unsafe or unavailable terrain, insufficient
materials and overlapping setup sites refuse explicitly. The original bounds
remain: sixteen furnishing slots, sixty-four combined actions, and bounded
source, geometry and evidence domains. A request requiring more excavation
pieces than the action bound is refused.

## Preserve original history

Each placement's exact building identity derives from its original action key.
A replacement placement therefore belongs to the new pursuit. Completing it
does not rewrite an old cancelled action, manufacture an old receipt, or prove
that the original action created a building it never created.

The child goal binds both retained original installations and newly planned
installations. Successful execution proves the complete room request for that
reviewed pursuit. The original plan's exact terminal condition, action outcomes,
source and first achievement remain unchanged. Its current exact goal can stay
False even after the child satisfies the complete room request.

Continuation lineage makes this distinction explicit:

```json
{
  "semantic_request_preserved": true,
  "historical_exact_goal_unchanged": true,
  "completion_proves": "this_reviewed_pursuit_only",
  "requires_explicit_commit": true,
  "replacement_work_dispatched": false
}
```

After a same-root pursuit currently proves the complete goal True and its work
is quiet, another continuation of the old root is refused with the satisfying
pursuit's digest. The old root's continuation affordance is suppressed without
altering its exact historical proof.

## Current authority, quiet work and stale evidence

Continuation requires current Observe and Plan authority, a reconstructable
nonabandoned goal in a verified active observation lineage, and current False
goal evidence. Unknown evidence requires reconciliation. Earlier physical work
and deferred actions must be proven quiet, including every retained pursuit
sharing the original root. Terminal receipts alone do not establish that.

These checks run again at commit before leases, reservations, durable admission
or effects. A reviewed proposal gives no permission to duplicate a sibling that
started work meanwhile. The existing unchanged-seal retry exception applies only
to the identical already-admitted candidate; a new seal cannot inherit it.

Every changed room-continuation anchor requires reviewed source replay, even
when only excavation remains and every furnishing is retained. The ordinary
action witness does not cover the complete inventory and occupancy census or
the generations of retained installations. A changed generation or custody
claim must be rechecked before a new seal can be reviewed.

## Durable source and compatibility

The private source uses `dfmcp.room-continuation/1` and fixes its compiler to
`provisioned_room_residual_v1`. Its flat envelope retains the parent and root
digests plus the entire original canonical `provisioned_room_v1` request,
including currently unused prerequisite permissions and workshop sites. Its
source digest is bound into the new intent summary and seal. Repeated
continuations do not nest or enlarge the lineage envelope.

The durable journal appends a distinct `room_continuation` source tag. Archived
reconstruction compiles the exact source at its original snapshot and intent ID;
it does not consult the current mutable goal book, grant authority or dispatch
work. Verified durable reopen preserves pursuit continuity under the existing
recovered observation frontier. Checkpoint restore abandons old pursuits, so it
cannot implicitly turn them into current work.

Ordinary new room requests keep the original `provisioned_room_v1` compiler,
source digest and exact seals. Legacy abstract `bedroom_cluster` and `dining_hall`
blueprints do not become resumable item-backed goals. No tool is added to the
eleven-tool surface. See [item-backed furniture](LAB_ITEM_FURNITURE.md) for the
original room request and custody model, and
[production continuation](PRODUCTION_GOAL_CONTINUATION.md) for the shared
authority, recovery and quiescence rules.
