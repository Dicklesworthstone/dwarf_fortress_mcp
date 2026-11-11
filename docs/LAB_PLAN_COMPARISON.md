# Compare laboratory plans before selecting one

The default laboratory `fortress_plan` tool can compare two to four alternatives
for the same original objective. It compiles every alternative against the same
current world, forecasts the prepared plans on discarded laboratory forks, and
reports their predicted goal, action and physical-work outcomes. A comparison
leaves the current world and the session's pending plan unchanged.

The caller then passes an emitted selection back to `fortress_plan`. Selection
checks the exact current source and authority, prepares a fresh ordinary plan,
and publishes that plan for review. Effects still require a separate
`fortress_commit` call and subsequent observation or bounded `fortress_wait`.
The existing eleven-tool surface and native protocol generations are unchanged.

## Compare, select, commit and observe

For a concrete example, open a private, unpaused `starter_fortress` laboratory
session. Request `observe`, `query` and `plan` at `read_only`, plus
`configure_production` and `control_clock` at `reversible`. Allow 12,000 game
ticks, 8,192 source records and 4 MiB of byte work for the comparison. Keep the
returned `session_id` for every call. These are laboratory grants; no native or
live-game capability follows from them.

The starter world has a working still and brewer. Compare its ordinary
production lowering with explicit dependency-preserving work orders. Pass the
following object **encoded as a JSON string in the existing `blueprint`
argument** of `fortress_plan`. Supply `session_id` separately and leave the
outer `summary`, `actions` and `paused_target` arguments unset.

```json
{
  "template": "compare_candidates",
  "version": 1,
  "horizon_ticks": 1000,
  "candidates": [
    {
      "key": "ordinary",
      "blueprint": {
        "template": "production",
        "quotas": [{"item": "DRINK", "minimum": 60}]
      }
    },
    {
      "key": "conditional",
      "blueprint": {
        "template": "production",
        "quotas": [{"item": "DRINK", "minimum": 60}],
        "orders": "dependency_preserving_v1"
      }
    }
  ]
}
```

A successful response has `kind: "compare_candidates"`, schema
`dfmcp.plan-comparison/1`, and an Agent Turn in the `compare` phase. `anchor`
identifies the one common current source. `pending_plan_changed` and
`effect_evidence` are both false. The rows are ordered by candidate key.

Inspect each row's `prediction`, `max_risk`, `requires_checkpoint`,
`action_count` and `required_capabilities`. `recommended_candidate` is either
one eligible key or null. The recommendation never commits a candidate.

To choose `conditional`, locate that row and pass its entire
`next_step.arguments` object unchanged to the indicated `fortress.plan` tool.
It contains the session ID and an already encoded `blueprint` string with a
`select_candidate` request. Do not reconstruct the request from the displayed
summary or substitute the row's candidate digest for a plan digest.

Selection returns `kind: "select_candidate"`, `plan_id`, `plan_digest`, the
freshly sealed `steps`, risk, capabilities, expiration and normal production
analysis. Its Agent Turn uses the `propose` phase. Successful selection replaces
any earlier pending plan only after the complete response has been accepted.
The comparison's prediction remains conditional; selection reports
`forecast: null` rather than presenting that earlier forecast as a new run.

Review the selected plan, then call the emitted `fortress.commit` next step
with its new `plan_digest`. Call `fortress.wait` with `max_game_ticks: 500` to
advance this example's unpaused laboratory work, and inspect its returned
`objectives` and action proofs. The original quota is proved by current
`predicate_truth: "true"` and goal status, independently of dispatch success.
Other worlds, workloads and budgets may require different bounded intervals.

The exact selection wrapper is an integrity-checked, stateless request. It
contains `expected_anchor`, the normalized original `candidate`, and
`candidate_digest`. Its anchor includes fortress ID, game tick, epoch, sequence
and state hash. A wait, commit, shared-world change or restore that changes this
anchor makes the old selection stale: compare again. Editing the candidate's
summary, source or digest is refused. Possession of a selection does not prove
that a comparison previously ran and grants no authority.

## Forecast the complete resource or room project

