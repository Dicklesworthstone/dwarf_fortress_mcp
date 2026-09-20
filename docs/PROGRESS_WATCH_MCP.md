# Progress watches in the existing MCP loop

The progress/1.12 server now exposes durable archive-bound predicates through
`fortress.query`, keeping the existing eleven top-level tools and signatures.
These are finite observation watches, not game commands, goods-production proofs,
creation reconciliation, or continuously running background jobs. All Rust source
and tests in this increment remain uncompiled and unexecuted in this environment.

## Operator setup and recovery

Alongside the existing optional `DFMCP_WORK_ORDER_PROGRESS_JOURNAL`, configure
`DFMCP_WORK_ORDER_PROGRESS_WATCHES` as a distinct normalized absolute path. Both
files require exclusive private custody: exact-mode 0700 parent directories and
single-link 0600 regular files. The native plugin needs no additional configuration
or RPC. The original progress opt-in, fortress ID and connected-mode credentials
are unchanged; other profiles' DFMCP environment variables remain refused.

The book is opened, bound to its archive, and fully checked BEFORE connecting to
DFHack or acquiring a bootstrap capture. Live mode can initialize a new book.
`fortress.open_session(recovery_only=true)` requires the existing nonempty archive
and, when WATCHES is configured, an existing book. It reads neither the token nor
the endpoint, constructs no source, and cannot register or cancel watches even
with a later injected Observe grant. Query-only historical evaluation works while
DFHack is unavailable. Missing/corrupt intent is never replaced automatically.

Every normal response exposes the bounded definition index, book/archive identity,
retention, mode and a watch-discovery request in `agent_turn.active_work.progress_watches`.
The index does not pretend that outcomes were evaluated or that no pending work
exists. Query `watch_list` for the complete derived outcome set. An unavailable
book is explicitly labeled unavailable, not silently represented as no watches.
Session closure releases both files and the source without cancelling definitions.

## Requests

As with existing history modes, send the following inner JSON as a STRING in the
`history` argument of `fortress.query`, with `session_id` alongside it. Do not mix
`history` with `expected_witness`. The inner envelope is at most 2,048 UTF-8 bytes;
unknown modes/fields, duplicate keys and invalid scalar/digest bounds are rejected.
The schema is `architecture/progress_watch_requests_v1.json`; existence, authority,
latest-origin identity and temporal feasibility remain runtime checks.

After opening and observing a present recognized order, use the returned archive
ID and exact latest record reference to register a finite predicate:

```json
{
  "mode": "watch_register",
  "archive_id": "<archive ID>",
  "key": "bed-approval-001",
  "native_order_id": 3,
  "goal": "validated",
  "deadline_game_tick": 12500,
  "cadence_game_ticks": 10,
  "stable_samples": 2,
  "origin_number": 1,
  "origin_digest": "<exact latest record digest>"
}
```

The numeric example assumes an origin tick strictly below 12,500 with enough room
for two cadence-spaced future samples. The caller must substitute observed values.
Goals are `validated`, `active` (validated, active and nonzero remaining), and
`remaining_at_most`. The last goal additionally requires integer `threshold` in
0..100 and no greater than the recognized total. The other goals omit threshold
(or use null); a numeric threshold with them is rejected. The origin is a baseline,
not a positive stability sample. Registration requires Query and Observe, a deadline
after the origin and at most 120,000 ticks away, cadence 1..10,000, and stability
1..16. The minimum schedule must fit. Exact-key replay never renews any field.

```json
{"mode":"watch_list"}
```

This replays all retained definitions together over the exact current archive,
returns every outcome and its evidence, and reports a pending count. There are at
most 32 lifetime keys, so the whole set is bounded rather than silently paginated
or truncated. A single definition is selected with its sealed digest:

```json
{
  "mode":"watch_status",
  "archive_id":"<archive ID>",
  "key":"bed-approval-001",
  "definition_digest":"<returned definition digest>"
}
```

