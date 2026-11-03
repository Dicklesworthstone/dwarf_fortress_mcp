# One labor and finite-order plan through MCP

`dwarf-fortress-mcp serve-gameplay-dev labor-order` connects the real
`live_production::Executor` and durable mutation dispatcher to the eleven-tool
MCP surface. One session owns one complete original two-step plan: configure
one labor for selected citizens, then insert a dependent finite furniture order.
The source is explicitly unadmitted development work. Current executed checks,
qualification and real-game evidence belong in `IMPLEMENTATION_STATUS.md`.
The private custody implementation supports Linux x86_64/aarch64; unsupported
platforms refuse private-journal opening.

Owner: `df-dfhack-bridge-plane-c-pic.4.1`. The broader dispatcher and recovery
tasks `.4` and `.5` remain open. This host does not supply a complete fortress
controller, game clock, material/workshop planning or a production-goal monitor.

## Operator configuration

Set all three opt-ins to exactly `1`:

- `DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION`
- `DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE`
- `DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS`

Supply the existing `DFMCP_SEMANTIC_WORKFORCE_CONFIG` and
`DFMCP_SEMANTIC_WORK_ORDER_CONFIG` described in
[the workforce guide](SEMANTIC_WORKFORCE_MCP.md) and
[the work-order guide](SEMANTIC_WORK_ORDER_MCP.md). Their optional standalone
`clock` and original single-goal `recovery` objects are forbidden in this host.
Both must use the same numeric loopback endpoint, initialization mode and exact
citizen world/site/DF/DFHack/bridge-version/generation pins. Native family
generations remain independently pinned; they need not equal each other.

`DFMCP_COMBINED_PRODUCTION_CONFIG` is a closed JSON object with schema
`dfmcp.combined-production-development/1`, an absolute normalized `directory`,
Boolean `initialize`, and optional `recovery`. All three custody directories
must be separate and nonnested. Existing private-file owners require real
owner-private directories and regular no-follow files with canonical modes;
the host neither repairs nor silently replaces custody.

Only the following additional family environment names are accepted:

- `DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR`
- `DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN`
- `DFMCP_SEMANTIC_WORKFORCE_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION`
- `DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN`
- `DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN`

Authority flags are absent or exactly `1`. Initializing in Control requires
both write flags. Opening authority can later be reduced; it cannot be
increased within that session. All three exact configuration strings are sealed
at opening and rechecked at native boundaries. A changed string, revoked opt-in,
other `DFMCP_` variable or production launch/admission state refuses execution.
Configuration, paths, credentials, pins and native methods are never MCP inputs.

## Public sequence

Use the exact names returned by `tools/list`. The logical operations below
retain the frozen eleven-tool waist; this profile's wire names are underscores.

1. Open `mode="control"`. Offline and Reconcile modes do not gain write authority.
2. Observe the paused canonical citizen roster. Retain its `observation.seal`
   and canonical citizen `entity_id` values; native unit IDs are not substitutes.
3. Plan a closed JSON request containing `expected_observation`, `units`,
   `labor`, `enabled`, `recipe`, `amount` and `deadline_tick`. Optional
   `poll_interval_ticks` and `stable_samples` default to 10 and 1. Select
   1–32 citizens, an existing single-labor detail supported by workforce/1.17,
   and 1–100 units of `wooden_bed`, `wooden_door`, `wooden_table` or
   `wooden_chair`. The future deadline is at most one game year away and must
   contain the requested cadence/stability span. Compilation archives one
   complete original plan; it does not prepare or dispatch either native effect.
4. Plan again with the closed JSON object containing `kind="step"` and the exact
   `original_plan_digest`. This prepares the next eligible original step.
5. Review that response and immediately Commit its exact `paired_seal` with
   `confirmed=true`. Native preparations expire; an extra Explain call is
   unnecessary before confirmation. Explain remains available afterward.
6. Repeat steps 4–5 for the dependent order only after the first step has a
   verified historical labor receipt. The actual original StepIds, keys, anchor,
   deadline and whole plan digest are retained throughout.

Wait performs one original-key reconciliation. It never dispatches, sleeps,
polls indefinitely or advances game time. An uncertain effect prohibits a
replacement key or a fresh original plan. Cancel with `scope="step"` retires
a proven preparation; it does not undo an Applied effect. `scope="session"`
releases local ownership while preserving custody and makes no native
cancellation claim. Checkpoint and Restore return explicit capability refusals.

The returned `frontier.all_applied` means both historical native effects were
proved at their respective boundaries. It does not mean current labor, produced
goods, workshop/material feasibility or the original terminal predicate is true.
This host always leaves `current_state_proven` and `original_goal_proven` false.
After an original exists, Observe reports retained original custody rather than
pretending to acquire a new canonical world or rebase its original anchor.

## Recovery and failure

Retain all native journals and semantic associations in both child directories,
plus parent `original.plan`, `original.citizens` and `execution.journal`.
Persist the latest complete `recovery` object outside those files. It includes
the original plan/object/citizen digests and anchor, and frontier journal/head/
authority-tick-floor. On reopen set all three configurations to
`initialize=false`, supply that parent object, and retain the original child
native journal identities required by their existing configurations.

Offline opens retain historical custody without contacting DFHack. Reconcile
mode can consult the original keys under Query authority. Neither reconstructs
a process-local review or native dispatch permit. A resumed Control owner must
prepare an eligible Pending step; it cannot reuse an old paired seal. A retained
Prepared step without its process-local review can be reconciled or cancelled,
but this host cannot renew its commit permit. Lost custody,
changed source or an inconsistent native/frontier record refuses rather than
inventing success or replacing a journal.

The parent archive and citizen identity are verified before and after each
child evidence refresh, including the order's post-dispatch-marker refresh.
Before first-step labor preparation, a separately prepaid canonical citizen
read primes the resolver under current Query/Observe authority and the same
shrinking deadline. It retains the exact original anchor and roster; it neither
revives a cached routing generation nor replaces the later native-capture checks.
The existing executor also verifies the predecessor's exact historical labor
receipt and native/semantic custody before permitting the dependent order.
Partial initial publication is retained and prevents another plan in that
owner; it is not repaired automatically. No native preparation or mutation has
occurred if initial publication has not reached the executor.

Errors withdraw local reviews. Recovery references preserve the newest known
frontier head and authority floor, but any copied frontier is explicitly stale.
Failure to publish a successful response does not erase a possible effect.
Reconcile the original key before deciding what is safe next.
Query verifies the parent archive/identity and durable frontier. Native family
custody and effects are rechecked through their owners at preparation, commit
and reconciliation boundaries; Query is not a new native effect observation.

## Bounds and evidence

Each call admits at most 60 seconds, two actions, zero game ticks and 32 GiB of
logical byte-work, defaulting to five seconds and 8 GiB. This accounts for
repeated bounded custody validation; it is not an eager memory allocation or
frame size. One shrinking deadline covers the nested work. The host reserves
128 KiB for response publication and an additional 512 MiB for opening
inspection. Complete escaped response text is limited to 60,000 bytes; overflow
withdraws the local review and returns bounded recovery/uncertainty instead of
truncating JSON. Smaller output budgets refuse before I/O.

The machine description is
[`live_production_executor_v1.json`](../architecture/live_production_executor_v1.json),
under `development_mcp_host`. Standard workspace checks compile and exercise the
host. Injected-source and loopback-peer tests are component evidence, even when
they run the actual executable and real private stores. A source-bound actual
DFHack/game trial, clean full-source qualification and production admission
are separate evidence requirements. The registry and production runner map
remain unchanged.
