# Cancel an explicit set of original laboratory plans

`fortress.cancel` accepts a closed `plans:` scope containing one to eight exact
original digests, including a mixture of current-session commits and exact
durable originals recovered after a restart. It drains chosen projects in one foreground call while
leaving other committed projects and pending reviews outside the selection.
The existing eleven-tool surface and handler arguments remain unchanged.

```json
{
  "tool": "fortress.cancel",
  "arguments": {
    "session_id": "<current-session>",
    "mode": "stop_future_steps",
    "scope": "plans:<first-original-digest>,<second-original-digest>"
  }
}
```

Every digest is exactly 64 lowercase hexadecimal characters. The comma-separated
list has no spaces, empty members or suffixes. One member is allowed. Input order
does not affect the canonical selection or execution order. Duplicate digests,
and different aliases of the same actual original, are rejected.

## What the selection owns

Each selected live member must retain its complete immutable commit receipt in
the current session. Selection verifies the original owner, actual sealed plan,
plan ID, complete ordered action list, original step identities and retained
adapter custody. A witness-rebased alias remains an alias: the response records
both the requested and actual digest rather than replacing either identity.

Recovered members require their exact sealed digest, complete original archive,
accepted recovery lineage and every anchored original step disposition. Their
historical owner, archive digest, custody digest and ordered step IDs remain
visible. Recovery does not restore former request aliases, live dispatch handles,
compensation authority or old grants. Current Observe, Query and each required
scoped effect grant are checked before any selected work stops.

Pending reviews, another live session's plans, unknown digests and incomplete
or contradictory custody refuse the entire set before effects. An existing live
receipt remains authoritative even when malformed; recovery cannot bypass it.
An explicit selection does not inspect or adopt unrelated original receipts as
members. The selected-set path never reconstructs dispatch handles.

Continuation descendants are separate original commits. They are included only
when their own exact digests are explicitly selected. The latest-plan pointers,
unselected original receipts, pending review and original goals are retained.
Stopping physical work does not withdraw a goal or establish that it succeeded.

## Complete admission before effects

The coordinator checks the complete set before the first cancellation request,
physical stop, compensation or emergency pause:

- Every selected original has complete, mutually consistent custody. No action
  may belong to two selected originals.
- Current Observe authority and every required original action grant are valid.
  Compensation also needs its own current grant, exact inverse custody and
  supported immediate semantics.
- Original and compensation scopes pass the existing private lease admission,
  including conflicting foreign reservations and unresolved physical work whose
  derived lease records were lost. Refused admission creates no real leases.
- All selected actions, physical stops, inverse effects and at most one emergency
  pause fit the shared action allowance. All current sources, receipts and
  temporary reservation records fit the shared source allowance.
- The complete requested-to-actual identity object is reserved in both the
  result and final certificate before effects. The output reservation is 8 KiB
  plus twice the identity-object bytes, twice the canonical selector bytes and
  68 bytes per requested digest; the token allowance must cover its ceiling at
  four bytes per token. Oversized historical per-step detail can still be omitted
  explicitly by the normal cancellation receipt projection.

Admission is atomic. Execution is the existing progressive durable drain:
newer original commits drain first, and each original's dependents drain before
its prerequisites. All steps share the wall-time deadline that began before set
selection. No polling dispatches prepared successors, and no game time advances.

Live-only sets support all three existing modes: `stop_future_steps`,
`compensate_reversible` and `emergency_pause_and_drain`. Any recovered member
refuses compensation for the complete selection before effects. Explicit emergency mode
pauses the shared fortress clock once under current ControlClock authority;
that global pause can affect other projects' progress even though their actions
are outside the drain. Compensation applies only the admitted original inverses.

## Outcome and final certificate

The result retains the usual `steps`, `drain_progress`, `paused`,
`emergency_pause`, `observed_anchor`, publication state and error fields, plus:

| Field | Meaning |
| --- | --- |
| `selection_scope` | `exact_original_set` |
| `requested_plan_digests` | Complete canonical sorted request |
| `selection_resolved` | Whether the complete requested set resolved to authenticated live originals |
| `selected_plans` | Every requested/actual digest and plan ID; live action IDs or recovered archive/custody, historical owner and ordered step IDs |
| `selected_plans_digest` | Canonical digest of that complete identity object |
| `descendant_plans_included` | `false`; no descendants are selected implicitly |
| `unselected_plans_included` | `false`; the certificate covers only the named originals |
| `dispatch_handles_restored` | `false` for mixed/recovered selections |
| `untracked_work_included` | `false` for mixed/recovered selections; unrelated unknown work is outside the proof |
| `drain_progress.original_plans_total` | Number of explicitly selected originals |

Unknown or incomplete custody leaves `selection_resolved=false` and
`selected_plans` / `selected_plans_digest` null. The full requested list remains
available in the refusal; unresolved action and remaining-work counts are null.
Live-only selections retain resolved identities on later admission refusal.
Mixed/recovered selection is marked resolved only after its entire admission
and initial census succeed. Any refused admission reports that no effect occurred.

Live-only identity and certificate schemas remain `/1`. A set containing
recovered originals uses `dfmcp-selected-originals-cancellation-selection/2`
and `dfmcp-selected-originals-drain-certificate/2`. The certificate includes the
complete admitted identity object and a digest of the final selected census.
Recovered cleanup is ordered by authenticated original cursor, with digest
breaking ties, after newer current-process originals. All members, including
already-quiet ones, remain in the selection and final census. Aggregate progress
separately reports live actions and recovered steps.

A final certificate is emitted only when every selected original action is
terminal, its exact physical work is proven quiescent, and publication is
confirmed. The certificate binds the complete selected identity object, its
digest, the existing original-action certificate and the final observed anchor.
It makes no claim about unselected work, overall fortress quiescence or original
goal success. The underlying Verified or Failed action proof is preserved when
its remaining physical work requires a separate stop.

A publication or execution failure can occur after earlier stops succeeded.
The result then describes the actual partial drain, preserves all selected
identities and withholds the final certificate. A failed durable append fences
later effects. Retry uses the original selected actions and does not replay
completed cleanup. Repeating a successful selection, or reordering the same
requested digests, performs no new physical effect. Request order does not
affect the selection digest at the same custody frontier. Recovered custody can
advance when fresh stop evidence is durably published; a later selection digest
then binds that newer evidence. A subsequent restart still uses the exact
original identities and final dispositions.

## Regression coverage

`plan_set_cancellation_mcp_tests.rs` exercises the real facade across subset and
pending-review preservation, repeats, canonical identities, original failure
proofs, malformed/duplicate/alias selectors, unknown/foreign/lost custody,
aggregate action/source/output refusal, all eight admitted originals, replay
roundtrips, and complete capability and inverse
scope admission before effects. `plan_set_cancellation_recovery_tests.rs` checks
mixed and recovery-only selection, chronological stop order, complete authority
and shared budget refusal before emergency pause, duplicate aliases, malformed
live custody, repeat/restart cleanup, unselected/pending preservation and injected
publication failure/retry while unselected work continues. Output-budget tests preserve all eight member identities
and the exact certificate or error under oversized session orientation. The
modern stdio suite independently exercises the actual registered tool surface.
