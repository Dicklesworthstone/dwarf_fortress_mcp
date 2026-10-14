## Explicit bounded construction progress for route batches

- Add a separately confirmed route-batch progress workflow that composes the
  current construction-pending frontier with exactly one run/1.13 clock burst.
- Derive durable clock record/key identity from the reviewed confirmation, reserve
  aggregate resources before mutation, and require the same loopback endpoint.
- Reobserve route and construction evidence only after an exact pause-verified
  stopped receipt. Never place furniture or automatically run another burst.
- Add query-only lost-reply recovery for the original run key; it never prepares
  or commits another clock effect.
- Eight source regressions are present but unexecuted in this environment. No
  protocol, MCP, native qualification or production-admission change.
