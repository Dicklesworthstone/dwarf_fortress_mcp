# Stop an active furnishing batch without its executor locks

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`,
`df-cancellation-progress-certificates-vvw` (broad execution/drain work remains open).

The ordinary `furniture_batch.py stop` opens the batch under its exclusive
root lock. That command can stop between calls, but cannot acquire custody
while a placement or final publication holds it. Use this separate signal
command to request a stop while those locks remain held:

```sh
python3 scripts/request_furniture_batch_stop.py \
  --directory "$ORIGINAL_BATCH" --batch-id "$ORIGINAL_BATCH_ID"
```

`ORIGINAL_BATCH` must be a normalized absolute path to the existing private batch
directory. Obtain the ID from that batch's original initialization or review.
The command needs no DFHack credentials, sockets, controller lease, original
terrain-journal access, or currently valid permission to place furniture. It
never loads the placement client or weakens any later placement checks. Paths
remain operator configuration, not an addition to the MCP tool surface.

## What is recorded

The command verifies the exact canonical sealed `batch.json`, its current
profile's size/depth bound, the manifest SHA-256 that the existing `Batch.id`
uses, and the original root/effects directory identities. This is identity-only
revocation: it does not validate semantic room/terrain/allocation evidence or
certify the child-effect inventory. All four existing furnishing batch profiles
use the same stop-record format.

It then exclusively creates and synchronizes the original `stop.json` format:

```json
{"schema":"dfmcp.furniture-batch-stop/1","batch_id":"<original SHA-256>"}
```

Those fields are enclosed in the existing `value`/`sha256` seal and canonical
newline. This is exactly the representation that `furniture_batch.Batch`
recognizes on reopen, not a second stop file or new journal generation.

The active owner's existing directory-membership checks notice the new file
and refuse their next checked boundary. A later full `Batch` reopen recognizes
the permanent stopped state. The original manifest, `steps.jsonl`, effect keys,
receipts, and external terrain journal are never written by the signal command.
The index is descriptor-pinned by identity but not read or locked, so concurrent
original index appends do not prevent a stop request.

## What success does not mean

A local permanent signal is not an interrupt delivered to the game. A native
call that has already passed its last check or entered dispatch can still
complete. A lost response can leave the original effect uncertain. Success
therefore retains `in_flight_effect_may_complete: true`,
`native_quiescence_proven: false`, and `native_effects_cancelled: false`.

There is no game pause, rollback, native cancellation, construction completion,
automatic reconciliation, commit retry, or permission to resume a stopped batch.
Do not remove a stop marker to continue. Preserve all original custody and use
the existing explicit original-key query/recovery workflow for uncertain effects.
No native effect is resolved merely because this stop was recorded.

## Publication failures

Directory, manifest, index and effects identities remain checked through marker
publication. Files are opened without following links; nonregular, multiply
linked, incorrectly owned/mode, replaced or over-bound files are refused.
Only competing stop writers contend on the marker's nonblocking lock; no root,
manifest, index, effect or terrain lock is acquired.

A fully written matching marker can be reverified and file/directory-synced
again without changing its bytes or inode. This handles loss of a synchronization
or output acknowledgment. A conflicting or incomplete marker is left unchanged
and refused, never overwritten or repaired. A process killed during publication
may leave a torn record that fences the ordinary owner on reopen. Retain that
record and its original evidence rather than creating replacement effect keys.
A refusal cannot certify that no stop was recorded.

Outputs are bounded to 8 KiB and include an Agent Turn explicitly lacking any
canonical world anchor or verified effect inventory. A short output write is
terminal; it never repeats publication or writes a second response.

## Executed validation and limits

```sh
python3 -W error -m unittest discover -s tests -p 'test_furniture_stop_request.py' -v
```

The 27 tests run with real POSIX files, locks, subprocesses, synchronization and
process death. They include a CLI call while another process holds root,
effects, manifest and index locks; concurrent index appends; every checkpoint
of a small publication; both synchronization-acknowledgment losses; process
kill during a partial write; marker/root replacement during synchronization;
all four profile bounds; file/link attacks; and single-attempt output failure.
The unchanged stop-file helper was checked against Git blob
`392872ecb63eae9d3c748ceaf5328f0aac569138` before execution.

Fixtures contain deliberately identity-only manifests, not real placement
receipts. Existing `Batch.check`, stop decoding and placement guard integration
were source-reviewed, not executed as a complete native batch in this component
checkout. The tests do not qualify real DFHack, TCP placement execution,
room/terrain semantic replay, native/Rust builds, physical drain completion,
full-repository qualification or live production admission. No existing native
protocol, placement dispatcher, journal schema or recovery command was changed.
