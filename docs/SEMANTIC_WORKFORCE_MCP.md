# Original labor goals through MCP

`dfmcp-live-semantic-workforce-dev-server` connects a canonical citizen
observation and one original `SetLabor` goal to the existing workforce/1.17
coordinator. It archives the exact plan and original citizen identities, prepares
one paired semantic/native review, applies that review once after confirmation,
and samples the original labor predicates through fresh selected-workforce
observations. The original deadline, cadence, stability requirement and first
achievement survive verified recovery. Optional separately reviewed bounded
runs advance time between those samples without another clock controller.

This is a separate, explicitly **unadmitted development executable**. It uses
the existing citizen V1, workforce/1.17 and optional bounded-run/1.13 sources and
preserves the eleven-tool waist. It adds no native protocol, production runner,
compatibility entry or
generic command route. The earlier `dfmcp-live-workforce-dev-server` remains a
separate low-level work-detail surface.

Owner: `df-dfhack-bridge-plane-c-pic.4`. The broad native mutation-dispatch bead
remains open; this is one original labor goal, not a multi-family executor.

## What the goal means

The request selects 1–32 canonical citizens, one exact native labor key such as
`MINE`, and a desired Boolean. Preparation resolves those citizens through their
observed canonical identity fields. It requires exactly one existing
selected-only work detail that grants exactly that labor. The client cannot
select a detail index, cast native IDs into canonical IDs, change the labor
schema, create a detail, assign a job or nominate a native idempotency key.

Disabling refuses another detail that could keep granting the labor to a selected
citizen. Native preparation requires an actual membership change; an already
unchanged membership is not a new assignment. Native readback must prove the
requested labor value for every selected citizen and preserve every other
captured labor column to count as a verified single-labor action.

The result keeps four claims separate:

| Claim | Required evidence |
| --- | --- |
| Historical assignment verification | Exact native Applied receipt and the original single-labor readback check |
| Current original-goal proof | Fresh independently authorized selected-workforce capture, all original predicates and the original temporal obligation |
| Semantic completion | Both the original goal proof and verified historical single-labor action |
| Historical clock coordination | Exact retained run intent and native receipt; elapsed ticks and a historical pause do not prove current pause or original-goal completion |

An Unknown assignment stays unknown even if fresh observations show the requested
labor values. An Applied receipt can remain historically valid after the current
labor changes again. Neither proves productivity, job completion, skill, tools,
path access, exclusive assignment or whole-fortress quiescence.

## Operator setup

The runtime accepts only these environment names in its own process:

```text
DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE=1
DFMCP_SEMANTIC_WORKFORCE_CONFIG=<closed JSON described below>
DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN=<operator-managed citizen credential>
DFMCP_SEMANTIC_WORKFORCE_TOKEN=<operator-managed workforce credential>
DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR=1
DFMCP_SEMANTIC_WORKFORCE_CLOCK_TOKEN=<optional operator-managed run/1.13 credential>
DFMCP_SEMANTIC_WORKFORCE_ALLOW_CLOCK=1
```

Credentials are 32–256 UTF-8 bytes, never MCP arguments or output. The labor and
clock gates must each be absent or exactly `1`. The clock token and gate are
optional; clock control needs its own gate and configured source. Reconcile and
offline operation do not require either control gate.
All other `DFMCP_` state, including production launch and another development
profile's variables, is refused. The native game process has its own separately
configured plugin environment; see [WORKFORCE_NATIVE.md](WORKFORCE_NATIVE.md) and
[LIVE_DFHACK_READ_PATH.md](LIVE_DFHACK_READ_PATH.md).

The config below illustrates the shape; the versions and incarnation numbers
must be the operator's exact observed source values:

```json
{
  "schema": "dfmcp.semantic-workforce-development/1",
  "directory": "/absolute/private/semantic-workforce",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "sources": {
    "kind": "operator-development-source-acceptance/1",
    "world_folder": "region1",
    "site_id": 7,
    "df_version": "exact-df-version",
    "dfhack_version": "exact-dfhack-version",
    "citizen_bridge_version": "exact-citizen-bridge-version",
    "citizen_generation": 10,
    "workforce_generation": 20
  }
}
```

