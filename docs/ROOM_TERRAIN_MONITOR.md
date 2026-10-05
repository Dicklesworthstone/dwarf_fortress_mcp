# Restartable whole-room terrain monitoring

`track_excavation.py start-rooms` connects the complete original room intention
to the existing private, append-only read-only goal journal. This closes a gap
between room planning and completion: a residual excavation can finish while
other original floors remain unsuitable, and a floor-only blueprint can finish
after a required bedroom wall has been opened.

The monitor uses `RoomTerrainGoal` from `room_terrain_goal.py`. It requires every
original floor and every required bedroom boundary wall in the **same capture
and the same advancing-tick stability streak**. It never substitutes a remaining
excavation mask, accumulates independent room successes, or discards furnishing
constraints, excluded items, excluded regions or the original recipe.

This is an isolated development CLI, not a new production MCP tool. Native
map/1.5 bytes and RPCs, dig/furniture effect journals, confirmation, idempotency
keys, dependency pins and production admission are unchanged. Owning bridge
beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/WP-10) remain open.

## Start and resume

Use only the existing isolated read-only map environment. Remove unrelated
`DFMCP_*` variables; no excavation or furniture mutation credential is accepted.

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin token, 32..256 bytes>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000

# The journal path must be absolute, with an owned private 0700 parent.
mkdir -m 700 "$HOME/room-monitor"
python3 scripts/track_excavation.py start-rooms \
  --journal "$HOME/room-monitor/goal.jsonl" \
  --plan-file room-plan.json --max-game-ticks 1200

# Each command performs at most one new coherent observation.
python3 scripts/track_excavation.py sample \
  --journal "$HOME/room-monitor/goal.jsonl"

# These do not connect or require any map credential.
python3 scripts/track_excavation.py inspect \
  --journal "$HOME/room-monitor/goal.jsonl"
python3 scripts/track_excavation.py cancel \
  --journal "$HOME/room-monitor/goal.jsonl"
