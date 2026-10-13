# Route-aware furniture assignment — Python development checked

The new allocator combines complete map/1.5 decoding and bounded candidate routes
with the existing global exact-item assignment solver. All original constraints
survive. Unknown target terrain is not zero supply; shortages never produce
partial executable plans. Sixteen actual Python core test functions pass.

An executable query-only command now acquires equal paused map/1.5 endpoints
around one complete released operations/1.4 inventory on the same TCP connection,
then closes before projection and solving. Existing paging and roster decoders
are reused. The full invocation shares wall, network, call, work and output bounds.
Nineteen integration functions pass using independently encoded loopback traffic
and real CLI subprocesses, including source/authority/timeout/release failures.

All 35 functions ran against the actual Python implementation. The network peer
is not DFHack; no real native plugin build or live game was exercised. No Rust/MCP
route, native/whole-workspace qualification, compatibility registry, mutation
authority or production admission is added. Reports do not retain future route
constraints in legacy placement plans. Broader beads
`df-franken-storage-mvcc-54h.4` and `df-dfhack-bridge-plane-c-pic.3/.4/.5` stay open.
