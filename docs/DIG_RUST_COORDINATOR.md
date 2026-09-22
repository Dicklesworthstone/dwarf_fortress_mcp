# Durable Rust mining coordinator — dig/1.16

`dfmcp_adapter::dig_designation::journal` composes the existing typed dig codec
and fixed native RPC trait with an append-only coordinator. It is not a Python
wrapper, an MCP server, a new native protocol or production admission.

## Source and authority

A journal binds its numeric loopback endpoint, native generation/version manifest,
world folder/site, and an operator-selected map cuboid. Multiple target rectangles
may be coordinated within that scope. Both each full observation halo and every
shared map block touched by scheduling must fit inside the declared scope.
Discovery requires current Query authority over that whole scope; native work
also uses the existing per-plan Query/Observe/Plan/Designate authorization checks.
Expiry checks do not roll back to a historical tick. Limited-use grants are not
cloned into reusable authority. A journal is bound to the current session on open.

Every native edge requires a caller-supplied `DigGuard`. The supervising runtime
must implement cancellation, I/O permission, operator enablement, exact fortress
selection, and the applicable live lease/checkpoint policy there. The coordinator
supplies no permissive implementation and repeats the guard after dispatch-state
synchronization, immediately before native work. It repeats outcome-stage guard
checks before retaining returned evidence. The guard is a trusted Rust boundary,
not a boolean or certificate restored from a client request or journal.

**This increment does not implement the runtime's live lease/checkpoint policy or
an MCP route.** The low-level source must be exclusively supervised by its caller;
other directories, journals, plugins or UI input are not globally fenced. Runtime
ownership, policy integration and end-to-end qualification remain necessary.

## Durable workflow

1. `prepare` reobserves the exact plan, writes and syncs `Intent`, then calls native
   preparation once. Only a fresh response creates `Prepared` plus a process-local
   dispatch permit. An existing key is never reprepared; conflicting content fails.
2. `commit` requires the exact confirmed digest, current grants, a still-live local
   permit, and another exact observation. It consumes the permit, syncs
   `DispatchStarted`, then repeats custody/source/runtime checks before the sole
   native commit. The native RPC's same-connection permit remains required too.
3. Complete native terminal proof is decoded again against the retained plan and
   synced as `Terminal` before acknowledgement. Native success still means only
   historical designation configuration, not excavation completion/current safety.
4. `reconcile` issues at most one QueryDesignation call. Missing native retention
   preserves uncertainty. Queried/replayed Prepared evidence becomes `Tracking`,
   not dispatch permission. Native Unknown is permanent and blocks new work;
   repeated reconciliation/cancellation does not poll it into a fabricated success.
5. `cancel` syncs `CancelRequested` and retires only the exact native preparation.
   It cannot undo designations, excavate, advance time, or declare local uncertainty
   resolved. Lost cancellation replies can be recovered by query/cancel, never commit.

Reopen never restores a dispatch permit, even in Control mode and even if the
last complete frame is Prepared. Recover permits reconciliation and separately
authorized native cancellation, not prepare/commit. Offline permits verified
historical get/list only and performs no sync or native work. Terminal lookups
and permanent-Unknown lookups do not need a functioning native source.

Every nonterminal record blocks new keys throughout the journal, including a
new rectangle within its scope. Prior terminal evidence is immutable. New plans
cannot regress the retained native tick/intervention sequence. There is no reset,
automatic compaction, migration from Python capsules, tail repair or replay path.

## Framing, custody and bounds

`architecture/dig_journal_v1.json` specifies the canonical bytes and reference
vectors. The header is magic DFMDJ001, u16 binding length, binding bytes, nonzero
32-byte journal nonce and SHA-256 domain `dfmcp-dig-journal/1`. Binding fields are
u16-prefixed endpoint/version/folder text, big-endian generation/site, and six
big-endian signed coordinates (min x/y/z, max x/y/z).

Each frame is DFMDJR01, u32 body length, u64 sequence, previous 32-byte digest,
body, 32-byte frame digest (domain `dfmcp-dig-journal-frame/1`) and DFMDJEND.
Hash domains are followed by one NUL byte. Body is state u8, u32 retained-plan
length and full DFMDGP16 plan, then u32 native-effect length and complete effect
(or zero length). The state/phase matrix and predecessor transition must both
validate; a checksum is not a substitute for legal history or native proof.

Limits: 128 keys, 896 frames, 16 MiB journal, 940-byte binding, 16,870-byte body,
16,962-byte frame, and 1..8 complete summary rows per page. Capacity reserves
worst-case recovery frames before new intent admission. Online operations require
an up-front work allowance of `30 * (current_journal_bytes + 16962) + 3 * 307200`
bytes, covering repeated verification, copies, appends and RPC reservations.
The shared cooperative wall allowance is at most 60 seconds. Blocking filesystem
operations do not acquire a false hard-deadline or cancellation guarantee.

