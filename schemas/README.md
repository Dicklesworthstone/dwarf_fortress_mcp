# JSON Schemas

`dfmcp.schema.json` is the phase-zero design contract for the 11-tool narrow waist. The per-tool
files are convenience entry points that reference the shared definitions. They are strict by
default: unknown top-level mutation fields are rejected, authority and digest fields are never
defaulted, and recursive values are bounded by runtime budgets in addition to schema limits.

These schemas describe the intended protocol and support review/example validation. They do not
claim that a live MCP server is implemented yet.

`lab_query_comparison.schema.json` describes the decoded `mode` string for the
default laboratory runtime's `query_changes` operation. It references the
existing entity-selection definitions in `mcp_query_v1.json` (`urn:dfmcp:query:1`)
and is also discoverable at `query.input.schema.json#/$defs/laboratory_query_changes`.
Its exact retained hashes, current authority and epoch checks, distinct-source
work budget and complete-page behavior are documented in
[`LAB_QUERY_COMPARISON.md`](../docs/LAB_QUERY_COMPARISON.md). This runtime mode
does not change the phase-zero root envelope or add a top-level tool.

`lab_query_baselines.schema.json` describes the decoded `mode` string for the
default laboratory's `temporal` baseline lifecycle: capture, changes, list and
explicit release. It reuses the same `mcp_query_v1.json` selection and anchor
definitions and is discoverable at
`query.input.schema.json#/$defs/laboratory_query_baselines`. Session ownership,
exclusive deadlines, restore invalidation, complete publication and bounded
process-local retention are documented in
[`LAB_QUERY_BASELINES.md`](../docs/LAB_QUERY_BASELINES.md). These operations add
neither a root tool envelope nor a native protocol, watch or durability claim.
