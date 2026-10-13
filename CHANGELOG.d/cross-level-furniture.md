## Supply furniture across observed floors

- Add explicit per-slot vertical endpoint limits to the route allocator; use
  observed complementary stairs instead of discarding all other-floor supply.
- Preserve default same-floor canonical bytes and reject vertical opt-in in the
  terrain-free Manhattan allocator. Keep material/type/exclusions and route bounds.
- Retain and recheck original limits through reviews and receipt-backed placement
  frontiers, without substitution, skipped pending work or new placement authority.
- Execute 26 Python component tests and six assertion-killed mutants, including
  81 stair pairs, 180 global-assignment oracles and original-prefix recovery values.
  No native transport, private batch parent, Rust/MCP or live-game qualification.
