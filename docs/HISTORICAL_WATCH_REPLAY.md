# Historical monitor replay

`historical_watch_replay` runs the existing foreground watch transition machine
against a bounded range of exact archived spatial/1.8 captures. Unlike
`historical_series`, which inspects each condition independently, this operation
answers a temporal question: would the declared sampled monitor have reached
satisfaction, failure, expiry, or invalidation under its cadence and stability
rules starting at the chosen first record?

This is request-owned historical analysis. It creates no retained watch, baseline,
obligation, timer, game effect, bridge read, or journal record. It neither proves
that a watch actually existed at that time nor certifies continuous game behavior.
The core reuses `Watch::advance_bounded`; there is no second transition algorithm.

## Request

Use `fortress.query` in either a live journal-backed spatial/1.8 session or an
archive-only session. A fenced live connection does not prevent verified archive
analysis. First discover exact record numbers and digests with `mode="history"`.
The following digest strings are placeholders; substitute the actual returned
identities and a game-tick deadline after the first selected capture:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_watch_replay",
    "from": {"record": 1, "record_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
    "to": {"record": 16, "record_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
    "definition": {
      "condition": {
        "op": "item_quantity",
        "scope": "observed_projection",
        "quantity_unit": "stack_units",
        "predicate": {"op": "field", "field": "type_key", "comparison": "eq",
                      "value": {"type": "text", "value": "DRINK"}},
        "comparison": "ge", "value": 50
      },
      "deadline_tick": 42337000,
      "poll_interval_ticks": 10,
      "stable_observations": 3
    },
    "detail": "summary"
  }
}
```

Pass this envelope as the tool's `query` argument, with the session ID separately.
Optional `expected_anchor` refers to the session's current retained anchor, not
the first archived sample. Paths, endpoints, credentials and protocol selectors
are not request arguments.

The definition accepts the same success/failure predicates as foreground watches,
including generation-fenced fields, relationship-scoped counts, stack quantities,
requested terrain masks and Boolean combinations. `failure_condition` is optional.
Cadence defaults to one game tick; stability defaults to two samples. The original
shared 64-node/depth-eight condition validation remains in force. There is no key,
label or live watch handle in the replay definition.

Raw stack units are not usable supply, nutrition, reachable inventory or native
requirement satisfaction. A satisfied replay proves its declared sampled predicate,
not successful production or action completion. The same qualifications apply to
workshop job counts and other observation-based predicates.

## Temporal rules

The first selected record acts as registration. Success streaks and cadence use
actual game ticks and observation cursors, not the count of archive records.
Unknown fields reset stability and block success; failure guards and recycled
entity generations retain the foreground evaluator's precedence. Regressed
clocks, observation-epoch changes and incompatible anchors invalidate unfinished
monitoring. A reached deadline expires work that has not stably completed.

The first terminal result remains terminal. Later selected records are still
fully decoded and verified, including corruption checks, but do not reopen or
resample that terminal monitor. `first_terminal_record`, `last_evaluated_anchor`,
`records_evaluated` and `records_after_terminal` make the distinction explicit.
A deadline between captures is detected at the next evaluated capture; no
unobserved tick or missing intermediate game event is invented.

This retrospective definition schedules no future work. Its historical deadline
is not limited by the current session's future `max_game_ticks` allowance, but must
follow the first archived tick. Current Query authority, cancellation, fortress
scope, acquisition limits, response limits and wall-time checks still apply.
Archived timestamps never revive an expired grant.

## Results and resource boundaries

The request selects 1..32 consecutive retained records, inclusively. It has no
pagination or resumable replay handle. Shorter independent ranges restart the
modeled monitor at their own first record; their streaks cannot be stitched
implicitly. All selected identities are validated before replay callbacks run.

One prefix pass reconstructs raw and compressed observations with the fixed
spatial/1.8 journal codec. Only the selected captures enter the monitor. Earlier
prefix records are still needed for payload bases and entity-generation history.
The operation is not constant-time random access and does not trust raw keyframes
as independent generation checkpoints.

All selected monitor evaluations share one million work units and a cooperative
wall-time allowance that also covers replay and rendering. Each reconstructed
capture must fit the session's native acquisition bounds. Output reservation
includes exact range witnesses and mode-specific Agent Turn metadata. The final
response is checked whole; insufficient space rejects it without truncating a
status, fact, or transition.

`detail="summary"` returns the final sampled status, counts, first terminal record,
last evaluation anchor and evidence identities. `detail="evidence"` additionally
returns status transitions and the final predicate evaluation. Summary responses
explicitly state that transition details are omitted; this is a requested
projection, not partial replay. Both detail modes have the same evidence identity.

Replay identity binds the normalized definition, archive incarnation and exact
endpoint digests. Incidental request/session IDs and later appends do not change
that identity. The response separately reports the current journal head and
session anchor. A `replay_id` cannot be used with poll, cancel, release, prepare
or commit operations.

Live responses retain current active-watch metadata without sampling it; paired
watch-journal bytes remain unchanged. Archive-only responses load no live or
persisted watch state. Observation state, archive bytes, generation history,
append compression base and statistics are not changed. Errors after a terminal
sample still reject the whole result if the remaining selected archive is corrupt.
There is no repair, compaction, archive migration, native method, dependency,
mutation capability or production-admission change.

## Validation status

Eleven core Rust tests and six actual spatial-handler tests are registered. They
cover cadence, stability resets, unknown/failure guards, recycled IDs, deadlines,
epoch transitions, terminal freezing, deterministic evidence, current authority,
shared evaluation work, full-output refusal, schema discovery, paired-journal
noninterference, archive reopen and corruption after early terminal satisfaction.
The handler fixtures use real private files with injected native-read boundaries.

Rust, Cargo and rustfmt are unavailable in this editing environment. **These
17 tests have not been compiled or executed.** Source wiring and balanced lexical
delimiters were checked, but those checks are not Rust compilation or execution.

Executed:

```bash
python scripts/test_historical_watch_replay_schema.py --envelope-only
```

The envelope reference passed 104 cases (16 accepted, 88 rejected), plus eight
independent range-boundary cases. It treats conditions as opaque objects and does
not validate the full composed condition schema or execute the Rust schema
composer, monitor, archive, authority checks, MCP or game. Exact tested schema
SHA-256: `a2a476c7c514ae7bc32eb0dd07b52481c29fa80a208fd2206cc51132b97ceba5`.

Focused Rust commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-mcp query_watch_replay -- --test-threads=1
cargo test --locked -p dfmcp-mcp spatial_history_watch_replay -- --test-threads=1
```

The preceding compressed-projection fix registers eight additional adapter tests.
Neither increment establishes full Rust, stdio, filesystem crash, DFHack-native,
live-game, or repository-wide qualification. All existing admission gates remain.
