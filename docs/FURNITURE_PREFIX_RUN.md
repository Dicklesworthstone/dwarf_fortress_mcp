# Run an original furnishing prefix

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` (WP-05/WP-10).

`scripts/run_furniture_batch.py` reviews and executes the next exact 1..8
furnishings of an ALREADY initialized batch. The complete original plan may
contain up to 32 items. The command does not initialize a replacement batch,
allocate replacement items, alter targets/dependencies, or choose new keys.

## Review and execute

For an existing batch with at least three unstarted original steps:

```sh
python3 scripts/run_furniture_batch.py review \
  --directory "$BATCH" --batch-id "$BATCH_ID" --steps 3

# Review the original plan and result.prefix_review.steps. Use
# result.prefix_review.confirm_run from THAT review, with the same count and ID.
python3 scripts/run_furniture_batch.py run \
  --directory "$BATCH" --batch-id "$BATCH_ID" --steps 3 \
  --confirm-run "$CONFIRM_RUN"
```

`review` is offline and does not load native credentials. It still reopens and
verifies original custody, including original terrain history when required.
`run` requires the existing isolated furniture/1.19 environment:
`DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1`, `DFMCP_BUILD_TOKEN`, the original
`DFMCP_BUILD_ENDPOINT`, and `DFMCP_BUILD_ALLOW_PLACE=1`. No token enters output.
Other DFMCP profile/admission state remains rejected by the existing authority.

This confirmation has a different scope from a single native-plan review. It
approves fixed original items, targets, keys and dependency order under FRESH
per-step native validation, not a frozen collection of unseen terrain captures.
No eligibility, current pathfinding, construction completion or reservation is
implied by the offline review. Counts are exact, never silently clamped.

## Execution and custody

The pure `furniture_prefix.Prefix` contract seals the complete owner inventory,
original plan digest and exact next steps. Every original room/terrain metadata
field also participates in the inventory commitment. Imported reports or a
self-consistent digest never authorize effects; execution rebuilds the review
from the actual original owner before matching the operator's confirmation.

The runner delegates each new step to the EXISTING `furniture_batch.review` and
`furniture_batch.advance` functions. These retain their native capture/source,
predecessor tick/sequence/ID-floor, write-ahead intent, registration, prepare,
one-shot commit, and receipt checks. The newly returned native plan digest and
review seal are used only after the complete inventory still matches. No old
native preparation permit is restored and no commit flag is reset.

The original locks are held by each existing review/advance call, not for the
whole prefix. Another controller or the existing furniture-batch stop command
can intervene between calls. Changed inventory invalidates the next step;
a concurrent action after review is rejected by the existing commit-time
confirmation. The final output reopens, rechecks and HOLDS the original batch
and required terrain owner through serialization and the single output write.
This is local cooperating custody, not a global fence or hard realtime stop.

## Budgets and outcomes

One initial cooperative wall deadline (default 10,000 ms, selectable in
1..60,000 ms), eight-million-check work allowance, RPC allowance and byte
allowance cover the whole foreground operation. Each selected step reserves
capacity for two existing connections: eight review frames and eleven advance
frames. The total cap is 19 frames and 6 MiB per selected step, at most 152
frames and 48 MiB. Native per-frame/transfer limits remain unchanged. No budget
is renewed at a step, connection or original-terrain reopen.

The complete original output plus conservative per-step growth is reserved
before native contact. Budgets may stop a large history before all selected
steps complete. There is no partial-plan shrink, automatic reconnect/recovery,
new item search, game unpause, background worker or hidden subprocess.

`result.prefix_run.stop_reason` is `reviewed`, `prefix_completed`, `all_placed`,
`preflight_blocked`, `native_refused`, `native_cancelled` or `indeterminate`.
The first three are successful command results; the others exit 2. A broader
exception returns `FURNITURE_PREFIX_REFUSED` with unknown effect status, no
unverified inventory, and no automatic query or second attempt.

The result retains the full original plan and all steps, plus compact selected
outcomes and budget counters. `steps_considered` includes blocked preflight;
`steps_recorded` counts newly retained original children. `dispatch_recorded`
is journal history, not a claim that an uncertain effect was applied. A placed
prefix is historical placement/job registration, NOT finished construction,
room assignments, current usability or live production admission.

## Explicit recovery

Earlier effects are not rolled back when a later step fails. Preserve the batch
and reconcile through its unchanged original-key workflow:

```sh
python3 scripts/furniture_batch.py inspect \
  --directory "$BATCH" --batch-id "$BATCH_ID"
python3 scripts/furniture_batch.py query \
  --directory "$BATCH" --batch-id "$BATCH_ID" --step "$ORIGINAL_STEP_NAME"
```

Unknown, indeterminate, refused or cancelled children prevent another prefix.
After explicit recovery, request a NEW review only for remaining original work.
Any recorded child changes the old confirmation. A blocked preflight with no
new record may leave the inventory unchanged; another run is still an explicit
call with fresh checks, never an internal retry. Do not recreate the batch to
evade uncertain keys. Short or failed output never causes a second execution.

## Executed evidence and limits

Fifty Python tests pass with warnings denied: 20 pure-policy tests (including
3,384 count/offset combinations in 1..32-step plans), 24 executor/CLI contract
tests with deterministic owner/native doubles, and six tests executing the
actual existing `review`, `advance`, `select_next` and `review_seal` function
bodies with lower-level owner/capture/effect doubles. The boundary suite loads
those functions from repository source via AST, not a replacement implementation.
For the partial local checkout, the same fetched function bodies were supplied
through its test-only `FURNITURE_PREFIX_TEST_SOURCE` path. The production command
has no backend/path/method selector. No validation adapter is shipped in runtime.

These tests establish policy and API-seam behavior, not real private-journal
recovery, native framing/receipts, DFHack execution or live fortress behavior.
No full-module import graph, full-repository, Rust/native or live qualification
is claimed. The existing dependency model was checked against Git blob
`52fe43ce2d6514f3f0d1a3d983cc53c31fb39143`; all new uploaded Python blobs match
the tested local bytes. Broad native execution and admission beads remain open.

```sh
python3 -W error -m unittest discover -s tests -p 'test_furniture_prefix*.py' -v
python3 -W error -m unittest discover -s tests -p 'test_run_furniture_batch.py' -v
```
