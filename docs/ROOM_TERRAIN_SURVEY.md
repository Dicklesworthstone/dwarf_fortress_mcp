# From a room intention to its remaining excavation

`scripts/room_terrain.py` reduces one coherent map/1.5 capture against the complete
original `RoomPlan`. It handles rooms that are partly excavated: existing dry,
undesignated floors are recorded as observed geometry, and only the remaining
visible wall targets enter a proposed excavation blueprint. Neither a terrain
sample nor a remaining-mask digest authorizes a game effect.

This is an isolated development path, not a production MCP tool. The original
room recipe, native map/dig protocols, journals and mutation admission are unchanged.
Owning beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open.

## Complete selection and explicit deficits

The original room intent is regenerated before trusting its compiled outputs.
The selection contains every proposed floor (rooms, doorways and corridors) and
all required bedroom boundary walls except each intended doorway. One bounding
capture includes a full one-tile three-dimensional pad; dimensions are not
clamped, split, translated or silently shortened to fit the map profile.

A target is either an observed dry, undesignated floor or a remaining dry,
undesignated, unoccupied wall shape. Hidden/missing evidence, liquid or a magma
flag, existing dig designations, occupied walls, and unsupported shapes block
the complete excavation proposal. A required bedroom wall must independently
have a visible dry, undesignated, unoccupied wall shape. An existing opening is
a deficit, not permission to fill it or change the room recipe.

For every remaining wall target, the unique one-tile 3D halo must be visible,
dry and undesignated. Known liquids, hidden/missing halo cells and existing dig
work block the proposal. This includes vertical and diagonal neighbors. This is
a conservative survey policy, not the native dig eligibility predicate: aquifers,
wall material, structural support, water pressure, protected native state and
unit paths are not established. Fresh native review can still refuse every part.

Unused capture holes are not invented targets or deficits. When no wall targets
remain, there is no remaining-excavation halo to check; this does not turn missing
context into safety evidence. Floor occupancy is counted separately. A floor
with a building or unit can satisfy a shape predicate without granting permission
to place a new furnishing there.

## Exact remaining blueprint, never a successful subset

The reducer retains the full original room plan, constraints and completion
blueprint. It reports every remaining wall coordinate and exact target, required
wall and halo category counts. Diagnostic rows are bounded to 128; all additional
failed checks remain counted. A coordinate can fail more than one domain check,
so the blocker count is a check count, not a unique-coordinate count.

Only an unblocked nonempty residual emits `remaining_blueprint`, using the existing
`dfmcp.excavation-blueprint/1` schema. A complete shortage of coverage, wall deficit,
hazard or capacity failure emits no executable subset. The proposal excludes
all existing floor cells, required walls and unselected holes, including between
levels. No native job, receipt, cancellation or uncertain effect is queried.

Two deterministic exact covers are compared: a global z/y/x width-first cover,
and the same cover clipped to each original disjoint recipe part. The smaller
part count wins, then canonical bytes break ties. This avoids unnecessarily
rejecting ten-bedroom recipes whose original decomposition fits the limit.
It is not a minimum-rectangle algorithm. The independently counted 8x8 partition
matches the existing normal-mining geometry rule; it grants no prepare token.

The existing 32-part, 512-target, 1,024-cell unpadded bounding capture and 128-native-
step limits still apply to the resulting excavation artifact. A fragmented mask
that exceeds either partition limit is refused in full, not truncated or split
into secretly independent plans. The survey's padded capture has at most 4,096
cells with each axis at most 128. Raw survey evidence is at most 82,495 bytes;
the entire result has a 128 KiB reservation including its eventual envelope.

## What success means

`excavation_proposed` means the remaining mask passed only these explicit sampled
checks. `terrain_shapes_satisfied_at_sample` means the full floor and required-wall
shape predicates held in this single capture. Neither means native excavation
eligibility, current terrain, continuous preservation, structural safety,
pathfinding, room assignment or construction completion.

The retained source includes the exact map profile/manifest, folder, site,
dimensions, padded region, tick, pause observation, raw SHA-256 and observation
witness. `survey_digest` binds the full result to the original room intent and
these source facts. Supplied bytes or exported digests alone do not independently
attest native acquisition. Map and dig generations remain separate namespaces.

**Never use a residual to replace an unresolved dig batch or to mint retry keys.**
Reconcile all original effects through their existing original-key query/cancel
workflow first. This survey has no effect inventory and cannot clear uncertainty.
Even successful execution and monitoring of the residual alone cannot discharge
the complete room project: the original floor blueprint remains the completion
goal, and required walls, furnishings and native room assignments are separate.

## Pure API and executed scope

```python
from room_provisioning import RoomPlan
from room_terrain import selection, survey

room_plan = RoomPlan.decode(original_plan_bytes)
region = selection(room_plan).region
# Obtain exactly this region with the existing read-only map client.
result = survey(room_plan, capture_bytes, map_manifest)
```

The 16 focused Python tests execute the unchanged room compiler, furniture request
contracts and strict map decoder. They enumerate all 4,096 4x3 residual masks,
all 250 bedroom count/size combinations (including whole-recipe refusals), and
all 64 dining count/column combinations. Additional cases cover partial and fully
excavated rooms, shared bedroom walls, multi-level holes, vertical/diagonal hazards,
capacity exhaustion, input/source substitutions, map edges, deterministic identity,
complete bounded deficits, and interruption at every guarded boundary of a small
survey. Source and log hashes are recorded in `docs/evidence/room-terrain-python.json`.

