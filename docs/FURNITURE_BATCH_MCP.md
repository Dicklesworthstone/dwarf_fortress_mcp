# Complete furnishing plans through MCP

The furniture/1.19 development server can execute a complete exact-item
`dfmcp.furniture-plan/1` through the existing eleven tools. The server retains the
whole plan and every original step across restart. Each placement still requires
its own fresh observation, witnessed preparation and explicit review confirmation.

This connects the [inventory allocator's exported plan](FURNITURE_ALLOCATION_MCP.md)
to the [Rust batch core](FURNITURE_BATCH_RUST.md) and existing
[single-placement control boundary](BUILD_PLACEMENT_MCP.md). It remains an
unadmitted development executable. `all_placed` proves historical registration
of all requested stage-zero buildings and construction jobs; it does not prove
construction completion or present usability.

After `all_placed`, the [whole-plan completion monitor](FURNITURE_COMPLETION_MCP.md)
can check every original building against shared later observations through the
same tools, with durable progress and restart recovery.

## Configure the original two files

Use the existing furniture server configuration, including the explicit
disposable-fortress checkpoint policy when placement is permitted. Add:

```sh
export DFMCP_BUILD_JOURNAL=/private/furniture/journal
export DFMCP_BUILD_BATCH=/private/furniture/batch
```

Both files need normalized absolute, distinct paths in existing owned `0700`
directories. The server creates missing files exclusively with exact `0600`
mode. They can share a parent directory. Paths, endpoint, token, scope, protected
regions and checkpoint policy remain operator configuration, never tool arguments.

A new batch requires an empty original placement journal. The parent retains
the full canonical plan, exact native source and that journal's identity. Its
format is separate from the Python batch directory; this server does not import
or modify the Python `.placement` files. The existing Rust child journal format
is unchanged. Without `DFMCP_BUILD_BATCH`, the single-selection workflow remains
available.

## Import and execute

Pass the complete plan as the **JSON string** argument `furniture_plan` to
`fortress.open_session`; omit `selection`. For example, the string can encode:

```json
{
  "schema": "dfmcp.furniture-plan/1",
  "steps": [
    {"name": "bed", "kind": "bed", "item": 42, "target": [15, 15, 2]},
    {"name": "chair", "kind": "chair", "item": 43, "target": [19, 15, 2], "after": ["bed"]},
    {"name": "table", "kind": "table", "item": 44, "target": [23, 15, 2], "after": ["chair"]}
  ]
}
```

The closed parser rejects unknown or duplicate fields, invalid selections,
duplicate items/targets, missing dependencies, cycles, more than 32 steps, or
more than 16 KiB of plan input. It validates every target against the operator
scope and protected regions before native bootstrap. The initial connection
establishes the actual source binding; it provides no preparation permit.
Neither opening nor querying performs a placement.

The response includes `result.batch` with the normalized full plan, its
Python-compatible digest, durable `batch_id`, every original step and its
deterministic key, plus one `next` selection when progression is possible.

1. Call `fortress.query(session_id, query='{"mode":"batch"}')` to verify both
   files and inspect complete progress without a native connection.
2. Call `fortress.observe(session_id, selection="next")`. The server derives the
   selection from its original plan. An explicit selection array is also accepted
   when it exactly equals that next step. The result contains the fresh
   `observation.observation_witness` and actual item/target evidence.
3. Call `fortress.plan` with that witness and the exact
   `result.batch.next.idempotency_key` obtained before preparation. Review the
   returned native plan and capture. Preparation returns `plan_digest` and
   `review_seal`; it creates no building.
4. Call `fortress.commit` with the exact step key, native plan digest and review
   seal. The seal binds the complete parent plan, original child journal, prepared
   journal head, native plan, session, lease and operator policy. Parent custody
   and the next-step constraints are rechecked at native boundaries, including
   the final check after dispatch intent has synchronized.
5. Inspect the returned complete batch inventory. Only a verified native Placed
   outcome synchronized in the coordinator's terminal state unlocks another step.
   Repeat with a new observation and review for the next original key.

The order is deterministic: choose the lexically first dependency-ready name
at every step. Even independent later steps wait behind the original execution
prefix. Refusal, cancellation, unresolved preparation, lost replies or uncertain
outcomes cannot be bypassed by another key or a smaller plan. The batch is not
atomic, and later failure does not undo earlier registrations.

Allocation constraints identify candidate items at the allocator's published
capture. Importing its exact-item plan does not authenticate cross-profile
continuity, reserve inventory, or make material/distance constraints permanent
mutation preconditions. The placement profile freshly observes the selected
item and target and requires the new review before each attempt.

## Restart and original-key recovery

Retain the same operator configuration and call `fortress.open_session()` with
**neither plan nor selection**. Reopening reads the two original files without
a native bootstrap connection. It returns the complete original plan, placed
prefix and any pending identity. Reopened prepared bytes never restore the local
review or original connection permit.

If a placement reply was lost, use `fortress.wait` with that original step's key
and native plan digest. This queries the original operation at most once and
synchronizes its verified outcome; it does not repeat prepare or commit. If the
native operation is still prepared and cannot safely commit, `fortress.cancel`
with `scope="effect"` and the original identity can retire that preparation
under Query authority. A refused or cancelled step halts the original batch.

`DFMCP_BUILD_MODE=offline` supports complete local inspection without credentials
or writes. `recover` also opens without a native connection, then allows an
explicit original-key Query or retirement with the current native credential.
Neither mode can start a new placement.

If the parent disappears or is substituted while a session remains open, new
placement work stops and `result.batch.inventory_verified` becomes false. The
original child journal still supports Query-authorized wait, exact evidence
inspection and effect cancellation. The returned parent plan and identities are
explicitly historical, and `next` is null. A parent failure after native dispatch
can leave synchronized `dispatch_started` without an acknowledged terminal
receipt; recover the original key to determine the outcome.

After restart, a missing or corrupt configured parent correctly refuses normal
batch opening. For recovery, preserve the original files, set
`DFMCP_BUILD_MODE=recover` (or `offline` for inspection), remove
`DFMCP_BUILD_BATCH`, and keep `DFMCP_BUILD_JOURNAL` pointing at the **same original
child journal**. Open without selection, then discover its retained records and
recover their original keys. This provides child-effect recovery without claiming
that the lost complete-plan custody was restored. Never create a replacement
journal or new keys to bypass uncertainty.

## Permanent batch stop

`fortress.cancel(session_id, scope="batch")` abandons local review and synchronizes
one permanent parent stop in Control or Recover mode under current Query authority.
It does not require current placement authority and makes no native call. Offline
mode refuses the write. The stop survives restart, removes `next`, and does not
erase pending effects or cancel game jobs. Explicit original-key recovery remains
available after stopping.

The inventory distinguishes `stopped` (verified durable stop) from
`advancement_fenced` (local custody/publication failure). Any failure during an
authorized stop fences local advancement until reopen. Refusal, cancellation,
pending recovery and `all_placed` outcomes retain their meaning while stopped.

## Complete progress and bounded output

`result.batch.steps` always lists every original name, exact key, native plan
digest when present, coordinator state, native outcome, receipt digest and
building/job IDs when registration is verified. Full native receipt bytes remain
available through exact `get` or `fortress.explain`. The Agent Turn's
`active_work.furniture_batch` carries the batch identity, progress and an explicit
`result.batch` inventory reference, so the complete plan appears only once in
the response. Canonical world `anchor` remains null.

Whole-response admission reserves all future step summaries, the complete plan,
maximum native record/capture rendering, policy and Agent Turn before parent
creation or preparation. A syntactically valid plan can still be refused when
its complete future response cannot fit the 64 KiB allowance. Every native
boundary also consumes a separately reserved bounded parent-custody read. Native
I/O retains one shrinking deadline and the existing supervised Asupersync worker;
there is no background executor or implicit commit loop.

## Verification

```sh
cargo test --offline --locked -p dfmcp-adapter --lib furniture_batch -- --test-threads=1
cargo test --offline --locked -p dfmcp-mcp --lib build_placement_server -- --test-threads=1
cargo build --offline --locked -p dfmcp-mcp --bin dfmcp-build-placement-dev-server
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 scripts/test_furniture_batch_mcp.py \
  --binary /absolute/path/to/dfmcp-build-placement-dev-server -v
```

The process tests use the actual compiled MCP executable, modern stdio discovery,
real native TCP framing and real private files. Their joined TCP peer uses the
independent Python furniture codec and engine evidence. This is development
execution against an explicit peer, not a real DFHack SDK or live fortress.
All 32 furniture MCP Rust tests (including seven new batch tests), two shared
runtime tests, nine new batch process tests and seven existing single-placement
process tests passed. The complete 32-placement process response measured
36,407 bytes against the 65,536-byte limit. Tests include parent removal or
same-byte inode substitution after synchronized dispatch intent and parent loss
after the native writer, with exact original-key recovery and no repeated commit.
Exact source, binary and results are recorded in
[`evidence/furniture-batch-mcp.json`](evidence/furniture-batch-mcp.json).
Owning beads `df-dfhack-bridge-plane-c-pic.4` and `.5` remain open.
