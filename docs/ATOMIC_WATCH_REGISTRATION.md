# Atomic monitoring-plan registration

The live spatial/1.8 server accepts `register_watches` through the existing
`fortress.query` tool. It installs one to eight related watches against the same
published observation, instead of requiring a sequence of independently committed
registrations that may stop halfway through a monitoring plan.

This is unadmitted development source. The Rust implementation and tests have not
been compiled or executed in the editing environment. Native protocols, effect
capabilities, dependencies and production admission are unchanged.

## Request

Each array member is an ordinary `watch` definition with its `kind` field omitted.
All existing field, population-count, item-quantity, pause, tick and Boolean
conditions, failure guards, deadlines, cadence and stability settings remain
available. Schema discovery derives the member schema from the actual single-watch
schema, including the composed population and quantity operators.

```json
{
  "session_id": "<live spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "register_watches",
      "watches": [
        {
          "key": "bar-stock-target",
          "condition": {
            "op": "item_quantity",
            "scope": "observed_projection",
            "quantity_unit": "stack_units",
            "predicate": {
              "op": "field",
              "field": "type_key",
              "comparison": "eq",
              "value": {"type": "text", "value": "BAR"}
            },
            "comparison": "ge",
            "value": 200
          },
          "deadline_tick": 42336100,
          "stable_observations": 2
        },
        {
          "key": "no-suspended-jobs",
          "condition": {
            "op": "entity_count",
            "scope": "observed_projection",
            "kind": "job",
            "predicate": {
              "op": "field",
              "field": "suspended",
              "comparison": "eq",
              "value": {"type": "bool", "value": true}
            },
            "comparison": "eq",
            "value": 0
          },
          "deadline_tick": 42336100,
          "stable_observations": 2
        }
      ]
    }
  }
}
```

The example tick and type key are illustrative. Use the current observation's
actual tick and observed keys. Every NEW watch deadline must be in the future and
within the session's game-tick horizon. Counting raw bar stack units does not prove
usable materials, and no observed suspended jobs does not prove production is
healthy. The existing uncertainty and observed-projection limitations are unchanged.

An optional outer `expected_anchor` must match the full current session anchor.
Registration does not acquire a native capture, advance game time or require
Observe authority. It requires current Query authority and a healthy live session;
archive-only/historical queries refuse it. The returned `next_step` selects the
unfinished requested watches for `await_watches`, which separately requires Observe
when a fresh capture is needed.

## All-or-none publication

The complete request is parsed and validated first. Duplicate keys are refused,
including identical duplicates. Definitions are sorted by key before assigning
identities, so array order does not alter new watch handles or evaluation order.
All new watches share one population/quantity evaluation-work allowance.

The candidate includes existing unselected watches unchanged. The full Agent Turn,
requested summaries, existing active work and persistence metadata must render
within the response budget before any watch checkpoint is committed. With durable
watch storage configured, a changed set produces ONE existing-format checkpoint,
then one in-memory root publication. Without durable storage it is one process-local
publication. A late invalid definition, quantity overflow, exhausted evaluation
budget, insufficient retention or rejected response cannot publish a subset.

Storage uses the existing journal failure semantics: uncertain writes/syncs fence
publication without acknowledging the set. A complete uncertain checkpoint may be
recovered on reopen; an incomplete frame is refused without implicit repair. This
operation adds no cross-file transaction or filesystem power-loss qualification.

## Retry and restart semantics

An existing key must name the exact normalized definition. Matching keys return
the retained record without sampling, renewing deadlines, resetting stability,
reactivating cancellation or changing terminal outcomes. A mismatch rejects the
ENTIRE request. A mixed request may reuse matching existing watches and install
missing ones together. Omitted defaults and their explicit values normalize as in
single-watch registration. Boolean expression rewrites are not silently treated
as equivalent definitions.

The response reports `registered`, `created`, `replayed`, per-key handles and a
`configuration_digest` over the ordered normalized definitions. That digest is a
configuration comparison aid, not a persistent group handle, authority token or
claim of goal completion. Replayed evidence may belong to older observations;
per-record anchors, recovery flags and freshness fields remain visible.

After reopening with the same paired watch/observation journals, submitting the
same definitions returns their freshly recovered handles. It does not count the
bootstrap observation as a sample or undo restart stability resets. Exact replay
adds no checkpoint. An all-terminal requested set has no automatic next step.

Idempotency lasts only while each key remains retained. Explicit `release_watch`
removes that key; a later registration may create a new watch under the normal
future-deadline rules. No tombstone, indefinite deduplication, group replacement,
bulk cancellation, baseline persistence or background monitoring is introduced.

## Bounds and verification status

The existing eight-watches/session and 128-watches/process bounds are preserved.
The ENTIRE registration request shares the existing 32,768-byte accounted input,
1,024 JSON-node and depth bounds. Individual success/failure definitions retain
their combined 64-condition and depth-eight limits. Entity and predicate visits
share one million work units across all new watches, with a cooperative wall-time
allowance. Existing keys are not reevaluated merely to acknowledge a retry.

Fourteen Rust tests are registered: eight engine tests and six actual-handler tests
using existing private-file/capture fixtures. They cover canonical ordering,
defaults, mixed/exact replay, retention, late failures, quantity overflow, shared
scan limits, authority, full-output refusal, one-checkpoint publication, returned
follow-up requests, restart handles/stability and archive refusal.

**These tests have not been compiled or executed here.** Rust, Cargo and rustfmt
are absent, and local DNS access to obtain a toolchain failed. Validation of this
increment is source review and GitHub diff/branch verification only. No Python
model is presented as evidence that these Rust transitions execute correctly.

Focused commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-mcp 'query_watch::batch::registration::tests' -- --test-threads=1
cargo test --locked -p dfmcp-mcp 'watch_batch::tests::registration_tests' -- --test-threads=1
```
