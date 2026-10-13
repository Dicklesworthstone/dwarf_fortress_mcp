# Exact-frontier furniture recovery

The furniture/1.19 value engine now provides `Engine::reconcile(key, plan,
reader, verifier)`. An attempted placement whose immediate readback failed can
recover its original `Placed` receipt without another call to the writer. This
closes the previous dead end where an unverifiable attempt blocked every new key
for the rest of the native engine's lifetime, even after its exact links became
readable. Bead: `df-dfhack-bridge-plane-c-pic.4` (with the broader integration and
qualification work retained by `.5`).

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

The method accepts no writer. It performs bounded reads and updates only native
retained evidence. A plugin must call it from its authenticated, suspended query
path; this first engine increment does not itself change RPC dispatch. Native
custody loss, time progression, construction progression and external controllers
remain outside this exact-frontier recovery guarantee. It is not a crash-proof
native journal, an attribution proof against other plugins, or construction-goal
completion. Existing durable Rust coordination must still validate and persist
any returned receipt before acknowledging a terminal result.

## Executed component evidence

`python3 scripts/test_build_placement_recovery.py --mutations` runs the actual
C++ engine, not a Python reimplementation. GCC 14 and Clang 17 pass 12 scenarios /
1,985 assertions with C++17, warnings denied and nonrecovering UBSan. The tests
cover all three furniture kinds, failures at each verification boundary, exact
receipt equality, subsequent new-key placement, absent effects, capture/proof
field changes, native source/intervention fences, reentrant callbacks, repeated
failures and immutable historical lookups. Three compiled mutants fail assertions:
cached-query-only recovery, omitted final capture, and failure to release proven
uncertainty. The runner hashes exact input bytes before and after execution.

These are component tests with injected native readers/writers/verifiers. They do
not establish a real DFHack SDK build, generated-protobuf behavior, Rust execution,
live-fortress qualification, global fencing or production admission. No dependency,
wire encoding, existing record state or admitted capability changes.
