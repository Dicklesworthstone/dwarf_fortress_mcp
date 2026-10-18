## Original furnishing batch reconciliation

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`; broad execution remains open.

Fixed an executable recovery dead end: `furniture_batch.recover` treated the
presence of any retained receipt as an offline terminal result, while the
existing placement recovery owner correctly requires a query for Indeterminate.
The batch now uses that same query/cancel predicate when acquiring Query-only
authority. Completed/refused/cancelled receipts remain offline. Cancelling an
uncertain attempted placement remains an offline no-op, never nonapplication.
The original read authority is retained through final batch/CLI serialization
for all batch profiles, not just room-backed definitions.

Exact-item batch imports no longer eagerly depend on room planning, inventory
observation or terrain monitoring. Explicit handoff formats load their original
validators after bounded format/field validation. Named compatibility exports
resolve to the same original types/constants; no missing-validator fallback or
weaker handoff decoding is introduced.

Executed 16 tests with warnings denied using ordinary full-module imports, real
private files, original batch and placement owners, canonical receipt/journal
codecs, actual TCP clients, a joined synthetic native peer, and a real CLI
subprocess. No AST-extracted function bodies, storage/client replacements or
room-import adapters were used. The original failing case was reproduced before
the recovery predicate was fixed.

Cases include query with placement disabled, same-key positive reconciliation,
unchanged/absent/contradictory/malformed evidence, lost commit acknowledgment,
changed native generation, stopped-batch recovery, exhausted RPC allowance,
revoked authority during query and rendering, and lost sync acknowledgment.
The original uncertain frame remains byte-preserved before the appended proof;
reopening resolved work remains offline and never repeats a placement.

Six unchanged native/plan dependencies were checked against their fetched Git
blob identities, as was the original batch module before modification. Python
syntax and whitespace checks pass. This is exact-item Python/TCP/private-file
component evidence with synthetic native data. Room/terrain-backed end-to-end,
DFHack producer builds, Rust workspace, live fortress, full-repository and
production-admission qualification were not executed. Native wire and durable
journal formats are unchanged.