The two incarnation numbers belong to different plugins and need not agree.
Source acceptance is an explicit development trust decision; it does not mean
the empty compatibility registry admits either source. The endpoint must be
numeric loopback with a nonzero port. The config is bounded to 16 KiB and rejects
unknown fields, source-policy additions and caller-selected runtime callbacks.

To enable reviewed clock runs, also add `"clock":{"generation":30}` to that
config, replacing `30` with the exact accepted run/1.13 source incarnation.
The clock shares the configured endpoint and DF/DFHack version pins; its
generation is independent of the citizen and workforce generations. Run/1.13
identifies a source incarnation, **not a named fortress lineage**. The operator
clock gate independently issues the source-bound authority; named-fortress
labor authority is never rewritten into that grant. Matching ticks do not prove
fortress identity or exclude other controllers. See
[SEMANTIC_WORKFORCE_CLOCK.md](SEMANTIC_WORKFORCE_CLOCK.md) for the complete
source, authority and private-custody contract.

Create the directory beforehand with exact mode `0700`. The runtime owns these
fixed files under existing descriptor-pinned Linux x86_64/aarch64 custody:

| File | Contents |
| --- | --- |
| `workforce.journal` | Exact native source binding, kernel-random journal identity and durable assignment history |
| `semantic.associations` | Immutable association of original plan and native assignment |
| `original.plan` | Lossless exact original `PreparedPlan` archive |
| `original.capsule` | Original canonical citizen bytes used to reconstruct exact identity lineage |
| `goal.history` | Durable original-goal observations, continuity and first-achievement history |
| `clock.journal` (when configured) | Independent exact clock source binding, journal identity and original-goal-bound run history |

Files must remain single-link regular files with exact mode `0600`; symlinks,
replacement, permission drift and incomplete bytes refuse. No file is silently
overwritten, recreated, pruned or repaired. One process retains the file locks
for one goal. Initialization and reopening are different operations: initialize
requires absent files and current labor control, while reopening requires the
existing exact native journal identity, even in Control mode.
Clock initialization additionally requires current clock control and an absent
clock journal. Every clock reopen requires its independently retained exact
journal identity. A missing clock journal is never recreated during recovery,
and omitting clock configuration cannot hide an existing clock file.

Build/run entry point:

```bash
cargo run --offline --locked -p dfmcp-mcp --bin dfmcp-live-semantic-workforce-dev-server
```

This command identifies the executable; it does not assert a successful
FastMCP executable build or a live-game qualification in this environment.

## Observe, select, prepare and confirm

The guide uses the conceptual dotted tool names. The existing FastMCP macro
registers the corresponding underscore wire names, such as
`fortress_open_session` and `fortress_commit`.

Open `fortress.open_session(mode="control")` for initialization. To permit
bounded clock runs, negotiate a nonzero allowance at opening, for example:

```json
{
  "mode": "control",
  "max_game_ticks": 1200
}
```

`max_game_ticks` defaults to zero and cannot exceed 1200. Opening never
connects to DFHack, reads a credential, chooses citizens or changes the game.
The default mode is `offline`, which can only reopen existing custody. Reconcile
permits fresh queries and goal observations without control. Opening Control
without the labor gate retains a Recover assignment journal; later enablement
cannot promote it. The optional clock journal independently follows its own
gate and also cannot gain control in place. Clock-only Control recovery is
possible after a verified assignment without enabling more labor mutations.
Close and explicitly reopen if a different mode is needed.

Call `fortress.observe(session_id=...)` to acquire a complete paused V1 citizen
observation. The development runtime bounds the complete roster at 128 citizens;
it does not silently truncate a larger fortress. Retain its observation seal.
The response identifies canonical citizens; additional bounded citizen pages
come from that exact retained observation:

```json
{
  "session_id": "<session>",
  "query": "{\"mode\":\"citizens\",\"expected_observation\":\"<observation seal>\",\"offset\":0,\"limit\":32}"
}
```

