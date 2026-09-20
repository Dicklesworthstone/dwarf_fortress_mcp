# Durable developer control for order-run/1.14

`scripts/order_run_client.py` provides an executable POSIX developer path from
an explicitly selected fortress/order to native conditional running and durable
receipt recovery. It uses `order_run_wire.py`, Python's standard library and the
existing isolated native order-run/1.14 plugin. It does not invoke a shell, another
client, arbitrary DFHack commands, or Rust/MCP. It does not replace the separate
Rust run/1.13 coordinator or confer production admission.

The native operation stops on sampled approval, activity, or a remaining-counter
threshold, or on a clock/safety limit. **Predicate evidence, pause readback, goods
produced, and current game state are different claims.** This client reports only
the first two when supported by their exact native record; it never promotes a
threshold, zero counter, missing order or missing record into production proof.
See `ORDER_CONDITION_RUN_RPC.md` for the underlying native contract and limits.

## Operator workflow

Build/load the new plugin against an exact matching DFHack development tree.
Use disposable fortresses. The game requires the two native variables; the
client additionally requires separate clock enablement for control:

```text
DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14=1
DFMCP_ORDER_RUN_TOKEN=<32..256-byte operator secret>
DFMCP_ORDER_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_ORDER_RUN_ALLOW_CLOCK=1
```

Endpoint defaults to the value shown. It must be canonical numeric IPv4 loopback;
no DNS or remote plaintext credential delivery is permitted. Other `DFMCP_*`
variables, including production admission state, are rejected by the client.
Secrets never enter the journal or output. Configure them outside command history.
Clock opt-in must be absent or exactly `1`; omit it for query-only recovery.

Create an absolute journal path under an existing owner-private `0700` directory.
The file will be exclusively created with mode `0600`; no existing file is
replaced. One journal binds one exact endpoint, software tuple, native generation,
world-folder and site. A replacement source cannot reuse it for new control.

```bash
umask 077
mkdir -p "$HOME/.local/state/dfmcp-order-run"
chmod 700 "$HOME/.local/state/dfmcp-order-run"

python3 scripts/order_run_client.py observe --order-id 9

# Explicit source selection prevents silently accepting whichever fort is loaded.
python3 scripts/order_run_client.py prepare \
  --journal "$HOME/.local/state/dfmcp-order-run/runs.jsonl" --key order-9-approval \
  --world-folder region1 --site-id 7 --order-id 9 --predicate approved \
  --samples 2 --interval 10 --ticks 1200 --wall-ms 10000

# Review the returned sealed plan, then supply its exact digest.
python3 scripts/order_run_client.py commit \
  --journal "$HOME/.local/state/dfmcp-order-run/runs.jsonl" --key order-9-approval \
  --confirm-plan <returned-plan-digest>

# One foreground receipt query; no implicit polling, unpause or budget extension.
python3 scripts/order_run_client.py query \
  --journal "$HOME/.local/state/dfmcp-order-run/runs.jsonl" --key order-9-approval

python3 scripts/order_run_client.py cancel \
  --journal "$HOME/.local/state/dfmcp-order-run/runs.jsonl" --key order-9-approval

# Offline: no development opt-in, token, endpoint or socket is required.
python3 scripts/order_run_client.py inspect \
  --journal "$HOME/.local/state/dfmcp-order-run/runs.jsonl" --key order-9-approval
```

Predicates are `approved`, `active`, and `remaining_at_most`; the latter accepts
`--threshold`. Threshold must be zero for flags. A condition already true is
refused before preparation rather than unnecessarily unpausing. Run limits are
1..1,200 ticks and 1..60,000 milliseconds. Samples are 1..16, interval 1..1,200
ticks, with their product within the run horizon. They describe sampled truth,
not continuous truth or a guarantee that the requested goal is attainable.
Clock limits take precedence over a new predicate claim at their boundary.

## Durable ordering and failures

Preparation synchronizes a complete sealed intent before native PrepareRun,
then validates and synchronizes the prepared receipt before returning it.
Duplicate keys cannot change their plan or renew native preparation. A still
pending or source-lost operation blocks new keys in the same journal.

Commit requires the exact reviewed digest, current prepared coordinator state,
and a fresh capture byte-identical to the originally paused source/order. It
synchronizes `dispatch_started` **before** the sole native CommitRun attempt.
A failure, timeout, lost reply, uncertain sync or process death after that marker
never makes the operation dispatchable again. A crash before the request was
actually sent can therefore conservatively leave an unresolved operation. Query
or cancellation is the recovery path; changing the key or journal is not recovery.

