# Lossless delta storage for spatial/1.8 observation history

The spatial/1.8 observation journal now stores a changed native payload as a
lossless delta when that saves at least 128 bytes. This addresses the byte-capacity
bottleneck of writing the entire citizen, item, job and terrain capture again when
only a few fields or the game tick changed. Every observation remains retained:
there is no pruning, downsampling, archive replacement or invented continuity.

This is unadmitted development **source**, not Rust-qualified functionality.
No Rust compiler, Cargo or rustfmt is available in the editing environment.
The registered Rust codec and journal tests have not been compiled or executed.
The independent Python evidence below does not qualify the Rust implementation.

## Runtime integration and compatibility

The existing `SpatialCitizenJournal` alias enables delta storage through its sealed
`Spatial18` profile. Authenticated live bootstrap/refresh and offline/historical
replay already use that journal, so no separate compressor, daemon, new tool,
operator path, native bridge method, credential or dependency is introduced.
The native observation format and canonical world/source digest algorithms are
unchanged. Operations/1.3, operations/1.4 and spatial/1.6 remain raw-only.

Existing spatial/1.8 archives open without rewriting a byte. Their old file header,
incarnation, raw records, record numbers, anchors and digest identities are retained.
New records can mix the legacy `DFMOREC1` framing with the new `DFMODLT1` storage
marker. Each new record still receives its own content digest over its actual
stored bytes. Old records are not recompressed or assigned replacement digests.

**Downgrade warning:** a reader predating delta support refuses a complete
`DFMODLT1` record, including under its incomplete-tail repair option. It cannot
reopen a journal after a new writer appends such a record. This is an additive
record encoding, not an in-place archive migration or backwards readability of
new records by old binaries. Other profile readers reject the marker as well.
Do not prune a paired observation/watch archive as a workaround.

First records, observation-epoch resets, and records numbered 1, 65, 129, ... are
raw. Incompressible, small or overfragmented payloads also use the raw format.
At most 63 consecutive records depend on earlier payload deltas. Keyframes are
not independent generation-map checkpoints: replay still walks the required prefix
and does not skip observations or promise constant-time random access.

## Encoding

The outer file header, frame-header checksum, record checksum and commit footer
retain their existing layout and hash domains. The frame marker participates in
both checksums. All semantic body metadata remains byte-for-byte in its original
position: record number, previous record digest, full anchor, native source digest,
bridge generation, DF and DFHack version strings. The final length-delimited body
field contains either the complete native payload or this bytecode:

| Field | Encoding |
|---|---|
| Magic | 8 bytes, `DFMDLT01` |
| Required predecessor payload length | Big-endian unsigned 32-bit integer |
| Expanded target length | Big-endian unsigned 32-bit integer |
| Command count | Big-endian unsigned 32-bit integer |
| Literal command | Tag 0, unsigned 32-bit length, literal bytes |
| Copy command | Tag 1, unsigned 32-bit base offset, unsigned 32-bit length |

Commands concatenate the target. Copies refer only to the **immediately preceding
verified payload**, never earlier output in the same command stream. There is no
recursive decompression, mutable dictionary, native pointer or arbitrary file read.
All lengths are positive; output length must match exactly and trailing data is
rejected. Copy ranges, command count and every input/output extent are validated
in a complete read-only pass before allocation of the reconstructed output.

The deterministic encoder takes aligned unchanged runs of at least 32 bytes and a
possibly shifted common suffix. This handles sparse edits and many insertion/deletion
cases without an unbounded search. It is not an optimal general-purpose compressor.
It returns raw fallback instead of a partially encoded delta when the saving or
command bound cannot be met. The byte codec is public as
`dfmcp_world::journal_delta`; it has no I/O, clock, randomness or dependency outside
`std`. It is not an integrity mechanism on its own.

## Exactness and durability