Pass this object to `fortress.query`. It performs no native call. Changed seals,
unknown fields or a page limit outside 1–32 refuse. Canonical unit IDs are
decimal strings; a native ID of zero is valid and has a different canonical
representation. Copy the canonical IDs from the observation. Omitting the
optional `query` argument inspects the retained goal and its recovery evidence.

The closed `fortress.plan` request is a JSON string:

```json
{
  "session_id": "<session>",
  "request": "{\"expected_observation\":\"<observation seal>\",\"units\":[\"1\",\"6\"],\"labor\":\"MINE\",\"enabled\":true,\"deadline_tick\":2000,\"poll_interval_ticks\":10,\"stable_samples\":2}"
}
```

Choose the absolute deadline from the observed game tick. It must be in the
future, no more than one game year away, and contain the requested cadence and
stability budget. Defaults are cadence 10 ticks and one stable sample; the
stability bound is 1–64. Input is bounded to 4 KiB, labor keys to 64 uppercase
ASCII letters/digits/underscores, and citizen selection to 1–32 unique canonical
IDs. Sorting selected IDs does not change the semantic request.

Planning archives the exact original goal and original citizen capsule before
preparation. The original semantic key is retained through the native journal;
repeating the request cannot replace the goal, choose another key, extend the
deadline or renew preparation. Plan returns a separately inspectable paired
semantic/native review and **does not assign labor**.

Inspect `result.original_plan`, `result.review` and the native evidence. Commit
requires `result.review.paired_seal`; a native plan digest alone is insufficient:

```json
{
  "session_id": "<session>",
  "paired_seal": "<paired semantic/native review seal>",
  "confirmed": true
}
```

Pass this object to `fortress.commit`. The coordinator re-observes the original
paused capture, rechecks original canonical evidence and synchronizes
`DispatchStarted` before the one assignment attempt. The original archive and
identity custody remain part of that goal. The concrete source repeats the live
Asupersync cancellation/I/O checkpoint and operator/source checks at the native
edge; cached entry checks cannot authorize an effect after cancellation or
revocation. A lost reply never permits another assignment attempt.

## Prove the original labor goal

Each explicit `fortress.wait` reconciles the original assignment and any
unsettled clock run before attempting one fresh paused goal sample. A running
or unresolved clock returns retained clock evidence without attempting a
paused labor read. Sources open only when needed and close on return. There is
no background polling, automatic clock advancement, preparation renewal or
assignment retry.

The fresh selected-workforce capture establishes only observed unit identities,
historical-figure IDs, eligibility and labor columns. It supplies no whole-roster
absence or unrelated world facts. Independent operator policy is checked again
before publication. The monitor evaluates every original step postcondition,
the plan terminal condition and the original obligation together.

Repeated polls at the same tick cannot create temporal stability. With multiple
stable samples, later valid paused observations must respect the original
cadence. The optional run workflow below provides explicit time advancement;
without it, the operator must arrange those later observations. Interrupted or
denied observations reset unfinished stability without changing the original
deadline.

`first_satisfied_anchor` records historical first achievement. A later false or
unknown sample remains visible, even after historical fulfillment. Stored native
Applied data and historical goal samples are never relabeled as a new current
observation. After restart, current goal output is empty until a new sample.

## Advance one reviewed interval

After the original assignment is historically Verified, call `fortress.wait`
for a fresh true original labor sample. An unfinished stability requirement can
then receive a separate run review. A false or unknown labor predicate, an
unverified assignment or an already completed original goal cannot authorize a
new run. Advancing time does not repair the assignment.

Pass this object to `fortress.plan`:

```json
{
  "session_id": "<session>",
  "request": "{\"kind\":\"run\",\"original_paired_seal\":\"<original labor review seal>\",\"game_ticks\":10,\"wall_millis\":5000}"
}
```

This is a closed request of at most 2 KiB with exactly those four fields. Tick
bounds are 1–1200, further limited by the negotiated allowance and the unchanged
original absolute deadline. The native wall stop bound is 1–60000 ms. Native
callbacks can overshoot a stop bound; these are not exact-tick or hard real-time
guarantees. The client cannot supply a new goal, deadline, citizen, native key,
stop predicate, endpoint or source grant.

