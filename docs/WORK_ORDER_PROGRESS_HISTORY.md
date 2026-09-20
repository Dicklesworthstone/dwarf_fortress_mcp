# Restart-safe progress/1.12 history

The progress archive retains complete batched native observations and their exact
DF/DFHack manifests. It is not a creation journal, a canonical world snapshot,
a game checkpoint, or evidence that goods were produced. Native protocols,
creation journals, dependencies, admission and the production runner map are unchanged.

## Durable observation boundary

`ProgressArchive` is an independently framed progress/1.12 archive. Live append
requires current Query and Observe authority for its fortress. Offline mode is
fixed on opening and refuses append even with a later injected Observe grant.
Cached access checks file identity and extent; exact-record reads re-read and
verify the full frame against the retained index before returning decoded data.
Grant expiry is evaluated at no earlier than the largest retained game tick.
Historical observations do not revive a grant expired at a later observed tick.

`ProgressSession::refresh_with_publication` validates the complete native read,
source identity, authorization at the observation tick and the comparison before
calling a caller-supplied publication barrier. The barrier must succeed before
replacing current in-memory evidence. Read, barrier, custody, sync or acknowledgement
failure clears the selection and fences the native source. An uncertain complete
append can recover after reopening; an incomplete frame is refused unchanged.
No game effect is reachable from this read-only path.

The archive has a 64 MiB / 4,096-record retention bound, 18 KiB payload bound and
at most 64 complete metadata entries per library page. Capacity must be reserved
before acquisition. Replay charges retained bytes; record reads and comparison
reserve bounded complete frame work. Filesystem operations have cooperative
elapsed checks, not forcibly interruptible fsync deadlines. There is no eviction,
compaction, truncation, automatic rotation or tail repair.

## Comparison segments

Every live reopening starts a new segment at its first appended capture. This
is true even when the plugin incarnation and observed order IDs are unchanged.
There is no claim of observations or stable monitoring during process downtime.
Within an opening, selection changes, source/software changes, incarnation
changes, clock regression and allocation-horizon regression partition history.
Sequence replay/reordering inside an unchanged source segment is refused.

Record references are archive identity plus record number and digest; they remain
usable after restart. Historical comparison requires two ordered, distinct exact
records in the SAME segment, so even an intervening reset cannot be hidden by
apparently compatible endpoints. The existing endpoint comparator still keeps
counter changes, changed configuration, disappearance and unlinked reappearance
separate. Missing/zero/decreasing counters do not prove goods production or
resolve ambiguous creation effects. These are endpoint observations, never a
complete game-event history.

## Custody and format

The operator selects a normalized absolute path beneath a real exact-mode 0700
directory. Existing files must be single-link exact-mode 0600 regular files with
the parent's owner and unchanged named/opened inode identity. A nonblocking
exclusive lock lasts for the owner lifetime. Offline uses a read-only descriptor
and requires an existing file. No request can choose or repair a path. This does
not protect against a malicious same-user rewrite of unqueried cached bytes or
provide an external anti-rollback floor. Removing a valid suffix before reopening
cannot be detected without such an external floor.

The 80-byte header is DFMWPA12, fortress u64, archive ID (32 bytes), and a SHA-256
header checksum with domain `dfmcp-progress-archive-header/1` plus NUL. Frames are
DFMWPR12, body length u32, consecutive record number u64, predecessor digest,
body, frame digest, and DFMWPE12. The frame hash covers the domain
`dfmcp-progress-archive-frame/1` plus NUL, archive ID, exact prefix and body.
The body contains segment u64, native generation u64, two u16-length software
strings, u8 selected-ID count and its sorted u32 IDs, then u32 observation length
and complete DFMWP012 bytes. All integers are big-endian. Replay checks full
native evidence, fortress, manifest, ordering and segment transitions; checksums
are integrity checks, not signatures or game truth.

## Evidence

Thirteen new Rust groups are registered: eleven archive/custody cases and two
session publication-barrier cases. They cover restart segments, full-record
replay, exact references, authority expiry, immutable offline mode, sync failure,
torn/corrupt/rehashed histories, capacity, pagination and real private-file
lock/permission/replacement behavior. **They are uncompiled and unexecuted:**
Rust, Cargo and rustfmt are unavailable. In particular, no filesystem durability
or actual MCP execution is claimed.

`python scripts/check_progress_archive_reference.py` independently validates a
604-byte two-record archive, rejects 604 byte corruptions, 602 incomplete prefixes
and 11 illegal rehashed histories, and checks 120 valid segment/counter cases.
This is a Python model, not execution of the Rust parser or filesystem operations.
The scoped result and source hashes are in `docs/evidence/progress-archive-reference.json`.
