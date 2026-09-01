# Live announcement pagination state machine

```text
Unstarted(after)
  -- first accepted page --> Frozen(after, oldest, latest, high_water, next, truncated)
Frozen(..., next, ...)
  -- accepted continuation with identical window --> Frozen(..., next', ...)
Frozen(...)
  -- complete page --> Complete(window_digest, next)
Any nonterminal state
  -- generation/window/cursor/order/bound failure --> Rejected(no publication)
```

The transition is transactional. Page validation, cumulative bound checks, and cross-page ordering checks all complete before the assembler changes its cursor, record vector, or completion bit. `Complete` is immutable. A caller starts a new assembler to observe reports appended beyond the frozen high-water mark.