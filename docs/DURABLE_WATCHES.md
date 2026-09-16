# Durable foreground condition watches

The unadmitted spatial/1.8 development server can persist foreground condition watches alongside
its exact observation archive. An agent can restart, rediscover what it was monitoring, and continue
collecting fresh evidence without reconstructing its previous conversation.

This is implemented development source, not Rust-qualified or production-admitted functionality.
The binary and native bridge protocols are unchanged. No new game effect, timer, detached task,
background poller, or top-level MCP tool is introduced.

## Configure paired journals

The existing observation archive is required. Watch entity generations and anchors cannot safely
be recovered from a fresh process-local observation universe. Configure two separate files under
an existing private directory, using the existing bridge credentials:

```bash
install -d -m 700 "$HOME/.local/state/dfmcp"

DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_8=1 \
DFMCP_SPATIAL_CITIZEN_TOKEN='<existing bridge credential>' \
DFMCP_SPATIAL_CITIZEN_JOURNAL="$HOME/.local/state/dfmcp/spatial18.bin" \
DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL="$HOME/.local/state/dfmcp/watches18.bin" \
cargo run --locked --bin dfmcp-live-spatial-citizens-dev-server
```

Open a normal spatial/1.8 session with the same bounded region and suitable acquisition/output
budgets. Both Query and Observe authority are required. Paths are operator environment settings,
not MCP arguments; callers cannot select a journal codec or another profile. Missing watch
configuration preserves the existing process-local behavior.

The new watch path must be absolute, normalized, nonempty UTF-8, and distinct from the observation
path. Both use the existing Unix private-file custody boundary: exact-mode 0700 canonical directory,
exact-mode 0600 single-link regular file, matching ownership and pre/post-open identity checks, and
an exclusive file lock. A second active owner cannot open the same files. Ancestors and the owning
account/root remain trusted; this is not a hostile-host sandbox.

An existing zero-byte file is not a new journal. Only successful exclusive new-file creation
permits initialization. Wrong-profile, wrong-archive, corrupt, or incomplete watch journals are
refused without repair. There is no watch-journal repair environment variable.

## Register and continue monitoring

The existing `fortress.query` interface is unchanged. For example, after observing the current tick,
register a watch whose deadline is in the future and within the negotiated game-tick horizon:

```json
{
  "session_id": "<current spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "watch",
      "key": "maintenance-paused",
      "condition": {"op": "paused", "value": true},
      "deadline_tick": 42336100,
      "poll_interval_ticks": 1,
      "stable_observations": 2
    }
  }
}
```

The tick is illustrative, not an instruction to use that value for another fortress. Field watches
can use generation-bound observed entity fields; unsupported, omitted, stale, unobserved, or
incompatible values retain the existing unknown semantics. Watches never authorize pause or any
other game mutation.

Registration, changed samples, terminal outcomes, cancellation and release are checkpointed before
acknowledgement. Watch results report `durable=true` and scoped `watch_persistence` metadata with the
journal identity, exact checkpoint number/head, observation-journal identity and retained bytes.
Unrelated baselines and query results do not become durable merely because their responses include
current watch metadata.

Use `poll_watch` to evaluate a newly published current anchor, or `await_watch` to request the
existing single foreground observation refresh and then evaluate. Repeated reads of one exact
anchor do not create additional samples, stability, or checkpoint writes. There are no automatic
background evaluations.

## Restart behavior

Startup first replays the observation archive and syncs the freshly acquired native capture. It
then checks the watch journal's fixed spatial/1.8 binding, every checkpoint frame and each stored
creation/evaluation/checkpoint anchor against the exact retained observation archive. A matching
fortress name, nearby tick, or compatible-looking entity ID is not sufficient.

Recovered watches receive new session-bound handles. Old handles are not revived. The open response
includes a recovery summary, active unfinished watches and a discovery request; the common Agent
Turn reports a partial continuity gap when monitoring records were recovered.

Rediscover the current handles through:

```json
{
  "session_id": "<new spatial session>",
  "query": {"schema": "dfmcp.query/1", "query": {"kind": "watches"}}
}
```

For an unfinished watch, recovery preserves its definition, original deadline, prior sample count
and evidence linkage, but resets the consecutive-success streak and sample cadence baseline. The
bootstrap capture is not counted as a successful sample. Its status is `blocked_unknown`, with
`fresh_observation_required=true` and `evaluation_current=false`, until a later distinct observation
is actually evaluated. The suggested next operation is `await_watch` using the newly returned handle.

A watch whose original deadline has been reached during downtime becomes expired; restart never
extends that deadline. An observation epoch change, clock regression or incompatible identity
invalidates unfinished work. A smaller current game-tick horizon cannot be bypassed by restoring
an older definition with a larger remaining horizon.

