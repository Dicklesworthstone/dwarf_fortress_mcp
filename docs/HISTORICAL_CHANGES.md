# Historical endpoint changes

Spatial/1.8 now answers what changed between two retained observations through the existing
`fortress.query` tool. It works in both journal-backed live sessions and `recovery_only` archive
sessions. It does not require a previously captured process-local baseline or knowledge of an old
session handle. After restart, discover the records with `history` and repeat the comparison.

This is implemented development source, not a Rust-qualified or admitted runtime. No native
protocol, dependency, game effect or production-admission boundary changes.

## Request

Take the exact record numbers and digests from `fortress.query` with `mode="history"`, then send:

```json
{
  "session_id": "<current spatial or archive session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "historical_changes",
      "from": {"record": 1, "record_digest": "<digest returned for record 1>"},
      "to": {"record": 2, "record_digest": "<digest returned for record 2>"},
      "select": {
        "kind": "entities",
        "kinds": ["job"],
        "fields": ["suspended", "worker_entity", "completion_timer"]
      },
      "limit": 8
    }
  }
}
```

The placeholder digests above must be replaced by actual 64-character lowercase hexadecimal
record digests. An optional outer `expected_anchor` refers to the CURRENT session anchor, not the
older `from` endpoint. Both references must belong to the configured spatial/1.8 journal, be in
forward record order, and lie in the same observation epoch. Comparing a record with itself is
valid. A reset across the interval is a `stale_anchor` error, not mass creation/deletion evidence.

`select` uses the existing entity-selection contract: `kind="entities"`, optional `kinds`, `where`,
`fields`, and `order`. It cannot contain page controls. The comparison finishes both selections
before emitting differences; it never compares two arbitrary partial query pages. A useful scope
can select particular item types, jobs, citizens, or observed terrain fields. Unsupported fields
retain the existing explicit unknown representation.

## Result semantics

The result includes both exact journal-record witnesses, their source digests, the current session
anchor, the comparison basis and target anchors, complete match/change counts, and whole before/
after rows. Differences are ordered by numeric entity ID and then generation:

- `entered_result`: selected only at the later endpoint;
- `left_result`: selected only at the earlier endpoint;
- `changed_in_result`: the same entity generation is selected at both endpoints and its selected
  semantic representation changed.

An ID reused with a new generation produces a departure and a separate arrival. A unit leaving a
filtered selection does not establish death or deletion. A changed known/unknown/absent/omitted/
redacted representation, epistemic class, or source kind remains a semantic change. Only revision,
observation-tick and source-digest bookkeeping are ignored for semantic equality; those differences
are separately counted in `provenance_only_refreshes`. These are the same comparison rules used by
process-local query baselines, not a second divergent comparison implementation.

`basis_result_digest` and `target_result_digest` identify the complete selected projections.
`comparison_digest` also binds the session, current authority anchor, archive incarnation/head,
record pair and selection. These hashes are identity checksums, not signatures or capabilities.

`change_count` and `change_counts` cover the complete bounded comparison, not just the current page.
The `changes` array contains the returned page and `returned` gives its length. Repeat the same
request with its `continuation` to advance; changing `limit` is allowed. A different session,
selection, record pair, current anchor or archive head invalidates the continuation. In particular,
after restart begin again without the old continuation; the record references remain usable while
retained. No baseline is allocated, advanced, persisted or released.

Every response explicitly marks the analysis historical and current freshness unproved. Its Agent
Turn carries the earlier basis, a compact change summary, exact evidence, and partial continuity.
There is no inference about events between endpoints, continuous condition satisfaction, rates,
causality, action success, births or deaths. Replay may traverse intermediate records to reconstruct
entity generations, but the comparison does not evaluate predicates at those intermediate times.

## Live and offline behavior

A live session keeps its current observation and current watch projection. Historical comparison
never samples those watches or evaluates them against old facts. It can still read healthy history
after its native source has been fenced; it does not reconnect or obtain a new capture.

An archive-only session performs the same comparison without bridge credentials, a native process,
Observe authority, or a writable journal. Its existing watch/baseline/effect refusals remain in
place. `historical_changes` is an additional history operation, not a nested `historical_query`
variant: wrapping comparisons recursively is not supported. Archive discovery now advertises ten
single-snapshot analyses and three history operations.

## Bounds and failure behavior

Each endpoint is limited to 256 selected rows and 256 KiB of selected row data. Existing acquisition
limits permit at most 64 query-page attempts and two million source-row visits per endpoint.
Oversized selections fail without silently dropping rows. A change page permits 1..128 whole changes
and must also fit the negotiated entity and output allowances.

Both archive replays, both selected-projection acquisitions, comparison and response rendering share
one cooperative wall-time allowance. The selected rows keep their own bounded retention allowance;
full Agent Turn, both record witnesses, continuation and current watches are reserved before replay
and page filling. A budget too small for one whole before/after change is an explicit failure.
Filesystem operations and individual computation steps do not claim hard preemption.

Current Query authority, cancellation, the exact pair and file custody are checked before replay;
current authority and custody are checked again before return. Exact record replay verifies bytes,
checksums, generation transitions and acquisition bounds. A same-length corrupt selected frame
cannot become a cached successful comparison. No failure repairs or rewrites the observation archive.

## Validation status

Sixteen new logical Rust scenarios are registered: nine shared comparison tests and seven tests
through the actual live/archive MCP handlers using real private journals. They cover generation
reuse, selected-set entry/exit, provenance-only changes, presence/source changes, filtering, epoch
fences, pagination, cold reopen, current authority, bounded output, same-length corruption and
unchanged live watches. Existing baseline/watch/archive tests are retained, and archive schema
expectations include the new thirteenth variant.

**These Rust tests have not been compiled or executed in this environment.** Rust, Cargo and rustfmt
are unavailable, and no full workspace, MCP process, native/live or admission qualification is
claimed. Focused commands on a configured checkout are:

```bash
cargo test --locked -p dfmcp-mcp query_history::endpoints -- --test-threads=1
cargo test --locked -p dfmcp-mcp spatial_archive -- --test-threads=1
python3 scripts/test_historical_changes_contract.py
```

The Python contract checker passed 66 cases (12 accepted, 54 rejected) in its explicit
`--envelope-only` mode. That run validates the new record/page envelope, including canonical digest
lengths and continuation whitespace refusal. It does NOT validate the delegated selector schema,
record existence/order, digest agreement, selection completeness, Rust comparison, filesystem
custody, pagination or MCP behavior. Both executed script and schema bytes were checked against
their committed Git blob identities. Script SHA-256:
`67ef27c59d25a5f83203b505c5e80f81fb487af178288f161622029cbdec69a9`.
