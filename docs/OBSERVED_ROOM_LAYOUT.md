# Place complete room projects against observed map constraints

The existing automatic layout engine can now use one complete map/1.5 capture
instead of requiring agents to manually encode all unsuitable locations.
`scripts/survey_room_layout.py` acquires the requested search region once, closes
the native connection, and runs the existing whole-project solver and corridor
compiler against constraints derived from those exact bytes.

## Use the same room request

The input is the existing `dfmcp.room-layout-request/1`, including its region,
complete area set, furnishing constraints, fixed/constrained origins, focus,
exclusions and search limits. Use the example in
`schemas/examples/automatic_room_layout_v1.json`, replacing its illustrative
fortress identity and search region. No source data or credentials go into that
request. All areas must fit one same-level region of at most 1,024 cells.

Native acquisition requires an isolated operator environment:

- `DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT` equals exactly `1`;
- `DFMCP_MAP_TOKEN` is the existing 32..256-byte map query credential;
- optional `DFMCP_ROOM_LAYOUT_ENDPOINT` is canonical numeric IPv4 loopback,
  defaulting to `127.0.0.1:5000`.

Other `DFMCP_*` variables refuse, including mutation and production-admission
settings. Credentials are not command arguments, artifacts, or error details.
This is an explicitly unadmitted development command, not a production MCP runner.

```sh
python3 scripts/survey_room_layout.py --request-file rooms.json > survey-report.json
python3 scripts/survey_room_layout.py --request-file rooms.json --emit survey > survey.json
```

Those are alternative commands; each request invocation makes one independent
capture. The original fortress must be paused at capture. Its identity and exact
region must match the request. The unchanged map client binds only Handshake and
ReadObservation, checks nonce/profile/generation/software consistency, bounds
frames/notifications and refuses truncated data. There is no automatic reconnect
or read retry. The enclosing deadline and current environment authority are
checked throughout native I/O and again through search and final serialization.

## What the map changes

Visible, dry, undesignated, unoccupied walls permit proposed floors or preserved
walls. Equivalent existing floors permit floors only: they cannot stand in for
a bedroom's required boundary wall. Hidden, missing, liquid-bearing, designated,
building/unit-occupied and unsupported-shape cells permit neither use. Required
map-edge context must fit actual dimensions. Unit occupancy is a conservative
current exclusion, not a claim that the obstruction is permanent.

The derived site mask is intersected with any original caller mask; observations
cannot widen a hard constraint. Original item and region exclusions remain.
Candidate pruning checks both floor and wall masks. The same mask constrains
all connecting corridors in the existing RoomPlan compiler, so routing may take
a different path or reject the whole candidate. No area or furnishing is dropped.
All complete candidates and node/work exhaustion retain the existing solver's
semantics. `no_layout_in_domain` means no compiler-valid layout under this finite,
conservative model; missing observations can cause that result. It is not a
proof that the real fortress has no possible layout.

This filter is NOT native digging eligibility. A visible dry wall does not prove
absence of aquifers, warm/damp warnings, adjacent water/magma or structural hazards.
The map/1.5 payload does not establish all those facts. Neighbouring hazards are
not acquired by this command. No unsafe override or mining permission is inferred.

## Retain source and intent together; export exact existing artifacts

The `dfmcp.observed-room-layout/1` survey artifact retains the normalized original
request, complete constrained layout, effective mask, classification counts,
source manifest/fortress/dimensions/tick, source witness and raw capture bytes.
Its digest binds all of them. Import reruns native decoding, mask intersection
and the existing layout search before comparing every canonical byte.
A forged chosen origin, source tick, mask, count, or status does not pass import.
These are comparison hashes, not signatures or independent acquisition attestations.

```sh
python3 scripts/survey_room_layout.py --survey-file survey.json --emit room-plan > room-plan.json
python3 scripts/survey_room_layout.py --survey-file survey.json --emit layout > layout.json
python3 scripts/survey_room_layout.py --survey-file survey.json --emit furniture-request > furniture-request.json
```

Import mode is offline and requires no query credentials. It does not refresh the
map or imply present conditions. `--emit room-request` and `--emit excavation`
use the existing exports too. Artifact outputs have no trailing newline. Default
`report` output carries an Agent Turn and the full survey, with acquisition in
this call distinguished from offline replay. Its canonical world anchor is null.

Keep `survey.json` for source evidence. Lowered RoomPlan/layout artifacts retain
only explicit geometry constraints, not an acquisition attestation. An excavation
export is a complete desired floor mask, not a residual dig plan. Use the existing
room terrain survey/residual-designation workflow and its fresh native review
before effects; existing floors must not be blindly re-designated. Allocation,
placement, exact effect journals and readiness remain separate reviewed stages.
None is started automatically by this command.

## Bounds and actual tests

Input remains 16 KiB. One captured region is at most 1,024 cells; raw map input
is bounded to 575 + 20 times its cell count. Whole survey artifacts fit 192 KiB;
complete command reports fit 256 KiB. Input files use the existing stable bounded
no-follow regular-file reader, not private-journal/all-parent custody. A single
1..60,000 ms foreground deadline covers input, preflight, native acquisition,
search and output; aggregate work cannot renew after acquisition. Native
transport retains its existing 4 MiB connection-byte bound. All original solver
area/furnishing/part/floor/node/work limits remain. No partial output is a plan.

Exit 0 means a valid result, not necessarily a found layout. Narrow artifact
exports require `layout_found`; invalid input/source/authority/budget or output
returns exit 2. A short stdout write produces neither a second object nor a retry.

All 34 focused tests pass in one run: 12 site-mask tests, 11 map/solver tests and
11 command tests. These execute the actual complete Python modules, existing
map decoder and TCP client, compiler, router, allocator and export/import codecs.
The CLI suite uses real files and subprocesses with a synthetic TCP peer. It
verifies one capture, close-before-search, all exports, actual 1,024-cell/32-furnishing
capacity, lost reads, nonce/profile/source drift, malformed/full-barrier maps,
read-only isolation, post-read/final-render revocation, shared budgets and short
writes. No native DFHack SDK, live-fort, Rust/MCP or full-repository qualification
was performed. The underlying map and offline CLI files were unchanged and
verified against their Git blob identities.

Run: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest test_room_site_mask test_observed_room_layout test_survey_room_layout -v`.
Owning beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (remain in progress).
