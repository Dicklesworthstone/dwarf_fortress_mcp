# Current construction quiescence before dependent delivery

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`.

A furniture/1.19 Placed receipt records registration of a construction job, not
completion of its hauling or installation. A placement sequence alone therefore
cannot enforce a physical delivery sequence while earlier jobs remain active.

`construction_prefix.assess_prefix` evaluates 0..32 original Placed receipts
against one strictly decoded complete operations/1.4 capture. Every selected
building must retain its exact kind, footprint and maximum construction stage;
the original singleton item must retain type, subtype and material, be installed
in that building at the exact target, have no container or job attachment, and
pass the existing installed-item flags policy. No held job of ANY kind may remain
on the selected building. Unknown job kinds cannot masquerade as quiescence.

Pending/suspended/missing-job construction, removal, missing objects and changed
identities remain distinct outcomes. The whole prefix is checked together; a
single completed predecessor cannot hide a second unfinished one. An empty
prefix means no predecessor requirement, never an empty-fortress assertion.

The stateless `construction_receipt.assess_placed_receipt` factors the existing
condition out of its temporal Goal wrapper. It creates no artificial deadline
or monitoring streak, including at the maximum representable tick. Existing
Goal assessment and monitoring semantics and encoded bytes remain unchanged.

The execution owner must authenticate completeness and ordering of the supplied
prefix, retain private journal custody, bind the exact acquisition/source and
current authority. The pure predicate does none of those things. Its report binds
all selected original receipts and the complete capture; its semantic digest
excludes only computational allowance fields. One capped work allowance covers
receipt validation, full roster decoding and all predicates. Output is bounded
to 64 KiB; abort, malformed bytes, exhaustion or final guard failure returns no
partial result. No native calls, new protocol, dispatch or retry authority exist.

This is single-capture selected-construction quiescence, NOT durable goal
achievement, continuous stability, global quiescence, causal attribution,
current usability, production admission or permission to discharge an effect.
The source module is ready for the execution-owner integration; until that
integration lands it does not itself change route-batch readiness.

## Executed component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest test_construction_prefix -v
```

23 tests execute actual unchanged furniture wire codecs and the complete
operations decoder. Fixtures independently encode native observation bytes.
They include all 512 item flag words, 32 predecessors, unknown held jobs,
missing/changed identities, every representative receipt/capture truncation,
receipt byte corruption, duplicate and reordered identities, exact work limits
and final publication refusal. Six weakened variants fail assertions. An
additional exact-baseline comparison exercises 1,566 assessment/monitor cases
with identical old/new results. These are component tests with synthetic native
observations, not TCP, private batch, Rust, plugin, live-game or full qualification.
