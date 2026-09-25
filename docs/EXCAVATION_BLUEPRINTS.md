# Sparse, multi-level excavation goal evidence

The read-only model in `scripts/excavation_blueprint.py` evaluates an exact terrain
mask against one unchanged map/1.5 native capture. It does not designate terrain,
advance time, authorize a mutation, or discharge a native effect obligation.
Owner: `df-action-coordinator-exec-ero.4` (bounded temporal verification increment).

## Blueprint contract

```json
{
  "schema": "dfmcp.excavation-blueprint/1",
  "parts": [
    {"region": {"origin": [10, 20, 30], "size": [3, 3, 1]}, "shape": "floor"},
    {"region": {"origin": [13, 21, 30], "size": [1, 1, 1]}, "shape": "stair_up"},
    {"region": {"origin": [13, 21, 31], "size": [1, 1, 1]}, "shape": "stair_down"}
  ]
}
```

Parts are disjoint cuboids in zero-based map tile coordinates. All selected cells
must have the exact requested normalized shape, zero liquid depth and no dig
designation in the same sample. Shapes are `empty`, `wall`, `floor`, `ramp`,
`ramp_top`, `stair_up`, `stair_down`, and `stair_up_down`, using the existing
map/1.5 tags documented in `LIVE_MAP.md`. Unsupported/other is not a target.

Bounds: 1..32 parts, at most 512 target cells, and at most 1,024 cells in their
single enclosing capture. Existing coordinate and 128-tile side bounds apply.
An oversized enclosure is rejected, not split into incoherent reads. Input JSON
is at most 16 KiB with depth at most eight, duplicate and extra fields rejected.
Even same-shape overlapping parts are rejected. The semantic digest is invariant
under part ordering and disjoint rectangle splitting; it is not an authorization
signature. Source and fortress identity must be bound separately.

Unselected holes in the enclosure are acquired but never evaluated as targets.
Hidden/missing selected cells remain unknown, with no fabricated attributes.
`diagnose` returns exact counts and 0..64 whole remaining-cell rows in native
x-fast/y/z order, with an explicit omission count. Shape, liquid and designation
mismatch counts may overlap; matched/mismatched/hidden/missing counts partition
all targets. These are observed deficits, not inferred causal blockers.

`BlueprintGoal` and `advance` implement whole-mask sampled stability. Every part
must qualify simultaneously. Strictly advancing game ticks count as new samples;
failed/unfinished reads, unknowns, contradictions and excessive gaps reset the
streak. The fixed deadline is inclusive. Source/software/generation, fortress,
dimension changes or a regressed clock invalidate the goal. Terminal progress is
immutable. Both evaluation and diagnostics re-decode native bytes, rather than
trusting caller-supplied derived fields in a Capture object.

## Durable foreground workflow

The existing `scripts/track_excavation.py` now supports `start-blueprint`; `sample`,
`inspect` and `cancel` auto-detect the retained goal profile. Save the JSON above
as `blueprint.json`. Configure the unchanged map/1.5 plugin and a matching token
as described in `LIVE_MAP.md`, then use a dedicated existing exact-mode 0700
journal directory and a new filename:

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin token, 32..256 UTF-8 bytes>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000

python3 scripts/track_excavation.py start-blueprint \
  --blueprint blueprint.json --journal /private/progress/rooms.jsonl \
  --world-folder region1 --site 1 --max-game-ticks 1200 \
  --stable-ticks 10 --required-samples 2

