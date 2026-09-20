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


## Integrated MCP history and offline workflow

`dfmcp-live-work-order-progress-dev-server` now accepts optional operator
`DFMCP_WORK_ORDER_PROGRESS_JOURNAL`. Existing live use without an archive remains
available. The path never appears in a tool argument. Live opening/refresh reserve
retention and complete output before acquisition, then append and sync the complete
observation before publishing it. Lost acknowledgements leave exact records
available for inspection; no record is deleted to hide a failed response.

To inspect an existing archive without DFHack, keep the exact 1.12 development
opt-in and operator fortress ID, configure JOURNAL, and call:

```json
{"recovery_only":true}
```

Do not send `native_order_ids` in this mode. An existing nonempty archive is
required. The branch executes before endpoint/credential reads and has no source
object. Query is the only granted capability; even a later injected Observe grant
cannot turn offline mode into a live session or permit archive append. Empty,
corrupt, foreign-fortress and custody-invalid archives refuse bootstrap. The
latest exact record supplies historical orientation, never a current-game claim.

In either archive-backed live mode or offline mode, `fortress.query` accepts a
`history` STRING containing one of these JSON objects. The examples show the
inner JSON; encode it as a string in the tool's `history` argument and include the
session ID separately. `expected_witness` and `history` are mutually exclusive.

```json
{"mode":"list","limit":8}
```

This returns complete metadata entries, the archive ID/head, total retained count
and an optional continuation. Follow it with the same page size:

```json
{"mode":"list","limit":8,"continuation":"<returned continuation>"}
```

Limits are 1..64 whole metadata rows. The 64 retained issued cursors bind session,
archive ID, exact head and page size; replay of the same page returns the same
cursor while retained. Unknown, evicted, restarted-session or changed-head tokens
require restarting discovery. A token is a scoped handle, not authority or a
client-selected file offset. Exact entry references do not depend on those cursors.

```json
{"mode":"record","archive_id":"<archive ID>","number":1,"record_digest":"<entry digest>"}
```

This re-reads the selected frame, verifies it against retained identity and returns
its full source manifest and native observation. The Agent Turn anchor is that
historical capture. The live session's current selection, witness and authority
clock do not change. A same-length rewrite of the requested frame is refused.

```json
{"mode":"changes","archive_id":"<archive ID>","before_number":1,"before_digest":"<first digest>","after_number":2,"after_digest":"<second digest>"}
```

Both exact records must be ordered and in the SAME archive segment. The response
returns the before reference, full after record and bounded endpoint comparison.
The two reads share a narrowing wall allowance and reserve two complete frames.
An intervening restart or discontinuity cannot be bypassed by choosing apparently
compatible endpoints. Neither history queries nor current queries reconnect a
failed source, sample a monitor, mutate a creation journal or dispatch game work.

Requests are at most 2,048 UTF-8 bytes with a closed tagged field grammar. Unknown
fields/modes, invalid/duplicate keys, noncanonical digests, invalid limits and
reversed/identical endpoints fail. Normal history pages carry complete-set flags;
metadata discovery does not reread every unrequested payload. All exact-record
and comparison responses verify their requested payload bytes again.

Session closure releases the source and archive while holding the runtime slot.
It remains available after source failure or operator revocation and never
cancels orders or erases historical evidence. Healthy history remains readable
after live source failure. Corrupt archive custody suppresses even already-built
historical packets before publication. Final output failure or acknowledgement
expiry cannot publish a new session.

### Runtime limits and executed evidence

Archive-backed startup defaults to 68 MiB byte work, sufficient for the 64 MiB
retained maximum plus bounded negotiation, capture, archive frame and output work.
Unarchived sessions retain their 2 MiB default. Maxima are 60 seconds, 68 MiB and
131,072 four-byte output-proxy units. Historical responses reserve 147,456 bytes
before work; this covers a 16 KiB envelope plus 32 complete 4 KiB row allowances.
An insufficient request returns a refusal rather than partial record JSON.

Eleven additional Rust tests exercise the actual generic history dispatcher and
shared runtime offline/render/publication helpers, including Unix private files.
They are uncompiled and unexecuted. The separate Python request/cursor/size model
passed 134 positive and 32 negative request cases, 7,263 pagination cases, seven
cursor rejection cases and two segment-comparison controls. Conservative modeled
metadata, exact-record and comparison responses measured 66,980, 89,921 and
98,848 bytes, respectively, including the full 16 KiB envelope allowance.
Those are model sizes, not measured Rust serialization. The scoped report is
`docs/evidence/progress-history-mcp-reference.json`.
