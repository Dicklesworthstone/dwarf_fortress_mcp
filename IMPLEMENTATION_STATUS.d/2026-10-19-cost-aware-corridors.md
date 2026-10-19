## Cost-aware corridor candidate comparison

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. Broad workflow remains open.

Added a bounded cost-first corridor candidate compared against the actual
existing connector. Only complete tile/part-feasible candidates can win; exact
original floors, protected cells and entries remain. Binary costs are assumptions,
not terrain observation or mutation authority. Global cost optimality is not
claimed. One work allowance covers both attempts and complete verification.

Executed 18 warnings-denied component tests with full original module imports,
including an independent oracle over 2,187 cost/barrier worlds and cancellation
at every checkpoint of a complete alternative. A free five-cell passage replaces
three charged cells; a three-cell cap instead retains the original route. Part
capacity likewise cannot be relaxed. Invalid types, original scope, exact work
exhaustion, deterministic ties, witnesses and no-effect claims are covered.

Original connector and solver bytes were verified as Git blobs
5ed92e085209fafc0f349c71cdb08edbb66b36a9 and
f3f42182206893e94396fa84749bc4f81a53994d before execution. This first increment
is the tested routing component; room/layout composition follows separately.
Native/Rust/DFHack/live and full-repository qualification are not claimed.
