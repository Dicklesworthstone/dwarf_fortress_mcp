### Durable bounded-run dispatch, reconciliation and cancellation

Add a source/endpoint-bound, append-only Rust effect coordinator with three fixed
control/recover/offline modes. Sync sealed intent before preparation and a
non-retryable dispatch marker before unpause. Sync validated native receipts
before acknowledgement; retain unknown outcomes, native source loss and pending
cancellation across restart. Never promote an uncertain native preparation back
to dispatchable state. Reserve cancellation and terminal space against monitoring.

Add private Linux file custody, exact-byte revalidation and torn-tail refusal;
no automatic repair or deletion. Extend the typed RPC source with its actual
connect endpoint and separate per-call/connection byte accounting.

Nine Rust coordinator groups are registered but uncompiled/unexecuted. Seven
independent Python format/state reference groups passed; they do not execute the
Rust coordinator, filesystem custody, MCP, DFHack or a live fortress. No new
wire, dependency, production runner or compatibility admission.
