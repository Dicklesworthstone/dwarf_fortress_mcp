# JSON Schemas

`dfmcp.schema.json` is the phase-zero design contract for the 11-tool narrow waist. The per-tool
files are convenience entry points that reference the shared definitions. They are strict by
default: unknown top-level mutation fields are rejected, authority and digest fields are never
defaulted, and recursive values are bounded by runtime budgets in addition to schema limits.

These schemas describe the intended protocol and support review/example validation. They do not
claim that a live MCP server is implemented yet.
