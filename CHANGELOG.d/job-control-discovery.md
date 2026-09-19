# Bounded job-effect discovery and least-authority reconciliation

Add custody-checked summaries and complete keyset pages over all retained job
interventions, including terminal/cancelled effects. Pages require an exact
journal head and fail stale continuations rather than skipping changed records.
Cached lookup, unresolved-work reads and terminal mutation replays recheck custody.

Allow Query-authorized reopening of an existing writable journal solely to make
reconciliation possible without granting production authority. The private-file
opener cannot create a missing file or repair a tail. Every preparation, dispatch
and cancellation still independently requires ConfigureProduction; no grant is
manufactured. Offline recovery retains a genuinely read-only descriptor.

Six Rust regression groups are registered for pagination, reopen, budgets, custody,
cancellation, authority and Query-only reconciliation. They are NOT compiled or
executed: Rust/Cargo/rustfmt are unavailable in this environment. Local source
bytes were checked against their uploaded Git blob identities. No native wire,
journal encoding, dependency, production runner or compatibility admission changes.
