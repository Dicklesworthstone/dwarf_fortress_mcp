# Furniture placement client and durable custody

The furniture/1.19 developer client binds one bed, chair or table, one exact item,
one target tile, and one complete native capture to a nonrenewable placement
attempt. Its retained `Placed` evidence proves historical stage-zero
construction-job registration. It does not prove a completed or usable building,
pathfinding, structural safety, a game checkpoint, or present pause.

This is an unadmitted development profile. It does not add a production MCP
mutation capability, an admitted registry tuple, or a protocol runner. The native
effect semantics and canonical receipt contract are in
[`BUILD_PLACEMENT.md`](BUILD_PLACEMENT.md).

## Executable review and placement workflow

The operator sets the separate development profile and loopback credential in the
process environment. The secret never appears in arguments, journals or output:

```text
DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1
DFMCP_BUILD_TOKEN=<32..256-byte secret>
DFMCP_BUILD_ENDPOINT=127.0.0.1:5000
DFMCP_BUILD_ALLOW_PLACE=1
```

`plan` performs a bounded read and returns the exact plan digest or explicit
blockers. `start` reobserves the complete selection and requires that same digest;
changing the item, target, native sequence, source or any witnessed field refuses
before an intent or placement is created.

```sh
python3 scripts/build_placement_client.py plan --kind bed --item 42 --target 15 15 2
python3 scripts/build_placement_client.py start --kind bed --item 42 --target 15 15 2 \
  --directory /private/placements --key bedroom-bed-1 --expected-plan <plan-digest>
python3 scripts/build_placement_client.py inventory --directory /private/placements
python3 scripts/build_placement_client.py inspect --directory /private/placements --key bedroom-bed-1
python3 scripts/build_placement_client.py query --directory /private/placements --key bedroom-bed-1
python3 scripts/build_placement_client.py cancel --directory /private/placements --key bedroom-bed-1
```

`query` and `cancel` use only the selection and endpoint retained in the original
intent. They cannot retarget, prepare or commit. Revoking placement permission
leaves authenticated preparation retirement and query available. Fully retained
outcomes can be inspected without credentials or a native connection, including
an immutable indeterminate outcome. Inventory and inspect never create or sync
files. Native effect records lost with a plugin restart remain unresolved.

The fixed client has one shared 1..60000 ms deadline, at most 32 native calls
including method bindings, and at most 512 KiB of connection bytes. It bounds
requests to 2048 bytes and replies to 8192 bytes, rejects unknown/duplicate or
noncanonical protobuf fields, and permanently fences every failed wire exchange.
Only a fresh preparation returned on that connection permits one commit. Query,
replay, restart and imported evidence cannot restore permission.

Complete JSON responses are limited to 64 KiB and include an authority-free
Agent Turn with local pending work, an exact original native capture reference
when available, and explicit historical/current distinctions. This standalone
client has no canonical world adapter; its canonical `anchor` stays null rather
than relabeling native generation/sequence as a world snapshot. The finite record and
view shapes are reserved before dispatch. Filesystem operations have cooperative
budget checks, not hard real-time cancellation guarantees.

