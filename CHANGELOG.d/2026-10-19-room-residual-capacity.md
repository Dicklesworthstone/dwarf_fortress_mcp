## Capacity-aware remaining excavation

Opt-in cost-layout searches now reject complete candidates whose exact remaining
wall mask exceeds the original blueprint/native-step capacities and continue to
another complete layout. Search and terrain reduction share the same partition
policy; no partial plan, relaxed target set, budget renewal or mutation is added.
See `docs/ROOM_RESIDUAL_CAPACITY.md`.
