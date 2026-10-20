# Original-order delivery-aware allocation

- Add exact whole-delivery item matching for the original canonical order,
  preserving every dependency and requested selector byte-for-byte.
- Route after all prior targets are removed, retain complete Hall shortages and
  independently replay every chosen delivery before returning a full plan.
- Execute 15 Python tests, including exhaustive assignment/route oracles, 32 slots,
  65,536 items, bounded work and cancellation. No native or admission changes.

## Allocation and immutable-review integration

- Expose --fixed-delivery-order in existing allocation/new-review commands;
  reject sequencing/search conflicts and all saved-review replanning pre-contact.
- Preserve exact original requests and selected-item review semantics, existing
  native read ownership, aggregate work reservations and final authority checks.
- Execute 29 distinct Python tests, four weakened-source checks and four legacy
  packet comparisons. Native acquisition/budget/roster boundaries are injected;
  no real-network, native-game, Rust/MCP or full-workspace qualification claim.