## Executed client and codec checks

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_build_placement_wire test_build_placement_store test_build_placement_client -v
```

The current source passes 52 Python tests: 19 strict codec groups, 15 private
storage groups and 18 actual loopback/RPC/CLI workflow groups. Native byte parity
uses all eight independently constructed engine fixtures. Failure scenarios
include lost preparation/commit replies, recovery after reopen, permission
revocation, unknown source records, receipt integrity, immutable uncertainty,
pre-dispatch publication failure and post-effect receipt publication failure.
The TCP peers are explicit joined test doubles. Maximal text receipts and complete
64-row inventory pages fit the output ceiling. The actual native handler tests
and producer-to-Python byte checks are described in `BUILD_PLACEMENT_NATIVE.md`;
neither test scope establishes a real DFHack SDK build or live fortress.

## Private durable placement directory

`scripts/build_placement_store.py` implements `PlacementDirectory`, one complete
bounded directory of append-only `.placement` journals. Each idempotency key maps
to exactly one permanently retained journal. All callers sharing an effect scope
must use the same directory: choosing a different directory does not establish
that an earlier attempt was absent or safe to repeat.

Custody requires a canonical absolute path, an existing real directory with exact
mode `0700`, and regular single-link files with exact mode `0600`. Directories and
files must be owned by root or the effective user. Every path component is opened
without following symlinks. Files are opened relative to the retained directory
descriptor; writable files also use append mode. Symlinks, hard links, special
files, noncanonical modes and unknown directory entries cause refusal.

The owner holds an exclusive nonblocking directory lock for its entire lifetime,
including offline inspection. Before changing any journal it rechecks the
directory pathname against the open device/inode, enumerates complete membership,
and verifies stable bytes and descriptor/pathname identities for every retained
file. Same-size byte substitution and entry replacement are detected. A custody,
write or synchronization failure stops that owner. The implementation preserves
partial files and never repairs, truncates, overwrites or evicts history.

The complete-directory bound is 256 keys, each at most 32 KiB, with at most four
16-KiB frames. Each frame is canonical ASCII JSON followed by one newline, with
exact fields, a sequence number, the previous checksum, and a domain-separated
SHA-256 commitment. Replay validates the complete chain, canonical serialization,
all native evidence and every state transition. A torn frame, unknown transition,
conflicting receipt or changed commitment is rejected. Frame checksums detect
corruption; they are not signatures or an anti-rollback root.

## Ordered publication and recovery

Every append is semantically validated and completely replayed in memory before
the first write. The client then finishes any short writes, synchronizes the file,
synchronizes the directory, and verifies readback before acknowledging that
publication. Placement dispatch requires the following order:

| Durable state | Persisted evidence | Permitted next work |
| --- | --- | --- |
| Intent recorded | Exact eligible capture, selection through that capture, key, plan digest, token, source manifest and numeric loopback endpoint | Native preparation in the same foreground owner |
| Prepared recorded | Native prepared receipt for that exact plan and source | One dispatch-intent publication by the owner that acknowledged preparation |
| Dispatch recorded | Exact plan digest and the prior durable preparation | One native commit on the original authorized foreground connection |
| Native receipt recorded | Exact complete native record and reply source manifest | Offline inspection and discovery; no further journal append |

The store also keeps nonpersistent preparation provenance. Reopening a prepared
journal cannot restore permission to publish a dispatch, even when all durable
preparation bytes remain valid. Querying a prepared record does not create that
provenance. This deliberately leaves restart recovery with query and cancellation,
not an implicit resumed commit.

A receipt can be recovered immediately after intent publication or after any later
complete journal boundary. This covers a lost preparation reply, lost commit reply,
and a foreground process that dies after dispatch. Recovery must retain the exact
matching native record and source identity. Source generation may advance, but
DF and DFHack software versions cannot change silently. Endpoint selection is
bound into the original intent.

`Prepared` is effect-free and may be retired by a native cancellation receipt.
`Placed`, `Refused` and `Cancelled` resolve the local pending fence. `Indeterminate`
is different: its native record is immutable, remains pending, and requires
operator attention. Neither a later observation nor another receipt may replace
it with success or nonapplication. Cancellation never removes a registered
building, detaches an item/job, or converts uncertainty into permission to retry.

A journal with no final native record remains pending, including after native
restart reports an absent key. A missing native record proves no nonapplication.
Every pending journal in the complete directory blocks preparation under any new
key. A resolved key remains reserved forever. Local state never authorizes another
native attempt merely because the original key changed.

An error after a write but before both synchronizations complete cannot acknowledge
durability. Complete bytes might still be visible to a later process; that process
can inspect their historical evidence, but it does not claim the failed call
acknowledged storage. Power-loss behavior still depends on the filesystem and host;
the executed tests below simulate process and I/O failures, not physical power loss.

## Discovery and bounds

Inventory scans the complete directory before answering, orders pending entries
first, and reports complete total, pending and operator-attention counts on every
page. Pages contain at most 64 rows. Continuations bind the exact directory
identity, every retained file digest, page limit and offset. A changed journal,
substituted page size or changed membership invalidates the continuation.

Offline views explicitly preserve `retry_permitted: false`, historical evidence
and whether storage was acknowledged by the current call. They never infer
current pause or completed construction from a placement receipt. Directory
locking is cooperative host-local custody, not a global controller lease or
protection from a malicious owner/root rewriting the whole directory.

## Executed storage checks

```sh
python3 scripts/test_build_placement_store.py
```

The tests use the independently constructed furniture-engine golden corpus and
real POSIX files and subprocesses. They exercise complete replay, every incomplete
byte prefix and every single-byte corruption of a complete journal, invalid
rehashed transitions, immutable indeterminate evidence, restart dispatch refusal,
same-size substitution, symlink/hard-link/FIFO and mode refusals, pathname
replacement, process-held locking, short and partial writes, dispatch/receipt
synchronization failures, pending-first inventory and stale continuation rejection.
These are source and Python custody checks; they do not establish a real DFHack SDK
build, live-fortress evidence, Rust qualification or production admission.
