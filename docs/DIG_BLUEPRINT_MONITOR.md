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
stability samples. Advance gameplay separately and pause before another sample,
or use the optional [separately reviewed finite clock
workflow](DIG_BLUEPRINT_CLOCK_MCP.md). Clock reconciliation and goal sampling
remain distinct calls. Configured clock custody is verified before and after
each fresh map capture; pending or unresolved runs block publication.

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
native journal and blueprint parent. When those files are unavailable, use the
independent monitor-only recovery modes below.

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
host lease, except when exact clock custody and an intact all-Designated monitor
are being opened for clock recovery. That case can reopen while a run is active
without attempting another paused digging observation. The maximum of native and monitor clock floors governs current
authority. Reopening never renews the monitor deadline or its attempt allowance,
restores fresh satisfaction, or resumes an interrupted read. Preserve changed
recovery heads from every acknowledged operation and from failure handoffs.

Configured monitor sessions admit at most 2048 entities for the complete bounded
capture and bookkeeping. Existing request byte/output ceilings still apply;
complete source, history, final audit and response work is prepaid before map
contact. Ordinary non-monitor sessions retain their existing entity allowance.

## Recover monitoring when mining custody is unavailable

An intact monitor history can be reopened through MCP even when the original
native journal or blueprint parent has been lost, damaged or remains locked by
another owner. Keep the original `DFMCP_DIG_BLUEPRINT_MONITOR_CONFIG` with
`initialize:false`, its independent source pin and the last acknowledged
`{monitor_id, head}`. No map or digging token is needed. The original folder,
site, scope and endpoint configuration must still match the history.

You may omit `DFMCP_DIG_BLUEPRINT_CONFIG` for this recovery. The existing
`DFMCP_DIG_JOURNAL` path setting remains required configuration, but neither it
nor any blueprint path is opened or recreated. Ordinary mining modes reject a
monitor configuration without its original blueprint; there is no fallback to
a new native journal.

Open one of these modes and **omit `region`**:

```text
fortress.open_session(mode="monitor-offline", max_wall_millis=60000)
fortress.open_session(mode="monitor-recovery", max_wall_millis=60000)
```

Use one session at a time. Both modes grant Query only and hold only the exact
monitor file. Neither mode creates a native session, issues a lease, contacts
the game, prepares work, samples terrain or restores native receipts.

| Mode | Available actions |
|---|---|
| `monitor-offline` | Verified historical monitor queries, restricted query-schema discovery, monitor-history doctor and session close; no append or synchronization. |
| `monitor-recovery` | The same inspection plus durable local `cancel(scope="monitor", monitor_id=...)`. |

`fortress.query` accepts only `blueprint_monitor` and `schema` in these modes.
`fortress.doctor` verifies this history, not live health or original native
custody. `fortress.wait`, observation, native plan/commit/reconciliation,
blueprint advancement, source replacement and new monitor registration are
refused. Recovering historical evidence does not authorize resumed sampling.

Use the same head-pinned query and monitor cancellation calls shown above.
Historical `satisfied` remains historical; every packet explicitly sets
`original_designations_verified:false`, `fresh_sample_acquired:false` and
`sampled_excavation_condition_proven:false`. Agent Turn coverage is restricted
to this monitor history and does not suggest a new terrain sample. An unfinished
read remains fenced, with its retained clock floor and finite attempt count.

`fortress.cancel(scope="session")` verifies and releases this local owner
without cancelling the monitor or settling any native effect. If monitor
custody itself becomes corrupt or configuration changes after opening, explicit
`release_for_recovery:true` releases the lock while retaining an unverified
identity/head handoff. It does not repair, erase or recreate any history. Current
runtime cancellation and the request deadline still apply to release.

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
the sample count. Changed capture bytes at the same tick also break the shared
streak, even when the selected tiles still match. Another observation at that
tick cannot seed a replacement streak; a later advancing tick is required.
Failed reads, unknown/false conditions and excessive gaps break unfinished
stability. Source or clock regression invalidates the monitor.
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

The store exposes local evidence withdrawal for an enclosing request or runtime
failure after an accepted sample. Withdrawing clears unfinished stability
immediately; the next authorized attempt first records that interruption durably.
It needs no new authority and performs no I/O, and never repairs history,
restores a consumed attempt, removes a known clock floor or erases a terminal
historical achievement.

The MCP host applies this withdrawal to rejected monitor requests, failed final
publication checks and missing acknowledgement of an owned worker's result.
Before the next request it checks whether the outer runtime successfully joined
and acknowledged the prior worker. This tracks local runtime completion, not
network delivery or receipt by an external MCP client. An invalidated source may
retain an older accepted target page; that page remains explicitly historical
and is never marked as a fresh sample.

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
