# Durable fortress-bound conditional running in Rust

`order_run::journal` connects the typed native order-run/1.14 client to a
restart-safe Rust coordinator. It binds one actual loopback endpoint, exact
folder/site identity, bridge incarnation and software pair. This is a new binary
format, **not** the Python developer client's JSONL journal and not run/1.13.
Existing formats are neither migrated nor silently reinterpreted.

## Execution, uncertainty and recovery

The fixed modes are Control, Recover and Offline. Control requires current
named-fortress Query/Plan/guarded ControlClock authority and the full requested
game-tick horizon. Recover has no control edge; it can query and retain native
evidence in an existing writable journal. Offline opens existing read-only
custody and cannot acquire observations, prepare, commit, cancel or write.
Replayed bytes never restore grants or promote a mode.

Preparation re-observes the exact paused fortress/order, synchronizes sealed
intent before PrepareRun and synchronizes its native receipt before return.
Duplicate keys preserve their complete original intent and do not renew native
preparation. A lost preparation reply can be reconciled by one native query.

Commit re-observes and rechecks authority, then synchronizes DispatchStarted
before the sole CommitRun attempt. After that marker no crash, timeout, malformed
reply, missing native record or uncertain write can restore dispatch eligibility.
A crash before the actual request is sent is conservatively unresolved too.
A native Prepared receipt after DispatchStarted becomes Tracking, not Prepared.

Receipts retain separate predicate samples, pause readback and stop reasons;
none claims goods were produced or the game is currently paused. Terminal
native evidence is immutable. Once stopping begins, sampled evidence is frozen;
a later source-loss indication cannot manufacture a new goal observation.
Current Query authority is rechecked at newly observed receipt ticks.

Cancellation before local dispatch is durable and makes no native call. After
dispatch it synchronizes CancelRequested before requesting the native safety
pause. Only this safety operation may repeat. A repeated cancellation can consume
the reserved final terminal slot. SourceLost remains unresolved and blocks new
control in this journal even though its native record is terminal.

One unsettled operation blocks new keys. Query never treats absence as verified
non-application and never deletes earlier evidence. The native owner continues
its bounded stop independently of this journal/connection's lifetime; neither
journal close nor client process death cancels or proves that stop.

## Custody and resource bounds

Private custody supports Linux x86_64/aarch64, with an exact-mode 0700 parent,
single-link 0600 regular file, no-follow/nonblocking final opens, exclusive lock,
and repeated inode/parent/extent/exact-byte checks. Directory synchronization
precedes native preparation; writable reopen synchronizes verified bytes.
Other platforms refuse rather than assume equivalent flags. The /proc/self
ownership check is deliberately conservative for non-dumpable processes.

The journal holds at most 256 intents, 4,096 transitions and 2 MiB. Complete frames
include sequence, previous hash, full intent/receipt, checksum and footer. Replay
checks legal transitions and source identity, not merely hashes. There is no
truncation, tail repair, deletion, eviction or automatic rollover.

Publication stages both retained bytes and the entry map before write/flush/sync,
then publishes the new root. A failed write/sync fences the open coordinator.
Complete bytes from an uncertain sync can be revalidated and synchronized on
writable reopen. Corrupt/incomplete frames are refused unchanged. This source
behavior is not a host power-loss qualification.

Ordinary publication reserves two final frame/byte slots, cancellation reserves
one, and terminal evidence may use the last. Each operation shares a narrowing
cooperative deadline/byte allowance across verification, native I/O and retained
state publication. Filesystem calls are not hard real-time operations.

## Binary format and evidence

All integers are big-endian, and a field is u16 length followed by bytes.
Header: DFMOJ014, field(binding), 16-byte session salt, 16-byte request salt,
SHA-256 of the preceding bytes under domain dfmcp-order-run-rust-journal/1 + NUL.
Binding: field(endpoint), generation u64, field(DF version), field(DFHack version),
field(folder), site u32.

Frame: DFMOF014, body length u32, sequence u64, previous hash32, body,
SHA-256 under dfmcp-order-run-rust-frame/1 + NUL, DFMOEND1. Body: state u8,
field(keyed sealed intent), field(native receipt or empty). Maximum body/frame
sizes are 2,166/2,258 bytes. States are Intent=0, Prepared=1, DispatchStarted=2,
Tracking=3, Terminal=4, CancelRequested=5, CancelledBeforeDispatch=6.

Fourteen Rust journal/private-file regression groups are registered, including
positive lifecycle, source/authority/mode refusal, synchronization ordering at
mock native setters, lost replies, restart without redispatch, torn/corrupt bytes,
cancellation capacity, exact independent fixture and private-file locking/modes.
Together with the adapter there are 27 Rust groups. **Rust/Cargo/rustfmt are
unavailable; none has been compiled or executed here.**

Six independent Python framing/state reference groups execute with:

```bash
python3 scripts/check_order_run_journal_reference.py
```

They reconstruct the 1,704-byte four-transition fixture, reject every single-byte
corruption and every incomplete prefix, check rehashed illegal paths and dispatch
counts, and exercise retention arithmetic. They do not execute Rust, real file
custody, native RPC, MCP, or live-game behavior. No qualification or admission is
claimed. The next integration is the existing eleven-tool MCP development path.
