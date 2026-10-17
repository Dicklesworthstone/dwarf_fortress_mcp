## Concurrent original dig-batch revocation

Beads: df-dfhack-bridge-plane-c-pic.4/.5 and
`df-cx-authority-budget-threading-lmu`; broad execution work remains open.

The standalone stop requester now publishes the existing original-batch stop
marker without acquiring the executor's directory or manifest locks. It verifies
canonical private manifest bytes and the original effects inode, then creates
or re-syncs only the exact immutable stop artifact. This narrows future work,
not current native authority. Running owners fail at the next custody check;
reopened owners retain their stopped state. Already dispatched native effects
may complete or lose acknowledgment and require original-key reconciliation.

Executed 29 tests against the actual new Python implementation, including real
private files, synchronization failures, subprocess CLI execution while another
process holds both owner locks, and SIGKILL during publication. The manifest
fixtures test identity-only revocation, not semantic room/terrain admission.
Existing running-owner compatibility is source-reviewed, not executed in this
increment. No native/Rust/live or full-repository qualification is claimed.
No production admission, native wire, journal framing or effect setter changes.
