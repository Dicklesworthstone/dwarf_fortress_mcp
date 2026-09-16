# Durable cancellation before pause dispatch

The control/1.7 development server now implements `fortress.cancel` for a pause effect that is
still durably `prepared`. This closes the gap between preparing an effect and deciding not to
execute it: the original key can be retired durably instead of remaining dispatchable forever.

This is implemented, unadmitted development source. The Rust changes and regression tests have not
been compiled or executed in the editing environment. No native method, native wire format,
dependency, production runner, compatibility entry or game-effect capability is added.

## Cancel the exact preparation

Send the exact key and plan digest returned by prepare or durable effect discovery:

```json
{
  "session_id": "<live control session>",
  "idempotency_key": "pause-maintenance-001",
  "plan_digest": "<same 64 lowercase hex characters>"
}
```

Call the existing `fortress.cancel` tool, not `fortress.plan` or `fortress.commit`. A prepare token
is not required to retire the preparation; current ControlClock authority, the writable journal,
and its exact retained key/plan identity are required. There is no bulk cancellation, arbitrary
command, native cancellation RPC, pause toggle or automatic compensating action.

This operation belongs to control/1.7. It is different from spatial/1.8's `fortress.cancel` with
`scope="session"`, which closes a read-only session. Cancelling a pause preparation does not close
its control session or release the native bridge's retained preparation slot.

A successful acknowledgement includes:

```json
{
  "cancelled": true,
  "scope": "this_control_journal_before_dispatch",
  "coordinator_dispatch_prevented": true,
  "same_key_reusable": false,
  "native_cancellation_performed": false,
  "mutation_dispatched": false,
  "global_effect_absence_proven": false,
  "current_freshness_proven": false
}
```

These fields occur under the control packet's existing `result` object. The complete record and
current journal identity/head are also returned. Cancellation does not prove the current pause
state or that another independent controller has never acted on the bridge.

## The state transition is not a native failure receipt

The sole new state-machine edge is:

```text
Prepared -> CancelledBeforeDispatch
```

The JSON state is `cancelled_before_dispatch`. It is terminal for this coordinator and permanently
blocks dispatch of this journal's retained key. The original plan digest, requested pause state,
expected tick, bridge generation and prepare token are retained unchanged. A cancellation record
has no observed pause state/tick or native receipt, and its `effect_known` and `effect_applied`
flags remain false. It is not relabeled `verified_not_applied`.

A cancelled record cannot become prepared, started, indeterminate or verified. The same key and
content passed to prepare return the existing cancelled record, not a new preparation; changed
content still conflicts. Commit refuses even an otherwise matching token. A newly considered
pause effect requires a new explicit preparation and a new key, not reuse of the cancelled one.

`commit_started` and `indeterminate` records cannot be cancelled: dispatch may already have
occurred. Their cancellation request returns `effect_indeterminate` with
`reconciliation_required=true`; `mutation_dispatched=false` describes the cancellation call, not
the earlier attempt. The original record is unchanged. Verified applied/not-applied outcomes also
cannot be replaced by cancellation. An applied pause is not undone by requesting cancellation.

The existing control-session mutex serializes cancel and commit. If cancel wins the Prepared
transition, commit is refused before transport. If commit-start wins, cancellation cannot remove
its durable attempt evidence. A caller racing cancellation against a commit cannot assume that
the cancellation won without its successful acknowledgement.

## Publication ordering and recovery

```text
Check current authority, identity and journal custody
-> construct and validate the exact candidate record
-> render and bound the complete acknowledgement, including Agent Turn
-> append and sync cancellation evidence
-> publish the in-memory record and journal head
-> return the acknowledgement
```

Optional `max_bytes` and `max_output_tokens` may only narrow session limits and must be positive.
The existing byte/4 token estimate is explicit; no tokenizer-exact claim is made. A response that
cannot fit is rejected before writing cancellation or retiring the key. The preparation remains
unchanged. A cooperative deadline is checked before publication; synchronous filesystem calls
are not hard-preemptible.

