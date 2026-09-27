# Whole-original-plan construction monitoring through MCP

**Validation status: source present and reviewed; MCP build and process execution
blocked.** The underlying adapter has 123 passing focused tests. The MCP integration
described here has not compiled or run: four attempts were killed inside the pinned
runtime dependency under persistent shared memory pressure. Its 13 process
scenarios are implemented, with syntax and independent fixture checks only. See
[`furniture-completion-mcp-source.json`](evidence/furniture-completion-mcp-source.json)
for the exact limitation. The workflow below documents the implementation contract,
not an executed MCP result or a production support claim.

The isolated furniture/1.19 development server source follows a complete original
furnishing batch beyond `all_placed` into later sampled construction conditions.
It uses the existing eleven tools and the
[Rust completion adapter](FURNITURE_COMPLETION_RUST.md). Every original building
must satisfy its condition in the same captures, across a fixed global stability
window. Per-building successes from different times cannot complete the plan.

This is development functionality. `sampled_condition_satisfied=true` means a
historical condition met the fixed sampling policy with verified original custody.
It does not prove current usability, continuous stability, causal attribution or
that an original placement effect can be forgotten or retried.

## Configure the third private file

Keep the original [batch and placement configuration](FURNITURE_BATCH_MCP.md).
Add a distinct normalized absolute path for the completion journal, and configure
the operations/1.4 credential for explicit native sampling:

```sh
export DFMCP_BUILD_COMPLETION=/private/furniture/completion
export DFMCP_OPERATIONS_PAGED_TOKEN='<operator-provided operations credential>'
```

`DFMCP_BUILD_JOURNAL` and `DFMCP_BUILD_BATCH` continue to identify the exact two
original files. All three paths must be distinct and use owned `0700` directories
with private `0600` files. Credentials and paths remain operator configuration;
they are never accepted as tool arguments. The original furniture credential is
also needed for sampling, which queries retained native placement receipts.
Local opening, goal creation, inspection and cancellation need no native call.

The monitor begins only after the original batch audit proves that every original
step has a synchronized Placed receipt. Missing, unresolved, refused or substituted
steps prevent goal creation. The complete original plan and receipt mapping are
retained in the monitor; the caller cannot select a smaller successful subset.

## Start, sample and inspect

Use the existing session for the completed batch, or reopen with neither
`selection` nor `furniture_plan`.

1. Create the fixed goal with `fortress.query`. The `query` argument is a JSON
   string, for example:

   ```json
   {"mode":"completion_start","deadline":403200,"interval":120,"stable_samples":3,"stable_span":240,"max_gap":1200,"max_observations":128}
   ```

   `deadline` is an **absolute game tick**, chosen after the original receipts.
   Optional defaults are interval `1`, stable samples `2`, stable span `1`,
   maximum gap `1200`, and maximum observations `512`. The timing must be feasible
   within the deadline. Repeating the identical goal is idempotent; a different
   target set or timing policy cannot replace or renew it.

2. Call `fortress.observe(session_id, selection="completion")` for **one**
   bounded foreground sample. Durable read intent precedes native contact. On
   one connection, the server queries all original receipts, acquires and
   releases one complete paged operations capture, then queries all receipts
   again. Accepted evidence is synchronized before progress is acknowledged.

3. Call `fortress.query(session_id, query='{"mode":"completion"}')` for local
   inspection. It verifies the monitor and original files without native I/O.
   `fortress.doctor` and other local inventory responses also retain the monitor
   in the common Agent Turn.

4. Repeat explicit observations while the goal is active. There is no background
   watcher, automatic reconnect, sleeping poll loop or implicit advancement of
   game time. `fortress.wait` retains its original role of recovering an exact
   placement outcome; it is not a construction polling loop.

Monitoring uses Query authority. The normal `recover` mode can open the three
original files and explicitly sample with valid credentials while carrying no
Plan or Construct grants. Normal `offline` mode opens and inspects the same
files without credentials or writes; it refuses active sampling and cancellation.
Already terminal monitoring can be inspected locally in either mode.

## Read the complete result