```

`start-rooms --request-file rooms.json` can replace `--plan-file`: the complete
original room request is compiled and validated before any socket opens. Its
world folder and site come from the original intention, not separate overrides.
The input is a bounded regular file with final-component no-follow and stable
file/path identity checks. It is operator input, not all-parent journal custody
or an MCP filesystem capability. It is copied into the new journal and never
reopened by sample, inspect or cancel. Existing journals cannot be overwritten.

The start command accepts `--stable-ticks` (default 10), `--required-samples`
(default 2), `--max-gap-ticks` (default 1200) and the required `--max-game-ticks`
(1..403200). The initial observed game tick fixes the inclusive deadline. Each
operation accepts one `--timeout-ms` (1..60000, default 10000). It creates no
background poller, changes no clock state and never unpauses the game.

## Meaning of results

Exit 0 means a complete result was obtained, not that the goal is satisfied.
Read `goal_status` and `room_terrain_goal_satisfied_at_sample` in the
`dfmcp.room-terrain-progress/1` result. Floors must be visible floor shapes, dry
including the magma flag, and undesignated. Required walls must be visible dry,
undesignated, unoccupied wall shapes. Shared walls count once. Occupied floors
are reported but do not establish furniture placement eligibility. Unselected
capture holes and intervening levels are not invented completion requirements.

A mismatch yields `pending`; hidden or missing required evidence yields
`unknown`. All matching requirements start one `stabilizing` streak. Repeated
or paused ticks cannot increase the matching-sample count. An excessive sample
gap resets the streak. Source manifest, observed fortress identity or dimension
changes, or clock regression, make the goal `invalidated`; time beyond its game
deadline makes it `expired`. `satisfied`, `expired`, `invalidated` and `cancelled`
are terminal: later sample/inspect/cancel calls return retained evidence without
network access or changing history.

The complete original room plan, source binding, journal identity/head, goal and
plan digests, last trusted observation witness, exact floor/wall category counts,
and up to 64 ordered whole-row deficits survive every response and restart.
Unshown deficits remain counted. The Agent Turn includes the whole-goal digests,
explicit bounded operation budget and no invented world anchor or native effect
inventory. Category precedence is documented in `ROOM_TERRAIN_GOAL.md`.

**Satisfied terrain is not a completed room project.** It does not establish
native room assignments, furniture availability/placement/construction, native
pathfinding, structural safety, continuous wall preservation, current terrain,
or attribution to a digging operation. Furniture and original effect recovery
remain separate workflows. Cancelling this monitor never cancels an excavation
or furniture action, clears uncertain effects, or permits replacement keys.

## Durable recovery and custody

The same all-parent no-follow journal traversal, private 0700 directory, private
0600 single-link regular file, nonblocking exclusive lock, identity checks,
append/fsync verification and fenced-error behavior apply. The room profile has
its own `dfmcp-room-terrain-journal/1` hash domain. Replay reconstructs the room
intention and recomputes progress from every retained native-format sample; no
stored success flag or caller-derived tile objects are authoritative.

Before each new sample a `read_started` event is durably appended. A lost reply
or refused read produces `read_failed` when the remaining budget/custody permit.
A crash or failed append can leave an unfinished read; inspection exposes it as
unknown with zero matching samples, and the next explicitly requested sample
resets the streak before continuing. Nothing reconnects or automatically retries
a read. Torn, oversized, aliased, substituted or malformed history is refused,
not truncated, repaired, migrated or rebound to a new fortress.

Map-read authority and the shrinking budget are checked through compilation,
replay, native I/O, reduction and final serialization on live room operations.
Revocation withholds the live result, even if a sample was already persisted.
Offline inspection can still recover historical evidence. A short output write,
broken pipe or flush failure returns failure without issuing another read or
writing a second JSON object. Preserve and inspect the original journal after
an output failure. Read/goal/custody failures emit sanitized errors without
credentials, operator paths or native reply text.

The hashes detect content substitutions under local custody; they are not
signatures, external attestations, cross-profile incarnation identities, or a
proof against an owner rewriting the entire history and recomputing its hashes.
Filesystem calls and output writes are not forcibly interruptible by the wall
allowance. Use a larger permitted timeout to inspect long histories rather than
repairing or trimming them.

## Bounds and compatibility

The original room compiler's 32-slot/32-part/512-floor-target limits remain.
The coherent padded room selection has at most 4096 cells, at most 128 per axis,
and at most 82495 raw capture bytes. A room journal frame is at most 262144 bytes;
a journal is at most 32 MiB, with 260 events and at most 128 additional read
attempts after its initial observation. Capacity exhaustion rejects another
sample before connecting while preserving offline inspection and cancellation.
The full canonical room response has a 128 KiB reservation before publication.

One command shares eight million work checkpoints, four native RPC frames (two
bindings, handshake, one observation), 4 MiB native bytes and one wall allowance.
Native and per-sample helpers cannot renew them. The initial room file read is
bounded by the request/plan byte limit plus one growth/EOF probe. Only existing
`Handshake` and `ReadObservation` are bound. The existing floor and blueprint
profiles retain their original schemas, hash domains, byte/nesting/output bounds
and canonical response bytes; no legacy journal migration is necessary.

## Executed qualification

```sh
PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_terrain_goal test_track_room_terrain -v
```

All 32 focused methods passed: 14 pure goal methods and 18 journal/CLI/TCP/
compatibility methods. These execute the actual room compiler, map decoder/client,
private files, subprocess CLI commands, and a fragmented, joined synthetic TCP
peer. They cover partial excavation and opened walls, full-intent retention after
input deletion, source change, ten native-fault variants, lost replies, crash
markers, revocation, final serialization, output failure, work/network/call
exhaustion, custody and corrupt journals, and offline terminal/cancel behavior.

Twenty legacy floor/blueprint event traces and their twenty serialized responses
match the exact pre-change tracker byte-for-byte; both legacy CLI paths also
complete against the peer. Full-retention testing replays 129 observations in
257 events, refuses another read and still appends offline cancellation.

Measured fixtures acquired a 3072-cell / 61510-byte capture in one observation.
A 32-slot plan retaining 646 excluded IDs generated an 80591-byte initial journal
frame and a complete 36000-byte JSON response with 64 deficit rows, exceeding
the old floor/blueprint response limit without truncation. Both fixtures used
one connection and four native frames. The 128-read history occupied 15938426
bytes. These are fixture measurements, not universal runtime guarantees.

Three independent mutation checks failed their targeted assertions when required
wall checks, unfinished-read streak reset or final output authorization were
removed. Original sources were restored and the complete suite rerun. Exact
source/dependency hashes, commands, fixture measurements and log hashes are in
`docs/evidence/room-terrain-monitor-python.json`.

The peer and terrain are synthetic native-format fixtures, not a real DFHack
plugin or fortress. Native digging, furniture placement/construction, room
assignment, Rust/MCP, SDK/ABI, live-game, release/publication and full-repository
qualification were not executed. No production capability was admitted.
