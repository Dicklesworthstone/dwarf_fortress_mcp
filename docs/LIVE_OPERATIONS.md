# Coherent operations/1.3 development read profile

This profile observes **jobs, buildings, items, and their relationships in one
native DFHack RPC suspension**. It does not concatenate snapshots obtained by
separate citizen, announcement, or jobs servers. Every published fact and edge
belongs to the same source digest and canonical observation anchor.

The implementation is an explicitly unadmitted development path, not production
compatibility or evidence of live-game success. Existing native profiles 1.0,
1.1, and jobs/1.2 are unchanged. The production runner map remains unchanged.

## Components and entry

- Native plugin: `bridge/dfhack-operations-v1_3/dfmcp_operations_v1_3.cpp`.
- Native envelope: `proto/DfmcpOperationsV1_3.proto` in that directory.
- Canonical codec/state: `dfmcp_adapter::live_operations`.
- Closed client: `dfmcp_adapter::live_jobs_rpc::operations`.
- MCP runtime: `dfmcp_mcp::live_operations_server`.
- Cargo binary: `dfmcp-live-operations-dev-server`.

The plugin registers only `Handshake` and `ReadObservation`, both with native
RPC flags zero. There is no arbitrary plugin/method name, memory address, Lua,
command, filesystem path, or effect in the tool arguments. The separate protocol
package is `dfmcp.operations.v1_3`; exact major/minor are 1/3.

Build the plugin within a named DFHack source checkout as an external plugin
using its supplied `CMakeLists.txt`; the target is `dfmcp_operations_v1_3`. This
requires real generated DF headers and protobuf linking. Mock compilation below
does not establish that build. Load only in a disposable development game.

Configure the same operator-selected 32..256-byte `DFMCP_OPERATIONS_TOKEN` in the
DFHack process and the MCP process. The token is never a tool argument. Start the
Rust development runtime with the token already present in its environment:

```bash
DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_3=1 \
DFMCP_OPERATIONS_ENDPOINT=127.0.0.1:5000 \
cargo run --locked --bin dfmcp-live-operations-dev-server
```

The public library entry repeats the development gate. Other `DFMCP_*` settings,
including all production admission state and other profiles' credentials, are
rejected. This profile has its own process-scoped session family. Parsing an
untrusted raw numeric live ID cannot mint a current-process session handle.

## Acquisition and bounds

`fortress_open_session` accepts `max_jobs`, `max_buildings`, `max_items`,
`max_bytes`, `max_output_tokens`, `max_wall_millis`, and a subset of `observe`,
`query`, and `doctor`. Defaults are 1,024 jobs, 1,024 buildings, 8,192 items,
2 MiB, 8,192 output tokens, and 5,000 milliseconds per native call.

Hard ceilings are 4,096 jobs, 4,096 buildings, 32,768 items, 65,536 attachment
records, and 2 MiB for the complete native observation payload. The RPC envelope
has a separate bounded 4 KiB allowance. At most eight sessions are retained by a
process; session lifetime currently ends at process shutdown. Native calls use
an absolute deadline across fragmented reads/writes and text notifications.
Handshake/bootstrap and first observation are separate bounded calls.

The entire native roster must fit. This is **not native pagination**: an oversized
world, byte budget, cyclic list, invalid record, unresolved same-roster reference,
or incomplete relationship set refuses publication. Raising a limit cannot exceed
the implementation ceiling. Large-fortress snapshot paging remains future work.

No game clock is controlled, and no unpause occurs. `observe` and `wait` each
refresh one native observation. A failed read fences the source and preserves the
prior published snapshot. It is not silently relabeled fresh. Queries do not
refresh unless `await_watch` explicitly requests its one-observation step.

## Fields and relationships

Jobs retain the jobs/1.2 data-format fields: native ID, type/key, reaction,
suspension/repeat flags, position, native worker and holder references, raw timer,
and attached-item/filter counts. Their source is operations/1.3, not a separately
fetched jobs/1.2 observation.

Buildings add native ID, type/key, bounding corners, and raw build/max-build
stages. The current decoder admits nonnegative stages at or below the reported
maximum. This is a bounded profile, not a complete building subsystem.

Items add native ID, type/key, subtype, material type/index, stack size, raw
`item.pos`, and explicit flags: `forbidden`, `in_job`, `dump`, `removed`, `rotten`,
`trader`, `on_ground`, `in_inventory`, and `in_building`. Container and building
holder fields carry canonical entity IDs when present and explicit absence when
the corresponding native reference is absent. Raw position is not recursively
resolved world position and does not prove access.

Observed graph edges are:

| Edge | Meaning |
|---|---|
| Job `contained_in` building | The same-read `Job::getHolder` reference. |
| Item `contained_in` item | The same-read `Items::getContainer` reference. |
| Item `contained_in` building | The same-read `Items::getHolderBuilding` reference. |
| Job `uses` item | An actual job-item attachment, with raw role and filter index facts. |

