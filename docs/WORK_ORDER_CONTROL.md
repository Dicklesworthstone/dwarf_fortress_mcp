# Restart-safe work-order creation coordination

The separate `dfmcp_adapter::work_order_control` module connects the existing
work-orders/1.10 codec and fixed transport to a creation-specific durable journal
and a session-owned observation/intent loop. This is unadmitted development source.
It does not change the native wire, dependencies, production runner map, or registry.
Read WORK_ORDER_CREATION.md and WORK_ORDER_RUST.md for the exact four-recipe native
scope and the distinction between inserted orders and completed production.

## Durable transaction

WorkOrderJournal retains the complete sealed plan, complete native effect and
DF/DFHack manifest for each key. It never stores authentication tokens or nonces.
The only native creation edge is:

1. Query and reversible ConfigureProduction authority are checked against the
   journal's fortress. Native IDs cannot satisfy canonical entity/map scopes;
   scoped and limited-use grants are refused, not silently widened.
2. Prepare requires the exact queue witness and game tick, spare retention and
   complete bounded record/RPC capacity. The native preparation is synced before
   acknowledgement. Exact key replay returns the original evidence without a
   native call or preparation-lifetime renewal.
3. Commit first persists and syncs DispatchStarted. Only then can the one native
   insertion call run. A verified Created or Refused outcome, or an Indeterminate
   state, is synced before acknowledgement. Errors after beginning the dispatch
   record are conservatively EffectIndeterminate, including failed sync.
4. Reconciliation requires only Query, and is compiled against WorkOrderQuerySource,
   an interface without prepare/commit methods. A lost successful reply can resolve
   to Created from exact native evidence without another insertion. Missing records,
   changed generations/software, Prepared replies after dispatch, and errors do
   not prove non-creation or restore dispatch permission.

Any unresolved key blocks new preparations and all other prepared dispatches at
THE JOURNAL boundary, not merely in the MCP presentation layer. Native Unknown
is immutable; a contradictory later receipt is rejected. Terminal replay cannot
call insertion. Cancellation permanently retires only a locally Prepared key;
it does not delete a manager order, send a native cancellation, undo creation,
or manufacture a Refused receipt. There is no automatic reconnect or retry.

## Identity and recovery modes

JournalMode is fixed when opening custody:

- Control requires Query plus ConfigureProduction and may initialize a new file.
- Reconcile requires Query, opens existing writable bytes, and may append verified
  reconciliation evidence. Even an injected later production grant cannot promote
  this journal into prepare/commit/cancel access.
- Offline requires Query and an existing genuinely read-only descriptor. It has
  no append/flush/sync/truncate or native connection path.

Private files use an operator-selected normalized absolute path, exact-mode 0700
parent and single-link 0600 regular file owned by the same owner as that parent.
The descriptor has an exclusive lock. Named/opened inode identity, parent identity,
permissions and expected extent are rechecked, including cached reads/replays.
A symlink, renamed/replaced file, permission change or unexpected appended data
fails custody. Synchronous filesystem operations have cooperative budget checks;
this is not a hard fsync timeout or protection against malicious same-user rewrites
of existing bytes without changing their extent. Files currently require Unix.

This is one coordinator's at-most-once dispatch guarantee. An operator must keep
one authoritative journal for this fortress/control profile. Choosing a different
journal, another controller or bypassing the coordinator is not made safe by a
checksum or a per-file lock. Missing/corrupt evidence must not be replaced with a
new empty journal to resume production.

## Journal format and retention

The new file is not compatible with pause or job-suspension journals; opening
those files fails rather than interpreting their bytes as creation evidence.

Header: `DFMWOJ10`, 32-byte incarnation hash, fortress u64, SHA-256 of the preceding
48 bytes. A frame contains `DFMWOR10`, body-length u32, transition-number u64,
previous-head digest, body, frame digest and `DFMWEND0`. Integers are big-endian.
Frame digest hashes `dfmcp-work-order-journal-frame/1` + NUL, journal ID, prefix
and body. Numbers start at one and are consecutive; predecessor heads must match.

The body contains durable state u8, key blob, complete observation blob, recipe
u8, amount u32, DF version blob, DFHack version blob and complete native effect
blob. Blobs use u32 byte lengths. Plans and effects are re-decoded/resealed on
replay and every transition is checked. No raw strings become native commands.
States are Prepared=1, DispatchStarted=2, Indeterminate=3, Created=4, Refused=5 and
CancelledBeforeDispatch=6. Native and journal state numbers are distinct domains.

Bounds: 4,096 retained keys, 16,384 transitions, 19 KiB body and 64 MiB file.
Both transition slots and byte capacity are reserved before native work. Torn
headers/tails, corrupt frames and illegal rehashed histories are refused without
repair, truncation, migration, eviction or evidence deletion. An exact complete
frame can recover after uncertain acknowledgement; a torn frame cannot be ignored.

Summary exposes prepared/unresolved/terminal counts. Whole-record keyset pages
include terminal records, are ASCII-key ordered, and require an exact head plus
current Query authority/custody. Pages contain 1..64 records and fail insufficient
bounds instead of silently dropping data. Session binding belongs above the journal.

## Session-owned intent

WorkOrderSession accepts an already-opened journal and optional native source;
it does not manufacture grants. Each operation requires the same session and
fortress plus current authority. Observe validates one complete native queue and
its manifest, rechecks Query expiry against the newly observed tick, and clears
old selection on a failed refresh. These are queue observations, not canonical
world snapshots or existing-order configuration/feasibility evidence.

Plan accepts only a closed finite recipe/amount, stable key and expected witness.
The plan digest/token are generated from server-retained observation bytes.
Commit requires both the retained digest and witness plus the exact current
selection. Any attempted commit invalidates selection; a new creation requires
a new observation. After restart, a still-prepared plan can commit only after
reacquiring identical native evidence; native TTL and witness revalidation remain
mandatory. Prepared and terminal wait calls return stored evidence without queries.

## Evidence for this increment

Eighteen Rust regression groups are registered: thirteen journal scenarios and
five session scenarios. They cover native fixture agreement, fsync-before-dispatch,
lost replies and syncs, reopen, cross-key fencing, immutable Unknown, cancellation,
Query-only recovery, scopes/expiry/cancellation/budgets, paging, corruption/torn
prefixes, illegal rehashed histories, retention reservation and exact selection.
These tests have NOT been compiled or executed: Rust, Cargo and rustfmt are not
available. No Rust, Clippy, filesystem fault, MCP, native SDK or live-game
qualification is inferred from their presence.

`python scripts/check_creation_journal_reference.py` executes an independent
fixed-plan Python reference. It matches the previously captured native Created
fixture, validates the new journal framing and legal paths, rejects 1,385 byte
corruptions, 1,382 incomplete prefixes and eight rehashed illegal histories.
Its report records source hashes. It does not execute the Rust implementation,
filesystem custody, RPC or MCP; it is reference-only evidence.

The next source integration is the separately gated MCP runtime using this loop.
No production runner or live-game admission follows from this library increment.
