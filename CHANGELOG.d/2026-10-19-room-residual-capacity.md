## Capacity-aware remaining excavation

Opt-in cost-layout searches now reject complete candidates whose exact remaining
wall mask exceeds the original blueprint/native-step capacities and continue to
another complete layout. Search and terrain reduction share the same partition
policy; no partial plan, relaxed target set, budget renewal or mutation is added.
See `docs/ROOM_RESIDUAL_CAPACITY.md`.

Observed layout now derives and independently rechecks the residual partition
from original map bytes. The padded one-capture workflow validates the final
terrain reduction before exporting its original-room excavation handoff. Its
strict optimality option applies to all nine exports without widening authority.
