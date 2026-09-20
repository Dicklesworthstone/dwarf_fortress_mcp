# One-shot bounded simulation developer client

`scripts/bounded_run_client.py` makes the isolated native `run/1.13` bridge usable
from a POSIX development host without adding a runtime dependency or changing the
MCP surface. It provides `observe`, `start`, `inspect`, `query`, and `cancel`.
It is not a production runner, Rust authority coordinator, MCP server, or
compatibility/admission mechanism. Use disposable fortresses only until the exact
DFHack SDK, plugin lifecycle, and live-game campaigns have been qualified.

Read [the native run contract](BOUNDED_SIMULATION_RUN.md) first. A requested
maximum is a stop trigger checked at native callback opportunities, not an
exact-tick or hard real-time guarantee. Native stop ownership does not depend on
the client remaining connected. Observed pause evidence is historical, not a
promise that another controller cannot unpause the game later.

## Start, inspect, recover, or cancel

Build/load `dfmcp_run_v1_13` in an exact matching DFHack development installation.
Both the game process and the client require `DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1`
and the same operator-supplied `DFMCP_RUN_TOKEN` (32..256 bytes). Supply the secret
through the process environment; never put it in an intent file or publish it in
logs. The client optionally reads `DFMCP_RUN_ENDPOINT` (default
`127.0.0.1:5000`). Only numeric IPv4 loopback endpoints are accepted: this native
DFHack transport is not encrypted and the credential is sent over the connection.
Production `DFMCP_ADMITTED_BRIDGE_PROTOCOL` state is refused.

Create an owner-private directory. Paths must be absolute; the final parent must
have exact mode `0700`, and intent files exact mode `0600`. Files and path
components cannot be symlinks. Root or the effective user must own the parent and
file. Intent files must be regular files with one link.

```bash
umask 077
mkdir -p "$HOME/.local/state/dfmcp-run"
chmod 700 "$HOME/.local/state/dfmcp-run"

python3 scripts/bounded_run_client.py observe

# Use a new key and a NEW intent pathname for every intended run.
python3 scripts/bounded_run_client.py start \
  --record "$HOME/.local/state/dfmcp-run/run-001.json" \
  --key run-001 --ticks 100 --wall-ms 5000

# Reconnect and inspect native evidence. This never replays CommitRun.
python3 scripts/bounded_run_client.py query \
  --record "$HOME/.local/state/dfmcp-run/run-001.json" \
  --wait-ms 7000 --timeout-ms 10000

# Request a safety pause, or retire an uncommitted preparation.
python3 scripts/bounded_run_client.py cancel \
  --record "$HOME/.local/state/dfmcp-run/run-001.json"

# Offline: no credentials, endpoint, opt-in, or native connection required.
python3 scripts/bounded_run_client.py inspect \
  --record "$HOME/.local/state/dfmcp-run/run-001.json"
```

Every completed command writes one JSON result to stdout. Success exits `0`;
protocol, custody, and I/O refusals exit `2` and keep `effect_status="unknown"`.
Command-line syntax errors use argparse's stderr diagnostics. Raw wire bytes,
credentials, and arbitrary server error text are not printed. A `start` response
may report `running`, `stopping`, or a terminal record; an RPC success is not goal
completion. `query --wait-ms` polls only QueryRun, not observe, prepare, commit, or
cancel. The wait never extends the original native run budget. A still-active
record at the foreground wait limit is returned with `wait_expired=true`.

The `--timeout-ms` bound is 1..60000 ms (default 10000), shared by connect, native
handshake, method binding, and all RPC reads/writes on that connection. Partial
reads do not renew it. `--wait-ms` is 0..60000 and shares that remaining network
deadline. File write/sync latency is not a hard real-time guarantee; a deadline
exhausted during intent publication prevents later dispatch on that connection.

## Crash and uncertain-effect discipline

`start` obtains a clock observation and computes the exact native plan/token
before writing a new immutable intent capsule. Exclusive creation and an
exclusive file lock prevent overwriting or reusing an existing pathname. Both the
file **and its parent directory are fsynced before PrepareRun or CommitRun**.
The lock remains held through the one commit attempt. Either sync failing leaves
no mutating request dispatched by that attempt.

The capsule contains the selected endpoint, key, limits, complete observation,
software tuple, plan digest, and preparation-token commitment. It contains no
bearer secret. Canonical JSON plus SHA-256 detects corruption or noncanonical
rewrites, not a malicious trusted owner. Reads verify size, canonical bytes,
private-file custody, and the named inode against the opened descriptor. Opening
special files is nonblocking so a FIFO cannot hang before regular-file validation.

**The capsule is recorded intent, not a durable effect journal or a completion
receipt.** It intentionally always says `indeterminate_until_native_query`.
The client does not append a terminal result to it. `inspect` exposes only this
intent and reports the effect unknown. Retain command output separately when
collecting evidence; this client does not qualify such output as durable custody.

There is deliberately no resume/replay-commit command. After a timeout, lost
reply, interrupt, or uncertain setter result, use the existing capsule with
`query` or `cancel`. Query does not advance time. Cancel can repeat a safety pause
in the native owner; it does not repeat an unpause. A refused, malformed, or absent
Commit reply is not proof of non-application. A missing native record after
reload/process death remains unknown, never an invitation to retry the old
unpause. An incomplete/corrupt capsule is refused without repair or overwrite;
manual investigation is required rather than inventing missing intent bytes.

The native plugin retains only 256 records per loaded lifetime and is not a
cross-process durable coordinator. The client checks software identity, nonce,
protocol, source generation, plan, key, preparation token, phase/flag semantics,
and receipt hash before presenting a matching record. It reports actual tick
advancement/overshoot when observed and `current_pause_unproved=true`. Retained
records from an older source remain historical; source changes do not promote an
absent record into a known outcome. Concurrent UI/plugins/controllers are not
fenced by this developer profile.

## Executed tests and evidence limits

```bash
python3 scripts/test_bounded_run_client.py
python3 scripts/test_bounded_run_bridge.py
```

Twenty Python test groups execute codec/custody logic and joined real-loopback
TCP protocol doubles. They cover 480 phase/reason/flag combinations, every-byte
corruption of a record, canonical key-length limits, partial responses, exactly
one commit attempt, reconnect/query after an ambiguous commit, absent-record
uncertainty, query-only polling, one cancellation, nonce and method-ID rejection,
notification limits, deadlines, private modes/links/FIFOs, and independent file
and parent-directory sync failures before native preparation. Offline inspection
is tested with no environment credentials and a client-constructor trap.

The client also decodes a frozen record emitted by the **actual C++ canonical
encoder**, with the Python hashes independently agreeing. The separate actual
plugin translation-unit suite passes 341 assertions and three independent hash
vectors on both GCC and Clang with warning denial and UBSan. Its SDK/protobuf
interfaces are explicit API doubles.

These tests do not connect the Python client to a real DFHack plugin, run actual
protobuf serialization/SDK ABI or plugin-manager callbacks, execute Rust/MCP, or
qualify a live fortress. Full repository qualification and production admission
are not claimed. The existing eleven-tool MCP surface still needs typed bounded-
run authority, durable effect coordination, and cancellation/drain integration;
this standalone developer client does not substitute for that work.