Obtain another sample using the existing `fortress.wait` with its exact current
observation witness, or `fortress.observe`, then inspect watch status/list again.
The returned `next_sample_tick` is an earliest useful future game tick, not a
scheduled task or instruction to advance the clock. Same-tick reads never inflate
stability. A false observation between cadence points still breaks a positive run.
Every sample is already in the archive, so process failure before status evaluation
does not lose it. No watch query contacts DFHack or implicitly acquires a sample.

Local cancellation requires the exact definition and the archive head observed by
the caller. Intervening evidence is evaluated before the cancellation is synced:

```json
{
  "mode":"watch_cancel",
  "archive_id":"<archive ID>",
  "key":"bed-approval-001",
  "definition_digest":"<returned definition digest>",
  "expected_archive_head":"<current archive head>"
}
```

Cancellation is allowed only while pending; it cannot rewrite a satisfied or other
terminal outcome. It never removes a manager order, sends a native cancellation,
forgets a key or revives a cancelled watch. Retrying the exact cancelled definition
returns the retained cancellation without another event. Live Query+Observe authority
is still required even for that replay; offline mode remains immutable.

## Interpretation and bounds

States are `pending`, `satisfied_observation`, `expired`, `cancelled`,
`missing_outcome_unknown`, `configuration_changed`, `counter_increased`, and
`continuity_lost`. Satisfaction means only that the named predicate held at the
listed positive samples. A deadline sample may satisfy; later samples cannot.
A new archive segment retires unfinished watches, including the first sample after
live reopening. Old terminal evidence remains historical. Neither the same native
ID nor apparently identical configuration establishes continuity across a restart.
Explicitly register a new key/origin to monitor again after a discontinuity.

Definitions and cancellation events are durable, while outcomes are deterministic
projections of verified archive bytes. Historical queries never replace the live
selection, revive expired grants, alter the creation journal, or publish a fabricated
goods-production receipt. File custody is rechecked before operations and before
rendering; uncertain local writes remain discoverable only after verified recovery.
No same-user hostile rewrite protection or external anti-rollback floor is claimed.

WATCHES-configured sessions default/max to 192 MiB of byte WORK allowance, accounting
for maximum archive replay, reference checks, shared watch replay, native bootstrap
and output. This is an allowance, not an allocation or retained size. Archive-only
and transient defaults remain 68 MiB and 2 MiB. Watch startup conservatively reserves
the maximum bounded replay even for a small book. Normal evaluations charge actual
traversed frames. The book itself is at most 128 KiB / 64 events / 32 lifetime keys;
the archive remains 64 MiB / 4,096 records. No automatic compaction, pruning or repair.
The watch horizon allowance is for predicates only and grants no clock-control power.

Watch queries reserve the existing 147,456-byte complete-response envelope before
any durable change. Wall limits stay 1..60,000 ms; output defaults to 65,536 and is
capped at 131,072 four-byte proxy units. Local filesystem calls use cooperative
elapsed checks, not a hard fsync cancellation guarantee.

## Executed evidence and limits

Eight new Rust MCP dispatcher/parser/projection/runtime groups cover registration,
replay, discovery, cancellation, offline mode, current authority, exact pairing,
maximal stability proofs and private-book custody loss. Together with eleven pure
predicate and thirteen book groups, this session registered 32 Rust groups.
NONE was compiled or executed because Rust, Cargo and rustfmt are unavailable.
No Rust formatting, Clippy, actual MCP, filesystem durability, native/live DFHack,
workspace qualification or admission is established.

The independent Python checks pass 2,018 truth schedules, ten temporal controls
and 144 predicate cases; book framing rejects 307 corruptions, 306 incomplete
prefixes, seven illegal rehashed histories and a foreign archive. The MCP reference
passes 17 positive and 55 negative envelope cases with JSON Schema meta-validation.
A maximal watch model is 2,563 bytes; the 32-key index is 5,713 bytes; a complete
32-watch result plus the full envelope allowance is 98,823 bytes, below 147,456.
Those are Python models and lexical source checks, not measured Rust serialization.
Exact source hashes and separate scopes are retained under `docs/evidence/`.
