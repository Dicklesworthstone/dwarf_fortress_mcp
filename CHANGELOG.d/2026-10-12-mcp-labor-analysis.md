# Observed staffing analysis through MCP

`fortress.query` now accepts `labor_allocation`: inspect current labor settings,
identify missing evidence or staffing deficits, distinguish already-correct
configurations, and return a complete guarded workflow for explicit plan/commit.
One work budget spans reads, allocation and compilation. The handoff is an atomic
JSON string under output fitting. Ten Rust regressions added, unexecuted here;
Python structural schemas/example/mutation/handoff checks pass. No native or
compatibility authority changes.
