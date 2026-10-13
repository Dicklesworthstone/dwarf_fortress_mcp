# Original route constraints at the placement frontier

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` and
`df-franken-storage-mvcc-54h.4`.

`route_furniture_frontier.assess` connects an immutable original route review to
an exact fixed-order prefix of native furniture/1.19 Placed receipts. Each receipt
must preserve the original selected item, native type, material, subtype, target,
source and batch-derived key. Prepared, indeterminate, refused, cancelled,
foreign, reordered, truncated or corrupt records cannot unlock the next step.
Every predecessor's native sequence, tick and job/building ID horizons are checked
against the next capture. Map and operations generations remain separate from
one another and from the native placement generation.

Earlier verified insertions are reported as `historical_placed`, not as current
loose inventory or completed construction. Every remaining original selection
is revalidated against the complete candidate inventory and current captured
route graph. No solver chooses replacement items. The next native capture must
agree with that inventory's paused tick, job/building horizons, map dimensions
and selected item's position. The full original plan remains visible as history.

The checker returns `ready`, `route_blocked`, or `route_unestablished`; none is
permission to dispatch. Evidence digests cover the original review, complete
remaining result, receipt prefix, native capture and source identities. They do
not vary with a narrowed computational allowance when the evidence is identical.
One bounded allowance covers receipt checks and the existing full revalidation.
Results are at most 96 KiB. No private storage, native call, mutation or production
admission is provided by this pure module.

## Executed development evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_frontier -v
```

All 14 actual Python tests pass. Coverage includes post-placement inventory,
blocked later routes, exact receipt identity, every receipt truncation and byte
corruption, source/clock/horizon differences, no replacement, exact shared work
bounds and every cooperative callback failure. The five unchanged dependencies
were reconstructed and checked against their exact Git blob identities before
execution. This is not a live DFHack, Rust or full-workspace qualification.
