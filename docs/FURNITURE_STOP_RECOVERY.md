# Recover original placements after an interrupted stop publication

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`,
`df-cancellation-progress-certificates-vvw`. Broad native execution and drain
qualification remain open.

A concurrent stop writer can die after creating `stop.json` but before writing
its complete sealed record. New placement must stay blocked. Previously the
same incomplete marker also prevented the ordinary batch owner from inspecting
or reconciling an already-dispatched placement, including a lost commit reply.
Recovery no longer requires deleting, completing or replacing that marker.

## Existing commands, original identities

```sh
python3 scripts/furniture_batch.py inspect \
  --directory "$ORIGINAL_BATCH" --batch-id "$ORIGINAL_BATCH_ID"

python3 scripts/furniture_batch.py query \
  --directory "$ORIGINAL_BATCH" --batch-id "$ORIGINAL_BATCH_ID" \
  --step "$ORIGINAL_STEP_NAME"
```

Inspection is offline. An unresolved query still requires the existing isolated
furniture development opt-in, original endpoint and current query credential.
`DFMCP_BUILD_ALLOW_PLACE=0` is sufficient; recovery never needs permission to
prepare or commit another placement. Already resolved receipts stay offline.

The existing `cancel` command can retire the original preparation, but cannot
turn an attempted/indeterminate placement into nonapplication. A retained
Indeterminate receipt remains pending until exact original-attempt Placed
evidence is independently validated and appended. The prior uncertain frame
remains in the same journal. Native absence alone resolves nothing.

No new CLI option, effect key, native method, protocol generation or journal
schema is added. The complete original plan and all prior children remain
required; selecting one recovery step does not create a reduced batch.

## A marker is a fence, not a receipt

Only `inspect`, `query` and `cancel` select the recovery opening internally.
That opening can retain an empty or malformed `stop.json` as opaque bounded
bytes. It does NOT accept a corrupt original manifest, index or effect journal.
The original semantic profile, checksums, plan, source, directory identities,
index chain and complete native history are replayed normally first.

The marker still needs the existing exact-mode 0600, private, single-link regular
file custody and at most 32,768 bytes. Symbolic links, wrong ownership or mode,
oversized markers and substituted files refuse. The file's exact bytes and
identity stay pinned during the operation; it is never repaired or written.
All other metadata retains its nonempty-file requirement.

A marker matching the exact original sealed stop record has the old behavior.
Otherwise the recovered inventory states:

```json
"stopped": true,
"advance_allowed": false,
"stop_evidence": {
  "status": "unverified_marker",
  "marker_bytes": 1,
  "marker_sha256": "<SHA-256 of retained marker bytes>",
  "marker_validated": false,
  "new_work_fenced": true,
  "recovery_only": true,
  "marker_modified": false,
  "native_quiescence_proven": false
}
```

`inventory_verified` describes the independently verified original plan and
child history, not positive validation of this stop record. The marker digest
is bound into the inventory digest, and the Agent Turn exposes its uncertainty
without echoing arbitrary marker contents. Different damaged markers cannot
share a review inventory merely because both imply stopped work.

Ordinary review, advance, stop publication and prefix execution remain refused
for the invalid marker. Even a recovery-opened owner has `advance_allowed=false`.
The existing stop-request command still refuses a torn marker unchanged.
Removing the marker to resume is not part of this workflow. A completed original
query does not grant new placement, certify a game pause or prove physical drain.

## Durable recovery and final publication

The ordinary recovery owner may register a crash-before-registration child, but
only after verifying the already-existing original intent. It never recreates
that intent or sends PreparePlacement/CommitPlacement. Native recovery retains
its exact source, full receipt, current authority and durability checks.

For an unverified marker, CLI publication reopens the SAME original batch under
the SAME shrinking operation budget, without repeating a native query. It
compares the complete inventory and marker witness, then holds custody through
serialization and the sole output write. File replacement, changed bytes,
expired deadlines or revoked query authority prevent a success acknowledgment.
A late failure or short write can leave output partially visible and returns
failure; it never emits a second object or repeats recovery. A valid receipt
already synchronized before publication failure remains available offline.

## Executed evidence

```sh
python3 -W error -m unittest discover -s tests -p 'test_furniture_stop*.py' -v
```

57 tests pass: 18 new full-owner cases, 12 new CLI/recovery cases and 27 unchanged
stop-writer cases. The new suites use ordinary complete modules, real private
journals, operating-system locks/synchronization, the actual TCP client and CLI
subprocesses with a joined synthetic furniture peer. One case kills the actual
stop-writer process after its first marker byte during a lost commit reply,
then recovers the same placement at the CLI without another placement attempt.

Other cases cover every truncated marker prefix, all 32 original slots,
indeterminate reconciliation, preparation cancellation, orphan registration,
source/receipt/custody faults, final-output locking, exhausted budgets and
single-attempt output. A separate baseline comparison preserves 24 original
inventories, 24 encoded packets and all corresponding guard counts exactly.

This establishes Python exact-item integration against synthetic sources, not
real DFHack, native/Rust builds, room/terrain-backed semantic recovery, physical
quiescence, live-game execution or full-repository qualification. The existing
native client, journal codecs, stop writers and production admission are unchanged.
