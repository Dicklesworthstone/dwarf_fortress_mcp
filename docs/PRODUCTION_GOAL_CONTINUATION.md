# Continue a retained original production goal

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
and reuses currently completed setup. New continuations choose the fixed
`consumption_aware_v1` generation, which reserves population consumption through
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
False under eligible current evidence. A True goal requires no continuation;
an Unknown goal requires reconciliation. Restoring a world abandons its old
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

## Lineage, restart and bounds

Continuation responses and goal observations identify both the immediate
`parent_plan_digest` and the original `root_plan_digest`. They also expose a
`continuation_source_digest` covering the complete original request and lineage.
That digest participates in the actual prepared-plan summary and therefore its
seal, including when a custom human summary is supplied. Changing an unused
setup site cannot leave the continuation seal unchanged.

The durable `production_continuation` source kind retains one flat versioned
envelope. New continuations use `dfmcp.production-continuation/2`; historical
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