python3 scripts/track_excavation.py sample --journal /private/progress/rooms.jsonl
python3 scripts/track_excavation.py inspect --journal /private/progress/rooms.jsonl
python3 scripts/track_excavation.py cancel --journal /private/progress/rooms.jsonl
```

The input is a bounded stable-read regular file; a final-component symlink or
special file is refused. Ancestor symlinks in the operator-selected input path
are not a confinement boundary. In contrast, the private journal rejects symlinks
in every path component. The complete validated specification is retained in the
journal, independent of the input file's later modification, removal or path.
No later command consults that file. This is a developer CLI file input, not an
MCP filesystem capability.

Start and each explicit sample acquire at most one coherent observation of the
whole enclosure. They do not designate tiles, poll, run detached tasks, or change
the game clock. `inspect` and monitor-only `cancel` are offline. Terminal commands
perform no native read and preserve the existing terminal bytes. A fixed deadline
is calculated from the first native tick; overflow is refused. Time options,
credential isolation and shared 1..60000 ms operation budget match the existing
floor workflow documented in `EXCAVATION_PROGRESS.md`.

### Separate retained profile, unchanged legacy semantics

Blueprint headers use `dfmcp.excavation-blueprint-goal/1`. Their checksum domain is
`dfmcp-excavation-blueprint-journal/1` followed by NUL. Bounds are 64 KiB per frame,
8 MiB per journal, 21,055 native sample bytes, 260 events and 128 subsequent read
attempts. Retention reserves space for cancellation before admitting another read.
The original floor profile retains its existing format, checksum domain, 16 KiB
frame, 2 MiB journal and 2,048-byte sample limits. There is no automatic migration,
format substitution, or relaxation of legacy goals.

Both profiles use the same actual private-file backend: exclusive nonblocking
lock, nofollow custody checks, 0600 single-link file, 0700 parent, append-only
hash-chain replay, and file **and parent-directory** synchronization before
acknowledgement. A durable read intent precedes each subsequent connection.
Failed or unfinished reads reset whole-mask stability. Replaying native samples,
not trusting serialized status flags, reconstructs every goal transition. Torn or
corrupt histories are preserved and refused, not repaired. Complete frames surviving
an uncertain synchronization are historical evidence on reopen, not retroactive
certification of the failed acknowledgement or physical power-loss durability.

### Machine output

Blueprint results use `dfmcp.excavation-blueprint-progress/1`, including the
blueprint digest, source and journal identities, sampled status, exact selected-cell
counts, and the first 16 remaining target rows with an explicit omission count.
The complete Agent Turn is reserved before journal publication and the response
remains bounded at 32 KiB. `blueprint_goal_satisfied_at_sample` reports the goal
state; `floor_goal_satisfied_at_sample` remains false for this distinct profile.
The common native-effect, safety and continuous-stability claims remain false.

Exit zero means a valid result, not necessarily a satisfied goal. Pending, unknown,
expired and invalidated are valid retained outcomes. Operational failures return
exit two without credentials or uncontrolled exception details. When an unreadable
journal cannot establish its profile, the shared error envelope reports unknown;
it does not guess a blueprint identity or success.

## Executed regression evidence

```sh
PYTHONPATH=scripts python3 -m unittest \
  test_excavation_observer test_track_excavation \
  test_excavation_blueprint test_track_excavation_blueprint -v
```

All 63 tests pass: the unchanged 35 observer/floor-journal tests, 12 blueprint
model tests and 16 new actual blueprint journal/CLI tests. Coverage includes
4,608 exact shape/liquid/designation combinations, 500 floor-transition traces
against the unchanged legacy evaluator, fragmented real loopback TCP, multilevel
subprocess start/sample/restart, deleted input files, full 1,024-cell captures,
512-target/32-part output bounds, source changes, fixed deadlines, unfinished and
failed reads, independent file/directory sync failures, torn writes, private-file
custody, cross-process locks, profile-domain substitution and local cancellation
at the 128-read retention limit. The four suites are registered in `verify.sh`.

Test captures are explicit native-layout doubles and the unchanged retained
map/1.5 fixture. These are Python execution and POSIX/TCP fault-injection results,
not Rust/MCP execution, a real DFHack SDK build, a live fortress, physical power-loss
or full-repository qualification. No Rust toolchain was available in the editing
environment. The new workflow remains a standalone unadmitted developer tool,
not part of the Rust/MCP native-effect obligation inventory.

Matching stairs do not prove connectivity; matching walls do not prove continuous
preservation. No material, construction provenance, structural support, occupancy,
temperature, route safety, continuous history or mining causality is established.
No native protocol, dependency, MCP route, production map or admission is changed.
