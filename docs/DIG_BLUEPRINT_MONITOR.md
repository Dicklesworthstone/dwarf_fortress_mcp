# Whole-original-blueprint excavation monitoring

`dfmcp_adapter::dig_blueprint::monitor` evaluates the entire original sparse
floor blueprint after every native child has a terminal **Designated** receipt.
This closes the gap between recording designations and observing their intended
terrain result. It uses the existing map/1.5 observation protocol; dig/1.16 alone
does not expose a normalized floor shape.

The existing `dfmcp-dig-control-dev-server` exposes the workflow through
`fortress.plan`, `fortress.wait`, `fortress.query` and `fortress.cancel`. Native
designation and monitor sampling remain separate operations with separate
histories. No additional top-level MCP tool is added.

## MCP workflow

Start with the [retained blueprint workflow](DIG_BLUEPRINT_MCP.md). Add these
operator variables before opening the session:

```sh
export DFMCP_DIG_BLUEPRINT_MONITOR_CONFIG='{"path":"/private/mining/project/excavation.history","initialize":true,"source":{"endpoint":"127.0.0.1:5000","generation":9,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"}}'
export DFMCP_DIG_BLUEPRINT_MAP_TOKEN='<matching native map token>'
```

Replace the example generation and software strings with the independently
accepted map/1.5 source values. The endpoint must match the original digging
endpoint. Configure the unchanged map plugin separately in DFHack as described
in [LIVE_MAP.md](LIVE_MAP.md); its own opt-in/token variables belong to that
process, not this isolated MCP profile. The monitor file must differ from both
the native journal and blueprint parent, under a real private `0700` directory.

Opening with `initialize:true` does **not** precreate an empty history. The file
is created only when the explicit monitor registration succeeds. This allows a
long designation project to close and reopen before monitoring starts. You can
also add the monitor configuration when reopening an already designated project.

After the project reports `all_designated`, call `fortress.plan` with `monitor`
alone. Its value is a serialized closed JSON object containing:

| Field | Meaning and default |
|---|---|
| `blueprint_id` | Required exact original project identity. |
| `deadline_tick` | Required absolute game tick, at or after the retained start and far enough away to fit the requested stability. |
| `stable_ticks` | Elapsed ticks in the shared matching window; default `10`, maximum `403200`. |
| `required_samples` | Advancing-tick matching samples; default `2`, range `1..64`. |
| `max_gap_ticks` | Largest permitted gap between samples; default `1200`, range `1..403200`. |
| `max_samples` | Total read attempts, including failed or unfinished reads; default `128`, range `1..128`. |

For example, if the current retained tick is `12345`, an illustrative request is:

```text
fortress.plan(session_id,
  monitor='{"blueprint_id":"<original id>","deadline_tick":13345,"stable_ticks":10,"required_samples":2,"max_gap_ticks":1200,"max_samples":128}')
```

Choose the deadline from the actual project clock. Omit `blueprint` and the
individual native review arguments. Registration performs no map read or game
mutation and requires only scoped Query/Observe authority. It works in writable
`reconcile` mode without new Plan/Designate permission. All original children
must have verified terminal Designated receipts; a stopped completed parent is
eligible. Identical registration returns the same original monitor. Changed
policy or a later deadline cannot replace it.

Keep `result.blueprint_monitor.recovery` outside the session. Sample once with:

```text
fortress.wait(session_id, monitor_id="<returned monitor id>", max_wall_millis=60000)
```

Omit the native key and plan digest. Each call acquires one complete paused
map capture, durably retains it, rechecks the original designation inventory,
then publishes recomputed progress. This call does not advance game time or
poll in a loop. If the game remains at the same tick, repeated calls cannot add
stability samples. Advance gameplay separately and pause before another sample.

The result reports exact selected-cell counts, overlapping deficit categories,
matching samples and start tick, the original policy, remaining attempt status,
latest accepted sample tick and first historical satisfaction tick. Status is
`pending`, `unknown`, `stabilizing`, `satisfied`, `expired`, `invalidated` or
`cancelled`. A fresh acknowledged satisfying sample sets
`sampled_excavation_condition_proven:true`. Historical query, terminal replay,
restart, errors and retained handoffs never set that current-result flag.

Inspect local history or page through selected targets using `fortress.query`:

```json
{"mode":"blueprint_monitor","monitor_id":"<original monitor id>","offset":0,"limit":16}
```

Use the returned `target_page.next_query` for later pages. Its `expected_head`
rejects mixing pages from different history generations. `status:satisfied` in
a query is historical achievement, with `fresh_sample_acquired:false` and no
claim that the terrain remains satisfied now. Query performs no map read.

To stop only the read-only monitor:

```text
fortress.cancel(session_id, scope="monitor", monitor_id="<original monitor id>")
```

This needs current scoped Query in a writable mode and preserves all evidence.
It neither stops miners nor changes the native project. Already terminal
histories retain their original terminal status. During an open session,
monitor query and cancellation remain available if the native journal or parent
becomes damaged; their response explicitly withholds current original-inventory
verification. Reopening the **combined MCP session** still requires its original
native journal and blueprint parent. The adapter's independent `MonitorStore`
API can reopen monitor history without those files; this MCP profile has no
monitor-only restart mode.