A partial write or uncertain sync fences the journal and returns no cancellation acknowledgement.
Normal reopening refuses an incomplete trailing record without modifying it. A fully written
cancellation frame may replay as cancelled even when its original sync/response failed. Inspect
that original journal before making any subsequent control decision. Existing explicit operator
tail repair remains separate: it cannot turn an unacknowledged, incomplete cancellation into a
claim that cancellation succeeded, and it never silently discards a complete cancellation record.

Repeated successful cancellation returns the same cancelled record without another append or sync.
If unrelated transitions advanced the journal, the response still reports the actual latest head
and transition count; the older cancellation record does not rewind global metadata. Discovery
continuations obtained before a new cancellation become invalid because the journal head changed.

This is not an anti-rollback mechanism or a distributed exclusion guarantee. Removing/replacing
journal evidence, independent coordinators, hostile-host behavior and native retention management
remain outside this local cancellation contract.

## Discovery and bridge independence

`fortress.query` now accepts `state="cancelled_before_dispatch"`, and its state counts distinguish
cancellation from native verified outcomes. `nonterminal` and `reconciliation_required` exclude
cancelled records. An empty cancellation listing is scoped only to the selected journal.

`fortress.explain` returns cancelled evidence without querying or reconnecting to DFHack. A bounded
reconciliation pass skips cancelled keys, just as it skips other terminal records. It must not
construct a native non-application receipt for them.

Cancellation itself has no bridge call or reconnect path. It therefore works in an already-open
writable control session even when its connection is fenced or unavailable. Opening a new live
control session still uses the existing bridge handshake; this increment does not introduce an
offline writable bootstrap. Recovery-only sessions can list/explain saved cancellations but cannot
write them, even if an internal caller injects a clock grant.

Public journal mutation entry points also recheck authority and file custody before returning
cached idempotent or terminal results. The low-level `lookup` accessor remains a cached accessor;
MCP handlers perform their checked journal access before using it.

## On-disk compatibility

Existing header, frame, digest domains and state tags 1..5 retain their encodings. Cancellation uses
previously unsupported state tag 6 and the existing immutable identity fields. Older readers reject
tag 6 rather than mistake it for an ordinary failure or a dispatchable prepare. Do not downgrade a
journal that contains cancellations to an older reader. Complete unsupported-state records are not
incomplete tails and cannot be removed by normal incomplete-tail repair.

Replay rejects cancellation after dispatch or another terminal state, and cancellation records
carrying native outcome evidence. It also rejects an absent pause observation accompanied by a
nonzero raw observation tick instead of silently dropping that contradictory field.

## Validation status

Twenty-one new Rust test functions are registered: ten adapter cancellation/fault tests, two public
binary-vector tests, eight actual MCP-handler scenarios and one discovery-state test. Existing
recovery tests are retained and extended; the control handler suites share one test gate so their
one-session fixtures do not compete for the runtime's fixed capacity.

Coverage includes restart, exact identity, read-only refusal, authority/expiry, pre-write rendering
failure, partial writes, uncertain sync, forged transitions, idempotent head preservation,
cancellation/commit-start races, no-connection handlers and query-only reconciliation skips.
**None of these Rust tests has been compiled or run here.** Rust, Cargo and rustfmt are unavailable.
No full-repository, native, live-game or filesystem crash qualification is established.

The executed independent Python framing reference rejected 564 single-byte corruptions, 562
incomplete prefixes, 43 correctly hashed but contradictory outcome mutations, and four additional
invalid/legacy cases. It generated a 564-byte Prepared/Cancelled golden journal whose SHA-256 is
`25f115661f385a23817e59069ac9c7317bc53002244af4671a4ef27e39c46c9d`; the Rust tests assert that exact
vector when executed. These checks run a narrowly scoped independent Python model, not the Rust
codec, state machine, MCP transport or real storage. They do not prove crash durability.

Focused commands on a configured checkout:

```bash
python3 scripts/test_control_cancellation_vectors.py
cargo test --locked -p dfmcp-adapter control_effect_journal
cargo test --locked -p dfmcp-adapter --test control_cancellation_vectors
cargo test --locked -p dfmcp-mcp live_control_server -- --test-threads=1
```

The executed Python script SHA-256 is
`9f5f194590348d8552767857001ac7018624c4e95e5f375fdabe9e2de884b0f2`.
Its Git blob was verified against the committed script. This evidence does not widen production
admission or qualify the prior/current live mutation path.
