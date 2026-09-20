# Durable Rust workforce coordination

`workforce_control::journal` connects the typed workforce/1.17 source to durable
assignment coordination. `open_private_workforce` supplies Linux x86_64/aarch64
private-file custody. It does not change native protocol 1.17, dependencies,
production runners or admission. This binary journal is deliberately distinct
from the Python developer client's JSONL format; there is no implicit migration.

## Effect ordering

A journal binds one exact numeric loopback endpoint, native generation, software
pair, world folder and site. Every source is checked against that binding, and
every observation against the explicit fortress. Fresh current authority is
required; stored bytes never restore grants or turn recovery into control.

Prepare re-observes the original complete paused workforce capture, checks the
entity allowance and current Plan/ConfigureLabor authority, and synchronizes an
Intent frame before native preparation. The validated native preparation receipt
is synchronized before acknowledgement. Duplicate preparation neither calls the
bridge nor renews its native lifetime. A lost preparation reply leaves Intent;
one explicit query can recover the original preparation.

Commit accepts only locally Prepared records. It re-observes the exact paused
capture and synchronizes DispatchStarted before the single native assignment
attempt. A crash, failed reply, malformed readback or uncertain later sync cannot
make that marker dispatchable again. Even a crash before the actual request was
sent stays conservatively unresolved. A native Prepared receipt recovered after
the marker becomes Tracking, not Prepared. Applied must pass the full typed
post-configuration reconstruction before its terminal frame is retained.

Reconciliation performs at most one QueryAssignment. Missing native records
preserve previous evidence and return explicit uncertainty, not nonapplication
or permission to change keys. Native Unknown is permanent in this protocol:
its exact evidence is frozen, new work remains blocked, and later fabricated
Applied evidence cannot promote it. Stored Unknown can be inspected locally;
repeated queries are not presented as a repair mechanism.

Cancellation before dispatch retires local intent without contacting DFHack.
This is not cancellation of other holders of native credentials. After a dispatch
marker, cancellation is synchronized before asking the native endpoint to retire
an unused Prepared token. It never undoes Applied membership or repairs Unknown.
Cancelled native receipts are retained as terminal evidence. Repeated cancellation
can use the slot reserved for the eventual terminal receipt without reassigning.

Only one unsettled key is allowed, enforced on append and replay. Terminal and
locally cancelled records are immutable. Replayed state pairs cannot bypass the
dispatch marker, remove prior evidence, switch plans or introduce a concurrent
intent. Source changes cannot reuse this journal for new control.

## Modes and custody

Modes are immutable: Control permits preparation/commit/cancellation; Recover
permits existing-evidence queries and receipt retention under Query only; Offline
opens existing storage read-only and makes no native calls or writes. Every
journal instance is scoped to its current session, not the session that created
its file. A context from another session or fortress is refused.

File custody uses exact-mode 0700 directories and single-link regular 0600 files.
Every directory open walks from a pinned root descriptor through Linux proc-fd
paths with O_DIRECTORY/O_NOFOLLOW. Final opens are no-follow and nonblocking so a
FIFO substitution cannot hang before validation. The final directory descriptor
stays owned. Named parent, pinned parent, file descriptor, inode, owner, mode,
link count, length and exact retained bytes are rechecked. Paths are normalized
absolute operator configuration. Exclusive nonblocking locks last for the open
journal, including read-only offline custody. Files are never overwritten.

A kernel-random 32-byte nonce distinguishes journal identities. Newly created
filenames are directory-synchronized before initialization; header/file sync
precedes native preparation. Writable reopen validates and resynchronizes complete
frames surviving an earlier uncertain sync. A failed append fences the instance.
Torn/corrupt frames are refused unchanged: no truncation, pruning, repair, rotation
or replay-protection eviction. Trusted-owner rollback/rehashing and actual host
power-loss behavior are not certified by this format.

## Binary format and bounds

All integers are big-endian. Header: DFMWJ001, binding length u16, binding,
nonce (32), checksum (32). Binding contains length-prefixed endpoint, generation
u64, length-prefixed DF/DFHack versions and folder, and site u32. Header identity
is SHA-256 of `dfmcp-workforce-journal/1` + NUL + all prior header bytes.

Frame: DFMWFR01, body length u32, sequence u64, previous checksum (32), body,
checksum (32), DFMWEND1. Checksum covers the prefix/body under domain
`dfmcp-workforce-frame/1` + NUL. Body: coordinator state u8, plan length u32,
sealed plan, native-effect length u32, optional effect. Sealed plan is key field,
detail u32, assigned u8, capture length u32 and complete canonical capture.

States: Intent=0, Prepared=1, DispatchStarted=2, Tracking=3, Terminal=4,
CancelRequested=5, CancelledBeforeDispatch=6. They are not native phase codes.
Limits: 64 keys, 512 events, 64 MiB/file, 65,675 bytes/plan, 73,876 bytes/body,
73,968 bytes/frame. Ordinary transitions reserve future cancellation/terminal
slots and bytes. Permanent native Unknown may consume a final evidence slot but
remains unresolved; it is never mislabeled a settled assignment.

Operations share a cooperative deadline and byte allowance across complete-byte
verification, native calls, staging and publication. Full roots publish only
after write/flush/sync. Socket deadlines do not renew; filesystem calls are not
hard real-time. Large journals require proportionately larger work allowances.

## Evidence

Twelve new Rust journal/custody regression groups are registered, totaling 23
with the typed/RPC increment. They cover durable markers at injected setters,
lost replies, sync failures, crash-before-send recovery, mode/authority fences,
local/native cancellation, permanent Unknown, capacity and real private-file
locking/modes. **All are uncompiled and unexecuted here; Rust tooling is absent.**

Six independent Python binary/state groups pass, including an independently
encoded 1,803-byte fixture, every single-byte corruption, all incomplete prefixes
(with complete frame boundaries correctly accepted), seven illegal rehashed
histories and cancellation-space arithmetic. Run:

```bash
python3 scripts/check_workforce_journal_reference.py
```

These are reference checks, not Rust, actual filesystem custody, MCP, native
DFHack, live-game, power-loss or full repository qualification.