Before reconstruction, the journal verifies the stored frame's checksums and
footer. A delta then requires an existing verified predecessor with the exact
record digest, fortress and epoch, and an earlier observation sequence. It cannot
be used for a required raw keyframe. Reconstructed bytes pass through the same
sealed native decoder as raw bytes. Both restart and historical replay recompute
the native source digest and canonical anchor at every retained record, including
entity retirement/reappearance and epoch transitions.

Append first compiles the candidate canonical state, chooses the storage form,
decodes its prospective frame, and rechecks the reconstructed source identity.
Only then can it write and sync. The candidate payload base, accounting, entries,
head and canonical state are published together after the existing durability
boundary. Failed writes or uncertain sync leave all those in-memory values at
their prior versions and fence further appends. A complete frame surviving an
uncertain sync may be recovered on reopen; no error proves that nothing reached
disk. Partial tails require the existing explicit repair permission. Complete
corruption, malformed deltas, wrong predecessors and nonreproducible observations
are not tail repairs.

Historical queries use their own sequential reconstruction base and never replace
the current observation or sample current watches. Paired watch journals retain
the same observation incarnation and exact anchor universe. Current authority,
private-file custody and byte/entity/time checks remain required. Compressed size
does not stand in for the expanded acquisition allowance.

## Bounds and accounting

The expanded payload limit remains the native profile's limit, at most 16 MiB,
and may be narrowed by the caller's acquisition budget **before reconstruction**.
A delta has at most 32,768 commands and must save at least 128 payload bytes.
The writer retains at most one extra full payload as its next compression base;
this reduces disk storage, not the canonical projection's RAM footprint.

The existing journal retention limits remain unchanged: default 64 MiB/1,024
records; implementation ceilings 256 MiB/4,096 records. Compression extends useful
retention only when byte capacity is the limiting factor. Record exhaustion,
cooperative replay deadlines and poor compression still refuse further work.
No automatic rotation, record pruning or durable generation-map checkpoint is
implemented. Synchronous storage calls retain their existing cooperative deadline
model, not a new hard-cancellation guarantee.

`ObservationJournal::storage_stats()` exposes raw/delta record counts and exact
expanded/stored payload totals without another disk scan. It is metadata about
already accepted records, not a fresh integrity check. Framing overhead remains
included in `retained_bytes()` and in each history row's existing `encoded_bytes`.
No extra metadata is injected into the size-constrained MCP responses.

## Evidence

Eight codec test groups and ten actual journal test groups are registered. They
cover fixed vectors, deterministic byte edits, maximum-size input, legacy raw
prefixes, mixed reopen/historical replay, periodic keyframes, identity churn,
expanded budgets, current authority, record capacity, every cut/corrupt byte of a
fixture delta, checksummed wrong metadata, and write/sync failure recovery.
These **18 Rust groups remain unexecuted** in this environment.

Executed independent references:

```bash
python scripts/test_journal_delta_reference.py
python scripts/test_observation_delta_frames.py
```

The byte-codec reference passed 15,625 checks: 3,800 compressed roundtrips and
348 raw fallbacks, plus malformed/truncated/range cases. Its fixed delta SHA-256
is `da05cd3ec21fe505b4502f56f2eb4eafb806844308f953ba215e1de483551d04`.
The Rust tests pin the exact same bytes, but have not executed that comparison.

The framing reference passed 1,187 checks. In a declared **synthetic** trace of
256 sparse-change 256-KiB payloads, raw framing occupied 67,171,408 bytes while
four raw keyframes plus 252 deltas occupied 1,129,523 bytes (98.3184% less). This
illustrates the targeted capacity failure, not a measured fortress workload,
CPU benchmark, real canonical-state replay, or storage-durability test.

The framing reference deliberately uses opaque synthetic payload/anchor identities.
It does not implement DFHack normalization. Neither reference executes the Rust
encoder, journal state machine, filesystem custody, MCP runtime or a game. No
Rust/Clippy/stdio, power-loss, native/live, registry, production-admission or
whole-repository qualification is established. The existing open stdio-lifecycle
bead and the production runner map are unchanged.
