# Joint readiness TCP reader: 10 tests executed

Ten actual-client TCP methods passed on Python 3.13.5, with joined fragmented
three-profile fixtures and the unchanged upstream construction transports.
The 32-slot/646-exclusion case read a multipage inventory with 2,000 extra items
and verified every receipt before and after both map endpoints. Faults covered
lost replies, malformed framing, source drift, release disagreement, revocation,
shared budget exhaustion and connection closure before local validation.

These are synthetic native-protocol tests, not live DFHack, native SDK/ABI,
Rust/MCP or full-repository qualification. Original placement receipts use the
production codec; map and operations payloads are independently assembled.
The durable original-batch consumer and terrain-bound batch/4 integration are
separate increments. No production admission or mutation authority is added.
Reproduce: PYTHONPATH=scripts:tests python3 -m unittest test_room_readiness_rpc -v
