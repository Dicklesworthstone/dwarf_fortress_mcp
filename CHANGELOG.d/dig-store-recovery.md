### Coordinate mining intents and expose offline recovery discovery

The existing dig/1.16 developer start path now holds private-directory custody,
durably registers each immutable intent before native preparation, and refuses
new work while any retained intent lacks verified terminal proof. New filenames,
keys, endpoints or native incarnations cannot bypass this same-directory guard.
Missing/substituted registered intents, torn registry appends and corrupt evidence
fail closed without eviction or repair. Valid legacy intents are conservatively
adopted; distinct fresh work remains possible after prior obligations resolve.

Add the actual `records --directory ...` CLI command with native-free, bounded,
snapshot-pinned pagination, source/plan identity and unresolved counts. Preserve
one-shot commit, terminal offline recovery and all native/operator checks.

All 51 executable Python/loopback/POSIX/CLI groups pass, including 14 new store
groups and cross-process locking. Two new tests fail against pre-store start.
Uploaded implementation/test hashes match locally tested bytes. This is directory
coordination, not a global lease, production MCP integration or live-game
qualification. See docs/DIG_STORE_RECOVERY.md for scope and recovery commands.
