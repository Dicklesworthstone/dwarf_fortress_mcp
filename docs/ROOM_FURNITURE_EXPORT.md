# Whole-room terrain completion to the original furniture request

`export_room_furniture.py` connects the persisted whole-room terrain goal to the
existing `dfmcp.furniture-request/1` consumer. It removes the need to reconstruct
slots, coordinates, dependencies, materials or exclusions after excavation.
It is an isolated, read-only development CLI, not a new MCP tool, an automatic
furnishing controller or an admitted runtime.

Beads `df-dfhack-bridge-plane-c-pic.3/.4/.5`, WP-05 and WP-10 remain open.

## Use the original whole-room journal

The input must be the private journal created by `track_excavation.py
start-rooms` and subsequently brought to `satisfied` by its normal sampling
workflow. A dig batch receipt, residual-blueprint monitor or generic floor goal
cannot substitute. Native designations being verified is not mining completion.

Use only the existing isolated map-read environment. Remove other `DFMCP_*`
configuration rather than mixing read and mutation credentials:

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin credential>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000

python3 scripts/export_room_furniture.py \
  --journal "$HOME/room-work/terrain.journal" \
  --emit request > furniture-request.json
```

Check the exit status before consuming the file. Exit zero returns the exact
canonical original furniture request, with no trailing newline. Exit two
returns a refusal object, not a partial furniture request. Broken or short
stdout is not retried and never causes a second object or native read.

The JSON text can be supplied unchanged as `open_session.furniture_request` to
the existing isolated furniture development MCP server. That separate consumer
must perform its own fresh inventory allocation and its existing per-step
prepare/revalidate/commit/recovery protocol. Its operator configuration and
credentials remain separate. This command allocates and reserves **no items**.
It creates **no batch, preparation, effect key, designation or building**.

Omitting `--emit request` (or selecting `--emit report`) returns a bounded audit
report with the entire original room goal, exact request and request digest,
original journal identity/head, completion sample reference, and newly captured
raw map bytes, manifest, diagnosis and report digest. The report does not contain
credentials. Neither its hash nor the narrow request is a signature or an
acquisition attestation; retaining the original journal is still necessary to
independently replay the claimed historical stability. A bare request does not
carry this evidence or grant downstream authority.

## Conditions for export

The existing private journal owner opens the exact absolute path read-only,
locks it, checks ownership/0700-directory/0600-single-link-file custody, and
replays the bounded complete history. The goal must be the whole-original-room
profile, terminal `satisfied`, with no unfinished read. Pending, unknown,
stabilizing, cancelled, expired and invalidated histories refuse before any
connection. No goal is rewritten, repaired, extended or silently restarted.

Under the original shrinking wall/work allowance, the command binds the saved
endpoint and existing map query authority. It performs at most one map capture
using the unchanged four native calls: two bindings, handshake and observation.
The connection closes before local re-decoding and projection. There is no
reconnect or cached-request fallback after a failed read.

The fresh raw bytes are decoded again. The map generation, software, fortress,
dimensions and selected region must agree with the retained source. Its tick
cannot regress, and the completion-to-export gap cannot exceed the original
goal's maximum sample gap. The original goal deadline remains a historical
completion deadline, not a new lease renewed by export.

Every original floor and required bedroom wall must hold together in that one
fresh capture. Each original furnishing target must also be visible, a dry
floor, undesignated and free of buildings and units. An occupied **non-target**
floor does not invent a placement blocker. Missing a single target or required
wall rejects the whole export; it never drops a room or emits a partial request.

The exact original request constructor preserves all slots, targets, ordering,
dependency edges, material/subtype/distance constraints and excluded item IDs.
Complete report serialization is reserved even for the narrow request export.
The report is bounded to 256 KiB and the unchanged request to 16 KiB. Existing
whole-history work/deadline bounds, four native calls and 4 MiB of network bytes
are shared, not renewed by helpers. Journal byte/path custody and live authority
are checked again after the capture and final serialization.

## What success does not prove

Success establishes sampled conditions, not current or continuous terrain,
structural safety, pathfinding, furniture availability, placement eligibility,
construction completion or room assignment. Matching map selectors cannot
identify an unseen same-tick restore. Other protocol generations are independent
namespaces, not evidence of a shared incarnation.

This command does not reconcile native effects. It cannot clear unknown digging
or furnishing work, authorize a replacement batch/key, or replace the existing
sealed allocation and placement origins. Resolve uncertain effects through their
original journals. A later furnishing step must still revalidate its own current
native preconditions. Production admission and the frozen MCP catalog are unchanged.

## Execution evidence and remaining checks

Twelve pure stage-guard/serialization methods executed successfully on these
sources. They cover complete 32-slot checking, source/clock/stability/whole-goal
failures, target visibility and occupancy, guard interruption, exact canonical
request bytes, full-report reservation for narrow exports, and refusal output.
Their lightweight inputs model **already replayed and decoded** records. They do
not test the journal owner, native codec, sockets, CLI process or MCP consumer.

The separate twelve-method integration suite is implemented using the actual
private journal/reducer, existing joined fragmented map peer, real raw fixtures
and CLI subprocess. It covers the full transition, wrong goal profiles, lost
walls/floors, target occupancy, source and clock changes, native reply failures,
missing/torn/replaced custody, revocation after read/serialization, budget
exhaustion, short stdout, and 32 slots with 646 original exclusions.

**That integration suite has not executed.** The remote execution connection was
unavailable; the local environment did not contain the full repository. Python
syntax compilation of the new files and the twelve pure tests is not integration
or live-game qualification. No Rust/MCP, DFHack SDK/ABI/game, inventory allocation,
furniture placement or repository-wide pass is claimed for this increment.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests \
  python3 -m unittest test_room_furniture_export_unit -v

# Required, not yet executed for this change:
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests \
  python3 -m unittest test_room_furniture_export -v
```
