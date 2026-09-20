# Durable bounded-run coordinator

`dfmcp_adapter::bounded_run::journal` connects the typed run/1.13 source to an
append-only effect journal. `private_file::open_private_run_journal` supplies
operator-selected, exclusively locked Linux x86_64/aarch64 file custody. It adds
no bridge generation, dependency, command execution or production admission.

## Dispatch and recovery

The journal binds one exact loopback endpoint, source generation and software
pair. A named-fortress capability cannot substitute for this source identity.
Native clients report their actual connect endpoint; an unbound injected byte
stream is not sufficient to satisfy a journal binding.

Preparation revalidates the complete selected paused observation, then syncs
sealed intent before native PrepareRun. Native evidence is validated and synced
before being acknowledged. Duplicate preparation returns the retained result
without a native call or lifetime renewal. Pending intent can be reconciled by
query, including when the preparation reply was lost.

Commit reads the exact native witness again, checks current clock authority and
the complete run horizon, then syncs DispatchStarted **before** calling CommitRun.
That marker is never dispatchable again, even after a crash before the RPC was
actually sent. A failed reply, receipt validation, post-dispatch deadline or sync
therefore leaves reconciliation required rather than making unpause retryable.

Reconciliation makes one QueryRun call and persists validated native evidence.
A missing record is unknown, not non-application. A prepared native record seen
after DispatchStarted remains Tracking and cannot regain Prepared eligibility.
Running/stopping receipts cannot regress into a fresh preparation. Terminal
records are immutable. SourceLost is retained explicitly without a pause proof,
and continues to block new control in this journal.

Cancellation before local dispatch permanently retires the local intent without
a native call or a claim that other controllers never acted. Cancellation after
dispatch syncs CancelRequested first, then requests the native safety pause.
Only cancellation can repeat that safety request; it never repeats unpause.
Native pause readback is historical, not proof that the game remains paused.

## Custody and limits

Modes are fixed at open: Control can prepare/commit/cancel, Recover can only
query and persist existing evidence, Offline uses a read-only descriptor and
cannot contact the bridge or change the journal. Recovered bytes never restore
grants. Every operation checks its current OperationContext and owning session.

The file uses an exact-mode 0700 parent and a single-link 0600 regular file.
Paths are normalized absolute operator configuration; links/special files are
refused. Linux no-follow/nonblocking opens prevent final-link and FIFO races.
Parent directory sync precedes native preparation. Repeated inode, extent,
parent and exact-byte checks detect custody changes and same-length corruption.
The Linux /proc/self ownership check is conservative for non-dumpable processes.
Other platforms fail closed rather than guessing their open-flag values.

Limits are 256 retained intents, 4,096 transitions and 2 MiB. Frames carry a
sequence, previous digest, full sealed record, checksum and complete footer.
The header binds endpoint/software/generation and a session/request-derived
journal identity. Partial or corrupt frames are refused unchanged: no automatic
truncation, repair, rotation, compaction or idempotency eviction exists.

Each operation shares a cooperative wall-time and byte allowance across journal
verification, native calls and publication. Capacity is reserved for active-work
cancellation/terminal receipts; monitoring cannot consume those final slots.
Staged roots publish only after write/flush/sync. Failed storage publication
fences the journal. A complete frame from an uncertain sync can be revalidated
and synced during writable recovery; offline parsing does not certify power-loss
resilience. No filesystem call is a hard real-time guarantee.

## Format and evidence

Header: `DFMRJ001`, binding length u16, binding bytes, session/request salts
(16 bytes each), SHA-256 over the preceding bytes with domain
`dfmcp-run-journal/1` plus NUL. Binding contains length-prefixed endpoint text,
generation u64 and two length-prefixed software version strings.

Frame: `DFMRF001`, body length u32, sequence u64, previous digest (32), body,
SHA-256 with domain `dfmcp-run-frame/1` plus NUL over the frame prefix/body,
`DFMREND1`. Body is state u8, sealed-plan length u16 + bytes, native-record
length u16 + bytes (zero means absent). Integers are big-endian. Maximum body is
452 bytes; fixed frame overhead is 92 bytes.

States: Intent=0, Prepared=1, DispatchStarted=2, Tracking=3, Terminal=4,
CancelRequested=5, CancelledBeforeDispatch=6. These are coordinator states, not
native phase codes. Full transitions are enforced on both append and replay.

Nine coordinator Rust regression groups are registered (22 together with the
codec/RPC increment). They remain **uncompiled and unexecuted**: Rust, Cargo and
rustfmt are unavailable. Seven independent Python reference groups executed and
passed: canonical fixture hashes, every-byte frame corruption, incomplete
prefixes, forks/gaps/reordering, all state pairs, legal paths with at most one
dispatch marker, and cancellation-space boundaries. Run with:

```bash
python3 scripts/test_bounded_run_journal_reference.py
```

That reference does not execute Rust, real filesystem custody, MCP, DFHack or a
live game. No full qualification, admission, or live safety claim is made.
