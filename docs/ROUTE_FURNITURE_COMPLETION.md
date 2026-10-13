# Complete original route-batch construction monitoring

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`; WP-10. Route placement receipts
prove historical construction-job registration, not completed furnishings.
This workflow joins the complete original batch to the existing receipt-linked
whole-plan construction condition instead of accepting an arbitrary successful
subset of receipts as the original objective.

## Complete original goal

`route_furniture_completion.Goal` retains the complete exact-item plan and its
DAG, original route review digest, original batch path/identity, original build
manifest, original operations incarnation, all original Placed receipts and one
fixed construction deadline/cadence/stability policy. Its bounded `DFMRGC01`
encoding is additive; no native wire, placement journal or old monitor changes.
Decoding requires exact re-encoding. The goal digest covers the complete object.
A digest identifies bytes, not authorization, acquisition or source custody.

`from_batch` requires a verified, locked Batch with all original steps placed
and registered. Receipt keys, kinds, items, targets, incarnation and chronology
must agree with that complete original plan. Missing, uncertain, cancelled,
foreign or unregistered children refuse. `verify_origin` rechecks the parent,
complete original plan and exact receipts, including for terminal history.
The original route contract stays in the unchanged parent review; the monitor
retains its identity and never treats construction progress as current routing.

Every sample uses the unchanged complete-roster decoder and whole-plan condition.
The first sample must already match both original native incarnations/software;
a new source is not learned from convenient first-sample data. Every selected
building and installed item is assessed in the same capture. Completion requires
all conditions together for the original advancing-tick stability window; prior
individual successes are not latched. Missing/replaced buildings, active removal,
unverified installed items, source regression, deadline or sample exhaustion
cannot become success. An interrupted read resets unfinished stability.

## Durable foreground workflow

`scripts/track_route_furniture_completion.py` provides `start`, `sample`, `wait`,
`inspect` and `cancel`. It opens the existing original route `Batch` read-only,
with its root/child locks and complete inventory checks, and a separate private
monitor journal. Both owners remain held through the single final output attempt.
The placement coordinator, original review and child journals are not modified.

The separate `DFMRGJ01` monitor uses the existing no-follow private-file owner,
file and directory synchronization, whole-file readback and exclusive locking.
Its first frame preserves the complete original goal and query endpoint. Later
frames retain read intent, complete raw receipt-bracketed operations samples, or
monitor cancellation. A read intent is synchronized before native contact; a
sample is published only after the full condition, output reservation, current
authority and both custodial owners have been checked. Changed bytes, missing
original children or a failed append cannot become a success response.

Reopening verifies every frame and replays the actual condition. Original timing,
receipt identities and historical progress survive. An interrupted read remains
unknown and resets unfinished stability at the next explicit read. Partial or
corrupt frames refuse without tail repair, replacement, pruning or retry. A full
frame surviving a failed sync acknowledgment can be inspected on verified reopen;
that does not retroactively acknowledge the failed call. Pending sampled history
is not a claim of continuous observations or coverage while the process was down.

Each acquisition uses the existing construction-plan read client: four fixed
bindings (build Handshake/QueryPlacement and operations Handshake/ReadObservation),
every original receipt before the capture, all retained inventory pages, verified
whole-capture digest and release, then every original receipt again on the same
connection. The first handshakes must match the original build manifest and
operations generation/software. The source closes before monitor publication.
These read brackets are not a claim of an atomic cross-profile snapshot.

### Operator invocation

The original route batch must already contain **all** its registered Placed
receipts. An `all_placed` status permits creating a construction monitor; it is
not itself a completed-construction claim. Keep the original batch ID from batch
initialization. Put the new monitor in a separate existing exact-mode `0700`
directory, outside the original batch's closed inventory. The monitor must be
an exclusively created or existing single-link regular exact-mode `0600` file.
All path components are checked by the existing private-file implementations.

Use the existing isolated construction-monitor read environment:

```text
DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR=1
DFMCP_CONSTRUCTION_MONITOR_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_TOKEN=<operator-configured build read token>
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured operations read token>
```

The endpoint shown is the default and must equal the original batch endpoint.
Tokens are 32..256 UTF-8 bytes without NUL and are never retained in output or
goal bytes. Other `DFMCP_` variables are refused; neither placement permission
nor map credentials are needed or accepted by this read profile. Plugins must
already be configured by the operator; the client does not load them.

```sh
python3 scripts/track_route_furniture_completion.py start \
  --batch-directory /absolute/private/route-batch --batch-id ORIGINAL_BATCH_ID \
  --journal /absolute/private/monitors/furnishings \
  --deadline-tick FIXED_GAME_TICK --stable-samples 2 --stable-span-ticks 1