Version 1 retains its current reviewed-stage behavior. To compare the original
production or room objective through its later stages, request version 2 with
the explicit scope `complete_project`. It uses the existing `fortress_plan`
blueprint argument and the same ordinary source parsers:

```json
{
  "template": "compare_candidates",
  "version": 2,
  "forecast_scope": "complete_project",
  "horizon_ticks": 5000,
  "candidates": [
    {
      "key": "current_labor",
      "blueprint": {
        "template": "production",
        "quotas": [{"item": "BED", "minimum": 24}],
        "acquire_resources": {"assign_labor": false}
      }
    },
    {
      "key": "allow_labor_setup",
      "blueprint": {
        "template": "production",
        "quotas": [{"item": "BED", "minimum": 24}],
        "acquire_resources": {"assign_labor": true}
      }
    }
  ]
}
```

Use a resource-model laboratory world, such as
`starter_fortress_resources`, with its observed finite resources and the
ordinary capabilities needed for both harvesting and production. The original
request remains each alternative's complete source. The first plan may contain
only finite resource acquisition. On its discarded fork, the evaluator waits
until that stage's actions are verified and its physical work is quiet, then
compiles the same original source at that actual predicted frontier. The next
stage uses the resources that the model actually produced. Future inventory is
never inserted into the initial plan as credit.

The complete-project scope accepts production, provisioned-room and
connected-room blueprints. A room includes every original terrain region and
furnishing; a connected room also includes its entrance, routing domain and
movement model. Final room truth inspects exact reciprocal installed-item
custody and current access. Matching a harvest stock target or newly generated
action identifiers cannot establish that the room is complete. Production
retains the original qualified stock-ledger identity and generation.

Alternatives must share the same semantic output quotas and optional reserve
frontier, or the same complete room geometry, furnishing and access goal.
Different authorized setup, staffing, acquisition and order strategies may be
compared. Those choices still participate in each complete candidate source and
selection digest; they are excluded only from the common semantic-goal key.
Unequal goals are rejected before a later no-work or failed-preparation result
can hide the difference. Version 2 excludes raw actions, pause requests and
owned-history continuation wrappers.

A version 2 result has schema `dfmcp.plan-comparison/2`,
`forecast_scope: "complete_project"`, and `semantic_project_goal`
with its digest. Each candidate retains the complete staged `prediction`.
Its `stages` array names acquisition, production or room work, predicted
frontiers, action verification, physical quiescence and model costs. These
stage plans exist only on discarded forks. Each row's `max_risk`,
`requires_checkpoint`, `action_count` and
`required_capabilities` summarize all simulated project stages, while
`current_stage` describes the initial executable plan. Recommendations
rank complete original-project outcomes using those aggregate costs.

The initial acquisition's output-stock or room goal may still be false when
its own actions finish. That does not prevent a predicted next stage. Failed,
canceled, unverified or physically active stage work does prevent advancement.
A common-horizon stop remains an incomplete prediction; unavailable later-stage
evidence suppresses the whole recommendation. The model assumes the agent
reviews each next stage without additional delay. It does not predict competing
agents or automatically dispatch deferred actions belonging to other plans.

The emitted selection is still a version 1 `select_candidate` request.
It revalidates the exact source at the current real anchor and prepares **only
the current executable stage** for review. The comparison explicitly reports
`selection_scope: "current_reviewed_stage_only"` and
`automatic_continuation: false`. After committing and observing that
stage, the agent uses the ordinary `continue_goal` workflow to review a
fresh later plan. Predicted later stages never become pending plans, effect
authority or current goal evidence.

All stages and alternatives share the original request deadline, source work,
action allowance, game-tick allowance and 400-slice ceiling. The complete
project generation permits at most four stages per candidate and twenty source
forks across at most four candidates, including reserve tails. Every actual
fork still consumes the negotiated record and byte allowance. A larger fork cap
does not multiply those budgets. The whole result must fit the negotiated
output allowance; the publisher refuses rather than removing stages or
decision-bearing outcomes. Default 4,096-token fit for arbitrary complete
project comparisons is not established.