Preparation obtains fresh original-goal evidence and durably binds the run to
the exact original plan, citizen capsule and Verified assignment receipt. It
does not unpause. Inspect `result.clock.active_run`, including its independent
`review_seal`, limits and original-goal association. Confirm that clock review
through `fortress.commit`:

```json
{
  "session_id": "<session>",
  "scope": "run",
  "paired_seal": "<clock active_run.review_seal>",
  "confirmed": true
}
```

The original labor review seal remains the `original_paired_seal` in run
planning; the distinct clock review seal belongs in this commit. Omitting
`scope`, or using `scope:"goal"`, retains the original assignment commit path.
Before native preparation and unpause, the runtime rechecks original custody,
the historical assignment and a same-tick true labor sample at the durable
effect boundary. Each run permits only one unpause attempt. A lost reply or a
post-marker refusal requires reconciliation of the exact run.

Call `fortress.wait` to reconcile that run, then obtain the next paused original
labor sample when permitted. Another interval requires another explicit plan,
review and confirmation. Agent Turn recommendations use the smaller of the
original cadence, remaining deadline and negotiated allowance; they do not
start runs themselves. There is at most one unsettled run and a maximum of 64
retained runs for one original goal. Original sampled stability, first
achievement and current labor truth remain independent of clock receipts.

## Cancel, close and recover

`fortress.cancel(scope="goal")` retires eligible preparation through the existing
coordinator. It does not undo Applied membership, repair Unknown or discard the
original goal history. `scope="run"` retires an undispatched run or asks its
native owner for a safety pause under current clock authority. It needs no new
labor-control grant, true current labor sample or unexpired original deadline;
it cannot undo elapsed time or labor membership. A stop receipt is historical
evidence, not proof of current pause. `scope="session"` releases resources and
file locks,
including after cancellation or fenced custody, and reports that game effects
were not cancelled. Checkpoint and restore tools explicitly refuse.

Before closing, retain `result.recovery` outside the private directory. It
contains the original semantic plan and archive-object digests, original citizen
identity digest, original canonical anchor, native journal identity and whether
goal history exists. With clock custody, it also contains `clock_journal_id`.
Preserve the clock journal identity even if opening failed before an original
goal existed: `clock.journal_id` remains in the retained recovery reference.
The identity capsule and retained clock records are historical data, not a
fresh read or an authority grant.

For reopening set `initialize:false`, set root `journal_id` to the independently
retained journal identity, and copy the complete recovery object to `recovery`:

```json
{
  "journal_id": "<original native journal identity>",
  "plan_digest": "<original semantic plan digest>",
  "object_digest": "<original archive object digest>",
  "identity_digest": "<original citizen capsule digest>",
  "anchor": {
    "fortress_id": "<canonical decimal fortress identity>",
    "epoch": 0,
    "sequence": 1,
    "tick": 120,
    "state_hash": "<original canonical state hash>"
  },
  "history_present": true,
  "clock_journal_id": "<exact clock journal identity when configured>"
}
```

This example is the value of `recovery`, not a complete replacement config.
Omit `clock_journal_id` when no clock was configured. For a clock-enabled
reopen, also retain `"clock":{"generation":30,"journal_id":"<exact clock journal identity>"}`
at the config root with the original generation. Its journal identity must
match `recovery.clock_journal_id`; neither can be silently omitted or replaced.
Retain the exact endpoint, source pins and directory from the original config.
The root and nested journal identities must match. Missing seals, another
fortress, changed source incarnations, replaced identity bytes, a missing
journal or changed original plan refuse; recovery does not reconstruct a new
plan from a remembered request.

The fixed original files cannot be hidden by omitting `recovery` on reopening.
Both original plan and citizen capsule must exist when a recovery handoff is
supplied, and `history_present` must match the actual history file. A failed
publication that leaves only an original citizen capsule does not provide a
complete restart handoff. The same live owner can retry the original request
if the plan archive was never created, but a new owner cannot automatically
reconstruct the missing archive or delete partial bytes. Partial or uncertain
publication remains an explicit failure, without any native assignment retry.

