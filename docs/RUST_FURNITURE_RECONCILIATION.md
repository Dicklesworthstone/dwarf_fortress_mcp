# Explicit Rust furniture receipt reconciliation

`BuildJournal::reconcile_receipt` closes the adapter-level dead end after an
Indeterminate furniture/1.19 receipt has already been retained. It queries the
original native key and can append exact original Placed evidence. It never
observes a replacement selection, prepares, commits, cancels, or grants a permit.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.

This is an explicitly unadmitted development API. The native engine and RPC
implementation from earlier commits remain unchanged. Session/MCP query routing
is separate integration work from this first journal increment.

## Separate proof operation

Ordinary BuildRecord::validate_successor, commit replay and BuildJournal::recover
retain their previous immutable-receipt semantics. A new
BuildEntry::can_reconcile_receipt identifies a Terminal coordinator record whose
native phase is Indeterminate; Terminal is a retained-receipt tag, not necessarily
proof that the effect resolved. Explicit reconciliation does not turn cancellation
into a recovery read or native nonapplication proof.

The new journal operation abandons preparation permission, validates current
Query authority, complete original journal bytes, mode, work budget and capacity,
and checks the exact original endpoint/software/generation binding. It repeats
source checks around the supervising guard and native query and again before
publication. Each exit fences the supplied source. Use a fresh recovery source,
not a previous effect connection or a source factory that grants placement.

A missing reply remains EffectIndeterminate. Identical uncertainty returns the
original entry without adding a frame. Only a complete decoded Placed receipt for
the identical original plan can proceed. Its existing decoder checks the exact
expected-after capture and original stage-zero building/job/item relationships.
No generic successor rule is relaxed. Dispatch/cancellation history cannot change.

## Append-only custody

Replay permits precisely Terminal(Indeterminate) -> Terminal(Placed) with the same
key, complete plan and dispatch/cancellation flags. The prior frame is retained;
there is no overwrite, truncation, repair or new key. The existing retain path
validates, appends, flushes, synchronizes, verifies full readback, then publishes
the new in-memory root. Older readers reject the new transition rather than
silently interpreting it as fresh authority. The journal grammar, native bytes,
record limits and enum tags are unchanged.

New uncertain records reserve one final proof frame within the existing 1,792
frame / 16 MiB ceilings. An old journal without sufficient room is refused before
query can update native retained evidence. A failed sync keeps the old local root
and fences the owner. Complete surviving bytes may be independently replayed on
reopen, without asserting the failed call acknowledged durability. A torn append
is not repaired. Resolved records remain immutable and offline-inspectable.

## Validation status

Eight Rust regression tests are added for positive append/reopen, repeated or
missing evidence, ordinary replay/cancellation isolation, exact source checks,
authority/custody/budget/capacity rejection, sync failure, torn append, and the
80-case post-receipt phase/history matrix. They use the actual journal and codec
with injected native replies and storage faults.

These tests have NOT been compiled or executed in the editing environment:
rustc, cargo and rustfmt are unavailable. The original journal was verified
byte-for-byte against GitHub blob da4740d5e78e168c861e3c0743b9f36752fdb738 before
editing, and uploaded source blobs were compared with the local authored bytes.
This is source review, not Rust, native SDK, live-game or full-workspace evidence.
No runtime, dependency, admission registry, game mutation surface or production
runner is changed. Historical placement evidence still does not prove a completed
or usable building, current pause or a game checkpoint.