For an explicit future reserve, all alternatives retain the same absolute
frontier. The evaluator reports stock truth at that exact future tick separately
from work completion. A true endpoint is not a proof of continuous maintenance,
and a result after the requested tick cannot rescue missing stock at the
requested frontier. Complete-project success requires that exact reserve
observation to be known True; a missing, Unknown or False endpoint keeps the
project incomplete. A True endpoint may precede later stable action verification,
because the reserve tick is an evidence frontier, not an added action deadline.
No forecast extends a sealed action deadline.

## Which alternatives are comparable

Every candidate has a unique `key` and exactly one of the existing source forms:

| Candidate field | Value and purpose |
| --- | --- |
| `blueprint` | A decoded ordinary objective object, including production or a supported room blueprint. |
| `actions` | A decoded semantic-action array using the existing action grammar. |
| `paused_target` | A Boolean pause target. |
| `summary` | An optional candidate-specific summary, included in its normalized source identity. |

For version 1, all successfully compiled candidates must have the **same normalized original
terminal predicate** and the **same optional absolute reserve frontier**.
The check occurs before any forecast, including when a candidate would later
fail preparation. Matching descriptions or similar-looking goals do not
establish equivalence. Pause and resume are different objectives; drink quotas
of 60 and 80 are different objectives. This operation does not compare their
relative utility.

Useful alternatives include different lowerings of the same production quota,
or equivalent labor changes expressed as separate actions and one batch. For
example, enabling `MINING` for units `1001` and `1002` in two separate actions
has the same terminal field predicates as one `set_labor` action naming both
units. When risk, checkpoint burden and completion time tie, the one-action
candidate is preferred even if its key sorts later.

The source is normalized through the ordinary versioned parsers. The emitted
selection retains that exact normalized source, including the selected
production planner and order lowering. Nested comparisons and selections,
`continue_goal`, and `civilian_release` are excluded: those owned-history
workflows require their own normal session paths.

If production alternatives request `reserve_until_tick`, give all of them the
same absolute tick. Choose a common horizon that reaches it if the comparison
is expected to recommend a candidate. The separate `reserve_forecast` must
establish that the original goal also holds at that frontier. Neither a
completion forecast nor an endpoint reserve forecast proves continuous stock
maintenance between observations.

## Reading predictions and recommendations

Each forecast distinguishes the original goal, action verification and physical
quiescence. `predicted_complete` requires all three at the same predicted
frontier; a transport acknowledgment or completed action alone is insufficient.

| Prediction field | Meaning |
| --- | --- |
| `predicted_goal_truth` | `true`, `false` or `unknown` for the original terminal predicate. |
| `predicted_actions_complete` | Whether every action is predicted verified. |
| `predicted_physical_quiescent` | Whether the candidate's original physical work is predicted quiet. |
| `predicted_completion_tick` | The common frontier where all completion requirements hold; null otherwise. |
| `horizon_tick` | The original plan's final obligation deadline, unchanged by comparison. |
| `simulation_horizon_tick` | Source game tick plus the requested common `horizon_ticks`. |
| `simulation_horizon_reached` / `simulation_horizon_truncated` | Whether evaluation reached its cap and whether that cap stopped incomplete work before its original deadline. |
| `resolution_ticks` | The common nominal maximum sampling stride for the candidate set; sealed deadlines and stability windows can require shorter advances. |
| `sampling_policy` | The rule that inserts original obligation deadlines and final cadence-spaced stability observations into the nominal schedule. |
| `simulation_slices` | Actual attempted model advances for this candidate, including its separate reserve forecast. |
| `blocked_by_pause` | The forecast cannot progress its outstanding work while the modeled world remains paused. |

Candidates may finish early. No candidate simulates beyond the common horizon,
and an earlier original obligation deadline still applies. A known incomplete
or paused result can have `available: true`; unavailable evidence is distinct
from a known negative result.

Pause state is checked again after every complete observation and deferred-action
round. If a newly dispatched action pauses the world, the forecast stops at that
exact frontier: it cannot force additional game ticks or collect advancing
stability samples while paused. An immediately verified pause can itself finish
the plan, and an authorized immediate resume later in the same round permits
normal progression. `blocked_by_pause` describes any remaining goal proof or
physical work at the final frontier. An exact reserve endpoint already reached
remains available; a later endpoint stays unavailable while paused, even when
the current plan is otherwise complete.

