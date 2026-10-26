# Recover discovered original construction clocks without restarting effects

Beads: df-dfhack-bridge-plane-c-pic.4/.5.

`scoped_clock_recovery.Recovery` adds a bounded foreground reconciliation pass
on top of the existing complete scoped-clock inventory. The independently
retained inventory digest selects the exact local history. The owner queries at
most 16 original-batch records, in deterministic confirmation order, once each.
Next-placement and final-construction scopes retain their exact frp-/frf- keys.
It cannot prepare, commit, cancel, observe routes, repair files, adopt a foreign
record or start a new clock. Existing exact-key recovery remains independent.

Recovery needs only the existing isolated run/1.13 opt-in, endpoint and token.
Map, operations and furniture credentials are not required. The original batch
must still be held read-only; its identity, review, plan and complete inventory
remain checked. Current permission is rechecked during native I/O and publication.
One shared deadline, call, network, disk and work budget spans the entire pass.
No per-record allowance is replenished.

The complete directory is validated, including foreign and non-queryable work.
Only an original scope with its exact native intent and no retained terminal
outcome is queried. Missing native records, Prepared, Running, scope-only crashes,
foreign/unassociated records and SourceLost are not resolved by inference.
Stopped/refused terminal receipts are decoded, bound and synchronized by the
unchanged OutcomeStore before another record is queried. SourceLost is retained
as terminal evidence but remains unresolved and still blocks new effects.

The same directory lock spans discovery, queries and outcome retention. Each old
file identity and byte commitment remains pinned. Only a successfully verified
missing outcome may be added to the inventory. Ambiguous retention fences the
owner and preserves all surviving bytes without repair. Reopening can inspect
valid historical evidence; it does not retroactively acknowledge a failed sync.

A failed query stops the pass with no retry and no later query. Earlier retained
outcomes remain available when current custody and authority permit the final
report. Lost custody, revoked permission or ambiguous publication refuses cached
success. Invocation counts are per-record attempts, not a claim that bytes reached
the peer. Full output is reserved before any native connection. The embedding
owner must retain both original batch and Recovery custody through stdout.

## Executed core evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_clock_inventory test_scoped_clock_recovery -v
```

All 46 tests pass: 27 unchanged inventory cases and 19 recovery tests. They execute
actual native clock framing and codecs over TCP to bounded, joined synthetic
peers, plus real private scope/intent/outcome files, locks and synchronization.
Coverage includes mixed scope families, exact 16-query caps, stale inventory,
source loss, absent/prepared/running replies, lost second replies, wrong receipts,
byte-identical file replacement, membership changes, revocation, budget/output
refusal, sync failure and no mutation method availability. Existing inventory
cases cover all 256 records. Every imported production file matched its exact
GitHub blob before execution.

Original route-batch custody and budget are explicit test ports, not complete
private-batch or live acquisition evidence. There is no DFHack SDK/game, Rust/MCP,
full-workspace or production-admission qualification. No native protocol or stored
scope/intent/receipt format changes. The CLI integration is a separate increment.
