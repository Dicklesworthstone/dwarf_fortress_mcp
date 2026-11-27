# Order-run predicate opportunity

Standalone and original-order MCP reviews now expose `predicate_window`.
Its earliest tick is the starting tick plus the required number of sampling
intervals, or null outside the representable native clock. A sample exactly at
the tick cutoff loses to that cutoff. The boolean reports a timing opportunity.
Only native receipts report whether the predicate was observed.

One-tick intervals remain available for separately sampling the parent goal.
Regression coverage includes equality, a sample at the maximum native tick,
and samples outside native time. The changed Rust sources were syntax-parsed.
Rust compilation, runtime tests and live-game qualification remain pending.

The Python operator's `review` uses one order observation and needs neither a
journal nor clock authority. Stored Python summaries derive the same window
from the original plan. In an isolated hosted Python interpreter the timing
calculation agreed with a tick-by-tick oracle on 365 cases. The actual CLI
review branch and plan codec produced the four- and five-tick boundary results
with injected arguments, a transport double and standard-library substitutes.
The order receipt decoder also accepted five unknown-date stop/recovery cases
and rejected three contradictory receipts. The POSIX/TCP suite and live game
remain unexecuted in this session.

An independent source-level comparison of all 60 excavation phase/reason pairs
found agreement across native, Rust and Python after the pending-pause fix.

Owners: `df-combined-order-clock-c8m` and `dfmcp-reality-wiring-proof-4bf.5`.
