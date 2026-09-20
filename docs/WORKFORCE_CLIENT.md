# Reviewed workforce control and durable recovery

`scripts/workforce_client.py` connects the isolated **workforce/1.17** native
profile to an executable POSIX developer workflow: observe, prepare, review,
commit once, query retained receipts, cancel, or inspect offline. It uses only
Python's standard library and does not spawn another client, shell, or runtime.
It is not a Rust/MCP integration or production-admitted control surface.

The action changes membership in **one existing selected-only work detail** for
1..32 sorted, explicitly selected citizens, then invokes DFHack's supported
labor recomputation for changed citizens. It does not create details, choose raw
labor enum numbers, change modes, move dwarves, interrupt jobs, or prove future
productivity. See `WORKFORCE_CONTROL.md` and `WORKFORCE_NATIVE.md` for native
preconditions, exact readback, and the scope of observed labor permissions.

## Operator workflow

Build/load `dfmcp_workforce_v1_17` against an exact matching DFHack SDK. Use a
disposable fortress until native/live qualification. Both game and client require:

```text
DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17=1
DFMCP_WORKFORCE_TOKEN=<32..256-byte operator secret>
DFMCP_WORKFORCE_ALLOW_LABOR=1
```

The client optionally accepts `DFMCP_WORKFORCE_ENDPOINT=127.0.0.1:5000`; only
canonical numeric IPv4 loopback is allowed. The transport is not encrypted.
Other `DFMCP_*` variables are refused, including production admission state.
The labor opt-in must be absent or exactly `1` and is required for preparation,
commit and cancellation. Read/query recovery needs no labor grant. Offline
inspection needs no configuration or credentials at all. Secrets are not stored
in the journal, echoed in results, or accepted as command-line arguments.

Create a real private directory, then inspect the selected citizens and details:

```bash
umask 077
mkdir -p "$HOME/.local/state/dfmcp-workforce"
chmod 700 "$HOME/.local/state/dfmcp-workforce"

python3 scripts/workforce_client.py observe --units 2,5

python3 scripts/workforce_client.py prepare \
  --journal "$HOME/.local/state/dfmcp-workforce/assignments.jsonl" \
  --key miners-001 --units 2,5 --detail 0 --assigned yes \
  --world-folder region1 --site-id 7

# Review the returned detail name, source, citizens and allowed labor keys.
python3 scripts/workforce_client.py commit \
  --journal "$HOME/.local/state/dfmcp-workforce/assignments.jsonl" \
  --key miners-001 --confirm-plan <returned-plan-digest>

python3 scripts/workforce_client.py query \
  --journal "$HOME/.local/state/dfmcp-workforce/assignments.jsonl" --key miners-001

# Offline, including after DFHack or the developer client has restarted:
python3 scripts/workforce_client.py inspect \
  --journal "$HOME/.local/state/dfmcp-workforce/assignments.jsonl" --key miners-001
```

Detail indices are **capture-local positions**, not persistent entity IDs. The
full list, all memberships, labor columns, selected citizen/historical identities,
pause state and source are sealed. An unrelated detail edit or citizen eligibility
change invalidates the old preparation. Commit requires the exact reviewed digest
and a fresh capture identical to the original. Use `--assigned no` in a new plan
to remove membership; other details can still grant the same labor permissions.

The client independently reconstructs the complete expected post-configuration
using only the explicitly permitted observed labor-mask changes. It checks the
native after-witness against those bytes, not merely against its own receipt
checksum. Applied proves immediate membership and recomputation readback, not
current state, exclusive labor permission, jobs performed or production completion.
Post-masks are returned with their exact native labor-key column order.

## Durable ordering and uncertainty

The append-only journal binds the exact endpoint, software pair, bridge generation,
world folder and site. It synchronizes sealed intent **before** PrepareAssignment,
retains and synchronizes the native prepared receipt, then synchronizes
`dispatch_started` **before** CommitAssignment. The commit is attempted once.
Receipt validation and synchronization precede acknowledgement. A lost reply,
uncertain synchronization, or crash after the dispatch marker can never make
that key dispatchable again—even if the native request was never sent.

Each journal permits only one unsettled intent at a time. A new key cannot bypass
Unknown or an unresolved dispatch within that journal. Native unresolved state
also blocks new preparations in that loaded plugin. Neither rule is a global
cross-process lease: creating another journal or reloading a plugin is **not** a
recovery procedure. Native idempotency does not survive plugin/process restart.

