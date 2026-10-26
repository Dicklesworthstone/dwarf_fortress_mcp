# Reconcile discovered original construction clocks without restarting effects

Beads: df-dfhack-bridge-plane-c-pic.4/.5.

`scoped_clock_recovery.Recovery` and `recover_scoped_clocks.py` add a bounded
multi-record pass over the existing complete scoped-clock inventory. They are
separate from `recover_route_furniture_clock.py`, the existing selected-record
inspect/query/cancel owner that does not require original batch custody. That
owner, its tests, runner and contract remain unchanged. The new pass requires
the held original batch and never permits cancellation or another clock effect.

## Discover and reconcile

Inspect the existing original batch and scoped-clock directory offline:

```sh
python3 scripts/route_furniture_progress.py inventory \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --clock-directory /absolute/private/clock-records --selection pending --limit 16
```

Retain `result.clock_inventory.inventory_digest` independently as
`INVENTORY_DIGEST`. It selects the complete local directory, not just the displayed
page. No remembered per-clock confirmation is needed for the following pass:

```sh
python3 scripts/recover_scoped_clocks.py \
  --directory /absolute/private/route-batch --batch-id "$BATCH_ID" \
  --clock-directory /absolute/private/clock-records \
  --expected-inventory "$INVENTORY_DIGEST" --max-queries 8 --timeout-ms 10000
```

Paths and variables are operator-supplied examples. Native query needs only this
existing isolated run/1.13 client environment; other DFMCP authority variables,
including route and placement credentials, are refused:

```text
DFMCP_ALLOW_UNADMITTED_RUN_V1_13=1
DFMCP_RUN_ENDPOINT=127.0.0.1:5000
DFMCP_RUN_TOKEN=<existing run/1.13 credential>
```

The native plugin must already be configured. The command does not install or
configure a game. With no queryable original records the pass is offline and
requires no credentials, native connection or synchronization. Offline inspection
never resolves foreign, scope-only, unassociated or source-lost work.

## Exact selection, bounded queries and partial results

The complete directory is validated, including foreign and non-queryable work.
Only an original scope with its exact native intent and no retained terminal
outcome is queried, in deterministic confirmation order. Next-placement and
final-construction scopes preserve their exact frp-/frf- keys. Each selected
record receives at most one QueryRun. Missing records, Prepared and Running stay
pending. SourceLost is retained as terminal evidence but remains unresolved.
Stopped/refused receipts are strictly decoded, bound and synchronized by the
unchanged OutcomeStore before another record is queried.

A failed query stops the pass without retry or a later query. Earlier synchronized
outcomes remain visible when current custody and authority permit the final
report. Lost custody, revoked permission or ambiguous publication refuses cached
success. Invocation counts are per-record attempts, not a claim that bytes reached
the peer. Existing exact-key recovery remains available independently of damage
to other directory records. The new pass never adopts, repairs or deletes files.

`--max-queries` is 1..16 (default 8); timeout is 1..60,000 ms (default 10,000).
One existing original batch budget spans all calls, bytes, storage, work and
publication. Caps are not promises that every selected query fits. The complete
output, at most 256 KiB, is reserved before native contact and retains the original
audited batch, compact outcomes, complete counts and initial/final inventory
identities. Agent Turn exposes unresolved work without creating placement consent.

Exit zero means a report, not that all history is resolved. Inspect
`result.clock_recovery.counts.unresolved`, `counts.query_supported`, `stop_reason`
and `directory_history_reconciled`. Failed query passes return exit two and
`ok: false`, with earlier results when still publishable. Otherwise the result is
null with a sanitized refusal. After retained evidence changes inventory identity,
inspect again before another explicit pass; old digests/page cursors are stale.

## Custody and limitations

Both original batch and clock directory stay locked through the single stdout
attempt. Short/failed output cannot trigger another object or query. Old file
identities and bytes are pinned across native queries and retention. Only one
successfully verified missing terminal outcome may be added for that selected
record. Ambiguous retention fences the owner, preserving all surviving bytes.
Reopening can inspect valid history but does not retroactively acknowledge a
failed synchronization. Current permission is rechecked during I/O/publication.

No native preparation, commit, cancellation, observation refresh, new scope or
intent, effect retry or quota renewal is possible. Recovery is local cooperative
custody, not cross-directory fencing or malicious-owner rollback protection.
Historical pause is not current pause, route readiness, construction completion
or permission to retry. Production admission and stored/native formats are unchanged.

## Executed evidence

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_scoped_clock_reconciliation.py --mutations
```

All 58 tests pass: 27 unchanged inventory, 19 new reconciliation and 12 CLI cases.
Eight weakened implementations fail assertion-based regressions, not syntax/import
checks. Coverage includes stale selection, query limits, source loss, continuation
after failure, ambiguous-publication fencing, pre-contact output reservation,
premature lock release and failed-pass exit status. All 15 measured production,
test and runner inputs stayed unchanged through execution.

Actual clock framing/codecs execute over TCP to bounded joined synthetic peers,
with real private scope/intent/outcome files, locks and synchronization. Tests
cover mixed families, 16-query limits, the 256-record inventory, absent/prepared/
running replies, lost second replies preserving the first result, wrong receipts,
byte-identical replacement, membership change, revocation and budget/output/sync
failures. Complete CLI/main/runpy and actual Agent Turn rendering execute.

Original route-batch custody and aggregate budget construction are explicit test
ports. This is not full private route-batch, DFHack SDK/game, Rust/MCP, workspace
or production qualification. The older selected-record recovery/cancel suite is
restored to its exact pre-change blobs; these 58 tests do not claim to rerun it.
The two suites have distinct test/runner/contract filenames. Broad beads remain open.
