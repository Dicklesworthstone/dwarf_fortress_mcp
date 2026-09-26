# From retained designation receipts to sampled blueprint completion

`scripts/track_dig_blueprint.py` joins the existing sparse normal-mining batch
workflow to the existing map/1.5 blueprint monitor. It executes no designation,
clock control, native retry or background polling. Each `sample` performs at most
one coherent map read for the entire sparse blueprint. `inspect` and `cancel`
are offline; a terminal `sample` also returns without credentials or a connection.

This is a development workflow, not an MCP tool or a production admission path.
The dig/1.16 and map/1.5 native profiles, client codecs, effect capsules and terrain
journal format are unchanged. See `DIG_BLUEPRINT_BATCH.md`, `EXCAVATION_BLUEPRINTS.md`
and `DIG_DESIGNATION_CLIENT.md` for their independent limits and operator setup.

## Start after all designation steps have receipts

Finish or recover the original batch first. A ready, partially designated,
refused or unresolved batch cannot start a linked monitor. Every expected child
must have its original, independently validated terminal designation receipt.
No incomplete child is skipped, discharged by terrain, or given a replacement key.

Create a separate, existing, empty exact-mode 0700 directory for progress. Do not
put it inside the batch or its effects directory: those directories have closed
inventories. The retained batch contains the blueprint, so the original blueprint
input file is not needed again.

Use a clean map-profile process environment: only these `DFMCP_*` variables are
accepted. Remove the dig profile's variables when switching commands; the monitor
never combines the two sets of permissions or passes credentials between them.
The endpoint must equal the retained batch endpoint. Both native profiles must
be configured there independently; this command does not load or enable plugins.

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin token, 32..256 bytes>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000

python3 scripts/track_dig_blueprint.py start \
  --directory /private/bedroom-progress \
  --batch /private/bedroom-batch --batch-id '<original batch_id>' \
  --max-game-ticks 1200 --stable-ticks 10 \
  --required-samples 2 --max-gap-ticks 120
```

Before journal creation, start verifies the complete designation inventory,
reconstructs the exact sparse blueprint and checks sampling feasibility. It then
obtains one coherent native capture. The first capture must match the retained
fortress folder, site, map dimensions, endpoint and DF/DFHack software versions,
and cannot precede the last retained designation tick. An unpaused capture is
allowed: this reader does not control or claim ownership of the game clock.

The initial observation fixes the game-time deadline. Reopening, retrying a read,
changing the environment, or inspecting history never extends it. The whole goal,
initial capture and source binding are retained in the normal blueprint journal.
An immutable link pins that journal's identity and inode, the original batch
manifest and directory identity, and the complete designation receipt set.

## Sample, inspect and cancel

```sh
python3 scripts/track_dig_blueprint.py sample --directory /private/bedroom-progress
python3 scripts/track_dig_blueprint.py inspect --directory /private/bedroom-progress
python3 scripts/track_dig_blueprint.py cancel --directory /private/bedroom-progress
```

All selected tiles must simultaneously be visible dry floors without active dig
designations. Unselected holes, room separators and intermediate levels are not
silently added to the predicate. Repeated reads at the same tick do not inflate
the matching-sample count. Hidden/missing evidence, mismatches, failed reads and
interrupted reads reset stability; excessive sample gaps restart it. A valid
capture with source changes or clock regression persistently invalidates the goal.

Every nonterminal sample syncs its read intent before connecting. Lost replies
are not retried. A process interruption retains an unfinished read that resets
stability during replay. A native read failure does not conceal a local custody
failure: original receipt/link/journal checks run before recording the outcome.
Every response revalidates the original complete receipt set independently of the
new terrain evidence. Corrupt or unavailable designation history cannot yield a
successful combined result. A legitimate local batch `stop` does not change its
immutable receipt set and does not invalidate an existing monitor.

Local `cancel` intentionally needs only the monitor directory, not a functioning
source batch or native endpoint. It labels designation evidence unverified for
that call. It cancels this observation goal only; it does not pause the game,
remove a designation, stop miners, undo terrain, or clear native obligations.
A malformed local link/journal still fails closed and is never repaired.

## What the result proves, and what it does not

The result carries the common Agent Turn envelope and separate fields for
`historical_designations_verified`, `blueprint_goal_satisfied_at_sample`, and
`designation_and_sampled_goal_evidence_verified`. Detailed aggregate deficits and
a bounded list of remaining coordinates come from the unchanged blueprint
classifier. Complete original effect records remain available from batch inspection.

The association between protocols is deliberately weaker than native causality.
Their generation counters are independent: a dig generation of 41 and a map
generation of 79 can be associated by the configured endpoint and fortress
selectors, but are never asserted to identify the same native incarnation. The
result always reports `shared_process_identity_proven=false` and
`mining_causality_proven=false`. A matching tuple could still be a different
process or a reloaded fortress. This path must not be promoted into canonical
lineage, cross-plugin incarnation, current-state or causal proof.

A satisfied result proves the sampled predicate under its explicit source
binding and cadence, not continuous stability, exact current terrain, structural
safety, job completion, global controller fencing or a verified game checkpoint.
Production admission and mutation capabilities remain unchanged.

## Custody and bounds

The progress directory contains exactly `designation-link.json` and
`progress.jsonl`. It uses existing no-follow, directory/file locks, exact 0700/0600
permissions, single-link regular files, byte checks and file-plus-directory sync.
Replacing or copying a pinned journal/directory is refused, even with identical
content. This is local custody, not an external anti-rollback authority or a
portable migration format. Do not manually remove or rewrite failed initialization
files; incomplete publication remains inspectable as original evidence but does
not become a newly authorized linked monitor.

The existing limits remain 32 blueprint parts, 512 targets, a 1,024-cell coherent
capture, 128 native steps, 128 additional map-read attempts, 260 journal events,
64 KiB frames and an 8 MiB journal. The immutable link and complete response are
each bounded at 32 KiB. Commands share one 1..60,000 ms cooperative wall deadline
across all work; filesystem synchronization is not forcibly interruptible.

## Executed validation

```sh
PYTHONPATH=scripts python3 -m unittest \
  test_dig_blueprint test_dig_blueprint_client test_track_dig_blueprint -v
```

The 22 new integration groups execute both real Python clients against one joined,
fragmented TCP peer implementing both fixed test protocols. They exercise actual
multi-level designation, restart-by-command, whole-mask completion, source drift,
clock/deadline/stability rules, failed/interrupted reads, all four initialization
and eight sampling sync failures, torn writes, concurrent receipt corruption,
closed environments, substituted files, local stop/cancellation, and CLI subprocesses.
The 128-step size test explicitly installs native byte fixtures; the separate
512-target/1,024-cell test actually executes its eight designation commits.

The 33 prior compiler/batch groups also execute unchanged. The dependency files
were verified byte-for-byte against their pinned upstream Git blobs. This is
Python, filesystem and test-peer execution, not real DFHack SDK/ABI, live-fortress,
Rust/MCP, power-loss or full-workspace qualification. Beads
`df-dfhack-bridge-plane-c-pic.4/.5` and `df-action-coordinator-exec-ero.4` remain open.
