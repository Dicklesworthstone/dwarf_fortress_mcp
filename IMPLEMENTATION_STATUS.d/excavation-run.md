# Excavation-conditioned clock control: engine evidence only

`docs/EXCAVATION_RUN.md` specifies the new bounded native-independent C++ engine.
It turns a strict dry visible floor condition into a native stop trigger, preserving
sample stability, source identity, cancellation and failed-pause ownership.
Both GCC and Clang execute 16 scenarios / 7030 assertions with warning denial and
UBSan; four separately compiled mutants fail. The existing clock header is exact.

There is not yet an RPC, Rust adapter or MCP route for this increment. It supplies
neither durable intent custody nor global controller exclusion, game checkpoints,
mining causality, structural safety or production admission. No real SDK/plugin,
live fortress, physical power loss or full qualification was exercised. Work items
`df-dfhack-bridge-plane-c-pic.4/.5` and `df-action-coordinator-exec-ero.4` stay open.
