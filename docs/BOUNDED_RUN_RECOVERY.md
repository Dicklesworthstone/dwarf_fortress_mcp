# Bounded-run discovery and query-only recovery

`scripts/recover_bounded_runs.py` adds executable offline inventory and a bounded
foreground reconciliation pass over existing run/1.13 intents and terminal
sidecars. It uses the original native client and the receipt custody described
in `BOUNDED_RUN_OUTCOMES.md`. It never creates a run, prepares, commits, cancels,
unpauses, repairs files or starts background work. This is not a Rust/MCP route
or production admission.

## Discover unresolved runs without transcript memory

Use a dedicated existing owned exact-mode 0700 directory containing only run
intents and their optional `.outcome.json` sidecars. Files remain owned exact-mode
0600 regular single-link files. Original intent filenames must match
`[A-Za-z0-9][A-Za-z0-9_.-]{0,191}`; the sidecar suffix is additional.

```sh
# Offline: no credentials, opt-in or native connection required.
python3 scripts/recover_bounded_runs.py inventory \
  --directory /private/dfmcp-run --selection pending

# Copy inventory_digest from the complete inventory packet into this variable.
# Native recovery requires the existing explicit run/1.13 opt-in and credential.
python3 scripts/recover_bounded_runs.py reconcile \
  --directory /private/dfmcp-run \
  --expected-inventory "$INVENTORY_DIGEST" \
  --max-queries 8 --timeout-ms 10000

# Inspect one exact retained native receipt without contacting the game.
python3 scripts/bounded_run_client.py inspect \
  --record /private/dfmcp-run/run-001.json
```

Inventory returns all-domain counts even when the current page contains no pending
run: total, unresolved, query-required, operator-attention and terminal-resolved,
plus up to eight pending names and the number omitted. It supports `all`,
`pending` and `terminal` selections with 1..64 whole rows per page, default 16.
Source-lost records remain unresolved operator work, not successfully stopped
runs. Their terminal native history is retained without repeatedly querying it.
Missing native evidence remains query-required; it never permits an unpause retry.

Continuations bind the exact directory identity, complete file-byte inventory,
selection, page limit and offset. Changed receipts, directory replacement,
filter/limit substitution, invalid offsets and checksum failures invalidate them.
The inventory digest is a consistency witness, not authorization or an external
anti-rollback root. Empty results describe this complete local directory only;
they do not prove the absence of runs in other stores or controllers.

## One bounded reconciliation pass

The caller must supply the exact inventory digest from a prior inventory read.
All files, duplicates, selected endpoints, output allowance and current authority
are checked before native work. Up to 1..16 query-required intents are selected
in canonical filename order, default eight. Each is queried at most once, over
one foreground connection with no reconnect. A dedicated transport rejects every
application dispatch except Handshake and QueryRun. Method binding uses the
unchanged native metadata and does not invoke the bound methods.

Only these environment variables may have DFMCP-prefixed names:

- `DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1`;
- `DFMCP_RUN_TOKEN`, 32..256 UTF-8 bytes matching the existing native plugin;
- optional numeric IPv4 loopback `DFMCP_RUN_ENDPOINT`, default 127.0.0.1:5000.

Other development mutation credentials and production admission environment state
are rejected. The exact settings are checked again before every native query.
All selected intents must match the operator endpoint. Credentials are never
retained in evidence or emitted in packets.

A single cooperative allowance of 1..60000 ms, default 10000, starts before
inventory loading and includes connect, handshake and all queries. Native I/O
uses its shrinking absolute deadline; a new query cannot renew it. A small
finalization reserve does not make synchronous filesystem operations forcibly
interruptible or establish a hard real-time completion bound.

Verified terminal outcomes are synced individually before acknowledgement. The
first query/custody/deadline failure stops the pass. When final custody remains
verifiable, the result includes processed, failed, deferred and not-selected work,
complete remaining counts and the new inventory digest. Earlier synced outcomes
survive a later failed query; a later invocation starts from a freshly inspected
inventory rather than replaying mutations. Corrupt or incomplete final custody
refuses the current packet instead of claiming a complete inventory. The error
explicitly preserves the possibility of earlier retained progress.

## Storage and bounds

A single exclusive nonblocking directory lock owns all pinned readers for the
pass. Per-intent stores duplicate that same parent descriptor, retaining the same
lock while revalidating named-directory identity, private modes, inodes and exact
bytes. Inventory membership is rechecked before returning. The scan stops at
512 directory entries and admits at most 256 intents plus their sidecars. Orphan
sidecars, duplicate native idempotency identities, unrelated/corrupt files,
symlinks, special files and oversized directories fail closed.

The inventory digest covers the directory path/device/inode and sorted names with
SHA-256 of exact intent bytes and optional exact outcome bytes. Its domain is
`dfmcp-bounded-run-inventory/1` followed by NUL. Continuations encode canonical
JSON as lowercase hex plus a checksum under `dfmcp-bounded-run-page/1` followed
by NUL. Neither is a signature or permission to act.

Complete output is capped at 64 KiB. Batch summaries are reserved before native
work and receipt publication. No mid-object truncation or eviction is used.
Directory advisory locking coordinates this host-local workflow, not external
DFHack/UI controllers or an owner intentionally rewriting all evidence. There is
no game checkpoint, rollback, global clock lease or downtime continuity proof.

All results preserve `retry_permitted=false`, `current_pause_unproved=true` and
`goal_completion_proved=false`. Historical pause evidence does not prove that the
fortress is paused now or that any production/excavation goal completed.

## Executed regression evidence

```sh
PYTHONPATH=scripts python3 -m unittest \
  test_bounded_run_client test_bounded_run_outcomes test_recover_bounded_runs
```

All 52 tests passed on the exact uploaded source bytes: 20 unchanged client tests,
18 receipt tests and 14 new inventory/pass tests. Actual Python, private POSIX
files, subprocess inspection/locking and joined fragmented real-loopback TCP
protocol doubles execute. Cases include lost commit reply then QueryRun without
a second commit, offline terminal evidence, 611 single-byte corruptions and 611
incomplete prefixes, partial writes and file/directory sync failures, complete
pending counts beyond page one, maximum filename/key output, stale continuations,
duplicate identities, first-failure deferral, authority changes, shrinking pass
deadlines and an actual query-only loopback recovery path.

Four additional syntax-valid weakened temporary source copies failed their focused
regressions: removing receipt file sync, removing the expected inventory check,
allowing non-query dispatch, and removing the per-key whole-pass deadline check.
These were local mutation experiments, not changes to checked-in implementations.

Exact final Git blob identities checked against executed files:

- bounded_run_client.py: f0ab88a05f908fa9f5354361aab3b81380f9b2bc;
- bounded_run_outcomes.py: 6e6894cac4e63abfb6585c843acae10e27d88a2e;
- recover_bounded_runs.py: 33b8d3d6005712554539d01353cac39c19bef11f;
- test_recover_bounded_runs.py: f5a17658bda96634c03531911a8e91fe6a1fcc8e.

No real DFHack SDK/plugin, generated-protobuf ABI, live fortress, Rust/MCP,
physical power-loss campaign or full repository qualification was executed.
Native protocol, dependency universe, compatibility registry and production
runner map are unchanged. This advances df-dfhack-bridge-plane-c-pic.5 without
closing its broader native/live recovery requirements.
