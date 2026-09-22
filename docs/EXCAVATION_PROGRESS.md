# Read-only excavation goal evidence

`scripts/excavation_observer.py` reads the unchanged map/1.5 native profile and
implements a pure sampled dry-floor goal evaluator. It supplies the measurement
needed after mining designation, rather than interpreting a designation receipt
as finished excavation. It is not an MCP route or a replacement mutation journal.

The goal is exact: every cell in one 1..8 by 1..8 rectangle at one z-level must be
visible, have normalized shape FLOOR, zero liquid depth and no dig designation.
Wall, empty space, ramp, stairs, unsupported shapes, wet floors and designated
floors do not satisfy it. Missing and hidden cells remain unknown with no attribute
payload. Occupancy, walkability, temperature and structural safety are not part of
this goal. A constructed floor may satisfy it; causal attribution to mining is
explicitly not established. Neither success nor failure authorizes another dig.

The evaluator records first/latest source evidence, matching sample count and the
first matching tick. Only observations at strictly advancing game ticks increment
the matching count. Both the sample-count and game-tick-span requirements must be
met, at or before the fixed deadline. Unknown/contradictory samples and explicit
interruptions reset the streak. A gap greater than the declared maximum also
resets it. This proves sampled endpoints, not continuous stability between them.
Source generation, software, fortress, dimensions or clock regression invalidates
the goal rather than manufacturing progress. Terminal results are immutable.

The transport binds only Handshake and ReadObservation. Numeric IPv4 loopback,
32..256-byte credentials, minimal protobuf, exact nonce/profile/selection, strict
UTF-8 and closed field values are checked. One connection carries one capture,
with an absolute 1..60000 ms deadline and 4 MiB cumulative wire allowance. No
reconnect loop, detached worker, native setter, dig prepare/commit or game-clock
control exists. Evidence hashes bind the source manifest as well as capture bytes;
map/1.5 generations are not silently treated as dig/1.16 generations.

## Executed tests

`PYTHONPATH=scripts python3 -m unittest test_excavation_observer -v` passes all
14 groups against the actual Python implementation. Tests include fragmented real
loopback TCP, fencing after malformed/lost replies, the unchanged 455-byte native
fixture (Git blob a2fcaae9b2fd4519241618f68b8aad612601e9bb), all 576 combinations
of normalized shape/liquid/dig state, hidden/missing evidence, tick/stability/
deadline/source guards, malformed/truncated captures and the 16384-cell map ceiling.

This is Python/loopback evidence, not a real DFHack SDK or live-fortress campaign,
Rust/MCP execution, power-loss durability or repository qualification. Native
protocols, dependencies, production admission and existing control/recovery paths
are unchanged. Durable goal workflow and an operator CLI are separate integration
work; the pure observer never clears an unresolved native-effect obligation.
