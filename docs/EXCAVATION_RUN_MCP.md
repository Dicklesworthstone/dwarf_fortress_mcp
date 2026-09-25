# Excavation-run/1.18 through MCP

`dfmcp-excavation-run-dev-server` connects the existing Rust excavation-run
coordinator, private journal and native RPC source to the frozen eleven-tool MCP
interface. This is explicitly unadmitted development source. It is not a
production runner, a new native protocol, a mining-designation API, or a new
implementation of the standalone Python blueprint monitor.

The workflow lets an agent inspect unresolved runs, observe one region, review a
finite run, commit once, and recover sampled terrain/stop evidence by the original
key. The native run owner can advance **already designated** work and request a
pause after its floor condition, tick/wall limit, cancellation or failure trigger.
This MCP layer does not designate terrain and does not poll in the background.

## Operator configuration and startup

Build with the repository's locked nightly toolchain. Configure the unchanged
`dfmcp_excavation_run_v1_18` plugin and matching token as described in
`EXCAVATION_RUN.md`. Only its existing four `DFMCP_*` environment names are
accepted; paths and fortress selectors are process arguments, not MCP inputs:

```sh
export DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18=1
export DFMCP_EXCAVATION_RUN_TOKEN='<matching plugin token, 32..256 bytes>'
export DFMCP_EXCAVATION_RUN_ENDPOINT=127.0.0.1:5000
export DFMCP_EXCAVATION_RUN_ALLOW_CLOCK=1

cargo run --locked --offline -p dfmcp-mcp \
  --bin dfmcp-excavation-run-dev-server -- \
  --directory /private/excavation-run \
  --world-folder region1 --site 2 --region '[15,15,2,2,2]' --initialize
```

The existing directory must be private and empty for initialization. The fixed
journal name is `excavation-run-v1_18.journal`; Linux x86_64/aarch64 private storage
retains its exact 0700 directory/0600 single-link file, nofollow traversal,
exclusive custody, append-only history, and file plus directory sync requirements.
`--initialize` permits one exclusive creation attempt when opening a **Control**
session. It never repairs or initializes an old empty file. Omit the option when
reopening an existing journal. A failed creation attempt may require restarting
the executable; preserve any file created, and inspect it rather than overwrite it.

The region is `[x,y,z,width,height]`, width and height 1..8. It scopes new goals;
recovery always uses each original retained plan's region. It is not a multi-level
or sparse blueprint selector. Paths, raw commands, credentials, native methods
and production admission selectors are absent from the tool schemas.

For offline recovery, only the exact development opt-in is needed. Do not set a
token or endpoint merely to inspect a journal. Unrelated `DFMCP_*` variables,
including production tickets or another development profile, are refused.

## Agent workflow

Call `fortress.open_session` with `mode="control"` for reviewed clock control.
Without a mode it opens **offline**. Initial Control creation obtains one coherent
native capture; reopening an existing Control journal does not imply a current
capture, so call `fortress.observe` before planning. Use the returned session ID
and observation witness, not an inventory digest, as the planning inputs.

`fortress.plan` takes a bounded JSON **string** in its `request` argument. Its
closed object is:

```json
{
  "key": "excavate-room-001",
  "observation_witness": "<64 lowercase hexadecimal characters from observe>",
  "game_ticks": 100,
  "wall_millis": 1000,
  "samples": 2,
  "stable_ticks": 2,
  "interval_ticks": 1,
  "max_gap_ticks": 10
}
```

The whole selected region must be visible and dry, not already satisfy the floor
goal, and have an eligible paused source. The native predicate requires FLOOR,
zero liquid and no designation in all selected cells. Matching coordinates in
another bridge generation do not establish equivalent evidence. The sampling
window must fit **strictly before** the native tick limit. Limits trigger a stop;
this is not an exact-tick scheduling guarantee.

Planning creates a local review only: no native preparation or unpause. Review
the returned plan digest, exact observation, region, bounds and evidence limits.
Then call `fortress.commit` with the same session and this `request` string:

```json
{"key":"excavate-room-001","plan_digest":"<returned digest>","confirm":true}
```

The review is consumed before the effect shell. Under the original private-file
lock, the backend rechecks the expected inventory, obtains a fresh native
connection, and delegates to the durable coordinator. The coordinator re-observes,
checks key absence, persists intent and preparation, syncs dispatch state, and
attempts one commit. There is no reconnect/retry path around that commit.

`fortress.wait` takes `{"key":"...","plan_digest":"..."}` as its request and
performs at most one native receipt query. It does not poll, unpause or extend the
native run deadline. A duplicate commit returns retained history, not another
attempt. Native record absence remains unknown, and a lost reply never permits a
new key to bypass unresolved work. The bounded native stop owner is independent
of the host request; a host disconnect is not proof of native cancellation.

## Fixed modes and cancellation

Modes never widen within a session, even with injected capability grants:

