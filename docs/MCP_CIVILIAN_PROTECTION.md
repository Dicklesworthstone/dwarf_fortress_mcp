# Named civilian protection through the eleven tools

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The laboratory accepts a `civilian_protection_v1` workflow in the existing
`fortress.plan` action input, and a `civilian_protection` query in the existing
`fortress.query` mode input. No extra tool or direct effect endpoint is added.
The intent crate compiles the actions and predicates; the existing server owns
capabilities, plan sealing, checkpoint policy, dispatch, replay and proof.

## Example: protect the starter fortress's named citizens

Start a `starter_fortress` laboratory session paused, with Plan, Query and Observe
at their needed read scopes, ConfigureLogistics at Guarded risk and Checkpoint
at Guarded risk. The standing order is an existing Guarded action, so its normal
checkpoint requirement is not bypassed. A running fortress must first be paused
through the ordinary explicitly authorized pause plan and commit.

Pass this JSON object as the **string** value of `fortress.plan.actions`:

```json
{
  "workflow": "civilian_protection_v1",
  "burrow": "3001",
  "civilians": ["1001", "1002", "1003", "1004", "1005", "1006", "1007"]
}
```

Inspect the sealed steps and commit the returned digest. Bounded zero-tick
`fortress.wait` calls prove the restriction activation and dispatch/verify the
dependent membership assignment without advancing the paused simulation.
A current pause and source-eligible named-citizen liveness/type observation are
required at preparation and again before deferred dispatch.

Query the current configuration by passing the following JSON as the **string**
value of `fortress.query.mode`:

```json
{
  "mode": "civilian_protection",
  "burrow": "3001",
  "civilians": ["1001", "1002", "1003", "1004", "1005", "1006", "1007"]
}
```

The query reports `configuration_truth` as `true`, `false` or `unknown`, an exact
anchor supplied by the existing session shell, and assigned/unassigned/unknown
membership counts. Only current eligible evidence can set
`configuration_proven=true`. A lost restriction, unassignment, death or lost
pause makes the corresponding complete configuration goal unproven. Unknown
membership is not reported as false. Current truth is recomputed, not copied
from an old completed action receipt. Existing authorized historical queries
retain their separate historical and current anchors.

`physical_arrival_proven`, `threat_clearance_proven` and `native_evidence` are
always false. Membership and a standing order do not prove that a dwarf moved,
that the burrow is safe, that every citizen was selected, or that hostiles left.
This is reference configuration, not native evacuation or automatic recovery.

## Stable recovery and a separately reviewed release

The laboratory now keeps one `CivilianAlertFsm` per observed burrow (at most 16)
in the world owner. Shared sessions use that same owner and history. The monitor
consumes accepted canonical observations, including every foreground wait slice;
repeated Observe/Query calls at the same anchor do not add clear samples.

Query `fortress.query` with `mode="civilian_recovery"`, or with the JSON string
`{"mode":"civilian_recovery"}`. It reports the complete current civilian roster
with entity generations, qualified threat assessment, clear-window start and
sample count, and `release_ready`. This mode accepts no historical `at` selector
and no caller-supplied clearance. `protection_observed=true` means that some
protection is active, not that every civilian has reached the burrow.

A release requires a complete current roster of 1..64 living nonmilitary
civilians, a qualified clear threat domain, observed retained protection, and at
least two consecutive samples spanning 100 game ticks with gaps no larger than
100 ticks. Unknown facts, changed roles/identities/domains, skipped cursors,
same-tick mutations, restore and restart reset stability. Advance the unpaused
world through explicit bounded waits to rebuild the interval. The monitor never
unpauses, protects or releases anyone automatically.

Once `release_ready=true`, pass this JSON as the **string** value of
`fortress.plan.blueprint`:

```json
{"template":"civilian_release","burrow":"3001"}
```

