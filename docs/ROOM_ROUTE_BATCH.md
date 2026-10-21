# Retain whole-room constraints through route-backed placement

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (remain open).
Machine contract: `architecture/room_route_batch_v1.json`.

The existing route batch can now explicitly start from a complete original
`RoomRouteHandoff`, rather than discarding the parent geometry and importing
only its furniture review. The room stays attached through private storage,
reopen, inspection, original-key recovery and each placement gate.

This is an unadmitted development workflow. No production runner, compatibility
entry, native method, credential, game checkpoint or new effect authority is
introduced. Paused map samples are NOT an atomic lock on the whole room/map.

## Initialize a new room-backed batch

Create or bind a complete original handoff with `review_room_routes.py` as
specified in `ROOM_ROUTE_WORKFLOW.md`. Keep its canonical artifact bytes and
independently selected handoff digest. The ordinary route-review artifact alone
does not establish a parent room and cannot upgrade an existing batch.

Use the existing isolated route-batch environment and existing operator-configured
read/build credentials described in `ROUTE_FURNITURE_BATCH.md`. In that environment:

```sh
python3 scripts/route_furniture_batch.py init \
  --directory /absolute/private/room-route-batch \
  --room-handoff-file original-room-route.json \
  --expected-room-handoff ORIGINAL_HANDOFF_DIGEST
```

The operator supplies an existing empty exact-mode `0700` directory. Paths and
digests above are illustrative. Initialization validates the whole handoff before
native contact and retains it with the first original native selection. It does
not claim that all room terrain is ready, grant a placement permit, or place
anything. A historical blocked handoff remains original intent, not success.

`--room-handoff-file` and `--expected-room-handoff` form one required pair. They
are mutually exclusive with the existing `--review-file`/`--expected-review`
pair and are refused on every operation other than `init`, before reading input
or contacting the game. There is no migration, late parent adoption or downgrade.

Existing review, advance, inspect, query, cancel and stop commands use the returned
batch ID exactly as before. Each advance still attempts ONE explicitly reviewed
original furnishing. For example:

```sh
python3 scripts/route_furniture_batch.py review \
  --directory /absolute/private/room-route-batch --batch-id ORIGINAL_BATCH_ID

python3 scripts/route_furniture_batch.py advance \
  --directory /absolute/private/room-route-batch --batch-id ORIGINAL_BATCH_ID \
  --expected-plan REVIEWED_NATIVE_PLAN_DIGEST --confirm-review PAIRED_REVIEW_SEAL
```

Only a ready review supplies confirmation values. Reports expose the complete
original room plan, original route review, parent identities and every original
step. Inspection means verified historical custody, not fresh room readiness.
A review/advance reports whether room constraints were checked in that call;
reopening resets that observation flag rather than reviving earlier readiness.

## Closed durable origin and legacy compatibility

A room-backed definition uses `dfmcp.room-route-furniture-batch/1`. It retains
`room_handoff` plus `room_handoff_digest` instead of duplicating the legacy child
`review` and `review_digest`. All shared native manifest, initial capture, nonce,
root and effects-directory identity fields remain mandatory. The exact sealed
`batch.json` bytes determine the batch ID and therefore every original child key.
Parent identity also enters the fresh frontier evidence and paired confirmation.

The root inventory remains `batch.json`, `index.jsonl`, `effects/` and optional
`stop.json`. There is no optional parent sidecar to lose or ignore. Unknown fields,
incomplete origins and relabeling the new definition as an old one refuse.
Unchanged older readers reject the new definition instead of silently executing
it as a furniture-only plan. Existing `dfmcp.route-furniture-batch/1` roots retain
their original bytes and report shape, without a new room prerequisite.

Existing owner-private storage checks, exclusive locks, exact file/parent identity,
append synchronization and readback remain responsible for custody. A copied root,
changed parent, missing registered child, partial write or unrecognized root file
is not repaired, truncated or discarded. The complete original room no longer
depends on the input artifact file being available after initialization.

## Mandatory whole-room gate

