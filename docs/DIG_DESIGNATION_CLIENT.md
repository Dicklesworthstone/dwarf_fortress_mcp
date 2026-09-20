# One-shot mining designation developer client — 1.16

`scripts/dig_designation_client.py` makes the isolated native designation handler
usable from a POSIX development host. It adds no dependency, Rust/MCP entry,
production runner or compatibility admission. Its fixed commands are `observe`,
`start`, `inspect`, `query` and `cancel`. There is no resume-commit command, generic
DFHack command, arbitrary Lua, automatic reconnect, background task or unpause.
Use disposable fortresses until an exact real DFHack SDK build and live campaign
establish compatibility and behavior.

This profile is **separate from the concurrently landed ordinary-mining/1.15
engine** in `bridge/common/dig_designation.h` and order-condition-run/1.14. The
1.16 implementation is in `bridge/common/dig_designation_v1_16.h` with its own C++
namespace, plugin, protobuf package, magic bytes and development opt-in. It leaves
those other implementations untouched. In particular, priority and scheduling
readback, explicit hidden-neighbor acknowledgement and cancellation are additional
semantics, not an in-place interpretation of 1.15 bytes or its stricter policy.

## Operator setup and flow

The game and client independently require:

- `DFMCP_ALLOW_UNADMITTED_DIG_V1_16=1`;
- matching `DFMCP_DIG_TOKEN`, 32..256 bytes;
- additionally `DFMCP_DIG_ALLOW_DESIGNATE=1` for preparation, commit or cancellation.

The client optionally reads `DFMCP_DIG_ENDPOINT`, default `127.0.0.1:5000`.
Only canonical numeric IPv4 loopback endpoints with a nonzero port are supported.
Transport is not encrypted; never expose the DFHack connection remotely. The
client refuses other `DFMCP_*` profile/admission variables. Secrets never appear
in intent capsules, successful responses or error messages. Supplying a secret
through the environment is a developer opt-in, not production admission or a
replacement for the future Rust capability/lease/checkpoint coordinator.

Read `DIG_DESIGNATION.md` before selecting a region. Normal mining only, at most
8 by 8 target tiles on one level, with a complete one-cell 3D halo. The game must
be paused. No call starts miners, advances time or actually excavates a tile.

```sh
umask 077
mkdir -p "$HOME/.local/state/dfmcp-dig"
chmod 700 "$HOME/.local/state/dfmcp-dig"

python3 scripts/dig_designation_client.py observe \
  --x 100 --y 100 --z 20 --width 4 --height 2
```

The response includes complete bounded visible observations, redacted hidden
cells, the exact witness, blockers and `plan_digest_for_confirmation`. No-blockers
means this limited local filter accepted the evidence; it is NOT a mining-safety,
structural-support, pathfinding, protected-region or global-hazard certificate.

Choose a new private pathname and key, then use the exact returned values:

```sh
python3 scripts/dig_designation_client.py start \
  --record "$HOME/.local/state/dfmcp-dig/dig-001.json" --key dig-001 \
  --x 100 --y 100 --z 20 --width 4 --height 2 \
  --expected-witness '<observation witness>' \
  --confirm-plan '<plan_digest_for_confirmation>'
```

Start reacquires the complete region and requires the witness to remain identical.
The confirmed digest must cover that region, witness and hidden-neighbor policy.
Changed terrain fails before publishing an intent or preparing a designation.
The exact content digest is a confirmation of bytes, not evidence that a human
reviewed them or a policy engine authorized excavation.

The default refuses hidden neighbors. `--allow-hidden-neighbors` on BOTH observe
and start explicitly acknowledges unknown neighboring terrain in the sealed plan.
It never permits hidden targets, reveals hidden attributes, overrides known visible
hazards, or claims the unknown region safe. Missing/unallocated halo cells remain
refused. This override is absent from the stricter 1.15 foundation; policy and
admission do not transfer between the profiles.

A successful start may report `designated`, `refused`, or `unknown`. Designated
requires the exact predicted region/halo readback, including priority 4000 and
block designation/cooldown changes. It is immediate historical configuration
evidence, **not excavation, current terrain, or completed production**.

## Durable intent and recovery discipline

The client exclusively creates a new immutable intent capsule under a real
exact-mode 0700 directory. All path components are opened using no-follow
directory descriptors. The file is a single-link exact-mode 0600 regular file,
with nonblocking special-file rejection and an exclusive advisory lock retained
through the command. File and parent-directory fsync BOTH precede native prepare
and the only commit attempt. Files are never overwritten, repaired or truncated.

