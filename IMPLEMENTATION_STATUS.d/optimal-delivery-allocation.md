# Complete furniture delivery-cost optimization and CLI integration

Implemented opt-in branch-and-bound assignment/order planning and integration
with allocate_route_furniture.py and NEW review_route_furniture.py create. Cost
accounts for preceding target removal. Matching bounds prove numeric optimality
or expose an explicit gap; strict mode exports no unproven plan/review. Existing
first-feasible search, original-review checking and native effect paths remain.

Executed: 39 actual Python tests with warnings denied, including 512 obstacle
masks and 200 random domains against independent exhaustive numeric oracles,
32-target review export, 65,536 candidates, work/cancellation/source/authority
checks, real input files and CLI-main subprocesses. Three weakened variants fail.
Five legacy modes match exact predecessor packets for a shared fixture.

Acquisition, operations decoding and the transport budget/endpoint are injected
in runtime tests. No native TCP/paging, real DFHack, SDK/plugin, Rust/MCP, live
fortress, full-workspace or production admission is established. The original
joint planner and review codec remain byte-identical. Broader beads stay open.
See docs/OPTIMAL_DELIVERY_ALLOCATION.md and docs/OPTIMAL_DELIVERY_WORKFLOW.md.
