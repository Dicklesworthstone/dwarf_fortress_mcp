# Executable exact furnishing batches

`scripts/furniture_batch.py` executes `dfmcp.furniture-plan/1` plans through the
existing furniture/1.19 Python placement client. One `advance` attempts at most
one exact bed, chair or table placement. It is not an atomic transaction, a new
native writer, an MCP endpoint, a verified game checkpoint or production admission.

## Define and initialize

Each of 1..32 steps names one existing item and one exact target. Step names and
optional `after` dependencies form a deterministic action DAG. Dependencies mean
**verified historical placement registration**, not completed construction.
Execution takes the lexical-first topological order and stops at the first
unknown, indeterminate, refused or cancelled child, even if later steps would be
independent. No target or item is substituted and no failed step is skipped.

```json
{
  "schema": "dfmcp.furniture-plan/1",
  "steps": [
    {"name": "bed", "kind": "bed", "item": 42, "target": [15, 15, 2]},
    {"name": "chair", "kind": "chair", "item": 43, "target": [18, 15, 2], "after": ["bed"]},
    {"name": "table", "kind": "table", "item": 44, "target": [20, 15, 2], "after": ["chair"]}
  ]
}
```

These IDs and coordinates are illustrative. Read `BUILD_PLACEMENT_CLIENT.md` for
the unchanged eligibility, token and native setup. Use an existing empty owned
exact-mode 0700 batch directory, never an existing placement directory. Set the
existing isolated client environment: `DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1`,
`DFMCP_BUILD_TOKEN`, optional numeric-loopback `DFMCP_BUILD_ENDPOINT`, and
`DFMCP_BUILD_ALLOW_PLACE=1` for advancement. Other DFMCP profile/admission variables
are refused. There is no environment union or credential transfer to monitors.

```sh
python3 scripts/furniture_batch.py init \
  --directory /private/bedroom-furnishings --plan furnishings.json \
  --world-folder region1 --site 2
```

Initialization reads the first selected item/target, checks the explicit fortress
folder and site, and verifies every target's full 3x3 context fits the observed map.
It retains the complete normalized plan, native software/generation, dimensions,
endpoint and starting tick. It does not prepare or place anything, or establish
that every other item is available. Later steps obtain their own fresh captures.
The input JSON file is not reopened after initialization.

The compiler rejects duplicate items or target tiles, unknown dependencies,
cycles, unsupported kinds and invalid coordinates before any placement. Plan data
is bounded at 16 KiB. See `FURNITURE_PLANS.md` for its independent pure-model API.

## Review and advance one step

Retain `result.batch_id` from initialization. Every subsequent command requires it;
a pathname alone cannot select a different batch unnoticed.

```sh
python3 scripts/furniture_batch.py review \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>'

python3 scripts/furniture_batch.py advance \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>' \
  --expected-plan '<result.expected_plan>' \
  --confirm-review '<result.confirm_review>'
```

Review performs only an observation. It reports the complete selected capture,
local eligibility blockers and native uncertainty/capacity. An eligible review's
confirmation binds the exact batch, full retained inventory, selected step, native
key and complete native plan. A confirmation is a commitment to bytes, not proof
that a human reviewed them. Advance reacquires the capture and refuses changed
terrain, items, source identity, software, map dimensions, or a mismatched seal.
The native generation cannot be silently adopted after a game/source reload.

Advancement uses the original placement client's observation, native-key preflight,
private intent, preparation, dispatch journal and verified terminal receipt path.
The only addition to that client is an optional internal guard at the pre-intent,
pre-prepare and final pre-commit boundaries. Its ordinary single-placement caller
remains supported. The guard adds restrictions; it bypasses none of the existing
checks and cannot manufacture the connection-owned commit permit.

A child intent is synchronized and registered in the batch index before native
preparation. Preparation and dispatch are synchronized by the original journal
before its sole commit. Predecessor receipts, exact selections, current local
custody and stop status are checked again before dispatch. Operator placement
revocation remains effective at the final boundary. No retry or unpause is added.

Repeat **review then advance** for the next step, using its new confirmation. An
old confirmation never advances a different step. `all_placed` means every exact
historical placement receipt was verified, not that any furniture is finished,
usable, assigned to a room, safe, reachable or still present now.

## Inspect, recover and stop

