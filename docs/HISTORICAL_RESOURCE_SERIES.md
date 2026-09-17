# Historical resource quantity series

`historical_series` adds a resource timeline to the existing spatial/1.8
`fortress.query` tool. It runs against journal-backed live sessions and offline
`recovery_only` sessions. It does not require a baseline registered before the
observations were collected, and it performs no new native capture.

This is unadmitted development source. The Rust implementation and tests have
not been compiled or executed in the editing environment. No runtime, native,
live-game, filesystem-crash or production qualification is established.

## Select a retained interval

First use the existing `history` query to obtain real record numbers and digests.
The interval is inclusive, chronological in record order, and limited by the
existing observation journal's retention. Use exact returned digests, not the
illustrative placeholders below.

```json
{
  "session_id": "<live or archive session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "historical_series",
      "from": {"record": 1, "record_digest": "<record 1 digest>"},
      "to": {"record": 3, "record_digest": "<record 3 digest>"},
      "measurement": {
        "kind": "item_quantity",
        "scope": "observed_projection",
        "quantity_unit": "stack_units",
        "predicate": {"op": "always"}
      },
      "limit": 8
    }
  }
}
```

The example sums raw stack units across all observed items. Narrow it with the
existing quantity predicate language, for example a typed equality on an
observed `type_key`, and optionally material fields. The measurement is the same
`item_quantity` operation used for current inspection and quantity watches; it
is not a separately implemented approximation. Schema discovery composes its
contract from `mcp_item_quantity_v1.json` and the existing predicate definitions.
The series schema's uncomposed measurement placeholder deliberately refuses
all values rather than acting as a permissive standalone schema.

Only item-quantity measurements are accepted. A measurement cannot capture new
observations, mutate/register/evaluate a watch, change a baseline, perform a
production allocation, recursively select history, or dispatch an effect.
`historical_series` itself cannot be nested inside `historical_query`.

## Interpret the samples

Each complete row includes the exact journal entry, its full anchor and source
and record digests, the ordinary quantity summary and evidence digest, and a
change relative to the preceding retained sample in the requested interval.
The first interval row has no preceding-sample comparison.

Quantities retain their conservative lower and upper bounds. If the earlier
quantity is [a,b] and the later quantity is [c,d], net change is [c-b,d-a]. An
unestablished endpoint bound leaves the corresponding change bound open. For
example, [80,100] followed by [85,95] gives [-15,15], not a proven decrease.

A comparison reports `definite_increase`, `definite_decrease`,
`unchanged_quantity`, or `indeterminate_change`. Equal uncertain intervals do
not imply unchanged stock. Signed changes are decimal strings so all differences
of two u64 quantities remain exact without floating-point or i64 overflow.

With a strictly positive game-tick difference, `net_rate` contains exact signed
numerator bounds and an integer game-tick denominator. It is an endpoint net
change rate, NOT a consumption or production rate. No extrapolation, time-to-empty
prediction, recipe, causal explanation, or future supply claim is made.

Distinct observations at the same game tick can have a quantity change, but
have no rate: `rate_unavailable_reason` is `same_game_tick`. An epoch change or
clock regression emits `epoch_or_clock_discontinuity` and no cross-boundary
change or rate. The following sample can start a new within-epoch comparison.
A restored fortress is therefore not reported as an enormous resource loss.

The measurement follows its predicate at each sample. A changed selector field,
ID reuse, or item entering/leaving the observed projection can change totals.
Raw quantity is not conservative usable supply, food portions, nutrition, access,
reserved stock, or complete-world absence. Use the existing inventory and
production models for their separately declared supply restrictions.

## Bound the work and continue

Pages contain 1..32 complete samples (default 8). A follow-up copies the request
and sets `continuation` to the returned `hs1` token. Limit and output budget may
change. Tokens bind the session, current session anchor, archive incarnation,
exact head, interval, and measurement. A new capture, changed selector, different
interval or reopened session invalidates the old token; rediscover/restart the
request rather than splice unrelated pages.

The first sample on a continued page still compares with its immediate
predecessor. That predecessor is reconstructed internally but not returned twice.
Pagination never silently drops the change at a page boundary. A response that
cannot fit one whole row fails explicitly instead of emitting a nonprogressing
empty page.

The adapter's `ObservationJournal::project_records` replays one prefix from its
verified header through the last requested page sample. It reuses the fixed
profile decoder and generation publication logic. It holds one reconstructed
world at a time and retains only the bounded projected outputs. It validates
all requested record identities before callbacks, verifies unselected prefix
records too, and returns no partial result on a failed projection or corrupt
record. A corruption failure fences the journal. Current world/generation state
and journal bytes are never replaced, appended, synced, truncated or repaired.

A page evaluates at most 33 quantities: 32 rows plus a predecessor. Every
measurement keeps the existing one-million-unit evaluation ceiling; the whole
page has one cooperative wall-time allowance across replay, measurement and
rendering. It is not a fresh wall-time allowance per sample. Projection output
is internally capped to 16 KiB per measurement. A smaller requested page can
reduce measurement work. Prefix decoding still depends on the selected record
position; this is not a random-access index or performance qualification.

Byte budgets include the full mode-specific Agent Turn, range witnesses,
measurement request, current watch metadata where applicable, and continuation.
The final response is checked again. `replay` reports how far the prefix was
verified and how many samples were evaluated, which can exceed the rows returned
when the byte budget fills before the requested limit.

## Historical evidence remains separate

Current Query authority and journal custody are checked before and after replay.
Old observations do not revive expired grants. No current watch is sampled or
advanced. Live responses carry current watch work, clearly scoped as current;
offline responses do not load persisted watches or claim current freshness.
A failed live connection still permits healthy verified history reads.

Agent Turn continuity is partial, and the response anchor identifies the last
returned sample, not an unreturned endpoint. Every sample has its own exact
anchor. `range` identifies the full requested interval; a partial page does not
claim its unvisited suffix was reverified. The archive records changed captures,
not a continuous event stream, and heartbeats need not create records. Reading
all pages therefore does not prove what happened between captures or during
process downtime.

## Validation status

Twenty-one new Rust test functions are registered: five journal projection tests,
eight mathematical/boundary tests, and eight actual MCP-handler/private-file
tests. The mathematical test enumerates all 1,296 pairs of intervals with bounds
in 0..7 against explicit possible endpoint differences. This is a registered
Rust test, NOT executed evidence.

The scenarios include exact single-record equivalence, one-prefix byte counts,
unchanged worlds/watches/journals, cross-page changes, current-authority expiry,
corrupt unselected prefixes, same-tick changes, reset segmentation, full-width
arithmetic, source failure, offline reopen, stale cursors, strict measurements,
and schema discovery. Existing archive tests retain all prior read variants.

No Rust/Cargo/rustfmt is available here. Validation was source review and GitHub
diff/commit/branch verification, not execution of these tests. Focused commands:

```bash
cargo test --locked -p dfmcp-adapter 'file_storage::projection::tests' -- --test-threads=1
cargo test --locked -p dfmcp-mcp 'history::changes::series::' -- --test-threads=1
```

Native protocols, journal formats, dependencies, top-level tools, game effects
and production admission are unchanged.
