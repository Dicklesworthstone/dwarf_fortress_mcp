# Complete furnishing batches through construction

`scripts/track_furniture_batch.py` follows the **entire original furnishing
plan** from verified placement receipts to a shared sampled construction
condition. It opens the original private batch directly, verifies every planned
step and its registered Placed receipt, and binds that complete selection into
its own durable monitor. An omitted furnishing cannot disappear from the goal.

Use this after the exact batch workflow in
[`FURNITURE_BATCHES.md`](FURNITURE_BATCHES.md) reaches `all_placed`. That state
means construction jobs were registered; this monitor checks later building and
item observations. The existing
[`track_construction_plan.py`](RECEIPT_CONSTRUCTION_PLAN.md) remains available
for independently selected receipt sets.

This is an explicitly unadmitted Python development workflow. Importing and
monitoring a batch grant no placement, game-job cancellation, clock, checkpoint
or restore authority. Its machine contract is
[`furniture_completion_v1.json`](../architecture/furniture_completion_v1.json).

## Start from the original batch

Keep the original batch directory and all of its child placement journals in
place. Obtain its `batch_id` from initialization or verified inspection. Every
step must have a registered canonical furniture/1.19 `Placed` receipt. An
unstarted, unregistered, refused, cancelled or uncertain child prevents import;
recover the original work through `furniture_batch.py` first.
An already completed batch remains eligible if it has subsequently been stopped;
the local stop marker prevents new placement, not observation of completed work.

The monitor verifies the complete normalized action DAG and an exact mapping
from every step to its original native key, kind, item, target and receipt. It
also binds the batch identity, native source, fortress, dimensions and endpoint.
The journal retains this linkage, so replay cannot substitute another plan or
weaken the original selection to the successful children.

Configure the monitor process with the existing isolated read credentials:

| Variable | Value |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR` | Exactly `1`. |
| `DFMCP_CONSTRUCTION_MONITOR_ENDPOINT` | Original batch's canonical numeric IPv4 loopback endpoint. |
| `DFMCP_BUILD_TOKEN` | Furniture plugin query credential. |
| `DFMCP_OPERATIONS_PAGED_TOKEN` | Operations plugin read credential. |

Remove the placement client's other `DFMCP_*` settings before running the
monitor; they are not accepted by its read profile. The native game process
retains its existing plugin settings. Credentials and process configuration are
not copied from the batch or stored in the monitor journal.

Choose an absolute future deadline from observed game ticks. Replace the
illustrative ID, paths and tick below with the actual values. The monitor's
parent directory must already exist as an owned real exact-mode `0700`
directory; `start` creates a new exact-mode `0600` journal exclusively.

```sh
python3 scripts/track_furniture_batch.py start \
  --journal /private/construction/bedroom.completion \
  --batch /private/bedroom-furnishings \
  --batch-id '<original batch_id>' \
  --deadline-tick 900000 \
  --interval-ticks 10 --stable-samples 2 --stable-span-ticks 10 \
  --max-gap-ticks 1200 --max-observations 512
```

`start` imports the complete original plan and performs one bounded foreground
acquisition. The new monitor is separate from the batch and its placement
journals; it does not modify their contents. Use a path outside the batch
directory, whose inventory is intentionally closed.

## Sample, inspect and cancel

```sh
python3 scripts/track_furniture_batch.py sample \
  --journal /private/construction/bedroom.completion
python3 scripts/track_furniture_batch.py inspect \
  --journal /private/construction/bedroom.completion
python3 scripts/track_furniture_batch.py cancel \
  --journal /private/construction/bedroom.completion
