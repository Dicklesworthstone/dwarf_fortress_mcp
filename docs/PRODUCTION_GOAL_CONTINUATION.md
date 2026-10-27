# Continue a retained original production goal

The same public `continue_goal` request also resumes retained item-backed rooms.
See [room goal continuation](ROOM_GOAL_CONTINUATION.md) for its exact installation
retention and separate child-goal proof. The production semantics below remain
unchanged.

The laboratory can compile a new pursuit of a retained production goal after
its earlier work is quiet and its original stock requirements no longer hold.
The new request uses the existing `fortress.plan` tool and `blueprint` argument:

```json
{
  "template": "continue_goal",
  "plan_digest": "<digest of the retained committed production goal>"
}
```

Pass that object as the JSON string in `blueprint`, just as for the production
and room templates. An eligible original-goal observation supplies a complete
`continuation_request` with the session and exact original digest already filled
in. It is a proposal to plan, not permission to dispatch. A custom `summary` is
optional. Do not combine this template with explicit actions, production, or a
pause target.

## What the new plan preserves

The server retrieves the complete normalized original production request from
its goal book. Every DRINK/FOOD quota remains, including a quota that required no
order when the original plan was compiled. Opt-in labor assignment permission
and every explicit Still/Kitchen site also remain. The caller cannot replace
those requirements, submit lineage assertions, or claim quiescence in the
continuation JSON.

The compiler uses current source-qualified inventory to calculate new deficits
and reuses currently completed setup. New continuations without an explicit
future reserve horizon choose the fixed `consumption_aware_v1` generation,
which reserves population consumption through
the complete sealed schedule while preserving every original minimum. It uses
the ordinary finite-capacity model, queued-service allowance, action
capabilities, checkpoint rules and conditional orders. Future output from
existing work is not stock. See `PRODUCTION_RESERVE_PLANNING.md` for the
consumption assumptions and bounds.
An active same-output producer causes explicit overlap refusal even if it belongs
to an unrelated goal.

For example, an earlier plan can finish four brewing batches and later
consumption can leave only 53 drinks against the original minimum of 60. Once the original work is proven
quiet, continuation proposes two new batches at the current world anchor.
The new batches have new action identities. The old plan, action receipts,
source and first verified achievement remain unchanged. Fresh observed stock
can subsequently prove the original goal true again.

## Review and current authority

Planning requires current Observe and Plan authority. The original goal must be
retained, reconstructable, in the same fortress/restore epoch, and currently
False under eligible current evidence. A True goal is ineligible for
continuation; an Unknown goal requires reconciliation. Restoring a world abandons its old
pursuits, so an abandoned goal cannot be continued implicitly.

Every original effect must be physically quiet. Terminal receipts alone do not
prove that; missing live deferred handles and unknown recovered work remain
unresolved. The same check covers all retained pursuits sharing the original
root. Two sessions may independently review proposals while earlier work is
quiet, but the first committed pursuit prevents the other from duplicating its
active or uncertain work.

`fortress.plan` returns a new pending seal and performs no game effects. Review
its steps, forecast, requirements and `production.continuation`, then pass that
new digest to `fortress.commit`. Commit repeats current authority, original-goal
truth and complete-lineage quiescence before leases, durable admission,
reservations or effects.

If the world anchor changes, the existing production witness cannot certify all
workload range/absence reads. The server must produce a newly reviewed source
replay. An exact candidate already admitted by a failed commit may be retried
idempotently under its unchanged seal; the exemption never carries into a new
seal after an anchor change. Its already sealed compiler generation remains
unchanged; lineage validation compares the original request independently of
which fixed generation a new continuation would choose. An unresolved admitted
candidate then requires
inspection or explicit recovery, rather than another pursuit.

## Durable reopen preserves a pursuit; checkpoint restore abandons it

A durable laboratory reopen changes the observation epoch so old action handles
and cursors cannot become current authority. That reset does not itself abandon
a retained production goal. Recovery verifies the exact persisted head, computes
its expected cursor reset and state hash, and requires that complete anchor to
match the actual recovered adapter. It then reconstructs each original plan
from its exact archived source and verifies its recorded history before binding
the goal to that recovered observation frontier.

New continuation planning requires current evidence to advance that verified
frontier within its bound epoch. A caller-supplied newer epoch, backward tick or
cursor, same-cursor fork, changed world bytes or goal without verified continuity
cannot create a new pursuit. Repeated genuine durable reopens establish new
process-local continuity while preserving original plan digests, source bytes,
first achievement and parent/root lineage.

Checkpoint restore follows a separate path: it abandons the old pursuits and
clears their active continuity immediately, even before durable publication
finishes. The recorded abandonment survives reopen. Current authority, original
False truth and physical quiescence of every same-root pursuit are still checked
when planning and committing a continuation. Recovery neither dispatches old
actions nor recreates their grants or preparation lifetimes.

## Preserve original steps that were never dispatched

