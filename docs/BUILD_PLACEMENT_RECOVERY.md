# Exact-frontier furniture recovery

The furniture/1.19 value engine provides `Engine::reconcile(key, plan,
reader, verifier)`, now called by the authenticated `QueryPlacement` RPC.
An attempted placement whose immediate readback failed can recover its original
`Placed` receipt without another call to the writer. This closes the native dead
end where an unverifiable attempt blocked every new key for the rest of the
engine's lifetime, even after its exact links became readable. Bead:
`df-dfhack-bridge-plane-c-pic.4` (with broader integration and qualification
work retained by `.5`). Durable client integration remains separately scoped.

Recovery is deliberately narrower than generic building discovery. The original
key and plan must be retained in `Indeterminate`, the uncertainty fence must
still be held, and generation/intervention sequence must equal the original
post-attempt values. A changed incarnation or pause intervention refuses even
to consult the native reader. An absent key is not nonapplication evidence.
Prepared, refused, cancelled and placed keys remain immutable historical lookups.

The actual proof path is shared with immediate commit. It reads the full capture,
requires byte equality to the original expected-after capture, independently
verifies the exact stage-zero building, construction job and selected-item links,
and repeats the full capture after that verification. The original tick, paused
state, map/site identity, both ID horizons, all nine terrain cells and all selected
item attributes must agree. A similar building, increased counts, a later completed
building, or a restored-looking state after a pause/source intervention is not
sufficient. No witnessed field is rebased or silently dropped.

Only a fully validated candidate record is published. Its receipt is byte-identical
to successful immediate verification. The original key/token/plan and creation time
are retained; the table entry is not replaced or evicted. Clearing the native
uncertainty fence permits a separately prepared new effect, never redispatch of
the recovered key. All exceptions or contradictions leave the prior uncertain
record unchanged. Cancellation still does not deconstruct, detach or undo anything.

The method accepts no writer. The RPC supplies only the existing capture and exact
insertion verifier. Authentication and exact optional-field checks precede reads;
revoking placement permission leaves authenticated query available. A lost recovery
reply can be queried again without repeating either placement or a completed proof.
No RPC fields, method names, wire encodings or production capabilities are added.
Native custody loss, time progression, construction progression and external
controllers remain outside this exact-frontier guarantee. It is not a crash-proof
native journal, an attribution proof against other plugins, or construction-goal
completion. Durable coordination must validate and persist returned evidence
before acknowledging resolution.

## Executed component evidence

`python3 scripts/test_build_placement_recovery.py --mutations` runs the actual
C++ engine, not a Python reimplementation. The engine increment's GCC 14 and
Clang 17 runs passed 12 scenarios / 1,985 assertions with warnings denied and
nonrecovering UBSan. Those tests cover exact receipts, source/intervention fences,
reentrant callbacks, repeated failures and immutable historical lookups.

The RPC integration adds:

```sh
python3 scripts/test_build_placement_rpc_recovery.py --mutations
python3 scripts/test_build_placement_rpc_recovery.py --compiler clang++ --mutations
```

Both GCC 14.2 and Clang 17 pass six grouped scenarios / 2,487 assertions against
the actual plugin translation unit and unchanged explicit SDK/message doubles.
The original cached-query handler fails the positive-recovery assertion. Three
compiled mutants are rejected on each compiler: cached-query-only dispatch,
missing final capture, and missing source/intervention pre-read fencing. Coverage
includes all three furniture kinds, transient capture/verifier failure, absent and
partial effects, exact relationship failure, authority revocation, all 511 invalid
query field masks, original-key/digest binding and lost recovery replies.

These component executions are not a real DFHack SDK build, generated-protobuf
serialization, Rust execution, a live-fortress campaign or production admission.
The existing broad native suite and full workspace are not qualified by this
focused RPC recovery run. Input hashes are checked before and after execution.