```

Each nonterminal `sample` acquires one observation. There is no automatic
polling, background worker, unpause or placement retry. The original batch,
endpoint, step mapping, deadline, cadence and observation allowance come from
the journal; later commands cannot replace or renew them. `--timeout-ms` bounds
the whole foreground operation, from 1 to 60,000 ms, with a default of 10,000.

Every successful sample or inspection rechecks the original batch's private
custody and complete retained evidence, including when the monitor is already
terminal. Inspection requires no native connection or credentials, but the
original batch must still be available at its pinned path. Missing or changed
original evidence prevents a successful completion report; it does not erase
historical monitor samples or make any placement safe to retry.

An active monitor can still be cancelled when the original batch is unavailable.
Cancellation verifies only the monitor journal and identifies the original
source as unverified, even when the batch remains available. It proves only that
this monitor was cancelled. It cannot remove furniture, cancel a construction
job, clear an uncertain placement or certify the original plan's completion.
Terminal monitor history remains immutable.

## What completion means

For each fresh sample, the existing query-only transport reads every original
placement record before and after **one complete operations/1.4 capture** on
the same foreground connection. It verifies all pages, the capture digest and
release acknowledgment. Both sets of native records must equal the original
receipts; missing records or source changes cannot publish a successful sample.

Every original building must have the receipt's exact type, footprint and maximum
stage, reach that stage, and have no held construction or removal job. Its
original singleton item must retain its type and material identity and be
installed in that building, outside any container and without a job attachment.
All targets must meet these conditions in the **same** capture. Job disappearance
alone does not establish completion.

One global streak requires the declared number of advancing game ticks, cadence
and minimum span. Repeated paused captures add no samples. A false or unknown
member, excessive gap, changed same-tick capture or interrupted read resets the
shared streak. A mismatch with the original native source refuses the sample
and retains its unknown read intent. Accepted observations with identity, clock,
horizon, stage or operations-source regressions invalidate monitoring; a removal
job fails it. A completed bed from an earlier
sample cannot compensate for a bed that no longer satisfies its condition when
the table completes.

`satisfied` is therefore a **historical sampled condition for every member of
the complete retained furnishing plan**. It does not establish continuous
stability, current usability, room assignments, reachability, terrain safety or
causality. It never discharges placement effects or grants retry permission.
The original DAG's dependency edges still mean placement ordering, not game
construction scheduling.

Responses use `dfmcp.furniture-batch-monitor-result/1`. The result includes the
complete `requested_plan`, each target's `plan_step`, and the batch, plan and
origin digests. `original_placement_history_verified` reports source custody
verified during this call. `complete_original_plan_sampled_condition` becomes
true only when that custody check succeeds and the retained monitor is
`satisfied`; a retained phase label by itself does not establish this claim.

## Durable handoff and bounds

The `DFMFCO01` origin binds the original batch manifest, full plan, ordered step
index, child journal bytes and directory/file custody identities. The `DFMFCG01` goal commits to
that origin and the existing whole-plan construction goal. Its separate
`DFMFCJ01` append-only journal retains the fixed policy and accepted samples;
these are distinct formats from the existing selected-receipt monitor.

Read intent synchronizes before native contact. Complete proposed results and
evidence are checked before publication; file and parent synchronization plus
readback precede acknowledgment. A failed acquisition or publication retains
the unknown read. Restart replays the actual evidence and never restores the
old process's publication permission. Original custody is rechecked around
publication, so substituted batch evidence cannot authorize a successful report.

The workflow inherits the whole-plan monitor's 1..32 targets, maximum 512 accepted
observations, 16 MiB operations capture, 327-call acquisition allowance and
64 KiB complete JSON/Agent Turn response. Origin bytes are bounded at 2 MiB;
the complete composite goal is bounded at 2,293,968 bytes. One shrinking deadline
and budget cover custody, all native queries, pages, evaluation and publication.
There is no per-target budget renewal. The 1 GiB monitor-file I/O allowance meters
the completion journal. Original batch reads retain their separate fixed file
and collection bounds and share the same wall deadline; those inherited readers
do not debit the monitor-file byte counter.

Source and monitor custody reject changed paths, symlinks, wrong modes, missing
child journals and corrupt history without repair, truncation or eviction.
Moving or copying an inode-pinned batch is unsupported. Local custody checks
and hashes are not signatures, global controller fencing or hostile-owner
anti-rollback protection. Filesystem deadlines remain cooperative.

## Executed validation

Run the combined placement and monitoring regressions with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_furniture_completion.py
```

The combined suite passed **183 actual Python test functions**, including
**40 new tests**: 14 original-goal tests, 14 durable-store tests and 12 complete
batch/monitor workflow tests. The workflow tests execute real client connections
and fresh CLI processes against a joined loopback test peer. They run the
original reviewed placement workflow before querying its retained receipts and
construction observations.

The maximum-size workflow executed 32 placements with long step names and a
16,345-byte dependency plan. Its complete completion response was **53,320 bytes**;
reservation including maximum journal counters was **53,326 of 65,536 bytes**.
The shared capture checked all 32 conditions and bracketed them with 64 original
receipt queries. The publication tests independently substitute original child
and index inodes during result rendering and after parent synchronization.

Exact execution results and all **32 input hashes**, rechecked unchanged after
execution, are retained in
[`furniture-completion.json`](evidence/furniture-completion.json).
An independent temporary-copy check also rejected four weakened implementations:
omitted receipt-set binding, omitted original software binding, omitted original
source checks during publication, and restored publication permission after
reopening. The actual mutations and assertions are recorded in
[`furniture-completion-mutations.json`](evidence/furniture-completion-mutations.json).

This Python workflow does not establish Rust/MCP integration, a real DFHack SDK
build, live-fortress behavior, physical power-loss safety, whole-workspace
qualification or production admission. Owning beads remain
`df-dfhack-bridge-plane-c-pic.4` and `df-dfhack-bridge-plane-c-pic.5` for their
broader subsequent-observation and recovery work.
