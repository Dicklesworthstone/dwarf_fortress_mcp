# Owned foreground mining sessions

`dfmcp_adapter::dig_designation::journal::session::DigSession` owns the existing
journal, at most one selected observation, and at most one native connection.
It supplies the missing lifecycle between separate observation, planning and
commit calls. It does not itself start an async runtime, grant capabilities,
implement the host's lease/checkpoint policy, or admit a production protocol.

## Why the original connection matters

The dig/1.16 RPC client deliberately gives commit permission only to the exact
connection that obtained a fresh native preparation. Opening a new connection
for every MCP stage cannot implement this contract: the commit client has no
permit, even when the persisted Prepared receipt is valid.

The session therefore keeps the original source through initial observation,
prepare's exact reobservation, native preparation, commit's exact reobservation,
and the one commit attempt. `commit` accepts no connection factory. It cannot
silently reconnect, reprepare or reconstruct native permission from stored bytes.
After a commit attempt, it drops the source and local selection/permission whether
the call succeeded or failed. The original durable obligation remains unchanged
unless the coordinator obtained and retained new evidence.

A new observation explicitly abandons older local preparation permission. An
unchanged repeat of prepare returns existing history without calling the bridge;
a conflicting key/review fails. Old Prepared history still blocks another key.
Moving or reopening a journal into a new session clears all local permission.
Neither query nor cancellation restores it.

## Typed foreground recovery

`reconcile` and `cancel` first verify the original key and digest. A terminal
record or permanent native Unknown returns locally without evaluating the supplied
connection factory. Other recovery opens at most one explicitly requested, exact
source connection and calls the existing coordinator. Reconciliation uses Query
only; native cancellation separately requires the existing Observe/Designate
scopes. Missing native retention does not prove nonapplication.

Offline mode can only inspect/list retained evidence. Recover mode cannot create
observations for planning, prepare, or commit. Grant injection cannot change these
fixed modes. An online recovery connection is local to that foreground operation
and is dropped on return; there is no detached worker, retry timer or background
poller. The journal's existing private-file lock and source binding remain the
cross-restart custody boundary.

## Authority, source floors and runtime ownership

The initial session floor includes every retained plan and the sequence increment
of each verified designation. Successful observations cannot regress behind that
history. Later caller contexts cannot lower the highest observed game tick to
revive expired grants. A validated observation that reveals an expired grant
advances this floor even though its presentation and planning cache are refused.

Before a connection is created, the session checks journal custody, current
capabilities, mode, complete halo/shared-block scope, entity allowance and work
budget. It checks the returned source's exact endpoint/software/incarnation and
rechecks the capture's fortress and requested region. Failed acquisition clears
selection instead of leaving stale terrain available for planning.

`DigSessionGuard` extends the mandatory `DigGuard` with explicit connection and
initial-observation checks. There is no permissive implementation. The trusted
runtime must supply cancellation, I/O, operator and applicable lease/checkpoint
policy, and run synchronous work in its owned blocking region. Per-stage journal
guards still run at the actual native edges, including after dispatch sync.

Each new source reserves `CONNECT_BYTES + 6 * RPC_BYTES` (3,993,624 bytes), covering
negotiation and the same connection's bounded lifecycle. Later request budgets
cannot replenish the client's connection allowance or absolute deadline. Session
calls reserve separate journal verification/result and operation allowances before
native work and narrow all nested deadlines against one foreground allowance.
These are accounting reservations, not allocated buffers or claims of hard
interruption for kernel filesystem operations.

## Registered regression tests

```sh
cargo test --locked --offline -p dfmcp-adapter dig_designation::journal::session
```

Sixteen Rust tests are registered. Fifteen use injected storage/native/runtime
interfaces to exercise ownership, lost replies, restart, replay, mode and authority
refusal, cancellation, source lifetime, clock floors, cached witnesses and retained
uncertainty. One composes the actual DigRpcClient with the actual session/journal
and a fragmented in-memory wire, asserting the complete outgoing method sequence
across separate observation, prepare and commit operations.

All sixteen tests are UNCOMPILED AND UNEXECUTED in the editing environment: Rust,
Cargo and rustfmt are unavailable. No actual runtime, TCP, real DFHack SDK, live
fortress, power-loss campaign or full qualification is established by this source.
Native framing, coordinator history bytes and Python recovery are unchanged.
