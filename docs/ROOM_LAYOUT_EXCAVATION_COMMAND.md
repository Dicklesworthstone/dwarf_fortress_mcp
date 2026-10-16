# Acquire room layout and original excavation in one call

Beads: `dfmcp-wp-pln01-spatial-blueprint-layout-pwd` and
`df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open.

`scripts/survey_room_excavation.py` is the executable acquisition/export path for
[one-source room layout and excavation](ROOM_LAYOUT_EXCAVATION.md). It reuses the
existing map/1.5 client and room-layout authority, performs one complete padded
read, closes that connection, then runs the actual layout and terrain/handoff
components. It does not run digging, create a native preparation, generate
replacement keys or retry source contact.

## Acquisition and retained replay

Input is the existing `dfmcp.room-layout-request/1`, including optional fixed
origins, permitted rotations, access point, site constraints and exclusions.
For example, save this as `layout-request.json` (coordinates are illustrative):

```json
{
  "schema": "dfmcp.room-layout-request/1",
  "world_folder": "region1",
  "site": 2,
  "region": {"origin": [10, 10, 2], "size": [10, 6, 1]},
  "access_point": [10, 10, 2],
  "areas": [{
    "name": "hall",
    "origin": [16, 10, 2],
    "template": {"kind": "dining_hall", "table_count": 1}
  }]
}
```

Native acquisition uses ONLY the existing isolated environment:
`DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT=1`, an operator-provided `DFMCP_MAP_TOKEN`
(32..256 UTF-8 bytes, no NUL), and optional numeric IPv4 loopback
`DFMCP_ROOM_LAYOUT_ENDPOINT` (default `127.0.0.1:5000`). Every other `DFMCP_`
variable is refused, including production state and dig credentials. Never put
tokens in request files. Load the token through the existing local operator
configuration before running:

```sh
DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT=1 \
python3 scripts/survey_room_excavation.py \
  --request-file layout-request.json --emit compilation > room-compilation.json
```

Check exit status before consuming redirected output. A successful compilation
may still report no layout, inconclusive search, blocked terrain or empty work;
inspect its status. To derive the original excavation handoff without contacting
the fortress again:

```sh
python3 scripts/survey_room_excavation.py \
  --compilation-file room-compilation.json --emit report
python3 scripts/survey_room_excavation.py \
  --compilation-file room-compilation.json --emit excavation-handoff > room-excavation.json
```

Offline replay needs no native credentials, performs no connection and reports
zero observations acquired in the current call. It regenerates the complete
parent/source/projection/layout/terrain chain before exporting. The unchanged
`dig_blueprint_client.py init --room-handoff` consumer accepts the resulting
legacy handoff; use its separately authorized private-directory and explicit
checkpoint-policy workflow. Initialization and subsequent step review/advance
remain separate operations. The map-read environment cannot grant dig authority.

## Exact exports and outcomes

`--emit` supports nine bounded outputs:

| Export | Meaning |
|---|---|
| `report` (default) | Compact decision and Agent Turn with full original request, source identity, counts, blockers and typed next step. |
| `compilation` | Complete replayable parent evidence and derivation, at most 512 KiB. |
| `layout` | Existing canonical layout result, including unsuccessful search status. |
| `room-plan` | Complete selected original RoomPlan. |
| `room-request` | Frozen selected room intent, including orientations and entrance. |
| `furniture-request` | Every original selected furnishing and constraint. |
| `terrain-survey` | Complete legacy survey, with its original RoomPlan restored and its digest valid. |
| `remaining-excavation` | Only the nonempty unblocked remaining-wall blueprint. |
| `excavation-handoff` | Exact legacy original-room handoff, at most 256 KiB. |

There is deliberately no full-mask `excavation` fallback. A missing handoff cannot
silently export already-open floors or preserved walls as digging targets.
Remaining-work exports return exit 2 with distinct codes for an inconclusive
search, no layout, terrain blockage or no remaining work. Reports/compilations
retain those well-formed outcomes with exit 0; check `result.status` as well as
`ok`. Empty work is sampled terrain truth, not room completion.

Other refusals are redacted JSON with `result: null`, no token, caller path,
partial handoff or automatic retry. Reports are bounded to 64 KiB. Standalone
artifacts have exact canonical bytes with no trailing newline; reports have one
newline. Short writes terminate without dispatching again or emitting a second
JSON object.

The one 1..60,000 ms wall allowance (default 10,000) includes stable input reads,
preflight, native connection/read, CPU derivation and serialization. Requested
work allowances apply across preflight and every later stage. The original
native client retains its 4 MiB stream and notification bounds. Current authority
is checked throughout and again after the final output is serialized. No
background task or later delivery is created.

## Executed scope

```sh
python3 -m unittest discover -s tests -p 'test_room_layout_excavation.py' -v
python3 -m unittest discover -s tests -p 'test_room_layout_excavation_cli.py' -v
```

The focused suites completed separately with 25 component and 20 CLI/TCP methods
passing (45 distinct tests). The CLI suite exercises all nine exports through
actual subprocesses, all nine offline exports, source protocol/nonce/generation
confusion, lost/truncated/refused reads, complete 32-slot projects, bad vertical
context, authority revocation after capture and serialization, cancellation
budgets, exact survey digest restoration, stable-file restrictions and short
output writes. The unchanged original single-level survey has a byte-identical
regression case. Two additional unchanged CLI dependencies were Git-blob checked,
bringing the verified pre-existing dependency count to fourteen.

Combined reruns exceeded the executor call limit; they are not claimed as
completed aggregate runs. The six final CLI cases also passed in a bounded
rerun. All 21 local Python source/test files stayed unchanged during validation.
The peers and game payloads are synthetic; no real DFHack producer, actual dig
batch execution, Rust build, full repository qualification or live-admission
campaign was run. Native eligibility, current game truth, existing effect
reconciliation, room assignment and production admission remain unproven.