A production plan can lose its process while workshop construction is active
and the dependent original order is still prepared. Recovery observes the
existing construction but never dispatches that old deferred order. Once
construction finishes, the original stock goal remains unmet. A new reviewed
continuation must be able to reuse the workshop and finish that same goal.

The goal book now retains positive nondispatch custody after the original
action commit retires. Retirement copies only a step's latest explicit
`not_dispatched` state with its exact atomically recorded world anchor. The
original objective owns that bounded map independently of the retired action
commit. Journal compaction retains each referenced snapshot, and subsequent
reopens recover the same original source, seal, step and anchor.

Both the first recovery of an active commit and later recovery of a retired
goal verify that evidence against the original sealed plan. The archived
snapshot must have its recorded hash and complete anchor, follow the original
seal, precede the verified recovery head, and contain no contradictory creator
under the original step's exact identity. Current contradictory creator presence
also blocks the claim. Invalid or unanchored legacy evidence stays unresolved;
even a physically completed conflicting creator cannot bypass reconciliation.
Current entity absence alone never creates retirement custody.

On the first opening, a final recovered `NeverDispatched` row carries the proof
while other original work finishes. On later openings, the verified retired map
provides the evidence without reconstructing an action handle. Authorized goal
observations expose `retired_undispatched_steps`, with each original step, proof
anchor and `verified_retired_dispatch_custody` basis. These historical facts do
not make a currently unmet stock goal true, supply current grants or permit old
idempotency keys to be dispatched.

The journal uses new `W` atomic-progress and `R` retirement records for custody
transfer; legacy progress and retirement records keep their previous semantics.
Compaction writes each objective source separately from its bounded `N` custody
record, preserving the existing source and ordinary-record limits. At most 256
step anchors can belong to one retained goal. Unknown objectives, live-commit
imports, duplicate or out-of-order custody, missing snapshots and mismatched
anchors refuse. An uncertain append fences further publication until verified
reopen reconciles the durable frontier.

## Lineage, restart and bounds

Continuation responses and goal observations identify both the immediate
`parent_plan_digest` and the original `root_plan_digest`. They also expose a
`continuation_source_digest` covering the complete original request and lineage.
That digest participates in the actual prepared-plan summary and therefore its
seal, including when a custom human summary is supplied. Changing an unused
setup site cannot leave the continuation seal unchanged.

The durable `production_continuation` source kind retains one flat versioned
envelope. New continuations without an explicit future reserve horizon use
`dfmcp.production-continuation/2`; historical
`/1` envelopes retain their exact canonical bytes, compiler and digest domain.
The `/2` envelope selects the new compiler without rewriting the original
request, including when that original request had no compiler field. A second
continuation changes its parent while keeping the same root and original
request; it does not recursively embed older sources. On restart, archived plan
reconstruction uses only this source's fixed generation and its exact sealed
snapshot. It does not inspect the current goal book,
reconstruct grants, or rerun effects. New planning and commit use the current
book and current authority again.

The existing bounds remain: 16 KiB for a retained source, the ordinary planner's
summary limit including the source seal, and 64 retained goals per fortress.
Unfinished, consumed, unknown or unverifiable goals are not silently evicted to
make room. Legacy action, pause and room sources are not upgraded into production
intent. An unavailable parent or exhausted goal book produces an explicit
refusal.

Task results keep original-goal proof separate from action completion. When the
original work finished and the goal is unmet, an eligible Task result can return
the continuation planning request as its next step. Compact Task evidence and
handoff data retain lineage; no background replacement work is dispatched.

## Historical /1 validation

Ten new pure-module tests execute the actual continuation codec/compiler,
StaticPlanner, MemoryAdapter, finite-capacity effects and source authority. They
cover consumed-goal recovery, a quota initially satisfied, unknown/true/active
refusal, current authority/epoch checks, changed lineage and unused-site seals,
exact source replay, bounded flat repetition, and closed public/durable inputs.

A durable-store regression executes objective retention after action retirement,
compaction and reopen; the source roundtrip test also covers the distinct new
tag. The final runs pass all 96 laboratory tests and all 65 actual-source
production/projection/continuation tests. Focused warning-denied Clippy also
passes for the laboratory, adapter library and actual source modules. All
inventoried source inputs remained unchanged through these runs.

Ten additional MCP integration tests cover actual facade intake, shared
sessions, repeated durable recovery, restore abandonment, unknown siblings,
authority expiry and the admitted-candidate retry boundary. These MCP tests are
source-reviewed and formatted but remain unexecuted: the full MCP build is killed
in unchanged Asupersync before reaching `dfmcp-mcp` in this environment.

This is laboratory production continuation. It adds no native wire generation,
compatibility issuer, native mutation admission or production runner entry.

## Consumption-aware /2 validation

The 2026-10-11 increment passes all 93 actual-source production, projection,
source, continuation and forecast tests plus focused warning-denied Clippy.
Five new continuation cases independently check historical canonical bytes and
a golden digest, original-request preservation, fixed compiler selection,
legacy-parent promotion only on new planning, and exact lineage comparison.
Additional source and forecast cases prove distinct modern seals, exact reopen,
original quota truth and physical quiescence. Public facade expectation updates
remain source-reviewed/formatted pending execution of the full MCP target.

