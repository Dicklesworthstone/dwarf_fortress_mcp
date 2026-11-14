# Wait for several original laboratory projects

The existing `fortress_wait` tool accepts an explicit set of one to eight
original plans. One call coordinates the shared clock and inspects every
selected project at each published frontier. This matters for future reserves:
separate scalar waits can advance beyond another original's exact reserve
horizon before its first endpoint has been observed.

Owner: `df-lab-selected-plan-wait-s4q.1`. The machine contract is
[`laboratory_selected_plan_set_wait_v1.json`](../architecture/laboratory_selected_plan_set_wait_v1.json);
the request schema is [`lab_wait.schema.json`](../schemas/lab_wait.schema.json).
This is the deterministic reference laboratory workflow. Native profiles retain
their independently defined authority and execution boundaries.

## Request and completion policy

Supply `plan_digests` with the original digests returned by ordinary Plan and
Commit. Set `plan_policy` to `all` (the default) or `any`:

```json
{
  "session_id": "<owning session>",
  "max_game_ticks": 2000,
  "plan_digests": ["<first original digest>", "<second original digest>"],
  "plan_policy": "all"
}
```

`all` waits until every selected original is complete at the same current
frontier. `any` can stop when at least one is complete. Every frontier reevaluates
every member, including a project that completed earlier: consumed food or drink
can make its current goal false again. Historical completion is not latched into
a later group success.

Completion combines current original project truth, verified original actions,
physical quiescence and any required exact reserve endpoint. A completed harvest
stage does not complete the room or production project it supplies. An
exceptional outcome in any member takes precedence over either success policy.
The response lists every attention cause in canonical actual-digest order; its
first cause determines the group stop reason.

`plan_digest`, `plan_digests`, `watch_id` and `watch_ids` are mutually exclusive.
`plan_policy` requires a non-null `plan_digests`. Empty sets, more than eight
members, malformed digests, duplicate digests and aliases for the same actual
original are rejected. Existing scalar and watch wrappers remain available,
and the server still registers eleven logical tools.

## Original identity, recovery and time

Selection accepts a complete current-session original commit or a verified
durable recovered original with its full anchored action custody. Requested
aliases remain paired with actual original digests in the response. Selection
does not adopt another live session's originals, follow descendants, replace a
pending review, change latest-plan orientation or recreate recovered dispatch
handles.

Positive time requires current ControlClock authority; the entire requested
unpaused horizon must fit the observation, query and clock grants and negotiated
tick budget before progression. Each later frontier rechecks current authority.
The shared progression owner continues ordinary authorized work and clips time
to action/recovery boundaries and each selected future reserve horizon. A wait
never unpauses, prepares a continuation or cancels another project.

A zero-tick request inspects the current frontier and may capture explicitly
selected exact reserve endpoints after complete admission. It advances no time
and dispatches no successor. Add `as_task: true` with zero or omitted ticks to
retain one passive Task for the same set and policy. Its reads and cancellation
leave original work unchanged; explicit foreground waits supply time and endpoint
capture. See [group observation Tasks](LAB_MCP_TASKS.md#retain-one-task-for-several-original-projects).

## Complete publication and durable endpoints

The complete selection, source union and prospective response are admitted
before any endpoint is written. The packet includes complete per-original
outcomes, objective summaries and current retained work. An insufficient initial
output allowance cannot capture the first member and then reject a later one.

Each original endpoint retains the existing independent immutable `E` record.
The set is not a new atomic journal transaction. If one append succeeds and a
later append fails, the response preserves the accepted prefix, identifies the
failed original, marks inspection/publication incomplete and reports actual
elapsed progress. No further frontier is attempted. A first false or unknown
endpoint remains immutable; later inventory cannot repair missed or contrary
historical evidence.

After the final append, the owner returns the already admitted Agent Turn
without another fallible projection or response fitting pass. Successful
selected evidence and the objective summary therefore describe the same retained
endpoints. Later errors retain selected identity and actual progress subject to
current read authority, and cannot claim a complete current group inspection.

## Reading the response

`selected_plan_set_wait` uses schema `dfmcp.selected-plan-set-wait/1`:

| Field | Meaning |
|---|---|
| `requested_plan_digests`, `plan_digests` | Paired requested and actual identities, ordered by actual digest. |
| `plans` | Complete scalar selected-plan outcomes for every member. |
| `group_complete`, `all_complete`, `any_complete` | Current inspected outcomes, with exceptional attention blocking group success. |
| `inspection_complete`, `evaluated_anchor` | Whether the entire set and endpoint publication completed at this frontier. |
| `attention`, `interrupted_plan_digests` | Every exceptional member and its reason. |
| `unfinished_plan_digests` | Members not currently complete, including pending peers when `any` succeeds. |
| `endpoint_publication` | Independent-record mode, publication status and any failed original. |
| `source_work` | Shared measured source costs, inspection count and applied bounds. |
| `proof_work` | Separately labeled conservative selected-predicate node charges. |

Top-level `wait_progress` continues to report actual elapsed/remaining ticks,
polls and dispatches. A tick-limit recommendation retains the selected original
set and policy. An attention recommendation inspects or separately reviews the
canonical first affected original; it grants no authority and executes nothing.

## Bounds and replay

Each frontier admits one complete union of the current world, distinct exact
historical endpoint snapshots, retained original sources and selected custody.
Shared snapshots count once by exact anchor. Original steps and evaluation work
share the request allowance, with one wall deadline, at most 4096 foreground
slices and 4097 inspections. Measured repeated inspection work is reported;
eight selected originals do not receive eight independent byte allowances.
Selected goal, stage and reserve evaluations share a 4096-node bound per
frontier. Sealed predicates contribute their complete node counts; the closed
semantic production/room models receive conservative node reserves. `proof_work`
labels these upper-bound charges explicitly, without presenting them as measured
runtime node visits. The shared wall deadline also covers retained-work and
continuation projections.

Replay records the caller's exact array order and optional policy. Eligible
private volatile sessions replay through the same selection path. Shared and
durable sessions retain their existing nonreplayable status. Malformed replay
argument types are refused before opening a disposable session; well-typed
semantic refusals are reexecuted exactly and never converted into ordinary
duration waits.

Execution evidence and remaining qualification limits are recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md). Source presence alone
does not establish full-workspace, transport or live-game qualification.
