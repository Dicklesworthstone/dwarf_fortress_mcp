## Order-run/1.14 safety pause with an invalid game date

The native clock reader preserves the original fortress pause observation when its year or within-year tick is outside the DF clock range. Order capture still requires a valid date. A same-source verified safety pause can therefore publish a terminal receipt with an unknown tick. A failed setter retains the existing owner, and a changed site fences that setter.

The native RPC fixture adds four invalid-date cases and two recovery cases. These cases were source-reviewed. Native compilation, test execution, live gameplay and qualification remain open.

Owning beads are `df-combined-order-clock-c8m` and `df-dfhack-bridge-plane-c-pic.5`.
