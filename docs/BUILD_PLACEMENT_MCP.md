# Reviewed furniture placement through MCP

`dfmcp-build-placement-dev-server` supplies the missing agent-facing path for
furniture/1.19: observe one exact item and target, prepare a witnessed plan,
confirm its review, commit once, and recover the original outcome after a lost
reply or process restart. It uses the frozen eleven-tool interface and the Rust
client, coordinator and private journal described in
[`BUILD_PLACEMENT_RUST.md`](BUILD_PLACEMENT_RUST.md).

This is a separate, explicitly unadmitted development executable. The production
runner map and compatibility registry do not gain furniture support. A placed
receipt proves historical stage-zero construction-job registration, not finished
or usable furniture. The coordination journal is not a game checkpoint.

## Operator configuration

Configuration belongs to the operator process. MCP callers cannot select a path,
endpoint, credential, protocol, protected region or checkpoint exception.

| Variable | Meaning |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19` | Required exact value `1` |
| `DFMCP_BUILD_WORLD_FOLDER` | Exact nonempty native fortress folder, at most 512 UTF-8 bytes |
| `DFMCP_BUILD_SITE_ID` | Canonical nonnegative decimal native site ID |
| `DFMCP_BUILD_SCOPE` | Ordered cuboid `[min_x,min_y,min_z,max_x,max_y,max_z]`, coordinates 0..32767 |
| `DFMCP_BUILD_JOURNAL` | Normalized absolute path in an existing owned `0700` directory |
| `DFMCP_BUILD_ENDPOINT` | Numeric IPv4 loopback endpoint; default `127.0.0.1:5000` |
| `DFMCP_BUILD_TOKEN` | Native matching credential, 32..256 bytes; needed only for native calls |
| `DFMCP_BUILD_ALLOW_PLACE` | Absent disables new placement; exact `1` enables consideration under all other checks |
| `DFMCP_BUILD_CHECKPOINT_POLICY` | Default `required`; explicit `disposable-fortress-no-checkpoint` enables the development exception |
| `DFMCP_BUILD_PROTECTED` | Optional array of at most 32 cuboids; default `[]` |
| `DFMCP_BUILD_MODE` | `control` (default), `recover` or `offline` |

Every other `DFMCP_` name is rejected, including an empty production admission
marker and the separate standalone native-client profile switch. The DFHack
process independently uses its native furniture opt-in and credential settings.

The default checkpoint policy refuses preparation because this profile has no
verified game-checkpoint provider. Only an operator deliberately selecting a
disposable fortress can enable the no-checkpoint exception. No tool argument
can select it, and no checkpoint certificate or rollback protection is invented.

Example for a disposable development fortress:

```sh
export DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19=1
export DFMCP_BUILD_WORLD_FOLDER=region1
export DFMCP_BUILD_SITE_ID=2
export DFMCP_BUILD_SCOPE='[0,0,0,63,63,7]'
export DFMCP_BUILD_JOURNAL=/private/furniture/journal
export DFMCP_BUILD_TOKEN='<matching native credential>'
export DFMCP_BUILD_ALLOW_PLACE=1
export DFMCP_BUILD_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
cargo run --locked -p dfmcp-mcp --bin dfmcp-build-placement-dev-server
```

The parent directory must already exist with the required ownership and mode.
Control may create a missing journal exclusively. An existing empty or damaged
file is refused, never repaired. Recovery and offline modes require the existing
original journal and never create one. The Python client's directory of
`.placement` files is a different format; this server does not import it.

## Agent workflow

The tool names below retain the exact `fortress.*` spelling returned by modern
MCP discovery and used in `tools/call`. Modern discovery metadata and stdio
conventions are documented in
[`FASTMCP_INTEGRATION.md`](FASTMCP_INTEGRATION.md).

1. `fortress.open_session(selection='["bed",42,15,15,2]', max_wall_millis=60000)`
   establishes native source identity and opens private custody. It returns a
   session ID and the complete coordination inventory. Its bootstrap observation
   is not retained as permission to prepare.
2. `fortress.observe(session_id, selection='["bed",42,15,15,2]')` retains an exact
   selected capture and its original native connection. The result includes
   `observation.observation_witness`, explicit blockers, selected item position,
   complete bounded capture bytes and a native evidence reference. Kind is
   exactly `bed`, `chair` or `table`.
3. `fortress.plan(session_id, idempotency_key, observation_witness)` validates
   current policy, reobserves the selection, checks that the native key is absent,
   synchronizes intent, and obtains and synchronizes native preparation. The
   response returns `plan_digest` and `review_seal`. The review binds the entire
   native plan, original key, journal, source, session, lease and operator policy.
4. `fortress.commit(session_id, idempotency_key, plan_digest, review_seal)` consumes
   the original local review. Current capabilities, lease, protected regions,
   checkpoint policy and the exact capture must still agree. Dispatch intent is
   synchronized before the one native attempt. Guards repeat after that sync;
   verified receipt history is synchronized before acknowledgment.
5. `fortress.wait(session_id, idempotency_key, plan_digest)` queries the original
   native operation at most once when recovery is needed. Fully retained immutable
   history returns locally. It does not advance game time or poll construction
   completion.
6. `fortress.cancel(session_id, scope="effect", idempotency_key, plan_digest)`
   retires only an unattempted native preparation. Query authority and the native
   credential suffice after placement permission is revoked. It cannot detach an
   item, remove a building, undo an effect or turn uncertainty into retry permission.

A placed effect exposes `effect.native.insertion` with typed native building,
construction-job and item IDs, kind, position, material, stage and verified links.
Agents can use these IDs as selectors for subsequent observations without decoding
the retained binary proof. They are historical native IDs, not canonical entity
generation handles. The spatial
[`construction_progress` query](CONSTRUCTION_PROGRESS.md) can inspect current
stage/job/item conditions, but selecting the same numeric ID across profiles does
not authenticate continuity with this receipt or discharge its recovery record.

Observation `eligible` describes the captured item and target only.
`result.source_summary` separately reports the last native reply's global
unresolved flag, retained-record count and preparation blockers. An empty local
journal does not prove that the native engine has no unresolved work. Planning
refreshes the native summary with its original-key preflight query and refuses a
global uncertainty fence or full retention before writing any new local intent.
Recovery of an already-owned original key remains available under Query authority.

The original connection's absolute lifetime is at most 60 seconds, including time
between tool calls. Opening with a large request ceiling does not make the native
preparation renewable. A new observation or outcome query abandons old local
commit permission. After restart, recovered prepared bytes have no review or
connection permit. Recovery is query or retirement of the original key.

The default request ceiling is 10 seconds and one GiB of conservative work
accounting, not a one-GiB buffer allocation. The maximum is 60 seconds. Output
admission reserves 64 KiB and uses an explicit four-byte-per-token proxy:
16,384..65,536 proxy tokens are accepted. Complete journal checks are reserved
before effect work. The server refuses insufficient budgets before dispatch.

## Local discovery and restart

`fortress.query` accepts one closed JSON string:

```json
{"mode":"records","limit":8}
{"mode":"get","idempotency_key":"bed-001","plan_digest":"<lowercase SHA-256>"}
{"mode":"selection","witness":"<lowercase SHA-256>"}
{"mode":"schema"}
```

Record pages contain at most eight complete summaries and order pending entries
first. `next_query` binds the exact journal head; a changed head invalidates later
pages. Unknown fields, duplicate fields, null optionals, nonintegral numbers and
invalid digests are refused. Omit absent optional fields. Local queries do not
open native connections. `fortress.explain` returns complete retained plan and
receipt evidence; `fortress.doctor` verifies local custody and inventory.

For offline inspection, retain the original folder, site, endpoint and journal
configuration, set `DFMCP_BUILD_MODE=offline`, and remove the credential and
placement permission. `fortress.open_session()` must omit selection. Offline
inspection neither creates nor synchronizes files. `recover` also opens without
a native connection; an explicit wait/cancel may then make one authenticated
query or retirement request. A missing native record remains unresolved.

`fortress.cancel(scope="session")` releases settled custody. When work remains
unresolved, explicit `release_for_recovery=true` releases local ownership while
preserving all original evidence. It does not claim that native work was cancelled
or that other controllers have stopped. Checkpoint and restore tools explicitly
refuse because there is no game-save implementation in this profile.

## Authority, response integrity and runtime ownership

Native operations need fortress-wide Query/Observe/Plan/Construct capabilities
because complete native registries and a distant exact item are part of the
effect boundary. The host adds a real exclusive core spatial lease over the
configured scope, expiring 1,200 game ticks from its initial observed floor.
Both the selected item's ground tile and the target's entire 3x3 context must
fit that live lease and avoid every protected cuboid. The lease is not renewed
automatically. Policy changes require session release and reopen; removing
placement permission takes effect at subsequent native boundaries.

All synchronous I/O runs in inherited Asupersync blocking work with joined
results. The original socket consults the current foreground request's runtime,
I/O, cancellation and operator checks; it does not retain permission from a
finished request. No fallback thread, detached watcher, alternate runtime or
subprocess performs native work. Kernel filesystem calls retain cooperative,
rather than hard real-time, cancellation limits.

Every success and error carries the shared Agent Turn packet. Pending work stays
visible independently of the current history page. Failed custody verification
withdraws inventory certainty while retaining historical identities and recovery
guidance while Query authorization remains current. Revoked or expired Query
authorization suppresses cached evidence as well as fresh reads; explicit recovery
release can still relinquish custody without disclosing retained facts. Complete
output is checked before publishing a response. The canonical
world `anchor` stays null: native generation, capture hashes and journal roots
are explicitly named evidence references, not fabricated canonical world state.

The local lease and private journal lock do not fence a different journal, server,
plugin or UI controller. Replacing evidence to bypass an unresolved operation is
not a valid recovery strategy. The source remains unadmitted development code
until its separate native, live-game and production evidence requirements are met.

## Executable verification

```sh
cargo test --locked -p dfmcp-adapter --lib build_placement
cargo test --locked -p dfmcp-mcp --lib build_placement_server
cargo build --locked -p dfmcp-mcp --bin dfmcp-build-placement-dev-server
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 scripts/test_build_placement_mcp.py \
  --binary /absolute/path/to/dfmcp-build-placement-dev-server
```

The process suite drives modern discovery, the exact eleven-tool list, real stdio
requests, the Rust native TCP client, and real private journal custody. Its native
peer is an explicit joined TCP double built from the independent furniture-engine
golden corpus. This scope is distinct from a real DFHack SDK or live fortress.
Results for the exact tested source are recorded in `IMPLEMENTATION_STATUS.md`.
The [focused execution report](evidence/build-placement-rust-mcp.json) records all
45 adapter, 25 MCP, two shared runtime-entry and seven process cases, along with
the tested binary identity and exact source hashes. It is development execution
evidence and does not qualify a production server artifact.
