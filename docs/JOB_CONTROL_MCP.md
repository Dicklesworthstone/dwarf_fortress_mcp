# Job-control MCP integration status

## Session-bound control loop

`dfmcp_mcp::job_control_session::JobControlSession` now connects selected native
job observations to the existing sealed `SuspensionPlan` and durable
`JobControlJournal`. It is an unadmitted library integration, not a production
runner or a live-game qualification.

Every operation takes current `OperationContext` authority and checks session,
fortress and journal custody. The constructor accepts already opened custody and
an optional injected native source; it never creates grants. Offline inspection
can therefore retain no credentials or connection.

`observe` acquires one selected job, validates its complete evidence and exact
source manifest, checks authority against the new tick, and invalidates the old
selection on a failed refresh. The observation is not a canonical world snapshot.
`plan` constructs its digest and native token server-side from that retained
observation, a stable ASCII key and the desired suspension bit. Callers cannot
supply arbitrary native plan bytes. Preparation is synced before acknowledgement;
exact key replay does not call the bridge or renew its native lifetime.

`commit` requires the retained plan digest, exact expected witness, current
selection and ConfigureProduction authority. It delegates the only setter edge
to the coordinator, which syncs DispatchStarted first. A successful or uncertain
attempt invalidates selection. Terminal replay performs no native call. Unknown
outcomes for any retained key block preparation and dispatch of other keys until
reconciliation. A failed acknowledgement with uncertain custody or dispatch
state is reported conservatively as EffectIndeterminate, not as proof of no effect.

`reconcile` requires only Query and never reaches prepare or commit. Prepared and
terminal records are returned without a native query; polling a preparation does
not retire it as unknown. Existing writable Query-only recovery can query and
retain evidence after restart. Offline read-only recovery reports uncertainty
without contacting DFHack. Cancellation durably retires a still-prepared key; it
is neither a native cancellation nor an undo operation.

Eight Rust regression groups are registered for the complete sealed loop,
forged seals, failed refresh, per-call authority/session/anchor/action budgets,
cancellation, lost replies, cross-key dispatch fencing, Query-only restart
recovery, absent receipts and dispatch-sync failure. **They have not been compiled
or executed:** Rust, Cargo and rustfmt are unavailable in the implementation
environment. Source inspection and blob identity checks are not Rust, filesystem,
native DFHack, live-game or admission evidence.

## Recovery discovery foundation

The isolated job-control/1.9 library provides `JobControlJournal::summary`
and `records_page`. Both require current Query authority and valid file custody.
A page holds at most 64 complete records, obeys caller row/byte/time bounds, and
includes terminal records as well as unfinished work. Ordering is ASCII key order.
`expected_head` binds discovery to one journal generation; `next_after` is the last
returned key only when more records remain. The MCP layer must additionally bind
its opaque continuation to the session and journal identity.

Summaries distinguish Prepared, reconciliation-required, and terminal counts.
They describe retained coordination evidence, not the current game or completed
production goals. Identity checks are not a full reread or protection against a
malicious same-user rewrite of already-cached bytes.

`open_private_job_reconciliation` opens an EXISTING private, exclusively locked
journal with Query authority and a writable descriptor. It neither creates a new
file nor repairs existing bytes. Its journal can append reconciliation evidence;
prepare/commit/cancel still require independently supplied ConfigureProduction on
every call. This opener grants no capability. `open_private_job_recovery` remains
read-only and cannot append even with a later mutation grant.

## Remaining executable integration

The next increment exposes this session loop through the frozen eleven-tool MCP
surface in a separately gated, unadmitted development binary. This library-only
increment does not expose an MCP job mutation route. The native wire generations,
production runner map, dependency graph and compatibility registry are unchanged.
