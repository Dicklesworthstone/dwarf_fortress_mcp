# Stop an original dig batch while it is running

The existing `dig_blueprint_client.py stop` serializes with the whole batch.
It cannot acquire that lock during a foreground prefix run. The separate,
identity-only stop requester can revoke the original batch without waiting for
its directory or manifest lock:

```sh
python3 scripts/request_dig_blueprint_stop.py \
  --directory "$ORIGINAL_BATCH" --batch-id "$ORIGINAL_BATCH_ID"
```

Use the batch ID retained from original initialization or review. The command
has no native credentials, endpoint, bridge method, blueprint override, or
replacement-key option. It performs no network operation and does not consume
or change any `DFMCP_` environment state.

## What it does

The requester reads the original immutable manifest through no-follow
owner-private descriptors, verifies its exact canonical envelope and checksum,
recomputes the existing v1/v2 batch identity, and verifies the original effects
directory inode. It then exclusively creates the **same** `stopped.json` record
that the existing batch owner already understands. It locks only that stop file,
never the running owner's directory or manifest. The file and containing
directory are synchronized and reverified before acknowledgment.

This is intentionally identity-only revocation, not semantic admission. The
requester does not recompile a room recipe or certify terrain/intent validity.
It can only prevent work in the named existing batch; the ordinary batch owner
still owns complete semantic decoding, custody, and recovery. The room manifest
bound is not borrowed by the legacy profile. New/unknown formats are refused.

An already open unstopped batch detects the added name at its next custody
check and fails closed. A reopened batch validates the marker and has no next
step. This revokes future admission; it is **not a hard real-time interrupt**.
A check can race a socket send, and an already admitted/dispatched native effect
may complete. A connection can be closed before its reply is retained. Preserve
all original intents and use the existing explicit original-key query/cancel
path. Never repeat an uncertain designation or create a replacement batch.

No native cancellation, game pause, rollback, game-effect drain, current terrain,
excavation completion, or production admission is proved by this command. The
output makes these limitations explicit. Other batches and other controllers
are outside its scope. No background worker or detached task is introduced.

## Failure and recovery

A byte-identical existing marker is reverified and re-synchronized without
rewriting it. A concurrent stop publisher can cause a nonblocking lock refusal;
repeat only the stop request to reverify the same local marker, never a native
effect. A valid stop can survive a lost write/sync/output acknowledgment.

An empty, torn, conflicting, oversized, symlinked, hardlinked, or nonprivate
marker is refused and left unchanged. A partial marker fences ordinary batch
reopening too; it is not permission to remove the marker or restart effects.
No repair, unlink, rename, replacement, or registry mutation is performed. All
original effect capsules, keys and receipts remain untouched.

The default cooperative wall allowance is 10 seconds, selectable with
`--timeout-ms` in 1..60000. File sizes, path traversal, directory entries, JSON
nesting, and work checks are bounded. Filesystem calls such as `fsync` are not
hard real-time guarantees. Output failure never causes a second execution or a
second JSON object.

## Evidence

The initial increment passes 29 actual Python tests of the new implementation,
including a CLI stop while a joined independent process holds both original
owner locks, a process killed during marker publication, byte-preserving sync
recovery, custody races, competing publisher refusal, and short output. These
use real private files and processes with synthetic identity-only manifests;
they are not a native executor, room semantic, Rust, DFHack, live-fortress, or
full-repository qualification. Existing executor compatibility is source-reviewed
against its original stop format and `Batch.verify`/`open_batch` behavior.
