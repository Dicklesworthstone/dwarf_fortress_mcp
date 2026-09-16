# Item quantity inspection and monitoring

The spatial/1.8 runtime now supports `item_quantity`: a read-only quantity query and a watch
condition over selected item records. Population watches count records; this operation sums their
observed `stack_size` fields. Splitting one seven-unit stack into stacks of three and four changes
the record count, not the quantity threshold.

This is implemented, unadmitted development source. The registered Rust tests have not been compiled
or executed in the editing environment. Native protocols, dependencies, production admission and
game-effect authority are unchanged.

## Inspect before registering a watch

Send a structured query to the existing `fortress.query` tool:

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "item_quantity",
      "scope": "observed_projection",
      "quantity_unit": "stack_units",
      "predicate": {
        "op": "field",
        "field": "type_key",
        "comparison": "eq",
        "value": {"type": "text", "value": "BAR"}
      }
    }
  }
}
```

`BAR` is illustrative: select the observed type/material keys relevant to the intended resource.
Predicates reuse the population-watch language: `always`, typed `field` comparisons, `all`, `any`
and `not`. Nested quantity/count expressions are not row predicates. Item kind and `stack_size`
are fixed by the operator; a caller cannot select a different quantity field or unit interpretation.

The result contains `quantity_min`, `quantity_max`, `quantity_exact`,
`upper_bound_established`, matched-record and uncertainty counts, exact snapshot/predicate digests,
and at most two matching and two unestablished entity examples. Examples are bounded witnesses,
not a partial population used to calculate the answer. There is no pagination: the complete bounded
measurement either fits and succeeds or returns a budget error.

This query performs no native capture, creates no watch and does not evaluate existing watches.
Current watch metadata is attached through the existing response path without advancing stability
or appending a checkpoint. The same measurement implementation is used by watches; inspection does
not contain a second approximation of their quantity semantics.

## Register a quantity threshold

Use this as an ordinary watch's `condition` or `failure_condition`:

```json
{
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
  "value": 20
}
```

Supply the normal watch key, future game-tick deadline, cadence and required stable observations.
The existing single-watch and batch APIs evaluate it. `await_watches` still acquires at most one
coherent capture and publishes the selected watch set together. Count and quantity conditions can
coexist in the same success/failure tree or batch.

Membership follows each new observation. A quantity condition does not capture a fixed set of item
IDs, and consecutive successful samples can involve different items. A one-unit decrease cannot be
hidden by treating every remaining stack as one unit; conversely a stack split does not by itself
reset a quantity threshold. Usual sampling gaps, unknown failure guards, deadlines and epoch fences
still apply. A retained terminal success is historical sampled evidence, not a continuously renewed
stock guarantee.

## Uncertainty is a bound, not an invented zero

A stack quantity is known only when `stack_size` is a U64 value with consistent presence, a native
DFHack field source, a nonzero source digest and the selected snapshot's game tick. Missing, absent,
omitted, stale, unsupported, redacted, contradictory and wrong-type values are not coerced. Signed,
textual and fixed-point values do not silently become stack units.

Each item contributes as follows:

| Predicate | Quantity | Contribution |
|---|---|---|
| False | Any | Zero; an excluded item needs no quantity evidence. |
| True | Known q | Exactly q. |
| Unknown | Known q | Between zero and q. |
| True or unknown | Unestablished | Nonnegative, without an established upper bound. |

Five known units plus an uncertain nine-unit stack produce [5,14]. `>= 5` is true, `>= 15` is false,
and `== 5` remains unknown. Five known units plus a selected stack of unestablished size produce
`quantity_min=5`, `quantity_max=null`; this null is an absent upper bound, not zero or u64::MAX.
Known zero-sized stacks contribute zero even when membership is unknown; the membership uncertainty
is still disclosed independently.

Comparisons are decided only when the entire conservative interval agrees. Weighted membership can
leave gaps in possible totals, so the interval may return unknown where a costly subset-sum analysis
could decide equality. No subset-sum exactness is claimed. Established lower/upper sums use checked
u64 arithmetic; overflow is an error, never a wrapped/saturated result or partial success.

## Raw quantity is not available supply

`scope="observed_projection"` and `quantity_unit="stack_units"` are mandatory. Zero establishes only
zero selected quantity within this projection. It is not complete-world absence, continuous truth,
food portions, nutrition, elapsed production or successful execution.

Every selected native item record contributes its own stack size once. Predicates may filter direct
observed flags, but this operation does not infer inherited container restrictions, job reservations,
path access, edible contents, native material requirements or labor availability. A container and
its contents remain distinct records, not an inferred containment-adjusted resource amount.
`usable_supply_proven` and `complete_world_quantity_proven` are explicitly false. Use the existing
`inventory_plan` or `spatial_inventory_plan` for their separately declared conservative supply and
restricted-route models; those models also do not grant game authority.

## Historical reads and recovery

The pure query is accepted inside `historical_query` with an exact retained record number and digest,
both in live sessions and archive-only recovery sessions. It computes from that reconstructed
capture without replacing the current world or sampling current watches. Archive results remain
historical with current freshness unproved. Archive-only sessions still cannot register, evaluate
or await any watch, including quantity watches.

Quantity watch definitions serialize through the existing durable checkpoint format. Reopening uses
fresh session-bound handles and resets unfinished stability; inspection of the retained capture
does not turn the restart observation into a successful sample. New binaries retain old definitions.
Older binaries that do not understand `item_quantity` reject those definitions rather than silently
reinterpret them. Do not expect downgrade compatibility for journals containing the new condition.

## Work, authority and publication

Quantity predicate nodes share the enclosing watch's 64-node/depth-8 definition bound. Each entity
visit, predicate node and selected quantity read consumes the same evaluation budget as count and
ordinary watch conditions. A whole watch batch shares the existing 1,000,000-unit ceiling; a new
allowance is not created for each aggregate leaf. Session entity limits and cooperative wall-time
checks remain in force. The pure query uses the same definition and scan rules with no stored state.

Current Query authority, exact anchors, snapshot integrity and complete output budgets are checked.
Candidate watch state is calculated separately, then completely rendered, durably synced when
configured, and published through the existing lifecycle. Failed evaluation, rendering or checkpoint
publication cannot expose a partially updated batch. Filesystem and scheduling deadlines remain
cooperative, not hard-preemptive guarantees.

## Validation status

Fifteen new Rust regression functions are registered: ten measurement/watch-engine tests and five
actual spatial-handler tests. They cover unit totals versus record counts, uncertain membership and
quantities, type/presence/provenance refusals, zero quantities, arithmetic/work exhaustion, failure
guards, response rejection, stack splits/merges in shared batches, unchanged watch checkpoints on
inspection, durable reopen, historical/offline quantities and schema discovery. Existing archive
schema tests are extended to thirteen stateless variants plus three history variants.

These Rust tests have **not been compiled or executed here**. Rust, Cargo and rustfmt were not found,
and direct network access to the toolchain host failed. No Rust/MCP/native/live-game qualification
or production admission is established by this increment.

The executable independent checker passed 520 schema cases (475 accepted, 45 rejected), 6,552 finite
interval cases, and 124,410 comparison checks across 1,885 small population models, plus overflow and
large-integer checks. It consumes the actual new extensions and only the base `name` and
`watch_literal` definitions; it does not validate the complete MCP envelope or execute Rust, stored
watch recovery, authority, native capture, filesystem publication or runtime schema composition.
Unknown-quantity substitutions are bounded adversarial witnesses, not enumeration of infinity.
The committed script and both extension files were checked against the executed Git blob identities.
Executed script SHA-256: `a02955780045531726d09b544f4db2ba17ea2b016064591cd14bdb49b17084dc`.

```bash
python3 scripts/test_item_quantity_contract.py
cargo test --locked -p dfmcp-mcp quantity -- --test-threads=1
cargo test --locked -p dfmcp-mcp quantities -- --test-threads=1
```
