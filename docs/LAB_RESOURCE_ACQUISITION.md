# Plan finite resource stock goals

The `resource_acquisition` blueprint turns original WOOD and PLANT stock
minima into a bounded, reviewable harvest plan. It uses the existing finite
harvest actions and the ordinary plan, commit, wait and proof loop. Agents no
longer need to select and size every source by hand.

The feature belongs to the deterministic `starter_fortress_resources`
laboratory. Its resource yields, labor and timing are reference-model rules.
It grants no native DFHack route or live compatibility admission.

## Request and review

Open the resource scenario with current Observe, Query and Plan authority,
guarded Designate and Checkpoint authority, and ControlClock when game time
must advance. Explicit labor enables additionally require ConfigureLabor.
Then pass this object as the existing `fortress.plan` `blueprint` JSON string:

```json
{
  "template": "resource_acquisition",
  "quotas": [
    {"resource": "WOOD", "minimum": 25},
    {"resource": "PLANT", "minimum": 45}
  ]
}
```

Each minimum is an absolute desired stock, not an amount to harvest. The
compiler reads qualified current stock and harvests only the positive deficit.
There may be one or two unique targets, using exactly `WOOD` and `PLANT`.
Minima are positive unsigned 64-bit integers; a large target is accepted only
when its actual deficit fits finite supply and the action bound.

`assign_labor` defaults to false. Set it to true to permit an ordinary,
separately authorized labor enable when the selected acquisition needs one:

```json
{
  "template": "resource_acquisition",
  "quotas": [{"resource": "WOOD", "minimum": 25}],
  "assign_labor": true
}
```

This setting permits proposing labor changes; it does not confer their
capability. Unknown fields, duplicate keys or resource targets, unknown
resource names, null options, zero/fractional minima and unknown planner
generations refuse. The decoded request is limited to 16 KiB.

Planning publishes its `resource_acquisition` analysis with the exact source,
current stock, original minimum, deficit, selected sources and generations,
amounts, proposed workers and queue/deadline inputs. It does not deplete sources,
credit inventory, enable labor or reserve workers. Review the generated steps,
ordinary capability/checkpoint requirements and forecast, then commit the
returned exact plan digest. A request whose targets already hold returns an
explicit no-work refusal without replacing an existing pending plan.

## Finite allocation and staffing

The compiler validates the complete qualified source/task census and original
stock ledger before calculating an acquisition. Each source offers its current
remaining units less complete active demand. The next allocation takes the
largest useful tranche, capped at 100 units, and breaks ties by canonical source
identity. Repeated bounded allocation may use several sources or split a large
source into several actions. The complete goal must fit at most 16 actions,
including any labor enables; insufficient supply or excess work refuses the
whole candidate.

An active original harvest for a resource with a new deficit refuses additional
acquisition for that resource. Observe its result or explicitly stop its future
work before preparing another stock goal. Pending harvests never count as
inventory, including when their predicted yield would satisfy the target.

When acquisition is needed, this first compiler also refuses known active
production that will consume any requested resource, including an already-met
companion target. It does not guess an additional material reserve for that
queue. Observe or drain the original consuming work before requesting the
acquisition. Already-held workshop materials have already been debited from
available stock and are not counted twice. Unrelated queued work still
participates in the ordinary shared-worker horizon.

WOOD requires `WOODCUTTING`; PLANT requires `PLANT_GATHERING`. Selected workers
must have qualified living status and a known relevant labor value. Selection
prefers already-enabled workers, then known nonmilitary workers, then unknown
military status, then known military membership, with canonical identity as the
final tie-break. Missing, stale or future facts cannot establish a worker or
authorize a labor enable. With explicit assignment permission, a known false
labor may produce `SetLabor` with its original guard and compensation. Dependent
harvest work waits for the enable to be verified.

The shared workforce still grants each living worker only one activity at a
time. The ordinary workforce planner counts the complete existing queue and
new service, including blocked work, in its conservative deadline floor. It
keeps the existing one-game-year bound and refusal behavior. Neither the
selected worker nor the horizon is a guarantee of future completion.

## Goal proof and subsequent production

The original WOOD/PLANT minima remain the terminal stock goal. Each generated
action independently retains the ordinary original-task, finite-amount,
source-generation and ledger completion proof. Mandatory task predicates bind
the original ledger identity and generation; the goal evaluator checks that
custody and conservation proof before reporting the stock objective true.
Action completion, current stock truth and physical quiescence remain separate
observations.

Forecasting runs the same sealed plan on a discarded laboratory world. A
predicted harvest does not become stock in the real source world. Other work
may consume resources, labor may change, and insufficient observation cadence
may defeat the original deadline; inspect actual current goal and work evidence.

After acquisition completes, observe the new stock and prepare an ordinary
production, workshop or room request against that new anchor. This preserves
the established workflow:

1. Review and commit the finite acquisition.
2. Advance and observe its original work under current authority.
3. Confirm actual stock and physical completion.
4. Prepare the original production or construction objective using observed
   inputs and its existing finite-material checks.

For example, with no existing beds, an initial stock of 20 logs cannot fund a
22-bed production goal. First request the absolute raw-material minimum:

```json
{
  "template": "resource_acquisition",
  "quotas": [{"resource": "WOOD", "minimum": 22}]
}
```

After committing and observing the two completed harvest units, prepare the
ordinary production goal under its own ConfigureProduction authority:

```json
{
  "template": "production",
  "quotas": [{"item": "BED", "minimum": 22}]
}
```

The completed production can consume all 22 logs. The earlier acquisition's
current stock condition then becomes false while its verified historical
achievement remains retained. A stock objective is not a permanent reservation.

Cancellation stops future harvesting and retains completed yield and source
depletion. The original request can then be pursued through `continue_goal`,
which recomputes only its deficit from actual observed stock.

## Continue the original acquisition

Once all work belonging to the retained goal and its related pursuits is
physically quiet, pass this object as the `fortress.plan` `blueprint` string:

```json
{
  "template": "continue_goal",
  "plan_digest": "<retained resource goal digest>"
}
```

The retained source supplies the complete original WOOD/PLANT minima, fixed
compiler and labor-assignment policy. The public request cannot substitute
new quotas, permissions, parent/root identities or completion claims. The
continuation reads the current qualified source, includes previously completed
yield in observed stock and allocates only the remaining deficit. It produces
a new proposal for review and a separate explicit commit.

Current Observe, Query and Plan authority, complete source and response budgets,
original-goal eligibility and quiescence across the entire retained root are
checked before publication. Commit checks current eligibility again. Active
or unknown sibling work blocks a new pursuit; a newly prepared sibling cannot
duplicate work once another sibling has committed. Already-satisfied original
stock minima do not create a replacement plan. Unknown stock or broken lineage
requires reconciliation rather than an inferred deficit.

The canceled original harvest's mandatory task proof stays canceled. A child
can establish the unchanged stock goal without rewriting that historical
action result. When a same-root pursuit has met the stock goal, the original
canceled pursuit does not keep offering redundant acquisition. If resources
are subsequently consumed, a new eligible continuation evaluates the original
minimum against the new observed stock again.

Responses retain `parent_plan_digest`, `root_plan_digest` and a
`continuation_source_digest` in `resource_acquisition.continuation`. A flat
`dfmcp.resource-acquisition-continuation/1` archive binds the complete original
request and lineage into the prepared seal. Repeated continuations change the
parent while preserving the root and request; they never recursively embed
older envelopes. The distinct durable source kind is
`resource_acquisition_continuation`; existing archive kinds retain their bytes.

Verified durable reopen preserves this lineage without restoring grants or
dispatching old effects. Checkpoint-abandoned goals, arbitrary epoch changes
and unverifiable history remain ineligible. If a continuation's source anchor
changes, stale Commit keeps the reviewed proposal and returns the public
`continue_goal` request naming its retained parent for another explicit review.
The private archive envelope is never used as a public planning request.

## Saved sources, comparison and budgets

New intake installs the fixed `resource_acquisition_v1` compiler generation,
explicit assignment policy and canonical WOOD-then-PLANT target order. Archived
sources must name that same generation. The complete canonical request has a
domain-separated digest in the prepared-plan seal, including a permission that
is not needed at the source anchor. Old blueprint and action encodings retain
their behavior.

The same source is available to the existing candidate comparison and
selection path. Comparison remains prediction only; selecting a candidate
prepares an ordinary plan for a separate commit. Anchor changes require a
new explicit Plan request; a stale Commit retains the old pending request and
returns the exact original request for review. Recovery guidance and the
pending-plan catalog keep that fresh Plan next step through later observations
and comparison/query responses until a replacement is accepted. A narrow action
witness cannot silently approve
different source selection, stock sizing, staffing or queue timing. Exact
archived-source reconstruction still uses the original fixed compiler.

Whole-source inspection must fit the negotiated source and wall budgets before
compilation. Complete response publication precedes changing the pending plan.
An insufficient output budget or refused request leaves the previous pending
plan intact. Resource proposals publish complete action steps and proof once,
in the response's top-level `steps` array. The pending-plan catalog retains its
`step_count` and an explicit `steps_reference` with `scope: "same_response"`,
`json_pointer: "/steps"`, the matching plan digest and `complete: true`.
Publication checks that reference before changing pending work. This applies
both to a direct resource request and to selection of an acquisition candidate;
it avoids repeating the proof inside the same response.

Fixed source bounds remain 65,536 records each for entities,
edges, chunks and events, with at most 1,024 sources and 4,096 retained harvest
tasks, independent of narrower caller limits.

The core API is `dfmcp_intent::resource_acquisition::ResourceAcquisitionPlan`;
the MCP request schema is
[`lab_resource_acquisition.schema.json`](../schemas/lab_resource_acquisition.schema.json).
The machine contract is
[`laboratory_resource_acquisition_v1.json`](../architecture/laboratory_resource_acquisition_v1.json).
See [finite harvest semantics](LAB_RESOURCE_HARVEST.md) and
[shared workforce](LAB_SHARED_WORKFORCE.md) for the underlying physical model.
Exact executed validation and remaining qualification limits are recorded in
`IMPLEMENTATION_STATUS.md` under beads `df-lab-resource-harvest-h2m.1` and
`df-lab-resource-harvest-h2m.2`.