The owner supplies the exact current roster and clearance to a private,
source-sealed `dfmcp.laboratory-civilian-release/1` record. Public input cannot
supply that record or assert a clear interval. Review and commit the digest at
the same observation. Any changed anchor refuses the old release; request and
review a fresh plan. This path does not automatically rebase clearance.

The two dependent actions remove the exact civilian memberships, then set the
restriction to INACTIVE. Each effect checks the complete **current** threat
census and the sealed burrow/civilian generations. A new hostile, unknown fact,
new civilian, role change or reused ID blocks the next effect. Release can run
while unpaused because the guarded census is checked before each effect; this
avoids requiring a pause mutation that itself invalidates a clear interval.
The normal ConfigureLogistics and Checkpoint grants still apply.

The terminal goal proves current released configuration plus the current safe
census. A later threat can invalidate that goal even if the historical release
receipt remains terminal. Protective compensation checks the original entity
incarnations and reasserts protection without requiring threat clearance.
It cannot roll back time or erase an already completed earlier effect.

Durable journals retain the exact source for historical plan reconstruction,
not a reusable clearance grant. Reopening reconstructs observed protection in a
new epoch and starts a new clear interval. See
[LAB_CIVILIAN_RELEASE.md](LAB_CIVILIAN_RELEASE.md) for the classifier, predicate
and compensation contract. The reference model does not establish native
pathfinding, physical evacuation, or burrow suitability.

## Cancellation and scope

Cancellation retains protection. Compensations are explicit protective
reassertions, not inverses that remove pre-existing members. Stopping a pending
step never dispatches that step. Unmentioned citizens' membership is untouched.
Cancellation is not clearance; releasing civilians requires a separate reviewed
request. Current original effect grants still gate every cancellation effect.

The compiler does not need an invented initial false membership matrix. Missing
prior membership may remain unknown while an explicit request sets it true.
Since the workflow cannot invert that effect, it cannot manufacture a false
baseline or remove citizens who were already assigned.

## Bounds, decoding and existing behavior

Requests are closed objects. Only the exact workflow/mode names above are
accepted. Civilian IDs are 1..64 unique nonzero canonical decimal u64 strings;
the burrow must be distinct. The roster decoder rejects an excess entry before
retaining it, rather than trusting an unbounded sequence size hint. Invalid IDs,
duplicate identities, duplicate request fields and unknown fields refuse.
No caller-supplied threat-clearance, source-policy, authority or native-endpoint
field is accepted.

The existing action input limit is 16 KiB and the query input limit is 1 KiB.
The intent compiler bounds its generated representation to 256 KiB and two
steps. The MCP route retains existing negotiated Plan/Query/effect entry checks,
action/entity limits and response fitting. The embedding `prepare`/`evaluate`
methods additionally require an explicit context with exact-anchor, Observe and
encoded-work budget checks; no such context is synthesized by the MCP decoder.
The pure configuration predicate grants no read or effect authority.

Legacy action arrays and the earlier `labor_allocation_v1` decoder retain their
implementations. Other query modes continue to the original laboratory query
handler. No original `lab_world.rs`, server, transport, native protocol,
production runner or compatibility registry source is replaced by this change.

## Validation posture

Eight new Rust facade/decoder regressions cover strict/canonical decoding and source-qualified projection,
example loading, real plan/commit/wait/query/replay, current truth after later
restriction changes and exact historical queries, pending cancellation with pre-existing members, missing
mutation/checkpoint/query authority, and running/malformed target refusals.
Together with the eight core and three actual-adapter cases, nineteen new Rust
test functions are present. At that original increment they were source-reviewed only. Later validation is
recorded in the current implementation status; the historical statement does
not describe the present build environment.

`python3 scripts/test_civilian_protection_schema.py` successfully checked two
Draft 2020-12 schemas, accepted both checked-in examples and rejected all 84
malformed structural mutations. Schema tests do not exercise Rust decoding,
capability checks, deferred execution, cancellation or the native game. Rust
also checks u64 overflow and cross-field burrow/civilian aliasing that these
structural schemas do not express. No Rust, native, live or admission
qualification is claimed.