A lost preparation reply can be recovered by one QueryRun. When that query proves
preparation and no local dispatch marker exists, the prepared state may be
retained without re-preparing. After a dispatch marker, a prepared native record
cannot restore local dispatch eligibility. Native state regressions and changes
to frozen trigger evidence are rejected. Each complete native receipt is retained
before acknowledgement; terminal evidence is immutable.

Query performs one native read for unresolved work. A missing record retains the
last evidence as historical and explicitly reports absence/uncertainty. It does
not erase a previous receipt or dispatch marker. Terminal queries return local
stored evidence. Ordinary offline inspection reports the historical receipt, not
proof that the game is still paused. No record is a game checkpoint or rollback.

Cancellation before local dispatch records `cancelled_before_dispatch` without a
native call; this describes this coordinator only, not other controllers. After
dispatch it synchronizes `cancel_requested`, then asks the native owner to pause.
Only this safety-pause request may repeat. Failed pauses retain native ownership;
source loss remains unresolved and blocks further control through the journal.
Closing the developer client never cancels or unowns the native bounded stop.

## Custody, bounds and format

The final parent must be a real `0700` directory owned by root/effective user;
the file a real, single-link `0600` regular file owned by root/effective user.
All path components are opened through directory descriptors with no-follow
semantics. Special files are opened nonblocking and rejected before reads.
An exclusive nonblocking file lock spans each writable operation and its native
calls. Offline inspection uses a shared lock and read-only descriptor.

The header and every event are canonical ASCII JSON lines wrapped as
`{"value":...,"sha256":...}`. Header format is `dfmcp.order-run-journal/1`, with
exact source binding and a random 32-byte journal identity. Events carry a
strict sequence, previous envelope checksum and a full entry: key, canonical
plan hex, coordinator state, and optional canonical native-record hex. Each
transition is revalidated on replay, not merely checksummed. Checksums detect
corruption, not a malicious trusted owner who rewrites and rehashes everything.

Bounds are 256 intent keys, 4,096 events, 2 MiB total, and 8 KiB per line. Ordinary
nonterminal writes leave two event/byte reservations; a cancellation request leaves
one. Monitoring cannot consume the final terminal slot. There is no deletion,
eviction, truncation, repair, rotation or implicit migration. Incomplete/corrupt
journals are refused unchanged. A complete event whose earlier sync outcome was
uncertain can be validated and resynchronized during writable reopen. This does
not establish host power-loss qualification.

Each append rechecks descriptor/name/parent identity, exact modes, link count,
extent and every retained byte. Complete file synchronization and directory
synchronization precede native preparation. Writable reopen also synchronizes
verified existing bytes before later work. Failed publication fences that open
journal object; reopening does not reset a recorded dispatch attempt.

`inspect` without a key returns up to 1..8 whole records (default four), counts,
journal head and `next_after`. A subsequent `--after-key` requires the same
`--head`; head drift requires starting pagination again. The output ceiling is
128 KiB. `--timeout-ms` defaults to 10,000 and accepts 1..60,000. One socket's
connect, binding and RPC work share an absolute deadline; partial reads do not
renew it. Filesystem operations remain cooperative, not hard real-time.

## Executed evidence and remaining integration

```bash
python3 scripts/test_order_run_client.py
```

Twenty-eight test groups execute the actual Python codec, journal and developer
control paths with real POSIX files and joined loopback TCP protocol doubles.
They cover reviewed confirmation, wrong fortress, lost preparation and commit
replies, no redispatch after reopen, one-query waits, cancellation, historical
predicate receipts, source loss, missing records, frozen evidence, all coordinator
state pairs, independent file/directory sync failures, partial writes, uncertain
sync recovery, same-length corruption, inode replacement, modes/links/FIFOs,
locks, retention reservations, malformed framing/metadata and deadline expiry.

The same test command compiles the real C++ order engine/encoder on GCC and Clang
with warning denial and UBSan. Python decodes all 15 emitted phase/trigger vectors;
both compilers agree exactly, and an independent Python encoder reconstructs the
prepared vector. Every byte corruption and incomplete prefix of a terminal record
is rejected. Rehashed impossible flags, false goal samples, same-tick stability
and source/sequence mismatches are refused. The maximal 1,425-byte native record
and an eight-record worst-case escaped-text output fit their bounds.

TCP doubles do not execute the actual native plugin or its protobuf serializer.
Filesystem fault injection is in-process, not a power-loss campaign. No real
DFHack SDK/ABI, plugin-manager or live-game execution, Rust/MCP execution, full
repository qualification, global cross-controller lease or production admission
is established. The new order-run/1.14 profile still needs typed Rust/durable-MCP
integration and exact native/live qualification. This developer journal's format
is not silently interchangeable with the existing Rust run/1.13 journal.
