# Condition inspection and historical condition timelines

The spatial/1.8 query runtime can inspect a complete success condition and optional
failure guard without registering or sampling a watch. The same request can be
replayed against an exact archived observation or across a retained interval.
This closes the gap between scalar stock history and the compound conditions
used by actual monitoring plans.

This is unadmitted development source. The Rust implementation and regression
tests have not been compiled or executed in the editing environment. No game
readiness, native compatibility, production qualification or admission is implied.

## Inspect a condition before installing a monitor

Call the existing `fortress.query` tool:

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "condition_evaluation",
      "condition": {
        "op": "all",
        "args": [
          {
            "op": "entity_count",
            "scope": "observed_projection",
            "kind": "unit",
            "predicate": {"op": "always"},
            "comparison": "ge",
            "value": 1
          },
          {
            "op": "item_quantity",
            "scope": "observed_projection",
            "quantity_unit": "stack_units",
            "predicate": {
              "op": "field",
              "field": "type_key",
              "comparison": "eq",
              "value": {"type": "text", "value": "WOOD"}
            },
            "comparison": "ge",
            "value": 20
          }
        ]
      },
      "failure_condition": {"op": "paused", "value": false}
    }
  }
}
```

`WOOD` and 20 are illustrative caller selections. Use the actual observed type
key and the intended threshold. The guard in this example deliberately treats
an unpaused capture as failure evidence; it does not change the pause state.
Raw stack quantities are not usable supply, food portions, nutrition, inherited
container eligibility, carrying feasibility or native reservations.

The `condition` and `failure_condition` values are exactly the existing watch
condition language: generation-checked fields, dynamic entity counts, item
quantities, pause/tick checks, and `all`, `any`, `not` composition. They use the
same validator, typed comparisons, three-valued evidence rules, non-short-circuit
identity checks and shared evaluation budget as foreground watches. There is no
second implementation of the predicates.

A successful query returns both truth values, leaf evidence, a predicate digest,
an evidence digest bound to the full observation anchor, and one classification:

| Classification | Meaning at this capture |
|---|---|
| `invalidated_reference` | At least one explicit entity reference has another generation. |
| `failure_condition_met` | The failure guard is definitely true and no generation mismatch overrides it. |
| `blocked_unknown` | A required success or failure result is unknown. |
| `condition_met` | Success is true and failure is false, with no reference mismatch. |
| `condition_not_met` | Success is false and failure is false, with no reference mismatch. |

This ordering matches watch predicate precedence. A decisive branch cannot hide
a recycled entity. Missing entities remain unknown rather than deleted/dead;
observing the same ID with a different generation invalidates the explicit
reference. Negating unknown evidence leaves it unknown. An absent or null failure
guard means false and produces the same normalized predicate identity.

`eligible_success_sample` means only that the predicate results could support a
success sample; it does not check a watch's cadence, lifetime, deadline or prior
streak. `watch_registered=false`, `watch_evaluated=false`,
`stability_evaluated=false`, `deadline_evaluated=false` and
`watch_completion_proven=false` make that boundary explicit. No Watch object,
handle or checkpoint is created, and no existing watch is looked up or advanced
by the evaluator. The enclosing live response can still include the current
unchanged active-work summary.

## Evaluate an exact historical observation

Use `history` to discover retained record identities, then put the same
`condition_evaluation` object inside `historical_query.query`:

```json
{
  "kind": "historical_query",
  "record": 1,
  "record_digest": "<actual 64-character lowercase digest from history>",
  "query": {
    "kind": "condition_evaluation",
    "condition": {"op": "paused", "value": true}
  }
}
```

This object belongs inside the normal `dfmcp.query/1` envelope. The result is
historical and does not replace the current session's world. Current Query
authority is checked before replay; an old observation does not revive an expired
grant. Exact replay retains the original entity generations and verifies the
required journal prefix.

An archive-only session also permits direct condition inspection of its latest
retained observation. It still cannot register or poll watches, acquire a live
capture or promote historical facts into current freshness. Reopening an archive
uses the existing Query-only recovery path without bridge credentials.

## Inspect the condition throughout an interval

`historical_series.measurement` now accepts either the existing `item_quantity`
query or a `condition_evaluation` query:

```json
{
  "kind": "historical_series",
  "from": {"record": 1, "record_digest": "<actual first digest>"},
  "to": {"record": 3, "record_digest": "<actual last digest>"},
  "measurement": {
    "kind": "condition_evaluation",
    "condition": {"op": "paused", "value": true}
  },
  "limit": 8
}
```

Each condition row has the exact record/anchor/source witness, the individual
query's evaluation and evidence digest, and an adjacent-sample classification
comparison. A true/false/true sequence remains visible even though the interval's
endpoints agree. Failure-guard results and generation mismatches are kept
separately, not flattened into one boolean.

`same_evaluation_classification` means only the same success truth, failure truth,
reference-mismatch flag and resulting classification. Leaf facts may still have
changed. Equal unknown classifications do not establish unchanged world state.
`evaluation_classification_changed` likewise records an endpoint difference, not
a causal diagnosis or an event time between captures.

Epoch or regressed-clock boundaries break comparisons. Same-tick samples report
zero elapsed game ticks but cannot manufacture time-based stability. Condition
series do not produce quantity rates or infer watch streaks. They never discharge,
fail, reactivate or backfill a saved watch, even when every returned condition is
true. Stored terminal watch evidence is untouched.

## Limits and compatibility

The single-query result must fit its complete allotted response. Success and
failure expressions share the existing 64-node/depth-8 definition bound and one
million evaluation work units under one cooperative deadline. Input shape and
byte bounds are checked before deserialization; oversized leaf evidence is
refused rather than silently omitted.

Historical pages keep the existing 1..32 returned samples plus at most one internal
predecessor, one verified generation-history prefix per page, and complete-row
pagination. The predecessor preserves cross-page transitions without duplicate
rows. Each measurement retains its own one-million-unit limit and a 16 KiB
internal result ceiling; the page shares its existing cooperative wall allowance.
Large compound evidence may require narrower predicates or fail explicitly.

`hs1` continuations bind the session, archive identity/head, selected interval and
full measurement request, including its failure guard. Page width and output
allowance may change. Changing the predicate or guard requires starting a new
series. Raw request representation participates in timeline identity, even where
individual predicate inspection normalizes omitted/null guards. Existing quantity
series identity, quantity bounds, net changes and exact rate calculations remain
unchanged.

Current work and complete historical Agent Turns are reserved before replay;
final output is checked again. Partial pages state what prefix was verified and
do not claim verification of an unseen suffix. Journal custody and current
authority are checked through the existing historical boundary. Filesystem and
CPU work remain cooperatively bounded, not hard-preemptible.

No native protocol, journal format, dependency, game effect, top-level MCP tool,
production runner or admission permission changes. Schema discovery uses the
existing condition definitions for direct inspection and the two timeline
measurement alternatives; recursive history and stateful measurements stay denied.

## Validation status

Seventeen new Rust test functions are registered: eight inspection tests, three
classification-transition tests and six actual MCP-handler/private-journal tests.
One classification test enumerates all 324 pairs of success/failure/reference
classifications. Coverage includes watch-evaluator parity, unknown evidence,
non-short-circuit generation checks, count/quantity composition, normalized guards,
unchanged retained work, full output refusal, exact individual/history equivalence,
paging, reset boundaries, offline recovery and current authority expiry.

NONE has been compiled or executed here. Rust, Cargo and rustfmt are unavailable;
validation consists of source review and GitHub diff/commit/branch verification.
The 324 cases are registered tests, not executed evidence. No Python mirror or
schema-only result is presented as Rust, MCP, native or full-repository evidence.
Focused commands on a configured checkout are:

```bash
cargo test --locked -p dfmcp-mcp 'query_watch::inspection::tests' -- --test-threads=1
cargo test --locked -p dfmcp-mcp 'series::conditions::tests' -- --test-threads=1
cargo test --locked -p dfmcp-mcp 'condition_tests::' -- --test-threads=1
```
