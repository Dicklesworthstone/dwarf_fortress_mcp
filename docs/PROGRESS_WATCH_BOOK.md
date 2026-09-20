# Durable archive-bound progress watches

A separate `WatchBook` retains registrations and local cancellations for the
progress/1.12 archive. Every outcome is reconstructed from ALL subsequent exact
archive records, not trusted from a cached status or a second sample checkpoint.
A crash after capture append but before evaluation cannot drop that sample. The
book stores no game credentials, native pointer, effect receipt or produced-goods
claim. No native method, dependency, production runner or admission changes.

## Commit and recovery model

The book header binds its fortress and exact archive incarnation. Registration
seals the key, native order, predicate, absolute deadline, cadence, stability and
exact origin record. New registration requires the latest archive record,
Query plus Observe authority at the archive's maximum retained tick, and a
horizon within the caller's game-tick allowance. The registration is synced before
its in-memory root or acknowledgement is published. A same-key, same-origin replay
returns the old definition and cannot renew its deadline or revive cancellation.

Cancellation is local monitor retirement, never a manager-order cancellation.
It requires the exact definition and expected archive head. The evaluator first
consumes every intervening sample through that head; only a still-pending watch
may be cancelled. A satisfied, expired, missing, modified or discontinuous watch
cannot be rewritten into cancellation. Cancelled keys remain retired forever.

Opening a book verifies framing, all referenced origin/cancellation records and
the semantic cancellation order. All watches share one ordered archive scan:
each encountered frame is reread and verified once, then fed to every applicable
watch. A rehashed cancellation after satisfaction is rejected. Results preserve
exact origin, frontier and positive-sample references. Terminal results are
historical observations, not current game-state or goods-production proof.

A new archive segment retires unfinished watches as `continuity_lost`. This includes
the first capture after live reopening; neither downtime observations nor object
identity across reload are invented. Already terminal historical results remain
unchanged. New monitoring after a discontinuity needs a new explicit key and origin.
Offline reopening before another capture can still report a historically pending
watch; that is not a claim that monitoring continued while the process was down.

## Custody, authority and limits

Live mode requires current Query and Observe and a live archive. Offline mode
requires existing bytes and Query only; its read-only descriptor and fixed mode
refuse writes even with an injected later Observe grant. A distinct operator path
uses an exclusive lock, exact 0700 parent, single-link 0600 regular file and repeated
named/opened file and parent identity/extent checks. Source failure does not by
itself prevent historical evaluation; corrupt archive or book custody does.
Synchronous filesystem calls have cooperative deadlines, not forced fsync interruption.

The book retains at most 32 lifetime keys, 64 events and 128 KiB. Keys are ASCII
letters, digits, period, underscore or hyphen, 1..64 bytes. There is no deletion,
key reuse, rotation, eviction, truncation or automatic repair. Torn writes and
uncertain sync fence the current owner. A complete uncertain event may be recovered
on reopen; an incomplete event is refused unchanged. A missing/corrupt book must
not be replaced with an empty file to forget work. Checksums and per-file locking
are not a hostile-host guarantee or external anti-rollback floor; a valid suffix
removed before reopening is not detectable without such a floor.

The 112-byte header is DFMPWB01, fortress u64, archive ID (32), book ID (32), and
SHA-256 of `dfmcp-progress-watch-header/1` + NUL + the preceding 80 bytes. Frames
are DFMPWR01, body length u32, sequence u64, predecessor digest (32), body (<=512),
frame hash (32) and DFMPWE01. The frame hash binds `dfmcp-progress-watch-frame/1`
+ NUL, book ID, exact prefix and body. All integers are big-endian. Register events
contain the canonical specification, exact origin and definition digest; cancel
events contain the key, definition digest and exact cancellation frontier. They
never serialize a derived success flag in place of the archived evidence.

## Validation status

Thirteen additional Rust groups cover persistence/reopen, sample recovery,
cancellation ordering, rehashed illegal histories, corruption/torn prefixes,
independent binary fixtures, file modes/locking/replacement, sync failures,
current authority, fixed offline mode, finite retention and shared-scan budgets.
They are registered but UNCOMPILED AND UNEXECUTED: Rust/Cargo/rustfmt are unavailable.

`python scripts/check_progress_watch_book_reference.py` validates independent
307-byte book and 866-byte archive fixtures, rejects 307 byte corruptions,
306 incomplete prefixes, seven rehashed illegal histories and a foreign archive.
This is a fixed-plan Python model, not execution of the Rust parser, private files,
MCP, a native plugin or a game. The report and source hashes are retained in
`docs/evidence/progress-watch-book-reference.json`.