Previously terminal outcomes remain terminal and explicitly historical. Their original observation
and counters are retained, the new handle links back to the prior evidence, and
`historical_outcome=true`. Even an identical bootstrap anchor does not relabel old terminal evidence
as a freshly performed evaluation. No terminal watch is silently restarted.

Cancellation is a durable monitoring transition with no game effect. A terminal watch may be
released through the existing `release_watch` query; its absence is checkpointed so it does not
reappear after restart. Re-registering a retained key with different content remains a conflict.

## Publication and failure ordering

The ordering is:

```text
capture -> sync observation archive -> publish current observation
        -> compute candidate watch transition
        -> stage checkpoint bytes and prospective receipt
        -> render/check the complete response and active-work metadata
        -> append + sync watch checkpoint
        -> publish in-memory watch root
        -> return response
```

A response-rendering or output-budget failure does not persist a watch transition. A write or sync
failure fences the watch journal and leaves the candidate in-memory watch root unpublished. A
complete frame from an uncertain sync may be recovered on a later open; an earlier caller error
therefore does not prove that no bytes reached storage. An incomplete frame is refused unchanged,
not silently removed. Checkpoint sequence, predecessor, length checksum, record checksum and footer
are all checked before accepting its state.

An observation already synced before a failed watch response remains a valid observation. The two
journals are ordered publications, not an atomic two-file rollback transaction. Recovery verifies
that all watch evidence still belongs to the retained observation archive.

Current Query authority and file custody are rechecked for durable watch reads, including
idempotent registration and terminal lookups. If the watch journal is fenced, failure reporting
retains the original error rather than disguising corruption as an output-budget failure.

Historical spatial queries may display current active-work metadata, but do not evaluate,
advance, or checkpoint watches against archived game facts. Pure current queries likewise do not
sample watches. Dropping session ownership releases process-local records and the file lock; it
does not cancel or delete the persistent monitoring intent.

## Bounds and deliberately absent features

The existing watch limits remain eight retained watches per session and 128 per instantiated
watch store. Definitions preserve the existing 64-condition, depth-eight, bounded-literal and
request-budget restrictions. Each serialized checkpoint is at most 1 MiB; the watch journal is at
most 64 MiB and 4,096 checkpoints. Capacity exhaustion refuses further changed checkpoints. There
is no automatic pruning, rotation, compaction or conversion. Unchanged reads do not consume new
checkpoint capacity. The observation archive has its own independent retention limits.

Replay cooperatively checks wall-time and current authority. Synchronous filesystem operations do
not have a claimed hard cancellation bound. Full checkpoint copies are bounded reference storage,
not a claim of optimal long-campaign storage economics.

Not provided by this increment: offline spatial bootstrap, continuous observation during downtime,
durable query baselines, durable plans/leases/MCP Tasks, automatic goal execution, another native
read profile, game mutation, production admission, authenticated signatures or anti-rollback custody.
Do not replace or prune either paired archive as an implicit migration. Losing the required
observation history causes watch recovery to fail closed.

## Validation status

Twenty-five new logical Rust tests are registered:

- seven binary-journal tests, including every cut within a frame, every single-byte corruption,
  partial write, uncertain sync, exact replay and binding/sequence refusal;
- thirteen durable-watch tests for actual private files, lifecycle/restart behavior, old terminal
  evidence, current authority/horizon, malformed checkpoints, publication failure and fixed vectors;
- two operator-configuration tests and three tests through the actual spatial startup attachment
  and MCP query handlers, using an injected coherent source plus real private journals.

The existing watch suite is retained. The new Rust tests have **not been compiled or executed in the
editing environment**: Rust, Cargo and rustfmt are unavailable. No Rust, Clippy, stdio, filesystem
power-loss, real DFHack/generated-protobuf, live-campaign or full-repository qualification is claimed.

The checked-in `scripts/test_watch_checkpoint_vectors.py` was executed successfully: **740 independent
Python framing-reference checks**, including 363 single-byte corruptions and 361 incomplete prefixes.
Its fixed hashes are also asserted in a registered Rust test. The Python reference uses opaque test
payloads; it does not execute the Rust serializer, watch state machine, custody backend or MCP server.
Exact tested reference-script SHA-256:
`6171594ba703f1b95a458826edcf6acfdf0af002ebe4ce3d8324d99ab3dfac9a`.

Focused commands on a configured checkout are:

```bash
python3 scripts/test_watch_checkpoint_vectors.py
cargo test --locked -p dfmcp-mcp watch_checkpoint
cargo test --locked -p dfmcp-mcp watch_durability
cargo test --locked -p dfmcp-mcp spatial_watch_runtime
```

These do not replace `scripts/verify.sh`, `scripts/qualify_local.sh` or the native/live evidence
required for the exact source generation. The compatibility registry and production runner map
are unchanged; spatial/1.8 remains explicitly unadmitted development functionality.
