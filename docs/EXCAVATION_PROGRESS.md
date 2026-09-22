# Read-only excavation goal progress and durable recovery

`scripts/track_excavation.py` implements a standalone Python developer workflow
for observing a bounded floor goal through the existing map/1.5 reader. The
underlying codec, two-method RPC client and pure evaluator are in
`scripts/excavation_observer.py`. This is executable monitoring, not a new MCP
route, native protocol, game-clock controller or mutation journal.

## What the goal establishes

Every cell in one 1..8 by 1..8 rectangle at one z-level must be visible, have
normalized shape FLOOR, zero liquid depth and no dig designation. Walls, empty
space, ramps, stairs, unsupported shapes, wet floors and designated floors do
not satisfy it. Missing and hidden cells remain unknown with no attribute payload.
Occupancy, temperature, pathfinding, material, structural support and safety are
not part of the predicate. A constructed floor may satisfy it. The program does
not attribute a changed tile to any particular mining operation.

The goal requires both a number of matching samples and a game-tick span. Only
strictly advancing ticks increment the matching count; repeated reads while the
game remains at the same tick do not manufacture stability. Contradictory, hidden,
missing, failed or unfinished reads reset the streak. A gap greater than the
configured maximum resets it too. Satisfaction must be observed at or before the
fixed deadline. A later sample expires the goal even if all floors then qualify.

Statuses are pending, unknown, stabilizing, satisfied, expired, invalidated and
cancelled. The last four are immutable terminal monitor states. A change in the
map reader's generation/software, fortress identity or dimensions, or a regressed
clock, invalidates the monitor rather than rebinding it to a replacement source.
Region substitution or malformed evidence is a refused read, not goal success.
An invalidating sample is retained in the journal; compact last-observation fields
continue to refer to the last accepted sample from the original source.

These are historical sampled endpoint conditions, not continuous stability or
coverage of the intervals between observations. The journal never changes,
imports or clears a native mining-effect obligation. Matching coordinates do not
make a map/1.5 generation equivalent to a dig/1.16 generation. In particular,
`floor_goal_satisfied_at_sample=true` does not make
`mining_action_completed_proven` or `retry_designation_permitted` true.

## Run a bounded foreground workflow

The existing `dfmcp_map_v1_5` plugin and matching `DFMCP_MAP_TOKEN` must be
configured in the DFHack process as described in `LIVE_MAP.md`. Use a dedicated
owned exact-mode 0700 directory and a new filename for this monitor. Do not use
an existing mutation journal, or place this file inside the Python dig client's
intent directory: that store deliberately refuses unrelated files.

Only these DFMCP environment names are accepted for native reads:

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin token, 32..256 UTF-8 bytes>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000
```

The endpoint is optional at creation and defaults to the value above. It must be
canonical numeric IPv4 loopback with a nonzero port. Later reads use the retained
endpoint; a conflicting environment value is refused. Other DFMCP variables,
including designation enablement, mutation credentials and production admission,
are rejected. Credentials are never retained in the journal or output.

```sh
# A new monitor; this observes but does not designate or unpause anything.
python3 scripts/track_excavation.py start \
  --journal /private/progress/tunnel.jsonl \
  --world-folder region1 --site 1 \
  --x 15 --y 15 --z 2 --width 2 --height 2 \
  --max-game-ticks 1200 --stable-ticks 10 --required-samples 2

# Each explicit invocation obtains at most one further map observation.
python3 scripts/track_excavation.py sample \
  --journal /private/progress/tunnel.jsonl

# Offline historical inspection needs no environment or native connection.
python3 scripts/track_excavation.py inspect \
  --journal /private/progress/tunnel.jsonl

# Stop this monitor only. No designation is removed and no game action is undone.
python3 scripts/track_excavation.py cancel \
  --journal /private/progress/tunnel.jsonl
