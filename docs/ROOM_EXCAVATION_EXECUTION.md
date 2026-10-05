# Original room intent through durable native digging

The existing dig/1.16 batch workflow now accepts the raw-evidence-backed room
handoff. It preserves the complete original room request, every furnishing
constraint and exclusion, the map capture, and the exact remaining excavation
through initialization, review, single-attempt designation, restart and recovery.
It is an isolated development CLI, not a production MCP tool or live qualification.
Beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/WP-10) remain open.

## Workflow

First export one complete handoff with the isolated map/1.5 environment:

```sh
python3 scripts/survey_rooms.py --plan-file room-plan.json \
  --emit excavation-handoff > room-excavation.json
```

Check the command's exit status. Blocked or empty residuals do not produce a
handoff. Switch to the existing isolated dig environment; remove the map-profile
variables rather than mixing credentials. Only a disposable test fortress is
eligible under the existing explicit policy:

```sh
export DFMCP_ALLOW_UNADMITTED_DIG_V1_16=1
export DFMCP_DIG_TOKEN='<matching dig plugin credential>'
export DFMCP_DIG_ENDPOINT=127.0.0.1:5000
mkdir -m 700 "$HOME/room-dig"

python3 scripts/dig_blueprint_client.py init \
  --directory "$HOME/room-dig" --room-handoff room-excavation.json \
  --checkpoint-policy disposable-fortress-no-checkpoint

python3 scripts/dig_blueprint_client.py observe --directory "$HOME/room-dig"
```

Initialization performs a native read, not a designation. Source, software,
clock and all future native halo extents are checked before creating batch files.
The room branch rejects `--world-folder`, `--site` and `--allow-hidden-neighbors`
overrides. The original request supplies the fortress; hidden-neighbor admission
is fixed to false. The dig endpoint must equal the retained map endpoint.

Each `advance` still requires the returned batch ID, step, exact observation
witness, native plan digest and review seal. Inspect the native blockers and
review before separately enabling `DFMCP_DIG_ALLOW_DESIGNATE=1`. The existing
`--expected-witness`, `--confirm-plan` and `--review-seal` flags are unchanged.
One advance dispatches at most one native commit, with a fresh observation and
original intent persisted before preparation. It never unpauses the game.

`query` and `cancel` use the existing `--batch-id` and `--step` flags. A lost
reply leaves the original step unresolved and blocks subsequent work. Query
reconciles that key; cancel retires its original preparation when permitted.
Neither dispatches another commit. Verified terminal receipts can be returned
offline without credentials. `stop` stops future batch steps but does not cancel
or erase an unresolved native effect. Never initialize a replacement batch to
work around uncertain effects; this workflow is not a global controller fence.

## What is retained and verified

The new `dfmcp.dig-blueprint-batch/2` manifest contains the complete handoff.
Reopening regenerates the original room plan and recomputes the survey from raw
map bytes before trusting the residual. Its domain-separated batch identity
covers the entire manifest, so every existing `bp-<batch-id>-<step>` recovery key
also binds the original room constraints, not merely excavation geometry.

The native bootstrap and all later child observation evidence must match the
original endpoint, fortress selectors, dimensions and DF/DFHack software; their
clock cannot precede the survey. Native generation and intervention-sequence
checks remain independent. Map and dig generations are different namespaces,
not evidence of a shared incarnation. Matching selectors cannot detect an unseen
same-tick restore. Exported hashes are not signatures or native-acquisition
attestations and do not defeat an owner rewriting all evidence consistently.

Raw evidence remains in the immutable private manifest. Responses carry the
whole original room plan, source/digests and exact residual without repeating
raw capture hexadecimal. Historical child audits reuse immutable canonical
summary bytes only after complete validation, under the still-pinned manifest.
This is operation-local reuse, not a persistent cache or renewed authority.

The input is a bounded regular file with final-component no-follow and stable
file/path checks. It is copied into private batch custody and is never reopened
after initialization. The original private directory/manifest/effect files,
locks, no-follow traversal, ownership, single-link checks and fsync boundaries
remain required. Missing or substituted custody is refused, never repaired.
Live room operations share one deadline, eight million work checkpoints, ten
native frames and 4 MiB of native bytes. Ten frames cover six existing bindings,
handshake, read, prepare and one commit. Native helpers cannot renew the budget.
Final serialization rechecks live authorization even after a durable effect;
short/broken output never triggers another effect or a second JSON object.
Filesystem calls and stdout writes are not forcibly interruptible.

Room artifacts are bounded to 256 KiB; room manifests to 320 KiB and nesting 14;
room responses to 256 KiB. Legacy batch/1 retains its 64 KiB manifest, nesting 8,
128 KiB response and original canonical bytes. Native protocols, effect capsule
and receipt formats, dependency pins and production admission are unchanged.

## Recover the original completion goal

```sh
# Offline; returns the exact canonical RoomPlan without a trailing newline.
python3 scripts/dig_blueprint_client.py inspect --directory "$HOME/room-dig" \
  --emit room-plan > original-room-plan.json
```

This refuses legacy batches that have no original room plan. The exported plan
can be used with the existing `track_excavation.py start-rooms --plan-file` path
under its separate map environment. That monitor evaluates all original floors
and required walls, not just the residual. Its process integration was not run
in this increment. The separate legacy `track_dig_blueprint.py` monitor remains
residual-scoped and must not be treated as original whole-room completion.

`designations_verified` proves retained native designation readback only. It is
not actual mining completion, furniture construction, room assignment, current
terrain, continuous wall preservation, pathfinding or structural safety. Neither
a successful handoff nor a completed residual grants any of those claims.

## Executed tests

Thirty-five distinct methods passed on restored final sources in three separate
runs: 17 handoff/map-export methods, 17 native batch methods, and the large-case
method. The combined invocation hit the executor limit; the separate final runs
all completed without failures or skips. The actual clients, codecs, private
filesystem journals and CLI subprocesses communicated with an independently
encoded, fragmented, joined synthetic map/dig TCP peer.

The large test retained 32 furniture slots and 646 excluded item IDs and executed
all 30 native designation steps covering exactly 157 remaining tiles, excluding
the already-dug floor and required walls. Other cases cover lost preparation and
commit replies, unknown queries, explicit cancellation, stale confirmation,
source changes, rehashed substitutions, revoked authority, invalid readback,
manifest loss, budget exhaustion and offline recovery after input deletion.

An additional check compared 22 canonical results byte-for-byte with the exact
pre-change tracker over seven native fixture steps. Its reference Git blob is
`30d4d71759f237a6a1d75f223e56b58fa80a17f1`. Three deliberately weakened versions
failed assertions when survey-source binding, original-intent batch identity or
final publication authorization was removed. Original sources were restored
before all final test runs. Exact hashes and logs are retained in
`docs/evidence/room-excavation-python.json` and `room-excavation-tests.txt`.

No real DFHack plugin/SDK/ABI or live fortress, Rust/MCP build, furniture effect,
room assignment, downstream monitor process or full-repository qualification
was executed. No production capability or compatibility tuple was admitted.