## Verified recovery validation

The recovery increment passes all 96 actual-source module tests and
warning-denied all-target Clippy with 644 unchanged inventoried inputs. The
three new tests exercise real production and two actual adapter recoveries,
exact-head/reset mutations, arbitrary epochs, cursor/tick regression, forked
anchors, current grant expiry, Unknown goal truth and unresolved work. Original
source, terminal predicate and plan seal remain unchanged through new pursuits.
Two facade tests add planning/commit refusal after injected epochs and expired
current grants; the existing durable test explicitly checks old seals across
successive new epochs. Full MCP execution remains unavailable after its bounded
Asupersync build reached the shared memory ceiling. These exact component tests
do not establish full-workspace or live qualification.

## Retired nondispatch validation

The retirement increment passes all 35 durable store/objective unit tests,
including eight new custody cases, all 13 durable-progress integration tests,
and all-target lab Clippy with warnings denied. The new cases cover both
retirement paths, compaction and repeated reopen, latest-state precedence,
legacy conservatism, exact archive retention, malformed imports, maximum source
and step bounds, and partial versus complete lost-acknowledgment writes.

All 109 actual-source module tests and warning-denied all-target harness Clippy
also pass. Five new helper tests execute the real production compiler,
StaticPlanner, MemoryAdapter, effect timeline and DurableLabStore. The positive
case completes original construction after recovery, retires and compacts its
commit, survives two reopen cycles, and uses a new continuation to satisfy the
original stock goal without dispatching the old deferred key. Other cases
reject missing or foreign custody, changed/future/forked archives, current
creator contradictions and abandoned history in a rewound world. All 653
inventoried Rust, Cargo and harness inputs remain unchanged during the final
helper test and Clippy window.

Three new MCP facade regressions exercise both recovery openings, the first
opening's unanchored/future/creator refusals, and the prohibition on falling back
from invalid custody to a physically completed original creator. Those tests
and the updated durable facade fixtures are source-reviewed and formatted;
they remain unexecuted because the full MCP build is blocked in unchanged
Asupersync before reaching the facade. Component execution does not establish
full-workspace qualification, a native build or live admission.

## Continue an explicit future reserve with /3

A production request can retain an absolute `reserve_until_tick`, for example
2401, using the `consumption_reserve_v1` generation. See
[explicit reserve planning](PRODUCTION_RESERVE_PLANNING.md#request-reserves-for-an-explicit-future-tick)
for the request example, ingredient accounting and separate future forecast.
The horizon must remain strictly after the current planning source tick and
within the bounded one-game-year span. It is a planning target, not an action
deadline or a continuous maintenance predicate.

An eligible new continuation of such a request uses the flat
`dfmcp.production-continuation/3` source envelope. It retains every original
quota, prerequisite permission, explicit site, the exact planner generation
and the exact absolute tick. The `/3` source digest has its own domain and
participates in the prepared-plan seal. A subsequent continuation changes its
immediate parent while preserving the root and the same original request; it
does not nest earlier envelopes or silently move the reserve horizon forward.

Archived `/3` reconstruction requires both `consumption_reserve_v1` and the
reserve tick. Historical `/1` and `/2` envelopes reject reserve fields and the
reserve generation, while preserving their previous canonical bytes, compiler
selection and digest domains. Exact saved-plan replay, durable reopen and a
reviewed stale-source replay preserve the original absolute tick. A newly
compiled pursuit uses current observed stocks, metabolism phase, workforce and
finite ingredients to budget through the later of the new work horizon and
that retained tick. Action deadlines still follow work duration.

If planning or replay reaches tick 2401 in this example, the retained horizon
has expired and compilation refuses. The continuation cannot turn the original
2,400-tick span into another 2,400 ticks from the new source. Choosing a later
date requires a fresh explicit production request with a new reviewed source
and seal.

The ordinary eligibility rules remain decisive: `continue_goal` requires the
original stock predicate to be currently False, eligible current evidence and
physical quiescence across the retained root's pursuits. Unknown truth needs
reconciliation. A prediction that stocks will fall in the future does not make
currently satisfied quotas False. Consequently, if the original minima remain
True after cancelled, interrupted or completed reserve work, `continue_goal`
cannot add more reserve production. Submit a fresh production request with the
desired future tick when that work is needed; ordinary overlap, source and
authority checks still apply. Active same-output production must first finish
or be explicitly stopped and reconciled.

`production.continuation` identifies the `/3` schema, reserve planner and
retained `reserve_until_tick`. The ordinary work forecast and
`forecast.reserve_forecast` keep action completion separate from original quota
truth at the requested future frontier. Neither that endpoint prediction nor
the preserved source establishes continuous stock maintenance or grants
permission to dispatch new work.