The normal acquisition shell still obtains one map/inventory bracket and closes
its read connection before analysis. It first authenticates the original placed
prefix, checks every remaining original item and fixed delivery suffix, and
checks current predecessor construction using the existing producer. A final
room gate then uses the SAME raw paused map; it acquires no additional sample.

`room_route_frontier.gate_frontier` revalidates the complete parent, exact review,
plan, prefix, next step, route limit and source identity. Every original floor,
doorway, corridor and required bedroom wall must match its raw observed condition.
A historical placed receipt never removes the floor beneath that furnishing from
the room goal. Floor occupancy alone does not contradict shape; a required wall
must remain present, dry, undesignated and unoccupied. Hidden/missing cells remain
unknown rather than becoming successful empty requirements.

A matching room cannot override a route or construction blocker. If those earlier
gates are ready, a contradicted room narrows the result to `room_blocked`, and an
unknown room to `room_unestablished`. Both suppress confirmation. The evidence
commitment includes the exact parent and the prior execution-frontier identity.
A room-backed batch rejects an old furniture-only report even when it says ready.

The unchanged coordinator calls the combined gate before creating its original
intent, and again AFTER its Dispatch record has been synchronized but BEFORE
native commit. The second complete evidence identity must match the first, and
the existing final selected-item native capture check still runs. A room change
at the second gate prevents commit while retaining the pending original journal;
it does not roll back uncertainty or allow another prepare/commit attempt.

The room pass consumes the unspent portion of the SAME six-million-unit analysis
reservation. It does not renew work. Complete definitions remain bounded at
128 KiB, indexes at 32 KiB, and final responses at 256 KiB with the original native
outcome reservation before intent creation. New frontier input/output ceilings
are 160/192 KiB. At most 64 room deficits are displayed, with exact complete-mask
counts and explicit omitted-row counts. Large combinations can still exhaust
work, time or final output and refuse without partial success.

## Recovery and remaining limits

Query/cancel use the existing original native child key and build-only recovery
authority. They do not require map/inventory credentials or reacquire room terrain.
Offline inspection and stop preserve the full room parent. Stop prevents future
advances, not historical effects. A resolved placement receipt is historical
stage-zero registration, not room or construction completion.

Another actor can change remote room terrain after the last map capture. Native
placement still checks its existing local selection, not the whole room. No
atomic room/placement, native hauling path, future route, room assignment,
completed-construction or current usability proof is supplied here. Separate
monitor/clock workflows and every native effect contract remain unchanged.

## Executed development evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_room_route_frontier test_room_route_batch -v
```

41 distinct tests pass with no skips: 15 pure gate tests and 26 private-batch,
coordinator, acquisition-shell, CLI and contract tests. The pure tests execute
the actual room/map/review codecs with an injected prior execution-report port.
Integration tests execute real private batch/child files, synchronization and
reopen, the unchanged placement coordinator, actual route/remaining-suffix
analysis, room gate, input readers, argument handling and publication checks.
Native placement I/O and current construction evidence are injected ports.

Default acquisition-shell tests call the real `acquire_frontier`, but inject
its read client, operations decoder/projection and construction-report producer.
They verify source closure, exact-map reuse, shared reservations and gate order;
they do not qualify native TCP, pagination/release or current construction facts.
The actual offline inspect CLI also runs in a subprocess with an audit hook
refusing sockets and child-process creation. Tests cover 32 targets, rotations,
all required cells, broken rooms despite ready routes, new schema/reopen, original
key recovery after a lost commit reply, post-Dispatch room loss, torn publication,
late authority revocation and single-attempt output.

Five weakened implementations fail regression assertions: ignore room defects,
rebind the source, accept a furniture-only callback, omit the default room gate,
and omit final review authorization. A shared legacy fixture has byte-identical
inspection/review packets against the exact predecessor source; its original
reader rejects a new room-backed root without changing bytes. This is scoped
component evidence, not the entire pre-existing test suite or all room/corridor
combinations. No live DFHack, SDK/plugin, Rust/MCP, automatic runner/clock,
full-workspace or production-admission qualification is claimed.