The nominal stride never skips an unfinished step's absolute deadline. Before
that deadline, an obligation requiring several stable observations reserves a
final window of cadence-spaced samples. Its eligible-read clock starts when the
adapter actually dispatches the action; polling deferred work or reading between
eligible samples does not move that clock. Only the ordinary obligation runtime
decides whether each sample establishes the terminal and failure predicates.
The forecast neither extends deadlines nor treats a late matching state as timely
completion. This prevents one long step from making a shorter valid step appear
to fail solely because the forecast chose a coarse stride.

Compilation and preparation failures remain visible as `status: "unavailable"`
rows with a `stage` and reason. A model-level refusal can instead appear in
`prediction.reason`. Such outcomes are not scored as ordinary losses. If any
candidate lacks a comparable prediction with known original goal truth, or
lacks requested reserve evidence, the whole report has no recommendation.
`selection_available` states whether a row has a source that can be submitted
for fresh preparation; it does not promise completion or permit commit.

Among eligible, predicted-complete candidates, the deterministic policy
minimizes these fields in order: risk tier, checkpoint requirement, completion
tick, action count, and lexical key. Requested reserve truth must also be true.
If no candidate meets these conditions, `recommended_candidate` is null and
`recommendation.reason` explains why. There is no invented weighted utility
score or automatic effect dispatch.

These are forecasts of the deterministic laboratory reference world under the
assumption that no other agent acts. Existing physical jobs evolve on each
fork, but only the selected candidate's deferred action chain is polled and
dispatched. Other retained plans' deferred actions are not dispatched on these
forks. Recommendations therefore do not model a complete joint schedule of
all session plans. The actual adapter, clock, pending plan, action book and
query-monitor roots remain unchanged by comparison.

## Bounds, current authority and publication

Comparison and selection require current `Observe`, `Query` and `Plan`
authority at `read_only`, plus the ordinary action capabilities required by
preparation and execution. The default session's output allowance is 4,096
tokens. Each request shares one wall-time deadline from public entry through
source admission, compilation, forecasts, current-work capture and final
publication. Opening another candidate does not reset that deadline.

| Bound | Limit |
| --- | ---: |
| Original candidate request | 128 KiB of UTF-8 |
| Alternatives | 2–4 |
| Candidate key | 1–64 ASCII letters, digits, underscores or hyphens |
| Candidate summary | 4,096 UTF-8 bytes |
| Forecast source forks, including reserve forks (version 1) | 8 across the set |
| Simulation slices | 400 across the set |
| Simulated ticks | Combined total at most the session's `max_game_ticks` |
| Compiled action count | Combined total at most the session's `max_actions` |
| Action polls | At most `max_actions * 400` across the set |
| Copied source records / canonical bytes | Combined fork totals within `max_entities` / `max_bytes` |
| Complete encoded response | At most `min(max_bytes, max_output_tokens * 4)` bytes |

The common horizon itself must fit the game-tick budget. For example, granting
1,000 ticks does not let four candidates each spend 1,000 simulated ticks.
`source_work` records complete canonical-source admission;
`forecast_work` reports actual candidate, fork, action, poll, slice and
simulated-tick costs. `actual_game_ticks_advanced` remains zero. Copied source
bytes measure canonical snapshot work; retained adapter actions, plans,
checkpoints and journal are not a measured heap-byte total.

Deadline and stability observations consume the same slice allowance as ordinary
advances. A single-plan forecast also has a hard 400-advance ceiling covering its
work and reserve forks together. If the required observations cannot fit, the
single forecast is unavailable and a comparison returns `budget_exceeded`; no
incomplete sampling run is scored as an ordinary failed candidate. These are
sampled conditional predictions, not a search over all possible agent polling
schedules or proof of continuous predicate truth between observations.

