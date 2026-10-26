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

## Executable discovery-to-recovery workflow

First inspect the existing original batch and scoped-clock directory offline:

```sh
python3 scripts/route_furniture_progress.py inventory \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --clock-directory /absolute/private/clock-records --selection pending --limit 16
```

Retain `result.clock_inventory.inventory_digest` independently as
`INVENTORY_DIGEST`. It covers the complete local directory, not just the displayed
page. No remembered per-clock confirmation is required for the following pass:

```sh
python3 scripts/recover_scoped_clocks.py \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --clock-directory /absolute/private/clock-records \
  --expected-inventory "$INVENTORY_DIGEST" --max-queries 8 --timeout-ms 10000
```

Paths and shell variables are operator-supplied examples. For an actual native
query, run in the isolated environment containing only these DFMCP variables:

```text
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_RUN_TOKEN=<existing run/1.13 credential>
```

Other DFMCP authority variables, including route and placement settings, are
refused. The existing native plugin must already be configured; this command
does not install or configure a game. If there are no queryable original records,
the pass is offline and needs no credentials, network connection or synchronization.
That does not resolve foreign, unassociated, scope-only or source-lost work.

The command validates arguments before opening custody, then checks the supplied
inventory digest before loading native credentials. Changed membership, bytes,
file identity or original batch inventory makes the selection stale. It uses the
existing route-batch budget and read-only Batch opening interface, and keeps both
batch and clock-directory owners open through exactly one final output attempt.
A short or failed stdout write never triggers a second object or another query.

`--max-queries` is 1..16 (default 8); the whole-operation timeout is 1..60,000 ms
(default 10,000). These are caps, not a guarantee that every selected query fits.
The original shared call/network/work/storage limits are not renewed. Output is
at most 256 KiB and retains the full original audited batch, compact per-attempt
outcomes, complete current counts, initial/final inventory identities, and Agent
Turn pending-work/retained-evidence summaries. Placement confirmations remain null.

Exit zero means a complete report, not that all history is resolved. Inspect
`result.clock_recovery.counts.unresolved`, `counts.query_supported`, `stop_reason`
and `directory_history_reconciled`. A transport/codec failure stops the pass with
exit two and `ok: false`; previously synchronized results remain visible if current
custody/authority permit. Otherwise a sanitized null-result refusal is returned,
with original files preserved. After any retained outcome, old pagination tokens
and old inventory digests are stale: inspect again before another explicit pass.

The reader never automatically repeats a query in one pass or refreshes current
routes/construction. Prepared/running/absent replies remain pending. SourceLost
requires operator investigation; deleting evidence or changing directories is
not a valid resolution. Guarantees are local cooperative custody, not a global
controller lease or malicious-owner rollback protection. A retained pause remains
historical and grants no effect retry, placement or construction-completion proof.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_scoped_clock_recovery.py --mutations
```

All 58 tests pass: 27 unchanged inventory cases, 19 recovery tests and 12 CLI
cases. All eight deliberately weakened implementations fail assertion-based
regressions, not syntax/import checks. They cover stale inventory, query caps,
source loss, continuing after failure, ambiguous-publication fencing, pre-contact
output reservation, premature lock release and failed-pass exit status. The runner
syntax-checks and hashes all 15 executed production/test/runner inputs before and
after testing; none changed during execution.

Tests execute actual native clock framing and codecs over TCP to bounded, joined
synthetic peers, plus real private scope/intent/outcome files, locks and sync.
Coverage includes mixed scope families, exact 16-query caps, 256-record inventory,
absent/prepared/running replies, lost second replies with first outcome retained,
wrong receipts, byte-identical file replacement, membership changes, revocation,
budget/output refusal, sync failure and no mutation-method availability. Complete
CLI/main/runpy entrypoints and actual Agent Turn rendering execute, including
partial-failure packets, credential isolation, original read-only opening, and
custody held through single/short/failed output attempts.

Original route-batch custody and aggregate budget construction are explicit test
ports; clock files, native clients/codecs, recovery algorithms and output packets
are not substituted. This is not a real private route-batch, DFHack SDK/game,
Rust/MCP, full-workspace or production-admission qualification. No existing native
protocol or stored scope/intent/receipt format changes. Broader beads remain open.