```

The identifiers, paths and tick above are placeholders. The deadline must allow
the fixed stability window after every original placement. `start` persists that
goal and takes its first sample. Retain the returned `result.goal_digest`
independently along with the batch ID. After restart, select that original goal:

```sh
python3 scripts/track_route_furniture_completion.py wait \
  --batch-directory /absolute/private/route-batch --batch-id ORIGINAL_BATCH_ID \
  --journal /absolute/private/monitors/furnishings --expected-goal ORIGINAL_GOAL_DIGEST \
  --wait-samples 8 --poll-ms 100 --timeout-ms 10000
```

`sample` makes one explicit read. `wait` uses the existing bounded foreground
scheduler with no background worker, time advancement or retry after a failed
acquisition. It stops at terminal state, sample/RPC/wall limits or repeated
nonadvancing game ticks. It does not renew the goal's game-time deadline or
observation count. Deadline/cadence/stability overrides are start-only; wait
options belong only to `wait`.

`inspect` and `cancel` take the same original selectors but need no native
credentials. Inspection always verifies the original parent, including when the
monitor already holds terminal history. Cancellation stops only this monitor;
it neither cancels construction nor deletes prior samples or effect records.
A terminal monitor stays offline for `sample` and `wait` as well.

Exit zero and `ok: true` mean a complete report was produced. Completion is
`result.progress.phase == "satisfied"`, an historical, simultaneous sampled
condition. Every response retains the original plan, target count, fixed policy,
current monitor state and evidence references. `current_usability_proven`,
`current_routes_proven` and `placement_effects_discharged` remain false. The Agent
Turn anchor is null: this Python workflow issues no canonical Rust generation.
Refusals exit two with null result and a sanitized `ROUTE_COMPLETION_REFUSED`
error; no partial completion or raw credentials/paths appear in that refusal.
A short stdout write or late close failure never prints a second object.

## Shared bounds

One foreground allowance covers parent verification, journal replay, acquisition,
condition work, waiting and publication: 1..60,000 ms (default 10,000), 327 RPC
calls, 20 MiB network, 1 GiB disk-byte accounting and 20,000,000 work steps. No
phase, target or page refills those counters. Native frame limits and exact page
release remain unchanged. Wait allows 1..32 samples and 10..5,000 ms between
foreground samples, subject to the same overall allowance.

There are at most 32 original furnishings and 512 successful observations under
the existing condition profile. The goal frame is bounded by the original
receipt-set/plan limits plus 8 KiB of source metadata. Journals retain at most
128 MiB and 1,030 frames. Maximum sample and future cancellation capacity are
reserved before read intent. Output must fit 96 KiB as one valid object; a
32-KiB allowance for expanded assessment rows is reserved before sampling.
Bounded hashing/serialization and synchronous file operations are cooperatively
checked, not hard real-time preemption or power-loss testing.

## Executed development evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error::ResourceWarning \
  -m unittest test_route_furniture_completion \
  test_route_furniture_completion_store test_route_furniture_completion_runtime -v
```

All **40 actual Python test functions pass**, with zero skips: 16 core tests,
17 private-monitor/workflow tests and seven TCP/CLI tests. They execute the
actual new goal/store/runtime, existing condition, private monitor owner, native
clients and foreground scheduler. Every original goal-fixture prefix truncation
is rejected. Monitor-frame tests cover all framing boundaries plus distributed
body corruptions/truncations, not every offset of every possible sample.

Cases include all 32 original targets, missing/foreign/unregistered subsets,
changed original selections and sources, simultaneous completion instead of
individual success latching, fixed deadlines, repeated ticks, failed acquisition,
read-intent recovery, torn append, lost sync acknowledgment, private monitor
locks/permissions, changed backing, output limits and final authority revocation.
The joined TCP peer independently encodes framing/protobuf and operations data,
including a three-page inventory, exact before/after receipt checks, first-source
mismatch, corrupted capture digest and lost/wrong release acknowledgment.

The **original Batch API is injected** in these tests. Positive CLI operations
run the actual `main` with that injected parent and real monitor files/TCP; one
negative real CLI subprocess verifies sanitized pre-contact refusal. The normal
`open_batch` call into the existing private route parent was source-reviewed,
not executed here. These results therefore do not qualify the full original
private-parent/CLI pipeline. Receipt fixture values use the actual strict codec;
the peer is not DFHack or an independently trusted native evidence issuer.

Four deliberately weakened source variants fail regression assertions: accepting
a changed first inventory incarnation, forcing early whole-plan satisfaction,
omitting original-parent verification, and omitting final current-authority
checks. Imported dependencies are Git-blob checked and unchanged. The passing
counts apply to these exact local sources, not all previous repository suites.
No native SDK/plugin build, live fortress, Rust/MCP execution, full-workspace
qualification, compatibility registration or production admission is established.
The broader .4/.5 beads remain open. Native room assignment, current route safety,
continuous stability and generic MCP semantic execution remain separate work.
