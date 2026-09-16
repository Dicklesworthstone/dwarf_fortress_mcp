# Watches over changing observed populations

The shared watch engine now accepts `entity_count` conditions. Live spatial/1.8 schema discovery
advertises the additive condition through the existing `fortress.query` tool. It can monitor a
changing group without listing every current entity ID, creating one watch per entity, or capturing
an immutable query baseline first. Both success conditions and failure guards can use counts.

This is source-present, unadmitted development functionality. Rust compilation and execution have
not been established for this increment. No native protocol, dependency, production admission,
new top-level tool, background task or game-effect authority is added.

## Example: several unassigned, unsuspended jobs

Use the normal watch registration request, with a future game-tick deadline inside the session's
negotiated horizon. The session handle and deadline below are illustrative and must be replaced
with values appropriate to the current session:

```json
{
  "session_id": "<live spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "watch",
      "key": "unassigned-work",
      "condition": {
        "op": "entity_count",
        "scope": "observed_projection",
        "kind": "job",
        "predicate": {
          "op": "all",
          "args": [
            {"op": "field", "field": "suspended", "comparison": "eq", "value": {"type": "bool", "value": false}},
            {"op": "field", "field": "worker_assigned", "comparison": "eq", "value": {"type": "bool", "value": false}}
          ]
        },
        "comparison": "ge",
        "value": 3
      },
      "deadline_tick": 42336100,
      "poll_interval_ticks": 1,
      "stable_observations": 2
    }
  }
}
```

This reports a count threshold in the observed job roster, not a labor shortage, stalled production,
or proof that the jobs cannot complete. Existing `poll_watch`/`await_watch` and
`poll_watches`/`await_watches` evaluate these definitions. Batch awaits retain the one-capture,
whole-watch-set publication behavior. Situation queries and ordinary observations do not implicitly
sample population watches.

Supported entity kinds are `unit`, `job`, `building`, `item`, `tile_feature`, and `announcement`.
Only kinds actually present in the current profile's projection contribute records. Predicate
operators are `always`, `field`, `all`, `any`, and `not`; nested counts are not accepted inside a
row predicate. Field comparisons reuse the existing typed watch literals and `eq/ne/lt/le/gt/ge`
semantics. `{"op":"always"}` counts every record of the selected kind in the projection.

## Count uncertainty is explicit

For each currently projected entity of the selected kind, its predicate is true, false or unknown.
A field contributes a known value only when its presence is consistent, its source is a native
DFHack field with a nonzero source digest, its game tick matches the snapshot, and its type agrees
with the literal. Missing, absent, omitted, unsupported, redacted, stale and contradictory fields
remain unknown. Negation does not make an unknown field known. Boolean composition uses the same
three-valued logic as ordinary watch conditions.

The count is a closed interval:

```
matched_min = definitely matching records
matched_max = definitely matching records + records whose predicate is unknown
```

The condition is true only if every possible count in that interval satisfies the comparison, false
only if every possible count fails, and otherwise unknown. For example, one known match and one
unknown record give [1,2]: `>= 1` is true, `>= 3` is false, and `== 1` or `!= 1` is unknown. An unknown
failure guard continues to block success under the existing watch state machine.

Count evidence includes the interval, population size, unestablished count, comparison and threshold,
exact snapshot hash, predicate digest, and up to two canonical matching and two unestablished
entity examples with generation/revision. Examples are bounded witnesses, not the monitored set.

## Projection scope is mandatory

`scope: "observed_projection"` is required. It cannot be replaced with `complete_world` or omitted.
A zero count proves only that the named projected set contains zero matches under the declared
predicate. It never proves global absence, continuous truth, game-effect success or safety.

Counts are entity records, not summed stack quantities, food portions, available workers, elapsed
job time or deduplicated events. Item flags and native availability rules are not silently inferred.
Retained announcements are not complete history; terrain is limited to the requested region; hidden
terrain facts remain unknown. An unsupported/unprojected entity kind can have zero records while
that category exists outside the observation coverage.

Population membership is dynamic: newly observed identities participate and departed identities
stop participating at the next sample. Consecutive successful count samples need not contain the
same individuals. Specific-entity Field conditions retain their existing generation checks when
combined with population conditions. Reset epochs, deadlines, cadence and sampling-gap rules are
unchanged. Terminal outcomes are retained historical evidence and are not resampled.

## Bounds, publication and recovery

Count-predicate nodes share the watch's existing 64-node and depth-8 success/failure definition
budget. They do not receive an independent 64-node allowance for every aggregate leaf. Every
condition node, population entity visit and row-predicate node is charged. A single watch evaluation
or complete watch batch has a shared 1,000,000-unit ceiling, in addition to the session entity and
wall-time bounds. A batch cannot reset that allowance for each selected watch. Deadline checks are
cooperative; this is not hard-preemptive execution.

Work exhaustion is BudgetExceeded, not a partial count, zero matches or an invented unknown result.
Candidate watch state is still computed separately, rendered completely, and then checkpointed and
published. A late evaluation, output or checkpoint failure cannot publish the first part of a batch.
No deadline check after checkpoint sync hides a committed watch root.

Existing checkpoint framing and evidence sealing are unchanged. New readers accept old definitions;
count definitions serialize into the same bounded definition slot and recover with their exact
predicate. Older binaries that do not know `entity_count` refuse such saved definitions rather than
reinterpreting them. Do not expect downgrade compatibility for a journal containing new conditions.
Restart retains definitions but resets unfinished stability and requires fresh evidence, just like
ordinary durable watches. Archive-only and historical query paths still refuse watch evaluation.

## Validation status

Thirteen new Rust scenarios are registered: nine engine tests and four actual spatial-handler tests
using the existing injected captures and private observation/watch journals. They cover count
intervals, uncertain/negated fields, native provenance, changing membership, failure guards,
definition/work bounds, refusal before publication, shared captures/checkpoints, restart, schema
discovery and explicit projection scope. **They have not been compiled or executed here.** Rust,
Cargo and rustfmt were not found in the editing environment; direct network toolchain access failed.

The independent Python checker passed 152 condition-schema cases (112 accepted, 40 rejected),
2,970 integer-interval cases and 59,022 population-completion cases. It validates the new condition
schema with only the base `name` and `watch_literal` definitions, plus an independent mathematical
model. It does not execute Rust, the full MCP envelope, journal recovery, authority, scan accounting,
response publication or DFHack. The executed script and extension match their committed Git blobs.
Executed script SHA-256: `cf5c802f4e4419622d711a33224305c35dcfb25a774c7e618557e10f37482edd`.

```bash
python3 scripts/test_watch_count_contract.py
cargo test --locked -p dfmcp-mcp query_watch::counts -- --test-threads=1
cargo test --locked -p dfmcp-mcp count_tests -- --test-threads=1
```

These source and reference checks establish no Rust, native, live-game or production qualification.
