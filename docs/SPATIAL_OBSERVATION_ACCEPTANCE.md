# Spatial capture acceptance

The spatial/1.8 development runtime now stages every refreshed world before publishing it. The same
acceptance path is used by `fortress.observe`, `fortress.wait`, `await_watch`, and `await_watches`,
whether or not an observation journal is configured. No request schema, tool name, native method,
wire field, archive format or admission permission changes.

This is source-present development functionality. The Rust code and tests have not been compiled
or executed in the editing environment. It does not establish production qualification.

## The publication defect

The previous nonjournaled path called `session.state.publish` immediately after the source returned.
The public observe/watch handlers checked target-tick authority only afterward. A refused response
could therefore leave a new snapshot and entity-generation history installed in the session.
Journal append already rechecked Observe at its candidate tick, but did not require Query at that
target. The runtime also checked only a journal's cached fenced flag and anchor before acquisition,
not current filesystem custody.

`Session::refresh` now delegates to one explicit acceptance boundary:

```text
check current Observe authority, session identity and exact anchor
check source mode/health and current journal custody
pass only the remaining refresh time to one source read
reject a late source result
validate the negotiated rosters, region and capture-byte limit
build an unpublished typed candidate, including generation history
check the actual projected entity count
reauthorize Observe at the candidate tick and fortress
require Query at the candidate anchor when persistence is configured
recheck custody and remaining refresh time
append and sync the journal, or publish the in-memory candidate
```

An Observe-only session without persistence remains supported. This change does not add Query
requirements to that case. Journaled sessions require both read capabilities at the current and
candidate anchors; prior observation timestamps do not authorize admission of newer facts.

A failed candidate does not advance the current source digest, snapshot, observation cursor or
entity-generation map. It does not become input to a watch transition. Once a source call has been
entered, rejection fences that source: its consumed capture is not represented as the session's
current world. Current live analyses then refuse; the existing explicit close/reopen workflow is
available. Historical/local records remain governed by their existing authority and stale-evidence
rules. This is not a continuously refreshed external authorization clock.

A refusal before acquisition does not unnecessarily fence a healthy source. Detected journal
corruption does fence it, and changed storage is not repaired or rolled back. Where a test injects
an extra byte into the archive, that byte remains as evidence of corruption; the runtime does not
append a new observation after it.

## One allowance through the refresh boundary

Before this change, source acquisition could consume its entire wall-time allowance and journal
append would start another full allowance. A nonjournaled projection also had no enclosing post-read
deadline check. The runtime now carries the refresh remainder through acquisition, candidate
validation and the entry to publication. The journal receives only the remaining whole milliseconds,
not the original duration. Less than one millisecond remaining is a refusal.

Checks use injected elapsed time in regression tests, not sleeps. The transport retains its own
absolute I/O deadline and fixed-profile checks. The source is still called at most once; this does
not add retries, reconnects, detached work or game-clock control.

The acceptance boundary is **not** a hard preemption guarantee. Filesystem calls, allocation and
projection are synchronous and cooperative. Once a journal append has synced successfully, the
session adopts the committed root without a fallible post-sync timeout check that would hide it.
Existing journal failures retain their uncertainty/fencing and reopen semantics. Staging also adds
a bounded projection copy; it is not a performance or peak-memory qualification.

## Bootstrap connection and first capture

The live opening path uses `connect_and_capture`: connection negotiation, the first capture, bounds
validation and initial typed projection share one source-bootstrap allowance. Previously, connect
and the immediate refresh each received the full configured duration.

Invalid configuration fails before connection. Exhaustion after negotiation prevents the first
read; exhaustion after the read or projection prevents returning a source/world pair. Unpublished
sources are dropped on error, before any observation/watch journal is opened by session startup.
The existing slot guard releases capacity when opening unwinds, and session registration still
occurs only after the opening response has been rendered.

The bootstrap source allowance does not include subsequent archive replay, watch recovery or final
response rendering. Those retain their existing bounds. This increment does not claim one global
hard deadline for the entire open-session request.

## What remains separate

An accepted observation and a watch checkpoint are separate publications. Later watch evaluation or
response rendering can fail after a valid observation has been retained. The runtime must not erase
that observation or claim it never happened. This increment prevents *rejected captures* from
becoming current state; it does not turn observation plus watch storage into a single transaction.

Archive-only sessions still refuse acquisition. No capabilities are inferred from stored data. No
game effect, reservation, save checkpoint, protocol admission or compatibility claim is added.

## Regression coverage and execution status

Fifteen Rust test functions are registered under the spatial runtime's `observation` module:

- eight acceptance scenarios using real typed projections and private observation files;
- four bootstrap deadline, bounds and unpublished-source cleanup scenarios;
- three actual MCP-handler scenarios for observe/wait and single/batch watch acquisition.

Coverage includes target Observe and Query expiry, unchanged rejected roots, caller/session fences,
pre/post-read deadlines, narrowed projection limits, corruption before/during acquisition, heartbeat
publication, epoch reset, no additional sampling after refusal, and successful shared-watch capture.
All new elapsed-time crossings are deterministic; they do not rely on timing sleeps.

**None of these Rust tests has run here.** Rust, Cargo and rustfmt are unavailable; container DNS
access to repository/toolchain hosts failed. Source/diff inspection and verified GitHub commits are
not substitutes for execution. No independent Python mirror is presented as Rust evidence.

```bash
cargo test --locked -p dfmcp-mcp 'observation::' -- --test-threads=1
cargo test --locked -p dfmcp-mcp -- --test-threads=1
```

The second command also checks compatibility with the existing archive, monitoring, lifecycle and
production suites. Full workspace, native DFHack and disposable-fort qualification remain separate.