These are supplied native-format byte fixtures, not real DFHack or live gameplay.
This first reducer qualification does not execute sockets, the native dig client,
Rust/MCP, the DFHack SDK, a game save, or the full repository suite.

## Executable one-shot map survey

`scripts/survey_rooms.py` connects the reducer to the unchanged map/1.5 reader.
Use a clean map-profile environment, removing unrelated `DFMCP_*` names before
running it. This command accepts neither inventory nor digging/placement tokens:

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5=1
export DFMCP_MAP_TOKEN='<matching map plugin token, 32..256 bytes>'
export DFMCP_MAP_ENDPOINT=127.0.0.1:5000

python3 scripts/survey_rooms.py --plan-file room-plan.json > room-survey.json
# Or compile the complete original request within the same guarded command:
python3 scripts/survey_rooms.py --request-file rooms.json > room-survey.json
```

The input must be a bounded, nonempty regular file. The final path component is
opened without following symlinks, and descriptor/path identity, size and timestamps
must remain stable throughout reading. This is operator input, not all-parent
no-follow custody, a new journal, or a portable proof of native acquisition.
Imported plans are regenerated before opening any socket. The complete request,
map selection and raw response are validated again before any successful output.

Each invocation opens one numeric IPv4 loopback connection, binds only the existing
`Handshake` and `ReadObservation`, and obtains at most one coherent capture. The
native profile, codec, nonce checks, manifest consistency and byte format are not
reimplemented or changed. A revoked environment or exhausted outer allowance can
stop the connection before a bind, handshake, send, receive or final disclosure.
Malformed, refused or lost replies never cause a reconnect or second observation.

The default command writes one canonical JSON envelope without a trailing newline.
Exit 0 means a complete survey result was obtained, **not that excavation is ready**:
check `result.status`. A coherent `blocked` survey still retains the whole original
room plan and complete deficit counts, with `remaining_blueprint=null`. A fully
satisfied sampled shape predicate also has no residual. Input/read/source/authority/
budget failures return exit 2 and a sanitized error envelope with no partial result,
credential, caller path or native text.

The common Agent Turn carries explicit unknowns instead of a fabricated canonical
world anchor or effect inventory. `acquisition` identifies the actual endpoint,
map profile, one-read count and survey digest. A separate `report_digest` binds the
entire envelope, including endpoint, to the underlying result. These digests detect
content substitution; they are not signatures, an external trust root, or a shared
incarnation identity with dig/1.16.

### Exact standalone residual export

```sh
python3 scripts/survey_rooms.py --plan-file room-plan.json \
  --emit remaining-blueprint > remaining.json
```

This is a **new** one-shot survey, not an export from a previous response. Export
succeeds only for an unblocked, nonempty complete residual, and writes the exact
existing excavation-blueprint schema without a wrapper or trailing newline. The
full report must still fit its byte allowance before the narrower artifact is
returned. A blocked or already-satisfied result returns exit 2, not an empty or
partial executable blueprint. Use the default survey form to inspect its deficits.

Retain the original room plan and check the command's exit code before consuming
redirected output. The residual can be supplied to the existing separate
`dig_blueprint_client.py` workflow; its fresh paused terrain review, eligibility,
confirmation, original-key custody and one-attempt commit rules are unchanged.
This command does not initialize a batch, inspect old receipts, start excavation,
create a checkpoint or unpause the fortress. Earlier uncertain effects must still
be reconciled through their original workflow, never replaced by a new residual.
All later whole-project checks must keep the original complete goal rather than
silently treating the smaller residual as the requested room project.

### Shared bounds and executed integration

The command has one 1..60,000 ms wall allowance (default 10,000), 250,000 work
checks, four native RPC frames, 4 MiB native bytes and a 49,153-byte file-read
reservation including a growth/EOF probe. Individual parts, pages or the native
client constructor cannot renew this allowance. Authority is checked throughout
compilation, selection, native I/O, reduction and after complete serialization;
narrow exports do not bypass the final check. Filesystem calls and stdout writes
are not forcibly interruptible. A short write, broken pipe or flush failure returns
failure without reconnecting or attempting to emit a second JSON object.

Run the complete focused suite:

```sh
PYTHONPATH=scripts:tests python3 -m unittest test_room_terrain test_survey_rooms -v
```

All 31 tests passed together: the 16 pure tests above and 15 additional integration
tests. The integration suite executes actual CLI subprocesses, stable-file reads,
the unchanged native map client and fragmented TCP replies from an explicit joined
test peer. Cases cover partial/already-dug/blocked rooms, complete standalone exports,
source or selection substitution, malformed profiles/frames, lost replies, operator
revocation during I/O and CPU work, shared budget exhaustion, final serialization,
output failure, and the unchanged default map-client path.

A 3,072-cell, maximum-width source fixture acquired 62,015 bytes and returned a
9,430-byte complete envelope. A separate 32-slot room recipe retained 646 excluded
item IDs and returned 31,875 bytes after a 22,870-byte capture. Both used exactly
one observation. These are measured fixture sizes, not universal runtime promises.
Five independent mutation checks rejected implementations missing original-intent
revalidation, fortress selection, required-wall checks, vertical halo checks, or
final authority checks. Final unmodified sources then passed all 31 tests again.

Exact source, dependency and log hashes are recorded in
`docs/evidence/room-terrain-client-python.json`. The peer and terrain are synthetic
native-format fixtures, not a real DFHack plugin or game. The native dig compiler/
client, digging effects, furniture placement/completion, Rust/MCP, SDK/ABI, live-game
and full-repository suites were not executed here. No native method, dependency pin,
journal format, mutation gate or production admission was changed.
