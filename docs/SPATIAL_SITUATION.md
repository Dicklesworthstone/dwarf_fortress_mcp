# Spatial operational situation and attention

The spatial/1.8 development runtime now turns its coherent citizen, operations and terrain
projection into a bounded operational briefing. A newly arrived agent can see observed signs worth
inspecting without first downloading every unit, job, building and tile. This is derived read-only
presentation, not a new native observation profile, game effect, causal diagnosis or admission claim.

The policy is `dfmcp.spatial-situation/1`. It runs over one exact canonical snapshot and requires
current whole-projection Query authority. Native source digests, game ticks, field types and presence
representations must agree before a fact contributes to a positive finding.

## Where it appears

Live `fortress.open_session`, `fortress.observe` and `fortress.wait` responses include the complete
bounded signal-count summary under `agent_turn.briefing.situation`, plus at most two detailed
attention groups. Observe and wait also compare the pre-call and post-capture situation counts.
A heartbeat has no invented changes; an observation reset has no cross-epoch count comparison.

Ordinary live structured queries use a deliberately smaller tactical presentation: at most one
compact highest-priority attention group, the total/omitted group counts, the number of groups with
unestablished inputs, and a ready request for the detailed situation. This policy is selected before
execution, not by dropping warnings after an output-budget failure.

Retrieve the detailed briefing without another native capture:

```json
{
  "session_id": "<live spatial session>",
  "mode": "situation"
}
```

This is a mode of the existing `fortress.query` tool. It is not a new top-level tool or a structured
`kind` accepted by historical queries. `summary` retains its existing aggregate result with compact
attention. `fortress.explain` exposes the complete rule catalog, priority order and limitations.

Detailed attention contains an exact-source identity, observed and unestablished counts, one
canonical example with its generation/revision, and a structured `fortress.query` inspection request.
The inspection binds the full observation anchor and entity generation. Once the observation changes,
that old request fails rather than silently inspecting a different version. Entity labels and names
are never interpolated into findings or executable next-step fields.

## Fixed inspection priorities

The order below is the complete policy. It is not a probabilistic threat score or a claim that every
matching entity is in danger. Each group retains the lowest matching canonical entity ID as its
example, independent of arrival order.

| Priority | Rule | Positive evidence | What it does not prove |
|---|---|---|---|
| 0 | `citizen_not_alive` | A strict citizen's native `alive` field is false. | Time or cause of death. |
| 1 | `living_citizen_not_sane` | Native `alive` is true and `sane` is false. | Diagnosis, cause or imminent violence. |
| 2 | `visible_magma` | Visible terrain has magma and positive liquid depth. | Exposure, flooding or danger; it may be contained. |
| 3 | `suspended_jobs` | A current job's native suspension flag is true. | Cause, age, criticality or inability to complete. |
| 4 | `unassigned_unsuspended_jobs` | A job is not suspended and has no assigned worker. | A labor shortage or blockage; queueing may be normal. |
| 5 | `incomplete_buildings` | Observed nonnegative build stage is below its observed maximum. | Stalled construction or missing materials. |
| 6 | `retained_rotten_items` | An item is not removed and is observed rotten. | Available food quantity or starvation. |

All seven observed/unestablished counts remain present in the detailed summary even when only two
groups receive full attention entries. Omitted detail counts are explicit. Zero positive findings
never emits an all-clear claim. Food, drink, health and threat coverage are explicitly incomplete.

## Unknown facts and redaction

A rule can use a field only when it is a native `DfhackField`, has the coherent source digest, was
observed at the snapshot's game tick, has the required type, and has a consistent known-value
representation. Unsupported, omitted, absent, redacted, stale, inferred, differently sourced and
contradictory representations do not become false observations.

Compound conditions use three-valued logic. A known false conjunct can rule out the condition, but
negating an unknown input does not establish truth. Hidden or unallocated terrain is not searched
for residual liquid values even when a synthetic input contains plausible-looking attributes.
Unestablished input counts remain visible, including changes in those counts after a refresh.

Counts refer to the strict-citizen roster, current operation rosters and requested terrain region.
They are not global fortress completeness witnesses. The source is the last coherent capture, not
real-time state at the moment a later query is returned.

## Observation changes

`fortress.observe` and `fortress.wait` keep the prior compact report before their existing one-capture
refresh. When the new anchor is comparable, the response adds `situation_comparison` and at most four
entries in `agent_turn.changes`. The comparison includes pause state, rule counts and roster counts,
ordered with pause and rule changes before ordinary roster changes. Total, returned and omitted
metric counts are explicit.

These are endpoint count differences only. Equal counts can hide replacement of one entity by
another, and a changed count does not establish cause, an event sequence, continuous satisfaction or
game-effect success. Cross-fortress, cross-epoch, regressed or forked anchors are not compared.
Use exact-record `historical_changes` for selected entity-level historical differences.

The comparison basis is this call's pre-refresh published anchor, not an assertion that the client
acknowledged every earlier response. No new acknowledgement protocol is introduced. If response
construction fails after an observation was durably published, the existing published observation
is retained; this increment does not add transaction rollback or pretend that acquisition did not
happen.

Situation derivation and ordinary observe/wait do not evaluate watches. Existing watch summaries
remain attached, with their original evidence digest and evaluation anchor until an explicit poll
or await evaluates them. `await_watch` retains its established one-refresh/evaluate behavior and gets
a compact situation for the resulting capture, not a second observation or separate background task.

## Authority, history and output bounds

Current Query authority is rechecked at the post-refresh anchor. An Observe-only or Doctor-only
caller gains no situation-query authority; an expired Query grant does not survive the refresh.
A fenced native source produces no current situation findings. Archive-only sessions refuse the
`situation` mode, and historical query/comparison responses do not inherit current live attention.
The existing historical explanation and watch evidence remain separate.

Derivation scans at most the negotiated entity limit and 131,072 entities, with a fixed seven-rule
ceiling, one retained example per rule and cooperative deadline/cancellation checks. There is no
background worker or retained situation store. Hash verification and native capture remain separate
existing operations; this is not a claim of hard real-time preemption or one shared deadline over
every stage of a tool call.

Full Agent Turn metadata and active watches are reserved before result pagination. Watch operations
use their own worst-case response-metadata sample through the actual final renderer, rather than
paying for unrelated baseline-comparison metadata. Complete watch output is still rendered and
checked before watch state publication or durable checkpoint append. Insufficient space causes a
visible error; no alert or watch is silently discarded to make a result fit. The complete seven-rule
explanation or many retained watches can require a larger negotiated output budget.

## Evidence status

Fifteen Rust scenarios are registered: nine canonical-projection tests and six actual live/archive
handler tests. They cover all rules, unknown/provenance distinctions, hidden-data noninterference,
deterministic generation-bound drill-downs, authority and budget refusals, endpoint/reset semantics,
compact/detail navigation, 8,192-byte paginated queries with a current watch, failed watch
publication, source failure, expiry after refresh and historical separation.

They have **not been compiled or executed here**. Rust, Cargo and rustfmt are unavailable in this
editing environment. A focused command on a configured checkout is:

```bash
cargo test --locked -p dfmcp-mcp situation -- --test-threads=1
```

An independent Python JSON-size reference checked six watch packet shapes at an 8,192-byte budget.
Those reference packets measured 6,553–6,590 bytes and fit the reference watch-result allowance.
This check does not execute Rust serialization, rule evaluation, watch transitions, actual MCP,
native DFHack or a live fortress, and is not runtime qualification. Current source still requires
Rust/Clippy/stdio and native/live validation; production admission and mutation authority are unchanged.