`result.batch` contains the complete original plan and every original step key.
`result.completion` includes the fixed goal and origin digests, monitor ID,
timing, observations, global streak, last sampled/counted ticks, capture digest,
interrupted-read state and an assessment for **every** original target.

Assessment rows identify the original receipt, building, construction job and
exact item. They expose observed stage and blocking construction/removal jobs
and item-job links. Before the first sample, every target is still present with
`status="not_sampled"`. A later pending member cannot hide removal or identity
failure elsewhere in the plan.

Phases are `active`, `candidate`, `satisfied`, `failed`, `invalidated`, `expired`
and `cancelled`. An unverified response instead exposes `phase="unverified"`
and retains `historical_phase`. Repeated paused ticks do not count toward
stability. Same-tick changes, excessive gaps and interrupted reads reset the
shared streak. Deadline and observation expiry, failure, invalidation,
satisfaction and cancellation are terminal; reopening does not reset them.

The Agent Turn references the complete result instead of repeating its plan.
It carries outstanding monitor work, verification status and an explicit next
inspection or sample. A verified local terminal monitor has no active monitor
obligation. Unknown original custody still prevents any claim that original
placement work is absent. The canonical world anchor remains unavailable.

## Restart and cancel after original-file loss

`fortress.cancel(session_id, scope="completion")` durably stops only the local
monitor under Query authority. It makes no native call, does not cancel game
construction jobs, and does not undo or discharge placements. It remains
available in an established session if either original placement file disappears.
The result withholds verified origin and construction claims.

After a process restart, missing or substituted original files correctly refuse
normal batch reopening. To inspect or cancel the independently retained monitor,
preserve the **same original configured paths**, keep `DFMCP_BUILD_COMPLETION`,
and select:

```sh
export DFMCP_BUILD_MODE=completion-recover
```

Open with neither selection nor plan. This mode opens **only the completion
journal**. It compares the sealed original path strings, fortress and endpoint
to operator configuration without opening either original file. Native
credentials are unnecessary. Query completion/schema/batch, local doctor,
completion cancellation and session release are available; start, sampling,
prepare, commit and original-effect recovery are denied.

The returned complete original plan and receipt identities are explicitly
historical: `origin_verified=false`, `inventory_verified=false`,
`phase="unverified"` and `sampled_condition_satisfied=false`.
`monitor_inventory_verified=true` separately reports successful verification of
the retained monitor itself. `monitor_terminal` identifies local terminal state,
including cancellation, without certifying construction or original-file custody.
Retargeting either original configured path does not authorize recovery of this
monitor under a different origin.

`DFMCP_BUILD_MODE=completion-offline` provides the same monitor-only inspection
without any writes. It refuses cancellation of an active monitor; already
terminal cancellation is an idempotent local inspection. Close with
`fortress.cancel(scope="session")`; explicit `release_for_recovery=true` can
relinquish custody without claiming native quiescence or clearing game effects.

## Bounds and validation

The server reserves the whole future response before creating the goal and
admits the complete proposed result before durable progress publication. Every
response fits the existing 64 KiB output profile; an unusually dense valid plan
can be refused instead of returning a partial target list. Monitor I/O, original
file guards, native acquisition and output use separate conservative reservations
within one shrinking foreground deadline and byte allowance.

```sh
cargo test --offline --locked -p dfmcp-mcp --lib build_placement_server -- --test-threads=1
cargo build --offline --locked -p dfmcp-mcp --bin dfmcp-build-placement-dev-server
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 scripts/test_furniture_completion_mcp.py \
  --binary /absolute/path/to/dfmcp-build-placement-dev-server -v
```

The process suite uses the actual modern MCP stdio executable, real private files
and a joined local TCP peer with the independent Python native codecs. It covers
all three furniture types, shared stability, full 32-target/multipage results,
interrupted reads, receipt/source/digest/release failures, original-file loss,
restart cancellation and denied placement paths in monitor-only recovery.
No MCP process result is claimed for this integration. `IMPLEMENTATION_STATUS.md`
records the blocked validation separately from the adapter's passing tests.
These checks do not qualify a real DFHack plugin, live fortress or production
admission.