| Mode | Local history | Native receipt query | Terrain/review/start | Native cancellation |
|---|---|---|---|---|
| `offline` (default) | Yes | No | No | No |
| `recover` | Yes | Yes | No | No |
| `control` | Yes | Yes | Current grants and operator start permission | Current Clock grant |

Every operation rechecks current Query authority at a monotonic historical tick
floor. Optional `expires_at_tick` is not refreshed by subsequent requests.
Revoking the separate unpause opt-in blocks preparation/commit but does not
prevent an otherwise authorized safety stop in a Control session.

`fortress.cancel` takes one closed request:

```json
{"scope":"plan","key":"...","plan_digest":"..."}
{"scope":"effect","key":"...","plan_digest":"..."}
{"scope":"session","release_for_recovery":true}
```

Plan cancellation is local and never cancels game work. Effect cancellation uses
the original retained key and native stop protocol. Ordinary session release
(`release_for_recovery` absent/false) refuses unresolved work. Explicit recovery
release can discard a fenced local session without reading broken storage, but
never erases the journal, claims native quiescence, or cancels the independent
native owner. Reopen the original journal to inspect/reconcile it.

`fortress.checkpoint` and `fortress.restore` explicitly refuse. Journals are not
game saves, replay is not terrain rollback, and no checkpoint or global
cross-controller clock lease is fabricated.

## Discovery, pagination and failure handling

`fortress.query` accepts `{"kind":"records","state":"unresolved","limit":4}`
or `{"kind":"schema"}`. Filters are `all`, `unresolved` and `terminal`; limits
are 1..4 complete records. Continuations bind session, inventory projection,
filter and page width. At most 64 are retained, and a changed inventory requires
restarting pagination. Cursor state is published only after complete rendering
and final runtime/deadline checks. Queries never make native calls.

Pending work, a local review and an uncertain attempted key remain in the shared
Agent Turn independently of the requested history page. Native SourceLost is
terminal **but unresolved**, not proven nonapplication. Failed final custody
reads label prior inventory unverified and retain the attempted key rather than
claiming no active work. Expired authority suppresses retained rows entirely.
`fortress.explain` shows the exact retained plan, before capture, receipt and last
native sample without a new read; hidden/missing cells have no attribute payload.
`fortress.doctor` verifies local coordination, not game health or admission.

`dfmcp.excavation-inventory-anchor/1` names a source-bound retained-state
projection. It is not a physical journal-frame head or canonical world snapshot.
The separate native observation witness is required for planning. A sampled
floor trigger and verified **historical** pause remain separate fields; neither
proves current pause, continuous stability, structural safety or mining causality.

## Bounds, ownership and evidence

Request-body strings are at most 2048 UTF-8 bytes. The published shape contract is
`schemas/mcp_excavation_run_v1.json`; runtime additionally rejects duplicate
fields, wrong integer representations, impossible sampling relationships, stale
reviews and unauthorized effects. It does not interpret arbitrary expressions.

A request reserves a complete 32 KiB response and two 8 MiB inventory-work
allowances before effects. Default ceilings are 15 seconds, 64 MiB of conservative
work reservations, 8192 output tokens and 1200 game ticks; ceilings can only be
narrowed within a session. The byte/token conversion is reservation accounting,
not a measured model-token count. One original wall allowance includes queue,
storage, native I/O and final verification. Blocking kernel calls have no claimed
hard preemption guarantee.

Async handlers use inherited Asupersync-owned blocking work and joined results.
Parent restrictions are preserved; there is no detached worker or cancellation
watcher. Abandonment/cancellation signals the existing native socket cancellation
handle; current runtime permission repeats at native boundaries, including after
dispatch synchronization. This does not manufacture a native stop receipt.

Twelve dispatcher/renderer/parser tests, four runtime tests and one executable
argument test are registered, in addition to fourteen adapter-session tests.
**All 31 Rust groups are uncompiled and unexecuted in this editing environment.**
Cargo, rustc, rustfmt and the locked graph are unavailable. On a provisioned
checkout run:

```sh
cargo test --locked --offline -p dfmcp-adapter excavation_run::session
cargo test --locked --offline -p dfmcp-mcp live_excavation_run_server
cargo test --locked --offline -p dfmcp-mcp --bin dfmcp-excavation-run-dev-server
python3 scripts/check_excavation_mcp_reference.py
```

The executed Python checker accepts 53 and rejects 104 JSON-value shape cases,
verifies all three unchanged native fixture Git blobs and their native
plan/token/receipt hashes, derives inventory golden vectors, checks 288 sampling
arithmetic cases, and checks conservative response models of 19,143–25,404 bytes
against the 32 KiB reservation. It also inventories source hashes and the eleven
registered tool declarations. These are Python reference/static checks, **not
Rust parsing, the real renderer, MCP execution, native SDK, live-fortress,
physical power-loss or full-repository qualification**. Its retained output is
`docs/evidence/excavation-mcp-reference.json`.

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-action-coordinator-exec-ero.4`. Their broader acceptance remains open.
The compatibility registry, production runner map, native wire, journal format,
dependency graph and separate Python blueprint workflow are unchanged.
