### Durable developer order-condition control and recovery

Add a closed order-run/1.14 Python wire adapter and two-stage developer client:
explicit fortress/order selection, durable prepare, reviewed-digest commit,
one-query reconciliation, cancellation and offline receipt discovery. Sync an
append-only dispatch marker before unpause; never replay it after loss or restart.
Retain native predicate and pause evidence separately, preserve uncertainty and
block new keys while unfinished/source-lost work remains in the journal.

Twenty-eight test groups execute codec/coordinator, real POSIX custody and joined
loopback RPC doubles. Both GCC and Clang produce 15 matching actual-engine/encoder
vectors decoded by Python. Include sync/partial-write failures, uncertain recovery,
corruption, rehashed illegal states, wrong sources, absent records and no redispatch.
This is developer functionality, not Rust/MCP, real SDK/live-game, power-loss,
full qualification or admission evidence. Existing native generations are unchanged.
