# Exact-clock construction progress between original placements

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`; `df-franken-storage-mvcc-54h.4`.
The current operator contract is `architecture/route_furniture_progress_v2.json`.
It supersedes v1 for new progress dispatch; no native wire generation changes.

The mandatory construction gate does not allow another furnishing while earlier
original hauling/construction remains unfinished. This separate foreground owner
uses the existing run/1.13 watchdog to advance game time by ONE explicitly reviewed
bounded burst. It never places furniture, retries a placement, loops the clock,
weakens the construction gate, or treats elapsed ticks as goal completion.

## Review, confirm, progress

Both review and progress now require the same isolated route-plus-clock environment:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH=1
DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_TOKEN=<existing furniture read credential>
DFMCP_MAP_TOKEN=<existing map credential>
DFMCP_OPERATIONS_PAGED_TOKEN=<existing inventory credential>
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_RUN_TOKEN=<existing bounded-run credential>
```

Endpoints must be identical canonical numeric loopback addresses. These values
are illustrative, not an established connection. Furniture placement permission
must be absent or zero; all unrelated DFMCP authority/admission settings refuse.
The progress authority permits only existing furniture read methods, not native
PreparePlacement, CommitPlacement or CancelPlacement.

```sh
python3 scripts/route_furniture_progress.py review \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --ticks 120 --wall-ms 5000 --timeout-ms 10000

python3 scripts/route_furniture_progress.py progress \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --ticks 120 --wall-ms 5000 --timeout-ms 10000 \
  --confirm-progress <independently-reviewed-confirmation> \
  --clock-directory /absolute/private/clock-records
```

Only `construction_pending` receives a confirmation. A read-only ObserveRun is
bracketed by equal complete route/construction evidence. Software and paused game
tick must agree; the native clock has its own generation and sequence, never
numerically equated with map or inventory generations. An existing clock owner,
full native record capacity, changed endpoint or changed second frontier refuses
consent. Normal read-only clock observations do not advance the native sequence.
Equal endpoints are not an atomic cross-profile snapshot or a map lock.

The version-2 confirmation binds the exact native clock observation, manifest,
batch/review/plan identities, original local inventory, next original step, full
source identity, route/construction evidence digest and tick/wall limits. Execution
reacquires this review and compares the independent confirmation before creating
files or effects. The exact clock observation is checked again immediately before
the unchanged native start prepares its original intent. Changed sequences,
generations, ticks, software or current authority never silently replace consent.

## Durable association and effect ordering

The clock directory must be an existing owned, real, exact-mode 0700 directory.
The confirmation deterministically selects `progress-<confirmation>.run` and
native key `frp-<confirmation>`. Three exact-mode 0600 single-link files participate:

- `.run.scope.json`: immutable complete batch association and approved clock;
- `.run`: the existing native intent capsule;
- `.run.outcome.json`: the existing immutable terminal native receipt, when known.

The scope is bounded to 8 KiB, created exclusively, fsynced with its parent, and
verified before native intent/preparation. The original native client synchronizes
its exact intent before PrepareRun. Before BOTH PrepareRun and CommitRun, the
context re-reads the original intent and scope and checks current authority and
original batch custody. Original intent identity cannot be replaced between those
effects, even with byte-identical contents. Only one preparation and one commit
can be attempted on the scoped write connection. The native engine still performs
its original commit-time clock revalidation and owns its stop watchdog.

The same exclusive clock directory lock spans scope creation, native execution,
and outcome retention; the outcome store borrows that lock instead of dropping
ownership between dispatch and publication. Ambiguous synchronization or transport
failures retain all surviving bytes. Orphan, torn, changed, foreign or already-used
scope/intent files are not repaired, pruned, adopted or used to authorize retry.
Original batch files are opened read-only and remain held through final output.

This is cooperative local custody, not protection against a malicious owner
rolling back every file or a global lease over other controllers. A digest is
an evidence identity, not a signature or an independent native-acquisition proof.

## Stop, query, and inspect without redispatch

A normal CommitRun reply may still be running. That is not completion and does
not trigger another clock burst. Recover the exact original record by query:

```sh
python3 scripts/route_furniture_progress.py query \
  --directory /absolute/private/route-batch --batch-id <original-batch-id> \
  --confirm-progress <original-confirmation> \
  --clock-directory /absolute/private/clock-records --wait-ms 1000
```

The query-only connection cannot prepare, commit or cancel. Repeated running
queries consume the same shrinking operation budget. Missing records remain
unknown, not proof of nonapplication. Before contact, the stored scope must bind
this exact original batch/review/plan and the complete native intent. Another
batch at the same endpoint cannot adopt the record merely by knowing its key.

A retained terminal outcome is inspected offline by default, without credentials,
synchronization, native contact or implicit route acquisition. Pass the existing
combined credentials and `--refresh-frontier` explicitly for a fresh route review.
Successful progress also attempts that post-stop review once. Current readiness
requires a pause-verified stopped record in the approved clock incarnation and a
fresh route source at or after its observed stop tick and original source floors.
A failed refresh retains the durable clock result without an earlier readiness
claim. Source changes, unknown stop ticks and clock regressions cannot qualify a
current frontier. Revoked authority or lost custody still refuses publication.

`next_step_ready` means only the freshly inspected next step. Whole-batch
`construction_completion_proven` stays false, even while more unplaced original
steps are ready. Offline custody never enables `advance_allowed`; normal placement
review/consent remains separate. Query does not report a clock commit this call.
Historical pause evidence always retains `current_pause_unproved: true`.

Old v1 confirmations cannot dispatch under v2. Old unassociated native intents
remain recoverable through `bounded_run_client.py inspect/query` with their exact
original path and key; this wrapper refuses to fabricate a batch association.

## Shared bounds and executed evidence

One existing route Budget spans custody, all reads, clock RPC and output. Every
actual binding/request and requested receive/send byte is charged, including
notifications and repeated queries. The original deadline is never extended or
replaced by the clock client's local timeout. Before start, ten remaining clock
calls and 3 MiB network headroom are required; these are admission floors, not
refunded reservations. Native intent/outcome I/O reserves 256 KiB; new scope I/O
is additionally charged. Full output is reserved before effect and bounded to
256 KiB. CPU/filesystem operations remain cooperative, not hard real-time calls.
Tick/wall ceilings remain 1..1200 ticks and 1..60000 ms. Insufficient shared
headroom refuses; the maxima do not promise every operation fits the deadline.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_route_furniture_clock_context.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_route_furniture_clock_context.py --mutations-only
```

64 tests pass: 27 original-run/readiness tests, 29 new clock-context/workflow tests,
and the eight original progress scenarios upgraded from source/port-only checks
to actual clock TCP/private-file execution. Six weakened implementations fail
assertion regressions, including removed association, revocation, request-budget,
scope-sync, intent-identity and post-stop clock-floor checks. Exact source hashes
are recorded before and after each run; base and mutation invocations may run
separately under the editor's per-command timeout. Syntax checks are included.

The real bounded-run Client, codecs, start/recover and scope/intent/outcome stores
execute against finite joined loopback peers with independently encoded frames
and records. Real private files, locks, synchronization failures, lost replies,
foreign/missing associations, file replacement, revocation, CLI argument/one-output
handling, running/absent/terminal records and stale post-stop reads are exercised.
Route observation, original batch custody/budget and the outer route packet are
explicit test ports. The real CLI functions execute, not a CLI subprocess-to-game
campaign. Native route/batch integration was source-reviewed, not fully executed.
No actual DFHack SDK/game, Rust/MCP, full workspace or production admission is
qualified. Native protocols, original intent/receipt formats and grants are unchanged.
