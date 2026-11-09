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

## Which alternatives are comparable

Every candidate has a unique `key` and exactly one of the existing source forms:

| Candidate field | Value and purpose |
| --- | --- |
| `blueprint` | A decoded ordinary objective object, including production or a supported room blueprint. |
| `actions` | A decoded semantic-action array using the existing action grammar. |
| `paused_target` | A Boolean pause target. |
| `summary` | An optional candidate-specific summary, included in its normalized source identity. |

All successfully compiled candidates must have the **same normalized original
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
| Forecast source forks, including reserve forks | 8 across the set |
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