```

The initial sample establishes the fixed deadline as its tick plus
`--max-game-ticks` (1..403200); creation rejects overflow. Defaults are ten stable
ticks, two matching samples and `--max-gap-ticks 1200`. Stable ticks may be
0..403200, required samples 1..128 and maximum gap 1..403200. The stable span may
not exceed the creation horizon. Selecting one sample and zero stable ticks
explicitly requests a single-sample condition rather than a temporal goal.

All commands accept `--timeout-ms` in 1..60000, default 10000. Replay, filesystem
checks and network work share one shrinking cooperative allowance. Synchronous
kernel filesystem operations are not claimed to be forcibly interruptible.
There is no polling loop, detached task, timer or automatic game advancement.
Once terminal, sample/inspect/cancel return the historical result without another
native read or file rewrite; they do not assert that the condition still holds.

## Persistence and interruptions

A new file is created exclusively. Existing empty, malformed or incomplete files
are not initialized, overwritten or repaired. Symlink path components, multiple
hard links, non-regular files, incorrect 0600 file/0700 parent modes and ownership
mismatches are rejected. An exclusive nonblocking file lock is held for each
whole operation, including reads. Named and opened inode identities, parent
identity, permissions, extent and complete bytes are rechecked at boundaries.

Each newline-delimited canonical JSON frame binds its sequence, predecessor and
exact event with SHA-256 domain `dfmcp-excavation-journal/1` followed by NUL.
The initial frame retains a nonzero random journal nonce, endpoint, complete goal,
source manifest and native baseline bytes. Later frames retain `read_started`,
`sample`, `read_failed` or local `cancel`. No serialized success flag is accepted:
replay decodes all native bytes and recomputes the state transition.

Before each subsequent native connection, the existing journal is synchronized,
then `read_started` is appended and synchronized. A crash after this point leaves
an unfinished read visible as unknown in offline inspection. A later explicit
sample resets the old streak before accepting new evidence. Failed native reads
persist `read_failed` when storage and deadline still permit; otherwise the
unfinished intent itself preserves the conservative interruption boundary.

Every append validates the resulting history and complete bounded response before
writing. It synchronizes both the file and parent directory, repeats custody
checks, and only then publishes the new in-memory state and acknowledgement.
Write/sync/custody failure fences that invocation. A complete frame surviving an
uncertain synchronization can be inspected on reopen as historical evidence; this
does not retroactively certify the failed acknowledgement or power-loss durability.
Incomplete/corrupt frames remain refused without truncation or automatic repair.
Offline inspect uses a read-only descriptor and performs no synchronization.

Bounds are 16 KiB/frame, 2 MiB/journal, 260 total frames and 128 subsequent read
attempts after the baseline. Every sample reserves room for its read intent,
outcome and cancellation. Retention exhaustion refuses new reads without eviction;
local cancellation remains possible for valid retained history. Hashes detect
inconsistent evidence, not an owner intentionally rewriting or rolling back the
entire journal. No external anti-rollback anchor or global controller lease exists.

## Output and evidence scope

After argument parsing, each invocation emits one complete bounded JSON line with
schema `dfmcp.excavation-progress/1`, goal status, exact journal/source/sample
identities, tick, floor/wall/unknown counts, matching streak and recovery guidance.
The additive Agent Turn spine preserves pending goal identity and uncertainty.
It never fabricates a canonical world anchor, semantic request ID or inventory of
native effects. Output is capped at 32 KiB, including the complete common packet;
no mid-object truncation or credential/error-detail echo is performed.

Exit zero means a valid monitor result was returned, not that the goal is satisfied.
Check `goal_status`; pending, unknown and expired are valid results. Operational
failures return exit two with explicit unknown outcome. Normal argument-parser
usage errors are emitted on stderr before a goal operation begins.

The map client binds only Handshake and ReadObservation, at most one capture per
connection. It enforces exact protocol/nonce/selection, minimal protobuf, closed
fields, bounded UTF-8 and source identity. Traffic is at most 4 MiB per connection,
with at most eight notification frames/256 KiB per RPC. The independent map capture
bound is 1 MiB/16384 cells; this goal's at-most-64-cell captures fit within 2048 raw
bytes. No commit, preparation, cancellation RPC or arbitrary command is exposed.

## Executed regression evidence

```sh
PYTHONPATH=scripts python3 -m unittest \
  test_excavation_observer test_track_excavation -v
```

All 35 tests pass: fourteen observer/transport/evaluator tests and twenty-one
journal/CLI tests. They execute actual Python, fragmented real loopback TCP,
private POSIX files, subprocess CLI and cross-process locking. Coverage includes
the unchanged 455-byte native fixture (Git blob
a2fcaae9b2fd4519241618f68b8aad612601e9bb), all 576 shape/liquid/designation
combinations, maximal captures/output, hidden evidence, restart, fixed deadlines,
failed/unfinished reads, torn writes, independent file/directory sync failures,
corruption, forged transitions, immutable terminal history and credential isolation.

Four syntax-valid weakened implementations are rejected by regression assertions:
counting same-tick reads, preserving interrupted streaks, omitting parent sync and
accepting visible walls as a floor goal. Source identities and test scope are
recorded in `docs/evidence/excavation-progress.json`.

This is not a real DFHack SDK/plugin or live-fortress campaign, Rust/MCP execution,
power-loss durability or full repository qualification. The standalone monitor
is not yet integrated into the Rust/MCP mining session's obligation inventory.
Native protocols, dependencies, production admission and all existing mutation
and recovery paths are unchanged. Game checkpoint/restore and cross-controller
coordination remain separate missing capabilities.
