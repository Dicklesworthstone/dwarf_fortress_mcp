# DFHack bridge protocol

`dfmcp.proto` is the proposed version-one boundary between the safe-Rust coordinator and a minimal
DFHack-side bridge. It is a design contract, not generated production code yet.

The bridge exposes typed read groups and allowlisted semantic actions. It deliberately does **not**
expose arbitrary Lua, shell commands, DFHack command strings, raw memory writes, or caller-selected
filesystem paths. Mutations use prepare, commit, operation lookup, and cancellation so a lost
transport receipt can be reconciled without blind replay.

Before implementation, the protocol still needs:

1. golden encoding vectors and explicit maximum field sizes;
2. deterministic serialization rules for every signed/digested message;
3. authentication and local transport binding;
4. generated Rust and DFHack-side code selection;
5. compatibility probe fixtures for supported DF/DFHack tuples;
6. fuzzing and partial-frame/resource-exhaustion tests.