```sh
python3 scripts/furniture_batch.py inspect \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>'
python3 scripts/furniture_batch.py query \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>' --step bed
python3 scripts/furniture_batch.py cancel \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>' --step bed
python3 scripts/furniture_batch.py stop \
  --directory /private/bedroom-furnishings --batch-id '<batch_id>'
```

Unknown outcomes block the remaining batch. Query reconnects explicitly to the
original endpoint and key; it cannot prepare or commit. Missing native records
remain unknown, never proof of nonapplication. A retained indeterminate receipt
is immutable and remains unresolved. Recovery may retire an uncommitted native
preparation after placement permission is revoked. Cancellation cannot remove a
placed building or cancel a construction job, and a cancelled child halts the plan.

`inspect` is offline. A child with a retained terminal/indeterminate record also
returns offline from query/cancel, using the original client behavior. `stop`
creates a permanent local stop marker and needs no native credentials. It prevents
future batch advancement but leaves original-key recovery available. It does not
change the game or clear uncertain work. There is no resume/unstopping command.

`inspect --step bed` includes the complete child evidence. For a Placed child,
`result.receipt` is the exact `{"canonical_record_hex":"..."}` object accepted by
`track_construction.py --receipt-file`. Save only that object in the existing
monitor's private input format and use its independently authorized setup; receipt
export does not start monitoring, grant new authority or prove construction.

## Durable inventory, limits and failure behavior

The batch owns exactly `batch.json`, append-only `steps.jsonl`, `effects/`, and an
optional immutable `stop.json`. Original child journals remain unchanged-format
`.placement` files inside `effects/`. All path components use no-follow opens;
directory/file modes are exact 0700/0600, files are regular and single-link, and
exclusive nonblocking locks cover batch and effects directories. File bytes and
path identity are rechecked. Metadata files are at most 32 KiB each.

Each index entry binds the batch ID, ordered step, original intent-frame hash,
child inode and previous entry. A registered child disappearing or being replaced
cannot turn into unstarted work. A valid original intent left by a crash before
index registration stays pending; explicit query/cancel may adopt its custody,
never its old process's dispatch authority. Torn/corrupt files are preserved and
refused without repair, truncation, compaction or eviction.

Copying or relocating these inode-pinned directories is unsupported. These checks
are local cooperating custody, not a malicious-owner anti-rollback system or a
global controller lease. Do not use another batch/directory to bypass uncertainty.
No game checkpoint or automatic rollback is implemented.

Commands inherit the native client's one shrinking 1..60,000 ms budget (default
10,000), 32 native-call and 512 KiB connection allowances. Filesystem operations
use cooperative checks, not hard real-time cancellation. Complete JSON/Agent Turn
responses are bounded at 64 KiB; output space is reserved before advancement.
All steps and unresolved identity remain visible, without pagination or transcript
memory. Errors mark the inventory unverified instead of presenting an empty plan.
A synchronization failure never acknowledges that write; fully present historical
bytes can be reverified on a later open without claiming the failed call succeeded.

## Executed validation

```sh
PYTHONPATH=scripts python3 -m unittest test_furniture_plan test_furniture_batch -v
python3 scripts/check_furniture_batch_mutations.py
```

All 35 actual Python test functions pass: seven compiler functions and 28 batch,
real-client/TCP/POSIX/subprocess functions. They include all 4,096 four-node graphs,
32 executed placements with maximum-width source fields and long step names,
all eight unchanged independent native vectors, original-API compatibility,
lost-reply/new-process recovery, 16 injected file/directory sync failures, torn
writes, source and horizon changes, substituted confirmations, removed/replaced
child journals, Query-only cancellation and stopping after preparation.

Four weakened implementations fail regression assertions: missing review binding,
missing final guard, missing index synchronization, and forgotten child custody.
The largest measured complete review/advance/receipt-export response in the
32-step run is 31,548 bytes. This is actual serialization, not a size estimate.
Exact source identities and executed counts are retained in
`docs/evidence/furniture-batch.json`.

The TCP peer is a test double, not DFHack. This evidence does not qualify a real
native SDK/plugin, live fortress, physical power loss, Rust/MCP integration or the
full workspace. Native protocols, dependencies and production admission remain
unchanged. Beads `df-dfhack-bridge-plane-c-pic.4/.5` retain their wider acceptance work.
