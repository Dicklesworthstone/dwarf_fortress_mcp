# Review an original production preparation after restart

The combined `serve-gameplay-dev labor-order` host can explicitly review an
original labor assignment or finite order that is still Prepared and has never
entered possible dispatch. This closes the gap between retaining a native
preparation across restart and obtaining a usable current confirmation.

The host remains an unadmitted development profile. Reopening by itself restores
no review, current goal proof or permission to dispatch.

## Public workflow

Keep the complete original archive, native journals, semantic associations and
latest recovery object described in [combined production](COMBINED_PRODUCTION_MCP.md).
Reopen the original owner in Control mode with the operator's existing native
configuration and current authority. Inspect the retained original.

Pass this object as the existing Plan tool's JSON `request`:

```json
{
  "kind": "review_prepared",
  "original_plan_digest": "<the complete original plan digest>"
}
```

The request uses the existing original selector. It accepts no replacement
idempotency key, native token, child path, source identity, expiry or step.
Exactly one eligible active Prepared frontier identifies the step.

A successful response uses the ordinary `review` and `native_review` shapes.
Review the selected native effect, retain the returned recovery object, then use
the existing Commit tool with that response's `review.paired_seal` and explicit
`confirmed=true`. Each successful review publishes a new frontier head and
paired seal. Earlier seals no longer authorize Commit.

Once the original effect is Applied, the existing sequencing workflow can
release its dependent step. Historical application still does not prove that
the complete original production goal has been achieved.

## What the fresh review checks

- The current owner and both native child owners must permit Control.
  Offline and Reconcile modes cannot issue a commit review.
- The selected original frontier must be Prepared with no possible dispatch.
  Attempted-dispatch history is sticky even if a later native query says Prepared.
- The complete original, selected step, semantic association, native preparation
  and all required predecessor receipts must remain in exact custody.
- Fresh independent canonical evidence and the fresh native capture must still
  match the original reviewed effect and source identity.
- Opening and current scoped authority must intersect, and the original
  preparation expiry and parent deadline must remain valid.
- The new review must publish durably and fit the existing response budget.
  Failure withdraws process-local commit permission.

The executor reuses the existing native preparation. It does not send another
native Prepare RPC, replace the original semantic key or native token, extend
expiry, alter the archived plan or move the original deadline.

The durable coordinator appends the existing `RecoveredPrepared` event before
issuing the new head-bound confirmation. Recovery replays history without
reconstructing a process-local review.

## Refusal and uncertainty

A changed source capture, revoked grant, missing original custody, mismatched
native state or expired preparation refuses without making a replacement
preparation. If dispatch may already have occurred, use the original key's
existing reconciliation path. Fresh review cannot turn that uncertainty into a
retry.

A failed response or publication does not leave a usable local confirmation.
Inspect retained recovery metadata before the next explicit action.

## Regression coverage and execution boundary

The Rust regression sources cover durable fresh-seal replacement, original
expiry and grant revocation, possible-dispatch refusal, failed publication and
concrete restarted workforce/order owners with source and custody checks.

`tests/stdio/combined_prepared_review.py` adds eight modern-server scenarios:
labor recovery through dependent order completion, order recovery with its
predecessor, repeated fresh reviews, Offline/Reconcile refusal, changed workforce
capture, changed order capture, lost-reply refusal and invalid selectors.

Run the campaign against a newly built exact-source executable:

```sh
python3 tests/stdio/combined_prepared_review.py \
  --binary target/debug/dwarf-fortress-mcp \
  --logs target/prepared-review-stdio
```

The log directory must be new. The campaign uses the actual modern MCP server
and authenticated TCP peer fixtures. Those peers model native replies; they
provide component evidence rather than a Dwarf Fortress game trial.

The execution environment became unavailable during this increment. These
new Rust and stdio tests have not been executed, and no full-workspace,
native-game or production-admission qualification is claimed.
