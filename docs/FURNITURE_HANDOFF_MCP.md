# Furnishing directly from requested slots through MCP

The furniture development server accepts `furniture_request` in
`fortress.open_session`. It acquires one complete operations/1.4 inventory,
selects a globally valid set of distinct items, and retains the resulting
exact batch with the original request and source evidence.

This implementation is **source present, with Rust compilation and process
execution still pending**. The execution backend disconnected after dependency
compiler memory failures. The source was preserved through GitHub from the
reviewed patch context; the reconstructed bytes require formatting and tests.

## Operator configuration

Use the existing furniture/1.19 development configuration, private journal and
batch paths, fortress identity, scope, and placement/checkpoint policy. The
request path additionally requires `DFMCP_OPERATIONS_PAGED_TOKEN` for the
existing operations/1.4 plugin on the same configured loopback endpoint.
`DFMCP_BUILD_TOKEN` authenticates the separate furniture source.

No client-selected paths, endpoints, credentials or policy exceptions are
accepted. New intent requires Control mode and current Plan authority. Recovery
and offline modes reopen original evidence without importing another request.

## Request, selection and placement

Supply a JSON string containing the existing bounded request format:

```json
{
  "schema": "dfmcp.furniture-request/1",
  "world_folder": "region1",
  "site": 2,
  "excluded_items": [],
  "slots": [
    {
      "name": "bedroom-bed",
      "kind": "bed",
      "target": [15, 15, 2],
      "max_distance": 30
    },
    {
      "name": "dining-chair",
      "kind": "chair",
      "target": [18, 15, 2],
      "after": ["bedroom-bed"]
    }
  ]
}
```

`furniture_request`, `furniture_plan` and `selection` are mutually exclusive.
An existing batch must be reopened with all three omitted. A complete request
has at most 32 slots and 16 KiB of JSON. Optional per-slot constraints include
`material: [type, index]`, `subtype` and same-level `max_distance`.

The source path performs one complete inventory acquisition and release, then
an independent furniture bootstrap observation for the first chosen selection.
Endpoint, fortress, software, clocks, ID horizons and selected-item attributes
must agree before original custody is created. The two plugin generations are
preserved separately.

A successful opening returns `session_opened: true` and the complete original
plan under `result.batch`. Its allocation summary retains the request,
handoff, source and capture digests, original operations generation and tick,
and the number of fixed items.

Continue through the existing explicit review loop:

1. Inspect `query: {"mode":"batch"}` and its next original step/key.
2. Call `fortress.observe` with `selection: "next"`.
3. Plan that exact key using the returned observation witness.
4. Review the resulting plan and confirm its exact digest and review seal.
5. Commit once. Recover the original key after uncertainty.

Every fresh capture must preserve the selected item's native type, material
and subtype and remain within the original level/distance constraint. Items
are never automatically substituted. Native eligibility, current authority,
operator scope, protected regions, host lease and checkpoint policy still
apply. Dependencies unlock on retained terminal Placed outcomes.

## Shortage and inspection

An infeasible request returns `ok: true`,
`status: "furniture_shortage"`, and `session_opened: false`. Its full
shortage witness names the deficient slots, candidate items and missing count.
There is no partial batch, placement bootstrap, local placement file creation,
native preparation or placement dispatch. The request can be revised explicitly
after inspecting that result.

Inspect original retained evidence without a native call:

```json
{"mode":"allocation","view":"request"}
{"mode":"allocation","view":"items"}
```

The default view is `request`. The item view includes every chosen item's
original handle, material, subtype, position and allocation distance. These
views remain separate so the full batch and completion inventories can also
fit in the bounded response.

## Restart and completion

Reopening performs no inventory read or reallocation and restores no dispatch
permission. The retained parent is verified before the new session and host
lease are constructed. Its original allocation tick initializes the clock
floor even when the child journal is still empty; otherwise a legitimate
late-game restart would create an already-expired tick-zero lease.

After every original step is Placed, use the existing fixed-goal
`completion_start`, explicit `observe` selection `completion`, and
inspection/cancellation modes described in
[the completion workflow](FURNITURE_COMPLETION_MCP.md). The completion origin
retains the complete allocation-backed batch definition.

Monitor-only recovery can inspect the retained request and item evidence from
its verified monitor copy after the original files are lost. It marks original
allocation custody unverified and cannot allocate, prepare or place anything.

## Tests and limits

The new process suite has nine methods covering constrained global assignment,
shortage without partial effects, malformed and unauthorized requests,
restart without another read, item/source drift, allowed movement, source and
paging disagreements, a full 32-slot multi-page batch, and empty-child restart
at game tick 806500.

Python syntax/import and independent fixture checks ran before the outage:
the scarce-item result selected IDs 43 then 42; one joined listener served
operations generation 987 and furniture generation 7; the wide fixture
contained 2,032 candidates in 95,618 bytes and selected the expected 32 items.

**None of these nine Rust process tests has executed.** The prior 13 completion
process methods and existing placement/batch regressions also need to run
against the new executable. See the exact source references and limitations in
`docs/evidence/furniture-handoff-mcp-source.json`.

The compatibility registry remains empty and the production protocol map
remains protocol 1.0 only. This workflow grants no live admission and does not
establish completed construction or current building usability from allocation
or placement alone.
