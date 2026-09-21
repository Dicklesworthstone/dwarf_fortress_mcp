# Coordinated mining intents and offline recovery records

The existing dig/1.16 developer client now coordinates all intent capsules in
one private directory. This supersedes the statement in DIG_TERMINAL_RECOVERY.md
that the client does not coordinate multiple capsule files. Its terminal-proof
contract still applies. The earlier DIG_DESIGNATION_CLIENT.md description of
per-capsule custody is extended, not a separate native implementation or MCP tool.

## Mandatory coordination for start

Keep dig intent records in a dedicated, owned directory with exact mode 0700.
The existing --record argument selects its parent as the coordination scope;
there is no separate opt-in that can accidentally omit the store check. Intent
filenames must use 1-128 ASCII letters, digits, dots, underscores or hyphens;
`.` and `..` and the `.dfmcp-` namespace are reserved. Names and native operation
keys cannot be reused within this store, including after a terminal outcome.
Unrelated files, directories and orphan receipt sidecars are refused, not ignored.

The client holds a nonblocking exclusive lock on the actual directory descriptor
through observation, preparation, the sole commit attempt and outcome retention.
It does not depend on a replaceable lock-file name. Before native preparation,
`.dfmcp-dig-registry.jsonl` durably registers the exact capsule filename and full
capsule SHA-256 in a closed, canonical append-only hash chain. Registry file and
parent-directory fsync must both succeed before proceeding. The original intent
still has its independent file/parent synchronization before registration.

Every start audits the complete bounded store. Any intent without verified
terminal proof blocks new work, including under a different filename, operation
key, endpoint or native incarnation. After a lost reply, restarting the client
or choosing a new record path in this directory therefore cannot redispatch.
The retained registry also detects missing or substituted registered capsules.
Valid legacy or interrupted, unregistered capsules are conservatively registered
before new-work admission; they are never discarded as irrelevant or considered
safe merely because native preparation might not have started.

The store rechecks registry, file identity, contents and other pending obligations
before both PrepareDesignation and CommitDesignation. The existing strict region,
halo, source, witness, confirmation, runtime deadline and one-shot native checks
remain authoritative. A replayed preparation never becomes a commit opportunity.
Historical terminal records remain intact when a distinct, newly reviewed plan
is admitted. Resolving an obligation does not authorize replay of its old key.

## Discover and recover

```sh
# No token, environment opt-in or native connection is needed for discovery.
python3 scripts/dig_designation_client.py records --directory /private/dig --limit 8

# Continue only against the same exact store snapshot and page size.
python3 scripts/dig_designation_client.py records --directory /private/dig --limit 8 --continuation '<returned token>'

# Inspect the original intent and any retained historical terminal proof.
python3 scripts/dig_designation_client.py inspect --record /private/dig/original.json

# Explicit native recovery, when no terminal proof has yet been retained.
python3 scripts/dig_designation_client.py query --record /private/dig/original.json
python3 scripts/dig_designation_client.py cancel --record /private/dig/original.json
```

`records` returns compact names, keys, region, source manifest, endpoint, plan,
intent identity, registration state, terminal receipt identity, total/unresolved
counts and a continuation. It never creates or appends a registry and never opens
a native connection. As with offline inspect, loading known terminal proof
re-verifies and re-syncs that existing proof before acknowledging it; no receipt
contents are rewritten. Old unregistered capsules remain discoverable.

A continuation binds the canonical directory, pinned directory identity, exact
registry, complete record/receipt snapshot and page size. Changes invalidate it;
records from different snapshots are not silently combined. Page size is 1-8,
complete output is bounded to 128 KiB, and the default cooperative inspection
budget is 10000 ms, configurable with --timeout-ms in the range 1-60000. This is
not a promise to interrupt a blocked kernel filesystem operation at a hard time.

Unknown recovery still requires the existing explicit developer opt-in and
credentials, exact saved endpoint/incarnation and, for cancel, separate operator
designation permission. Query and cancel never send CommitDesignation. Once a
valid terminal receipt exists, inspect/query/cancel return its historical proof
with zero native calls even when credentials or the original game are unavailable.
Missing native records or source changes do not prove non-application and cannot
clear an unresolved obligation. Cancel is not undo.

## Bounds and failure behavior

A store retains at most 128 intent records, with at most 257 directory entries
including terminal sidecars and its registry. The registry is at most 128 KiB;
individual intent and terminal bounds remain 64 KiB and 2 KiB. There is no automatic
eviction, truncation, repair, overwrite, key reuse or reset-on-error path. A torn
registration or corrupt receipt is retained and refused. Investigate such evidence;
do not erase it or switch directories as a way to retry an uncertain operation.

This is a private-directory coordination boundary, not a global game/controller
lease. Other directories, older clients, other plugins and UI input are not fenced.
The hash chain detects inconsistent retained evidence, not an owner deliberately
rewriting all evidence or restoring/deleting the entire directory; there is no
external rollback anchor. Terminal designation proof is historical configuration,
not present terrain, completed excavation, safety, quiescence or permission to
advance the game clock. Native wire, dependencies, Rust authority, MCP routing
and production admission are unchanged.

## Executed regression tests

```sh
PYTHONPATH=scripts python3 -m unittest test_dig_designation_client test_dig_terminal_recovery test_dig_store -v
```

All 51 groups pass: 23 existing client groups, 14 terminal recovery groups and
14 store coordination groups. They execute actual Python, fragmented loopback
TCP, subprocess CLI, private POSIX files and cross-process directory locking.
Fault cases cover lost replies followed by attempted new filenames, remembered
missing/substituted intents, registry header/entry and receipt file/parent sync
failures, partial appends/writes, mid-prepare registry corruption, immutable
history, native-proof forgery, bounded discovery and stale/cross-store cursors.

Two new store tests were also executed with the pre-store start implementation;
both fail their assertions, detecting unresolved-work bypass and missing new-intent
registration. The final implementation passes them. All five uploaded source/test
blob identities match the locally executed files. Python syntax checks pass.
Rust/Cargo, real DFHack SDK/plugin execution, a live fortress, full repository
qualification and power-loss durability have not been established by this work.