The wrapper rejects unknown and duplicate object fields from the original
JSON, including duplicates inside candidate sources. Candidate keys must be
distinct. The decoded request schema is
[`lab_plan_comparison.schema.json`](../schemas/lab_plan_comparison.schema.json).
It describes the existing tool's blueprint-string contents. The root
[`plan.input.schema.json`](../schemas/plan.input.schema.json) remains its
existing phase-zero design envelope; the added definition is not a replacement
top-level tool signature. Schema syntax cannot establish current authority,
source freshness, equivalent compiled goals or sufficient work budgets.

Both success packets include the complete pending-plan catalog and the bounded
`query_baselines`, `query_watches` and `condition_watches` catalogs. Owned action
receipts and obligations remain visible. Nonfinal recovered work appears in
`active_work.carried_obligations`, including a verified historical goal whose
physical work is still active or unknown. These are retained recovery records,
explicitly labeled `retained_not_reobserved`; comparison and selection do not
refresh their proof or work anchors. Broader current game work remains partially
covered, including deferred dispatch owned by other sessions.
Publication happens while the session and shared-world owner remain held. The
complete result, all candidate rows, actionable selection strings and Agent
Turn must fit before a selected pending plan is installed. No row is dropped
and no later renderer clips an already accepted selection. A budget, deadline,
source or authority refusal leaves the earlier pending plan intact.

When archived details of earlier objectives would exhaust the response budget,
the shared publisher uses an explicit `dfmcp.objective-briefing/1` projection.
Every retained objective stays in the briefing with its current goal truth,
semantic room/output truth, future-reserve state, historical-achievement class,
physical quiescence, continuation or refusal guidance, and other safety fields.
The full original request, original terminal predicate and historical proof
anchors move behind that row's `details` resource. Read the returned
`goal-{plan_digest}` resolver and follow its pinned manifest/source pages for
those retained artifacts. The detail read does not re-evaluate current truth or
grant effect authority.

An `observed_anchor_reference: "/anchor"` shares the response's exact complete
anchor only after equality is established. Missing, unknown and different
historical anchors are never relabeled as current. Existing
`anchor_encoding.reference_count` counts only the declared current-anchor
references retained in the packet; source and predicate literals never
participate. The projection records its complete membership and incomplete
forensic evidence explicitly. The selected plan's complete `steps`, original
source analysis, lineage, forecasts and every active-work/query catalog remain
intact. If these required fields still do not fit, publication refuses before
changing the pending plan. A larger allowance continues to return the original
full briefing when it fits.

Use `invalid_request` to correct ambiguous sources, unequal goals or a modified
selection. On `stale_anchor`, obtain a new comparison. On `capability_denied`,
obtain the required current grants through the normal session policy. On
`budget_exceeded`, reduce the bounded request or use an adequately negotiated
budget; a failed request never grants a partially prepared candidate.

## Implementation and validation scope

`lab_plan_comparison.rs` reuses ordinary `PlanSource` compilation and
`StaticPlanner` preparation. `plan_forecast.rs` runs the existing laboratory
model under the additive shared `ForecastBudget`; `plan_forecast_schedule.rs`
keeps deadline and stability frontiers inside the same bound for both single
and compared forecasts. `lab_plan_comparison_handler.rs`
owns source and pending-plan publication, while
`lab_plan_comparison_response.rs` performs pure complete-packet rendering.

Focused tests cover closed original-byte parsing, comparable goals and reserve
frontiers, deterministic ranking, source integrity, current authority,
aggregate budgets, common cadence and immutable canonical state. Public facade
regressions cover compare/select/commit/wait, current query catalogs, pending
preservation, stale selection and whole-packet refusal. Run the decoded-schema
checks with `python3 scripts/test_lab_plan_comparison_schema.py` in an environment
with the existing `jsonschema` test dependency available. These checks concern
the laboratory reference world; they do not establish native, live-game or
release qualification. Current execution evidence is recorded in
[`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md).

The complete-project implementation is separately described by
[`laboratory_project_forecast_v1.json`](../architecture/laboratory_project_forecast_v1.json).
Its focused regressions exercise staged harvest-to-room and harvest-to-production
forecasts, strategy differences, semantic goal equality, aggregate later-stage
costs, future capability failures, bounded horizons, ordinary current-stage
selection and pending-plan preservation on output refusal. Test source does not
by itself establish execution or full Rust qualification; the current status
document records the commands that actually ran.