Query makes one native receipt lookup for unresolved intent/dispatch. It does not
poll, capture workforce state, recompute labors, re-prepare, or re-commit. A native
Prepared receipt recovered after dispatch stays Tracking, never Prepared. A missing
native record preserves uncertainty and prior historical evidence. Native Unknown
is permanent in this profile; later forged success or changed Unknown evidence is
rejected. Stored prepared/terminal query results are returned locally; their current
freshness is not asserted. Terminal evidence cannot be rewritten.

`cancel --journal ... --key ...` retires undispatched local intent with no native
call. After a dispatch marker it synchronizes a cancellation request and calls the
native cancellation operation: this can retire a Prepared token when the prior
commit was not received. It cannot undo Applied membership or repair Unknown.
Returned `membership_undone=false` makes that distinction explicit. Repeated
cancellation never invokes the assignment setter. Local cancellation describes
this coordinator, not what another holder of native credentials might have done.

## Custody, bounds, and output

The journal requires an absolute normalized path, a non-symlink root/effective-user
owned `0700` parent, and a single-link regular `0600` file. Every directory component
is opened with no-follow semantics. Special-file opens are nonblocking; FIFOs are
rejected before reading. An exclusive nonblocking lock spans writable operations;
offline inspection uses a shared lock on a read-only descriptor. Descriptor,
pathname, parent identity, modes, extent, and all retained bytes are rechecked.
Complete file and directory synchronization precedes native preparation. Writable
reopen verifies and resynchronizes complete bytes after an earlier uncertain sync.
An uncertain append fences that journal object until reopening.

Format `dfmcp.workforce-journal/1` is canonical ASCII JSONL. Each envelope contains
`value` plus its SHA-256. The header contains format, random journal identity and
exact source binding. Events contain sequence, previous checksum and full entry
(plan, coordinator state and optional native effect bytes as lowercase hex).
Replay enforces semantic transitions as well as checksums. Checksums detect
corruption, not malicious rewriting by the trusted owner.

Bounds: 64 keys, 512 events, 64 MiB/journal and 160 KiB/line. Preparation reserves
room for subsequent preparation, dispatch and terminal receipts. Tracking cannot
consume the final terminal reservation. No pruning, idempotency eviction, repair,
truncation, migration or overwrite is provided. Torn/corrupt histories are refused
unchanged. Full game-memory or power-loss durability is not established by these
in-process filesystem checks.

Single-effect output is reserved before preparation/commit, using actual escaped
text sizes and conservative per-citizen post-mask bounds. Responses are at most
128 KiB. Offline inspection without `--key` lists 1..8 compact complete record
references (`--limit`, default 4); `--after-key` must include the matching `--head`.
Head changes require a new first page. Exact per-key inspection includes labor
details. `--timeout-ms` is 1..60000 (default 10000), shared across foreground file,
connect, binding, and RPC work; partial reads cannot renew it. File calls are
cooperatively bounded, not hard real-time. Success writes one JSON result and exits
0; operational refusals exit 2 and do not invent a negative effect outcome.

## Executed validation

```bash
python3 scripts/test_workforce_native.py --mutations
python3 scripts/test_workforce_bridge.py --mutations
python3 scripts/test_workforce_client.py
```

Thirty Python groups execute the actual codec, journal, coordinator and CLI,
including real private POSIX files and joined fragmented loopback TCP doubles.
They cover independent C++ fixture agreement; every-byte effect corruption and
torn prefixes; rehashed false readback; all eight-citizen membership subsets;
confirmation and source refusal; lost prepare/commit replies; recovery without
redispatch; native retirement after a crash-before-request; permanent Unknown;
local/offline authority; revocation after the durable marker; independent file
and directory sync failures; terminal sync recovery; partial writes; changed
inodes/modes/links/FIFOs; locks; all state pairs; 64-key pagination; deadline
expiry; terminal retention; and maximal escaped-text output reservation.

The client test command also compiles/runs the actual engine with GCC and Clang
and checks three native capture/record fixtures against an independent encoder.
The engine suite passes 8,497 assertions/compiler; the separate actual native
handler suite passes 3,208 assertions/compiler and rejects three compiled mutants
per compiler under warning denial and UBSan. These test doubles do **not** execute
real DFHack/protobuf serialization, SDK/ABI, plugin lifecycle, or live fortresses.
No Rust/MCP, full repository, host power-loss, or production-admission qualification
is claimed. The workforce family still needs typed Rust/durable-MCP integration,
real native validation, and cross-controller coordination before deployment.
