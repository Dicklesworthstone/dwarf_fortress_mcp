# Durable excavation-conditioned run workflow (development 1.18)

`scripts/excavation_run_client.py` connects the native 1.18 runner to a foreground
POSIX command-line workflow: plan, confirm/start once, inspect, query/wait, cancel
and inventory. It uses the typed decoder and transport in
`EXCAVATION_RUN_CLIENT.md`, without changing their native wire or the Rust/MCP
surface. This supplies a host-local durable owner; it is not a production clock
lease, a game checkpoint, or admission. Use a disposable fortress and no competing
controller. Read `EXCAVATION_RUN_NATIVE.md` for the native goal and stop semantics.

## Plan and start

Build/load the existing `dfmcp_excavation_run_v1_18` against the exact matching
DFHack installation. The native process and client must have the same explicit
opt-in and credential. In the client environment, only these DFMCP names are
allowed (do not combine another developer controller's settings):

```text
DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18=1
DFMCP_EXCAVATION_RUN_TOKEN=<32..256 UTF-8 bytes; never include in commands or journals>
DFMCP_EXCAVATION_RUN_ALLOW_CLOCK=1
DFMCP_EXCAVATION_RUN_ENDPOINT=127.0.0.1:5000
```

The endpoint is optional and defaults as shown. The native plugin does not read
the client endpoint variable. Clock permission is required only for starting;
removing it or setting it to zero leaves authenticated plan/query/cancel possible.
Inspect and inventory never read credentials or contact the game. Cached terminal
query/cancel also returns offline history without new native contact.

Create one dedicated, persistent, owner-private directory (0700). Keep it as the
authoritative journal directory for this controller; do not switch directories,
rename/delete journals or start a different controller to bypass unresolved work.
Paths are absolute, canonical and no-follow, including ancestor components.
Existing journals must be exact-mode 0600, owner/root-owned regular single-link
files. An empty newly created file after a failed write is intentionally a blocker.

```sh
# Example geometry only: choose already-designated, visible, dry terrain.
# --spec is: game ticks, wall milliseconds, samples, stable ticks, interval, max gap.
python3 scripts/excavation_run_client.py plan \
  --region 15 15 2 2 2 --spec 100 1000 2 2 1 10

# Copy result.plan_digest from that exact plan into PLAN_DIGEST.
# Start takes a new capture; ANY relevant change requires a new explicit plan.
python3 scripts/excavation_run_client.py start \
  --directory /private/dfmcp-excavation --key excavation-001 \
  --region 15 15 2 2 2 --spec 100 1000 2 2 1 10 \
  --expected-plan "${PLAN_DIGEST:?Set from plan output}"
```

The native plan digest covers the complete capture and six limits, not the key or
software strings. The connection pins its software manifest, and the journal
seals that manifest and key separately. Confirmation is explicit developer intent,
not a production authorization seal. No designation is created by this workflow.
The fixed goal accepts visible dry undesignated floors, including constructed
floors; no mining causality or structural-safety claim is made.

Before connecting, start validates EVERY journal in the directory and refuses any
unresolved work, existing key, corrupt file or full inventory. Before preparation,
it reads the selected region, checks the exact confirmed plan and performs a
QueryRun preflight. Any currently retained native key, even Prepared, is refused.
Absence alone is not retry permission: the start is new, its source capture is
current, no local unresolved journal exists, and the same key cannot be reused.

Publication order is immutable intent -> native Prepare -> durable Prepared
receipt -> durable dispatch intent -> one Commit attempt -> optional durable
terminal receipt. Each publication completes its write, syncs the file AND its
directory, and revalidates named-inode/full-byte custody before proceeding.
Authority and deadline are checked again after dispatch publication and by the
transport at the actual send boundary. No error initiates a second Commit.
A failed sync may leave complete or partial bytes; neither is erased or repaired.

## Observe progress, cancel, or recover

```sh
python3 scripts/excavation_run_client.py inventory \
  --directory /private/dfmcp-excavation --limit 16

python3 scripts/excavation_run_client.py query \
  --directory /private/dfmcp-excavation --key excavation-001 \
  --wait-ms 5000 --max-queries 16 --timeout-ms 10000

python3 scripts/excavation_run_client.py cancel \
  --directory /private/dfmcp-excavation --key excavation-001

python3 scripts/excavation_run_client.py inspect \
  --directory /private/dfmcp-excavation --key excavation-001
```

An existing journal can only be inspected, queried or cancelled. There is no
resume, retry-commit, overwrite, repair, compaction or forget operation. Recovery
checks the recorded endpoint/software before the keyed call. Query-only waiting
uses one connection and 1..32 queries, at most once per 100 ms after the first.
It returns pending progress when its query/wait limit is reached. Cancel sends
exactly one CancelRun and never loops; later progress is obtained through query.
A partial/lost frame fences that connection, and a later invocation may only
recover using the original journal.

Terminal receipts retain complete native bytes and a software/source manifest.
They are checked against the original intent on every replay. Identical terminal
replay writes nothing. Conflicting or forged terminal records fail closed.
Stopped/Refused resolve this local operation, not current fortress state.
SourceLost is retained terminal HISTORY but remains unresolved operator work and
blocks new starts. Missing native records likewise remain unknown and block new
keys. A successful response always preserves retry_permitted=false and does not
claim present pause, continuous stability, mining causality or production admission.

The native owner, not the client, maintains its bounded stop after disconnection.
No host daemon, thread, background poller, timer or automatic safety action is
created. The existing native safety-stop lifecycle is unchanged.

## Journal and inventory bounds

The directory is limited to 256 `<native-key>.exrun` files. One nonblocking
exclusive advisory directory lock owns the entire invocation. Complete membership,
file contents, permissions and descriptor/path identities are rechecked at effect
boundaries. A malformed unrelated entry also refuses the directory; do not share
this directory with run/1.13, dig/1.16 or other profiles.

A journal has at most four canonical ASCII JSON lines, 16 KiB per line and 32 KiB
per file. Each frame has format `dfmcp.excavation-run-journal/1`, a zero-based
sequence, previous checksum, kind, payload and checksum. The first previous
checksum is 64 zeros. SHA256 over `dfmcp-excavation-run-frame/1` + NUL + canonical
frame bytes excluding checksum seals the frame. Strict replay verifies both the
chain and legal transitions. An intent contains key, six-limit spec, full capture,
plan/token commitments, endpoint and manifest. Prepared/terminal frames contain
full native record hex and manifest; dispatch contains the exact plan digest.
Truncation to a valid preterminal frame stays pending and cannot enable replay.

Inventory puts pending work first, returns complete-domain counts independent of
page size, and supports 1..64 rows with exact-inventory-bound continuations. The
inventory digest binds path, directory device/inode, sorted filenames and exact
file-byte SHA256 under `dfmcp-excavation-run-inventory/1` + NUL. A continuation is
canonical-JSON lowercase hex containing inventory digest, page size and offset.
It is a consistency cursor, not an authorization credential. Any inventory change
invalidates old continuations. Coverage means this local directory, not all
controllers, all files on disk, or complete fortress history.

Output is capped at 64 KiB and space is reserved before starting an effect. The
single 1..60000 ms cooperative budget begins before opening the directory and
covers storage and native work. Filesystem syscalls cannot be forcibly timed out;
this is not a hard real-time durability guarantee. A complete receipt surviving
an uncertain sync can later be inspected, but storage_acknowledged_this_call=false
never retroactively certifies the failed sync. Errors return code 2 and an unknown
outcome without raw credentials, native errors or file contents.

## Executed evidence and limits

```sh
PYTHONPATH=scripts python3 -W error -m unittest -v \
  test_excavation_run_client test_excavation_run_store
```

All 48 tests pass on the uploaded code bytes. Tests execute actual Python, private
POSIX files, subprocess inspection/locking, a killed start process, new-process
query recovery and joined fragmented loopback TCP doubles. They exercise exact
confirmation, retained-key rejection, complete-directory pending fences, all six
predispatch file/directory sync failure positions, partial writes, terminal sync
failure, authority/deadline expiry after dispatch publication, stale inventory
pages, corrupted journals and rehashed contradictory native evidence.

Four syntax-valid weakened source copies fail focused regressions: removing
intent file sync, exact-plan confirmation, the pending-work fence, or durable
dispatch publication. These mutation experiments are not changes to main.

This increment did not compile/execute C++, a real DFHack SDK/plugin, generated
protobuf, Rust/MCP, a live fortress, physical power-loss campaigns or full
repository qualification. Native profile, dependency universe, compatibility
registry and production runner map are unchanged. OS advisory locks/checksums do
not exclude external controllers or a trusted owner rewriting/removing the entire
store. Rust supervised-runtime/capability/lease integration and live qualification
remain separate. This advances the bridge/action recovery beads without closing
their broader requirements.
