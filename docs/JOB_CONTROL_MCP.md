# Job-control MCP integration status

## Recovery foundation

The isolated job-control/1.9 library now provides `JobControlJournal::summary`
and `records_page`. Both require current Query authority and valid file custody.
A page holds at most 64 complete records, obeys caller row/byte/time bounds, and
includes terminal records as well as unfinished work. Ordering is ASCII key order.
`expected_head` binds discovery to one journal generation; `next_after` is the last
returned key only when more records remain. The MCP layer must additionally bind
its opaque continuation to the session and journal identity.

Summaries distinguish Prepared, reconciliation-required, and terminal counts.
They describe retained coordination evidence, not the current game or completed
production goals. Lookup, unresolved discovery and cached terminal mutation replay
also validate custody; they do not silently promote cached evidence after ownership
changes. Identity checks are not a full reread or protection against a malicious
same-user rewrite of already-cached bytes.

`open_private_job_reconciliation` opens an EXISTING private, exclusively locked
journal with Query authority and a writable descriptor. It neither creates a new
file nor repairs existing bytes. Its journal can append reconciliation evidence;
prepare/commit/cancel still require independently supplied ConfigureProduction on
every call. This opener grants no capability. `open_private_job_recovery` remains
read-only and cannot append even with a later mutation grant.

Six Rust regression groups are registered in `discovery_tests.rs`. They have not
been compiled or executed; Rust, Cargo and rustfmt are unavailable. Source identity
checks are not Rust, filesystem, native DFHack, live-game or admission evidence.
The existing native protocol, journal bytes and production runner map are unchanged.

## Remaining source integration

The next increment connects this recovery API and the sealed native job plans to
the eleven-tool MCP loop in a separately gated, unadmitted development runtime.
No MCP job mutation route is exposed by this recovery-foundation increment alone.
