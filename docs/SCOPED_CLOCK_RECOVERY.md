# Recover construction-clock effects without the furniture batch

`recover_route_furniture_clock.py` inspects or queries an original run/1.13
construction-progress effect even when the furniture batch is unavailable,
corrupt, or held by another owner. It requires no batch pathname, placement
credential, map/inventory credential, or fresh construction observation.
The selected original clock files must still be intact and exclusively held.

This is a recovery-only alternative to `route_furniture_progress.py query` and
`route_furniture_final_progress.py query`, not a replacement execution owner.
It accepts both unchanged next-step Scope and final-construction FinalScope.
Start, prepare, commit, source refresh and retry are not exposed. Explicit
original-key cancellation is described below; query itself remains read-only.

## Inspect and query

Retain the original batch ID and progress confirmation independently. The command
selects `progress-<confirmation>.run` and its `.scope.json` and optional
`.outcome.json` companions within the existing private clock directory:

```sh
python3 scripts/recover_route_furniture_clock.py inspect \
  --clock-directory /absolute/private/clock-records \
  --batch-id <original-batch-id> --confirm-progress <original-confirmation>

python3 scripts/recover_route_furniture_clock.py query \
  --clock-directory /absolute/private/clock-records \
  --batch-id <original-batch-id> --confirm-progress <original-confirmation> \
  --wait-ms 1000 --timeout-ms 10000
```

Inspection is fully offline. Query also remains offline for an already retained
terminal receipt. Only an unresolved online query needs this isolated environment:

```text
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_RUN_TOKEN=<existing run credential>
```

Use the exact original endpoint; the address above is illustrative. Other DFMCP
settings are refused for online recovery, including placement and admission
permissions. Credentials and source authority are rechecked throughout I/O,
retention and final output. No route batch is opened or synthesized.

## Original evidence, not replacement authority

The sidecar must decode with the existing closed Scope/FinalScope codec, match
both supplied identities, and generate exactly the retained run intent. Private
no-follow file checks pin the sidecar, intent and optional outcome; the directory
lock remains held through synchronization and the single final output attempt.
Missing, changed, linked, oversized, noncanonical or incomplete evidence refuses
without repair, deletion, migration or new intent creation.

The existing guarded query-mode transport sends only Handshake and QueryRun after
its fixed method bindings. A regressed handshake refuses before QueryRun. One
connection and a nonrenewable foreground deadline cover optional polling. A lost
reply never causes an automatic reconnect, preparation or commit. The unchanged
OutcomeStore validates native receipts and synchronizes terminal evidence before
acknowledgment. Failed sync or torn publication is never acknowledged or repaired.

Every response distinguishes `selected_clock_files_verified` from
`original_batch_custody_verified=false`. A sidecar is retained historical
association, not proof that the original furniture journals still exist or that
all original work was completed. No readiness, current pause, construction
goal, placement permission or complete-directory claim is made. Source-lost
terminal evidence still requires attention. Absent, prepared, running and stopping
records remain unresolved; absence is not proof of nonapplication.

Bounds are one 1..60000 ms deadline, 32 RPC frames (including six bindings and
handshake), 3 MiB actual wire bytes, 64 MiB conservative disk reservations and
100000 custody work units. Output is at most 64 KiB; a complete scope and the
maximum existing clock response are reserved before native contact. Files remain
bounded by the unchanged 8 KiB clock limits. These are cooperative limits, not
hard real-time preemption of filesystem synchronization.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_scoped_clock_recovery.py --mutations
```

23 tests pass: 11 file/authority/CLI tests and 12 real TCP recovery tests, including
both scope families, offline terminal replay, complete native-field decoding,
source regression/loss, lost replies, current revocation, private locking,
sidecar substitution, failed sync, torn writes, exact budgets and single output.
Subprocess tests run the actual CLI against a joined loopback peer with independent
frame/native-record encoding. Six weakened implementations fail assertion checks.
All seven reused production dependencies match their exact upstream Git blobs;
the runner measures all ten production/test/runner files before and after execution.

No DFHack process, game state, plugin build, Rust/MCP or full-workspace campaign
was executed. Host-local custody and hashes do not protect against a malicious
owner replacing all independently selected evidence. Existing start paths,
protocols, scopes, journals and production admission are unchanged.
Beads: df-dfhack-bridge-plane-c-pic.4/.5 remain in progress.

## Explicit cancellation when the original clock still needs to stop

```sh
python3 scripts/recover_route_furniture_clock.py cancel \
  --clock-directory /absolute/private/clock-records \
  --batch-id <original-batch-id> --confirm-progress <original-confirmation>
```

This is an explicit request to the existing CancelRun operation, not a new clock
start or a firmware/global emergency stop. It requires the same isolated run-only
credential and the exact retained scope/intent/token. It works without original
furniture batch custody, but it does NOT bypass another owner's clock-directory
lock. A historical terminal outcome stays offline even under `cancel`.

The dedicated cancellation transport reuses the unchanged wire codec and shares
the same counters, deadline and custody checks. After its fixed bindings it permits
only the initial handshake and one CancelRun. It consumes its attempt before the
frame and refuses another attempt even after success. A different native generation
refuses before cancellation; use query-only recovery to retain source-loss evidence.
The query transport is unchanged and still refuses cancellation.

A stopping response remains unresolved and is not a pause claim. Prepared or running
responses to CancelRun are contradictory and rejected. Stopped/refused/source-lost
receipts follow the original terminal-storage semantics; the last remains unresolved.
A lost reply never triggers another cancel, connection or automatic query. Query the
original key explicitly afterward. `--wait-ms` is query-only, so cancellation cannot
silently start a polling loop. No furniture placement is attempted or discharged.

The combined suite now passes **34 tests**: all 23 preceding tests and 11 explicit
cancellation tests using actual private files and joined TCP peers. Cases cover both
scope families, one-shot/identity/source guards, stopping and terminal outcomes, lost
replies, failed sync, revocation and actual cancel CLI subprocesses. Ten weakened
implementations fail assertion-based regressions. The runner measures all twelve
production/test inputs and preserves the seven reused upstream source blobs. These
are not native DFHack/plugin, Rust/MCP or full-workspace qualification.
