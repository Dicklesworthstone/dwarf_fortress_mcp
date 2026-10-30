# Whole-original-blueprint excavation monitoring

`dfmcp_adapter::dig_blueprint::monitor` evaluates the entire original sparse
floor blueprint after every native child has a terminal **Designated** receipt.
This closes the gap between recording designations and observing their intended
terrain result. It uses the existing map/1.5 observation protocol; dig/1.16 alone
does not expose a normalized floor shape.

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
