# Durable terminal recovery for the dig/1.16 developer client

This extends `scripts/dig_designation_client.py`. It supersedes the statements in
`DIG_DESIGNATION_CLIENT.md` that every offline inspection must report unknown and
that terminal evidence is never persisted. Existing immutable intent capsules,
operator opt-ins, region/halo validation and the native wire remain unchanged.
This is still a developer client, not an MCP wrapper or a production runner.

## Retained outcomes

A successful `start`, `query` or `cancel` now retains a fully verified native
`designated` or `refused` record before acknowledging a known terminal outcome.
The original intent is not rewritten. A private sidecar in the same directory,
`.dfmcp-dig-terminal-<SHA256-of-complete-intent-file>.json`, binds the exact original
capsule, endpoint, source incarnation and plan to the complete native record.

The closed canonical `dfmcp.dig-terminal/1` payload contains `intent_sha256` and
`effect_hex`; its envelope contains `receipt` and the SHA-256 of canonical receipt
bytes. The file is at most 2048 bytes. Loading recomputes the full native proof
and exact predicted terrain/priority/scheduling readback, not merely the outer
checksum. These hashes are corruption commitments, not signatures against an
operator who can deliberately replace all private evidence.

Files are exclusively created, never overwritten, with the existing no-follow,
single-link, owner, exact 0600/0700 mode and inode/content custody checks. Receipt
file and parent-directory fsync both precede acknowledgement, with custody checks
on each side. Offline acknowledgement also re-verifies and re-syncs both; it does
not rewrite the receipt. Identical proof is idempotent. Conflicting, substituted,
partial or corrupt proof fails closed without repair, deletion or native retry.

## Recovery flow

```sh
# After a successful start, this works without credentials or a native connection.
python3 scripts/dig_designation_client.py inspect --record /private/dig/intent.json

# After a lost reply, query the original capsule against its exact native source.
python3 scripts/dig_designation_client.py query --record /private/dig/intent.json

# Retire an uncommitted native preparation; this cannot undo a designation.
python3 scripts/dig_designation_client.py cancel --record /private/dig/intent.json
```

Once terminal proof is retained, all three commands return that historical proof
with `native_calls: 0`, before any environment/credential/native access. They do
not ask a replacement fortress to recreate evidence. Without retained proof,
inspection remains unknown and explicit query/cancel retain their existing source
and operator checks. Prepared, Unknown, missing records and source drift never
become terminal evidence. Recovery never sends CommitDesignation.

A complete proof whose sync or response was lost may be reverified and re-synced
on a later offline invocation. This does not retroactively make the failed call a
durable acknowledgement. An incomplete or corrupt sidecar is retained and refused;
there is deliberately no automatic repair or reset command.

`designated` proves historical configuration, not excavation completion, current
terrain, mining safety, or an ongoing lease. Cancellation is not rollback. This
increment does not coordinate multiple capsule files or directories, grant Rust
Designate authority, add MCP integration, or change production admission.

## Executed regression evidence

```sh
PYTHONPATH=scripts python3 -m unittest test_dig_designation_client test_dig_terminal_recovery -v
```

All 37 groups pass: the existing 23 groups plus 14 terminal-recovery groups.
Tests execute the actual client, fragmented loopback TCP peers, subprocess CLI,
private POSIX custody and fault-injected writes/fsyncs. They cover lost replies,
terminal cancellation, no redispatch, offline recovery, native-proof forgery,
corruption/truncation, source drift, conflict preservation and inode substitution.
The prior offline-unknown assertion is updated to require the persisted proof.
Rust, the real DFHack SDK/plugin, a live fortress, power-loss durability and the
full repository qualification ladder are not established by these tests.
