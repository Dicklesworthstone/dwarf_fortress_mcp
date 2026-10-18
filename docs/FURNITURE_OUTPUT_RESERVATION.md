# Complete furnishing plans without false output-capacity failures

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`.

The existing single-step and prefix runners reserve the complete possible result
before native contact. Earlier implementations added 32 KiB per single-step
result or 4 KiB per prefix step plus 8 KiB. Those unrelated allowances could
reject valid original plans partway through execution, despite the complete
result fitting the existing limit. A 32-item plan with 48-character names and
eight predecessor dependencies per later step is 14,888 canonical bytes. The
old prefix reservation refused its second eight-item run. Independently of that
prefix check, the old single-step reservation also refused later steps.

Both owners now render a private serialization-size envelope retaining the exact
original plan, prior receipt rows, optional room/terrain metadata and review. It
bounds every selected next row using the canonical codec's numeric, identifier,
phase, Boolean and insertion-field widths, plus the widest progress fields.
The prefix envelope additionally includes every compact outcome and the maximum
RPC, byte and work counters. At most one outcome contains blockers, because a
blocker stops the prefix. The runner enforces a 1,024-byte bound on that encoded
JSON list, including escaping. A 128-byte presentation margin covers remaining
small derived packet-width changes.

These envelopes are deliberately impossible size probes: they are never emitted,
stored, passed into proof admission, or used as execution authority. Actual output
still comes exclusively from the authenticated original owner and native receipts.
The independent prefix validator rejects a size probe as invalid evidence.

No output limit was raised: exact-item results remain bounded to 65,536 bytes and
room-backed results to 196,608 bytes. No steps are silently dropped, original
items substituted, native protocol changed, or confirmation widened. A genuinely
oversized result still refuses before native contact. All time/work/RPC/byte and
current-authority checks remain. The native one-shot client and original-key
recovery rules are unchanged.

The 14,888-byte plan now completes in four explicit eight-item runs. Its complete
final packet is below 45 KiB, including the entire original dependency graph and
all 32 compact receipt rows. The command remains:

```sh
python3 scripts/run_furniture_batch.py review \
  --directory "$BATCH" --batch-id "$BATCH_ID" --steps 8
python3 scripts/run_furniture_batch.py run \
  --directory "$BATCH" --batch-id "$BATCH_ID" --steps 8 \
  --confirm-run "$CONFIRM_RUN"
```

Each later prefix needs its own fresh review of remaining original work. A lost
reply or an Indeterminate receipt stops execution; explicitly query the original
key before reviewing new work. Recovery does not place the next item. A stop
between existing calls remains durable and fences later work.

Regression commands:

```sh
python3 -W error -m unittest discover -s tests -p 'test_furniture_output_bounds.py' -v
python3 -W error -m unittest discover -s tests -p 'test_furniture_prefix_tcp.py' -v
python3 -W error -m unittest discover -s tests -p 'test_furniture_batch_reconciliation_tcp.py' -v
```

The TCP tests use the real Python batch, journal, RPC and CLI code with joined
synthetic native peers. They are not DFHack producer, live-fortress, Rust,
room/terrain-backed end-to-end, or production-admission qualification.
