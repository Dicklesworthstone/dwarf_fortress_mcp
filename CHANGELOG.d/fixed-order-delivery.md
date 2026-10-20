# Original-order delivery-aware allocation

- Add exact whole-delivery item matching for the original canonical order,
  preserving every dependency and requested selector byte-for-byte.
- Route after all prior targets are removed, retain complete Hall shortages and
  independently replay every chosen delivery before returning a full plan.
- Execute 15 Python tests, including exhaustive assignment/route oracles, 32 slots,
  65,536 items, bounded work and cancellation. No native or admission changes.