The capsule stores the endpoint, source manifest, key, region, policy, complete
observation and exact witness/plan/token commitments; it contains no bearer token.
Canonical JSON and SHA-256 reject corruption and noncanonical rewrites. Descriptor,
parent, named-inode, permissions, extent and complete bytes are checked again
before each effect stage and before acknowledgement. Failure after insertion may
leave an unknown outcome; the original capsule must be retained.

Only a fresh native preparation can enter commit. Even a replayed *Prepared*
response from a new pathname cannot trigger dispatch. Terminal/unknown native
preparations return evidence without another commit. There is at most one commit
attempt on a connection. All failed wire/evidence validations close the connection.

```sh
# No token, endpoint, opt-in, or native connection is needed for inspection.
python3 scripts/dig_designation_client.py inspect \
  --record "$HOME/.local/state/dfmcp-dig/dig-001.json"

# Reconnect to the SAME saved endpoint/source and retrieve exact native evidence.
python3 scripts/dig_designation_client.py query \
  --record "$HOME/.local/state/dfmcp-dig/dig-001.json"

# Retire only an uncommitted native preparation. Never undo a designation.
python3 scripts/dig_designation_client.py cancel \
  --record "$HOME/.local/state/dfmcp-dig/dig-001.json"
```

The capsule always retains `indeterminate_until_native_query`. It is durable intent,
**not a terminal receipt journal**. Offline inspection therefore reports the outcome
unknown even after a successful command response. Query checks the exact source
incarnation/software and full plan-bound native record. Missing records, source
changes and malformed/lost responses never prove non-application. Cancellation
cannot undo a partial/finished designation or remove an existing game job.

This is not a multi-controller or cross-capsule durable coordinator. Native
unknown outcomes block other keys only during the retained native incarnation.
Choosing a new key/store after losing native retention cannot be made safe by
this developer client. Do not create another capsule to bypass unknown history.
Guarded Rust Designate authority, lease/checkpoint policy, global unresolved-work
custody, a durable terminal journal and MCP integration remain required.

## Bounds, output and executed evidence

`--timeout-ms` is 1..60000, default 10000, shared by connect, all fixed bindings,
handshake and every native operation in that command. Every blocking socket call
uses the remaining absolute deadline; fragmented reads never renew it. Synchronous
file operations use repeated deadline checks before dispatch, not forcibly
interruptible fsync. A timeout during intent publication prevents subsequent
native work on that connection.

Requests are at most 2 KiB, successful RPC frames 32 KiB, complete observations
16 KiB, and notification traffic eight frames / 256 KiB per call. Only the fixed
six-method 1.16 package is bound. Duplicate/unknown fields, nonminimal protobuf
integers, invalid reply shapes, aliases, wrong nonces and source drift are refused.
Capsules are at most 64 KiB and complete command responses at most 128 KiB; output
is never truncated JSON. Each command writes one structured JSON result, exit 0
on an acknowledged operation or 2 on refusal. Argument-parser syntax diagnostics
remain on stderr. Exit 0 alone never proves a known or completed game effect.

Run `python3 scripts/test_dig_designation_client.py`. Twenty-three groups execute
actual Python code, POSIX custody, joined fragmented loopback TCP peers, and the
actual CLI in child processes. Coverage includes independent file/directory sync
failures before preparation, lost reply followed by a new connection querying the
UNCHANGED original capsule, no duplicate commit, replayed preparations, malformed
receipts, stale confirmation, same-length intent corruption between stages,
private-mode/link/FIFO rejection, deadlines, source binding, offline operation and
maximum actual JSON serialization bounds. The four native vectors match the
actual C++ encoder and independent struct/hashlib reconstruction. The client rejects
1,704 single-bit designated-record changes, 639 incomplete effect prefixes,
1,616 incomplete observations, rehashed false outcomes and illegal state shapes.

Native engine/actual-handler suites separately pass 5,818 assertions on EACH of
GCC and Clang with warning denial and UBSan; six compiled mutants fail per compiler.
Those native tests use explicit DFHack/protobuf API doubles. Python peers implement
the fixed wire but are not a real plugin or protobuf runtime. These results do not
establish Rust/MCP execution, a real DFHack SDK/ABI build, power-loss durability,
live fortress behavior, full repository qualification or production admission.
Exact tested source hashes are retained in `docs/evidence/dig-client.json` and the
separate native reports.