Storage uses the existing `EffectJournalStorage` trait: implementers must supply
exclusive append custody, synchronization and identity verification. Complete
bytes and extent are rechecked before native/storage edges and after publication.
Failed writes/syncs or changed bytes fence the live journal. An incomplete frame
is refused unchanged. A complete frame surviving an uncertain sync can be replayed
and resynchronized by online reopen, without restoring dispatch authority.

Hash chains are integrity commitments, not signatures, a global lease, or an
external anti-rollback anchor. Restoring/replacing all retained history requires
operator custody outside this module. Missing evidence never proves no effect.

## Discovery

`list` returns key, sealed plan digest, coordinator/native phase, optional receipt,
process-local dispatchability, total and unsettled counts. Its opaque continuation
binds session, journal identity, exact head and page width. `get` returns the full
review/evidence for an exact key/digest. Neither operation needs transcript memory
or bridge credentials, and stored records grant no new authority.

## Evidence available for this increment

Sixteen Rust coordinator test groups are registered against the real journal,
codec and core authority types, with injected storage/native interfaces. They
exercise sync-before-dispatch, lost replies, reopen, native cancellation, cross-key
and cross-region coordination, absorbing Unknown, corruption, mode/authority/
budget denial, runtime revocation during dispatch sync, and cursor invalidation.
**They are uncompiled and unexecuted: Rust/Cargo/rustfmt are unavailable here.**

Run the targeted Rust tests on the pinned toolchain with locked dependencies:

```sh
cargo test --locked --offline -p dfmcp-adapter dig_designation
```

The independently executed Python reference is:

```sh
python3 scripts/check_dig_journal_reference.py
```

It rejects 7,752 single-byte corruptions and 7,748 incomplete prefixes, accepts
four complete crash-prefix boundaries, checks all 100 state/phase transition
pairs, and computes a maximum six-frame per-key path. Its four frame hashes and
header identity are fixed in the contract and actual Rust test assertions. The
four original native fixtures were reconstructed with exact Git blob equality.
This is reference/framing evidence, **not execution of Rust, its filesystem or
network I/O, the real DFHack SDK, a live fortress, or the qualification ladder**.

## Concrete Linux private-file backing

`journal::private_file::open_private_dig(path, context, mode, expected)` now opens
the real backing file for `DigJournal`. The path and expected binding belong to
the trusted operator/runtime, not client tool arguments. There is no native I/O,
capability grant, Python-capsule migration or inferred production admission.

On Linux x86_64/aarch64, each path component is opened no-follow through a pinned
directory descriptor. The final file must be regular, single-link, exact 0600,
and owned by the same root/effective-user owner as its exact-0700 parent. An
exclusive nonblocking file lock is held through replay and subsequent operations.
Named and opened inode/owner identities, parent identity, permissions and extent
are rechecked at storage boundaries. Writes require both append mode and the
expected end position. Every synchronization performs file sync followed by
parent-directory sync and then repeats custody checks. No truncation is supported.

Missing files may be created exclusively only in Control mode with an exact
binding. A new nonzero journal nonce comes from `/dev/urandom`; header and directory
entry use the same checked synchronization path as later frames. Existing empty,
incomplete, corrupt or mismatched journals are not initialized, rewritten or
repaired. Wrong expected bindings fail before online resynchronization. Recovery
and Control reopen never restore a dispatch permit.

Offline opens retain the lock but use an O_RDONLY file. Write, flush, sync and
truncate all explicitly refuse; ordinary historical get/list does not rewrite
or synchronize the file. Missing recovery files do not create directories or a
new journal. Unsupported platforms return a refusal before opening any path.
The open/replay path shares one cooperative deadline; kernel filesystem calls
are not falsely described as forcibly interruptible.

Sixteen additional Linux storage/runner tests are registered: fifteen substantive
tests and one child-process lock probe. They cover real-file coordinator lifecycle,
lost-reply recovery, non-restored preparation permission, offline no-write behavior,
exclusive locks across processes, independent file/parent sync failures, path/mode/
link rejection, inode/parent substitution, corrupt/torn history and current scope.
The child probe is owned and kill/reaped on failure or timeout. **None has been
compiled or executed here.** Together with the sixteen coordinator groups, this
increment registers 32 new Rust tests; the earlier 22 codec/RPC tests also remain
unexecuted in this environment. The passing Python reference is still only an
independent framing/state model, not Rust or actual storage execution.