Attachment counts must agree with the complete attachment list. The current
profile admits filter indices of -1 or an index below the job's filter count;
it does not interpret the full requirement filter language. Containment cycles
and dangling endpoints fail before any combined snapshot becomes visible.

**`uses` does not mean requirement satisfaction.** Item membership does not mean
usable supply. Stack counts cannot be added across incompatible types/materials
to infer fulfillment. Raw flags, stages, timers, and missing jobs are not blocker
causes, navigation proofs, estimated completion times, or successful completion.
Worker IDs remain native references; no placeholder citizen entities are created.

Canonical jobs use native ID + 2; buildings use `(1 << 40) + native ID`; items
use `(2 << 40) + native ID`; fortress root is 1. Always obtain handles and
generations from query results rather than relying on these formulas in agents.
Relations have stable semantic-key-derived IDs rather than positional row IDs.
Observed retirement/reappearance advances generation. Any native ID-horizon,
clock, or bridge-generation reset advances the shared epoch. Invisible same-ID
reuse between observations is not claimed detectable.

## Agent workflows

The eleven tool names are retained. Game-mutating plan/commit/cancel/checkpoint/
restore operations refuse without an effect. The existing structured query
schema is reused, including filters, generation-checked inspection, aggregates,
search, paths, baselines, and condition watches. Use `mode="schema"` for discovery.
Convenience modes are `summary`, `jobs`, `buildings`, and `items`.

Pass each following JSON envelope in the `query` argument of `fortress.query`,
with the session ID supplied separately. Do not combine `mode` and `query`.

Inspect forbidden inventory without calling it unavailable for every purpose:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "entities",
    "kinds": ["item"],
    "fields": ["type_key", "stack_size", "forbidden", "in_job"],
    "limit": 2,
    "where": {
      "op": "compare", "field": "forbidden", "comparison": "eq",
      "value": {"type": "bool", "value": true}
    }
  }
}
```

Group observed stack counts by item type, still without claiming usable supply:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "aggregate", "kinds": ["item"],
    "group_by": {"kind": "field", "field": "type_key"},
    "metrics": [{"name": "stack_units", "field": "stack_size", "numeric_type": "u64"}]
  }
}
```

Capture selected inventory facts, observe later, and use the returned baseline
handle with `changes`. This is endpoint comparison, not retained intervening history:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "capture", "key": "inventory-review", "max_game_ticks": 100,
    "select": {"kind": "entities", "kinds": ["item"], "fields": ["stack_size", "forbidden"]}
  }
}
```

The baseline retains at most 256 selected rows; it is deliberately smaller than
the acquisition roster. Narrow the selection rather than silently truncating it.

The checked-in fixture has canonical job ID `9`; this traversal follows its
actual attachment and holder relations. Use an observed ID for real sessions:

```json
{
  "schema": "dfmcp.query/1",
  "query": {"kind": "traverse", "roots": ["9"], "edge_kinds": ["uses", "contained_in"], "max_depth": 4}
}
```

Construction-stage conditions use generation-checked `watch` field predicates.
`await_watch` validates the handle and pre-refresh anchor, observes at most once,
then reauthorizes at the target before evaluation. Terminal retries skip I/O.
Local watch listing/cancellation/release remain available after source failure
and report stale source continuity. Full response rendering must succeed before
any retained baseline/watch mutation is committed. Acquisition may already have
published a newer snapshot before a later response-rendering failure.

## Evidence and limitations

Eighteen new Rust tests are registered: nine model/golden tests, four transport
tests, and five actual MCP-handler scenarios. They cover all 423 truncated
prefixes, wrong profile/nonce, count and relationship failures, atomic publication,
generation reuse, horizon resets, path witnesses, inventory changes, construction
watches, no-I/O terminal retry, source failure, authority, and response budgets.

They have **not been compiled or executed in this editing environment**, which
has no Rust compiler, Cargo, or rustfmt. No Clippy, stdio, full repository gate,
actual DFHack loading, or live-game execution has been established.

The actual native producer did compile against mock DFHack/protobuf interfaces
with both GCC and Clang using C++17, `-Wall -Wextra -Werror -pedantic`. Each run
passed 135 checks and matched the independent Python encoder's 423-byte golden
frame. Exact tested producer SHA-256:

```text
ecb555296d3b84111f2acd5290c48797027c12a830d7bb4d2ffdec4e3aa4acb7
```

Reproduce that source-level check without a game:

```bash
python3 scripts/test_live_operations_native_mock.py --compiler g++
python3 scripts/test_live_operations_native_mock.py --compiler clang++
```

Mock interfaces do not prove compatibility with real generated DF headers,
protobuf generation/linking, actual plugin loading, or native suspension behavior.
No registry, floor, artifact, or runtime admission changed. The shared framing
module gained only registration of the closed operations client; existing profile
constants and implementations were preserved, not used as qualification evidence.

Still absent: citizen/announcement integration into this coherent generation,
map/path observations, complete material-requirement evaluation, labor eligibility,
automatic blocker diagnosis, durable monitoring/history, and live mutations.