For restart, keep the native/blueprint configuration and change the monitor
configuration to `initialize:false`, with its original source pin and exact
returned recovery object:

```json
{
  "path":"/private/mining/project/excavation.history",
  "initialize":false,
  "source":{"endpoint":"127.0.0.1:5000","generation":9,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"},
  "recovery":{"monitor_id":"<original monitor id>","head":"<last acknowledged history head>"}
}
```

`offline` opening makes no native connection and permits historical queries;
it cannot append or cancel. `reconcile` opens without a native connection and
permits explicit sampling or monitor cancellation under read grants. `control`
also performs the existing independent native clock check before issuing its
host lease. The maximum of native and monitor clock floors governs current
authority. Reopening never renews the monitor deadline or its attempt allowance,
restores fresh satisfaction, or resumes an interrupted read. Preserve changed
recovery heads from every acknowledged operation and from failure handoffs.

Configured monitor sessions admit at most 2048 entities for the complete bounded
capture and bookkeeping. Existing request byte/output ceilings still apply;
complete source, history, final audit and response work is prepaid before map
contact. Ordinary non-monitor sessions retain their existing entity allowance.

## Exact original work and source

`MonitorSpec::new` takes the complete retained `BatchDefinition`, verified native
inventory, an independent `SourcePin`, fixed `Policy` and starting game tick.
It seals the full original geometry, native journal head, ordered original
key/plan/receipt set, source selectors, dimensions and temporal policy. Unknown,
prepared, refused, cancelled, missing or substituted children cannot register
a monitor. `verify_original` repeats this link against actual current custody
before a combined original-work/terrain result can be published.

The map endpoint and software must match the original native binding. The
operator pins the map generation independently: map and dig generations are
separate counters and must not be equated. Folder, site, map dimensions, complete
capture region, pause state and nonregressing game time must match. These checks
associate evidence; they do not prove shared process identity or mining causality.
The original native effects remain governed by their own journal.

## One whole-mask condition

Each attempt acquires one complete enclosing capture, at most 1,024 cells, and
evaluates only the original selected targets, at most 512 cells. Every selected
cell must be visible, exactly `Shape::Floor`, have zero liquid depth, have no
magma flag and have no dig designation. Unselected gaps do not participate.
Missing and hidden cells remain unknown. Wrong shapes, liquids and outstanding
designations are separately counted; those deficit categories can overlap.

Every target participates in one shared stability window. The fixed policy has
an inclusive absolute deadline, required advancing-tick samples, elapsed stable
ticks, a maximum gap and at most 128 attempts. Equal-tick reads cannot increase
the sample count. Failed reads, unknown/false conditions and excessive gaps
break unfinished stability. Source or clock regression invalidates the monitor.
Terminal results are immutable. A past satisfied sample does not establish
continuous stability, present terrain, structural safety or attribution.

## Durable bounded history

`MonitorStore` keeps a private append-only history of the immutable specification,
started attempts, complete raw captures, acknowledged publication, interruptions
and local cancellation. Progress is reconstructed from the raw observations and
fixed transitions. No serialized success flag is trusted. The history is bounded
at 8 MiB; attempts cannot be refreshed by reopening or issuing another request.

An attempt is synchronized before map contact. The acquired raw observation is
retained before authorization at its newly learned tick and before final source,
runtime or original-custody checks. Failed acknowledgement fences uncertain
publication. The known clock floor survives even when the new tick makes the
current grants expire. Reopening preserves terminal historical achievement and
breaks an unfinished streak; an unfinished durable read cannot resume sampling.

Keep the returned `{monitor_id, head}` recovery object outside the owner. Exact
reopening requires that acknowledged head in the verified append chain. A
complete later tail can be adopted after a lost reply; rollback, torn frames,
changed original identities and in-place repair are refused. The private-file
helper requires a real exact-mode `0700` parent and an exclusive exact-mode
`0600` regular file, separate from native and blueprint custody.

`begin_attempt` returns one bounded attempt token for `capture`, `publish`,
`interrupt` or `invalidate`. Query and cancellation can use the monitor's own
history without contacting the game or repairing a damaged native journal.
Stopping blueprint advancement does not prevent read-only monitoring after all
original designations are verified. Monitor cancellation does not stop miners,
cancel native effects, erase evidence or undo excavation.

## Foreground source boundary

`monitor::io::read_map` consumes an injected `ProofStream` and makes one typed
map observation through the existing closed handshake and read protocol. It
preflights the whole enclosure and byte allowance before I/O, checks current
scoped Query/Observe authority at every stream boundary and narrows one absolute
deadline. It creates no background worker, timer loop, arbitrary native method
or game mutation. The host owns the supervised loopback connection.

These are component semantics. They do not establish a native DFHack build,
live-fortress campaign, full MCP transport qualification or compatibility
admission. The production protocol map and compatibility registry are unchanged.
