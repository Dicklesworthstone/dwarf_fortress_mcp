# Python inventory-to-placement handoff core

The pure `furniture_handoff` module is implemented and its 18 focused Python tests
pass. It retains all original request constraints and exact selected item/source
identities in a bounded immutable artifact. It does not itself authenticate a
native read, create batch custody or execute placement; those integrations remain
outstanding in this first increment. This is Python semantic execution only, not
Rust/MCP, native DFHack, live-game, full-workspace or production qualification.
The production runner map and empty compatibility registry are unchanged.