Offline opens read-only custody, reads no native credential and makes no native
call. Reconcile may query the original assignment and clock and acquire new
observations under fresh Query/Observe authority, but cannot prepare, commit or
cancel. A clock that may still be running blocks a paused original-goal read.
Recovering a native Prepared receipt after a dispatch marker retains tracking
uncertainty, never redispatch eligibility. Missing native records and permanent
Unknown remain unresolved.
The same rule applies to uncertain unpause or clock source loss: recovery never
repeats unpause or treats a replacement source as proof that the old run stopped.

Durable goal history reprojects retained raw captures and reissues their exact
historical policy under the current authority floor. Historical first
achievement and terminal obligation history survive; unfinished stability starts
a new streak. Original absolute deadline, cadence and source/cursor floors remain.
The retained cadence floor prevents same-tick preparation and commit guard
reads from contributing new stability. For example, a reopened unfinished
two-sample goal with a five-tick cadence and retained sample at tick 125 needs
later qualifying samples at ticks 130 and 135; replaying tick 125 does not
supply the first new stable sample. Clock receipts cannot replace either sample.
The complete handoff is retained in bounded error, overflow and session-close
responses without inventing fresh goal proof or granting control.

Replaying a long history may exceed the default byte-work allowance even when
the history file itself is small. After installing the exact recovery config,
an explicit larger allowance can be supplied to `fortress.open_session`; this
example requests 8 GiB of logical work for offline recovery:

```json
{
  "mode": "offline",
  "max_bytes": 8589934592
}
```

The allowance accounts for reprojecting and independently reauthorizing each
retained published sample; it is not a memory allocation. Insufficient allowance
refuses recovery without exposing partially reconstructed proof. A supported
64-sample goal can require more than the 2 GiB default to reopen. Increasing the
requested allowance does not enlarge the history file, event limits, native
capture limits or deadline, and it does not permit an assignment retry.

## Budgets and evidence

The session defaults to 2 GiB of total logical byte work per call and accepts an
explicit `max_bytes` allowance up to 32 GiB. It retains the maximum 60,000 ms,
one action and 32,768 output token-proxy units. The game-tick allowance defaults
to zero; explicit `max_game_ticks` can admit up to 1200 per reviewed run, always
within the original goal deadline. `wait` never spends that allowance by
starting a run.
Its 8,192-entity work allowance covers the native profile's combined citizens,
work details and membership entries; it does not expand the profile's own limits. A proxy unit
is four output UTF-8 bytes, not measured tokenizer output. The complete 128 KiB
response and a separate 64 MiB opening inspection are reserved before resource
work. Budgets are accounting ceilings, not eager allocations, and narrow through
each operation.

The assignment journal keeps its existing 64 MiB/64-key/512-event limits. The
optional clock journal retains the existing 2 MiB/4096-transition bound, while
this owner limits it to 64 retained runs and one unsettled run. The exact
original plan is bounded to 64 KiB/one step/1,024 nodes; the immutable original
citizen capsule is bounded to 4 MiB. Goal history keeps the existing
8 MiB/1,024-event cap. Replay must fit both the requested logical work allowance
and the remaining wall deadline. Exhaustion refuses without silent pruning,
partial verified recovery or automatic budget renewal. Filesystem
latency remains cooperative rather than a hard real-time guarantee.

Focused Rust evidence exercises the real adapter, original-goal runtime and
source/presentation boundaries with injected native and storage responses.
Private-file tests use the actual descriptor-pinned implementation. They do not
execute DFHack's real SDK, a live fortress, FastMCP macro-generated handlers or
the stdio executable. The exact test counts and build limitations for this
increment are recorded in [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md).

No compatibility, production admission, live-game success, host power-loss
qualification or cross-controller exclusion is claimed. The native work-detail
system can be changed by another controller; its complete paused capture is
revalidated at dispatch, and later state requires a fresh observation.
