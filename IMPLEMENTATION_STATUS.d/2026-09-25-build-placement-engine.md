# Building placement engine

Added an executable C++ one-shot bed/chair/table placement engine over exact
terrain/item captures, with prepared/indeterminate/placed/refused/cancelled
records and an unresolved-placement fence. Placement proof requires the exact
building, construction job and linked item; it does not prove completed furniture.

Both GCC 14.2 and Clang 17 execute 22 groups / 979 assertions and eight independent
byte fixtures under warning denial and nonrecovering UBSan. Four GCC mutation
variants fail regression assertions. Native RPC, Rust/MCP, real SDK and live-fort
integration are not provided by this increment. No production admission changes.
Details: docs/BUILD_PLACEMENT.md. Beads df-dfhack-bridge-plane-c-pic.4/.5 remain open.
