# Recover original digging behind an interrupted stop signal

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`,
`df-cancellation-progress-certificates-vvw`. Broader execution remains open.

A stop writer can die after creating `stopped.json` but before completing it.
That file must continue to fence new work, but it must not make an earlier,
uncertain native designation impossible to inspect or reconcile. The existing
`dig_blueprint_client.py inspect`, `query` and `cancel` commands now open the
original batch in conservative recovery mode. No new flag is required.

```sh
python3 scripts/dig_blueprint_client.py inspect --directory "$ORIGINAL_BATCH"
python3 scripts/dig_blueprint_client.py query --directory "$ORIGINAL_BATCH" \
  --batch-id "$ORIGINAL_BATCH_ID" --step 0
```

Use the original batch and original recorded step. Query requires the existing
isolated dig opt-in, original endpoint and query credential. It does not require
`DFMCP_DIG_ALLOW_DESIGNATE`; leave that variable unset for query-only authority.
Cancellation retains its existing separate operator enablement requirement.
Resolved receipts remain recoverable offline without credentials or a connection.

## Only the stop signal may be unverified

The complete original manifest (including any room handoff), blueprint, source,
effects-directory identity, registry, intent capsules and terminal receipts
still undergo their original validation. A damaged registry, receipt or intent
is not repaired or excused. There is no alternative weaker room decoder.

Only `stopped.json` may contain zero to 1,024 opaque bytes, and only behind the
original private, single-link, regular-file, no-follow custody checks. Its inode
and exact bytes remain pinned throughout the operation. Active writers,
replacement, wrong permissions, links, special files or an oversized signal
still refuse. Bytes are compared to the existing canonical stop envelope; an
exact valid marker produces the old inventory representation. Other bounded
contents remain unverified, even if they look like another batch's stop record.
No JSON parser treats opaque contents as authority.

Such inventories retain `stopped: true`, `next_step: null` and
`retry_commit_permitted: false`. Their additive `stop_recovery` evidence includes
the marker's byte count, SHA-256 and file identity in the inventory digest.
No raw marker bytes, credentials or paths are disclosed. The owner does not
complete, rewrite, delete, rename or resynchronize the marker.

Default batch opening, new observation/review, advance, prefix execution and the
ordinary stop command remain strict. The recovery owner cannot prepare or
commit through the checked client while stopped. Query and cancellation act
only on an already recorded original key; they never allocate another key,
redispatch an intent, skip a step or continue the blueprint.

Native absence or Unknown evidence cannot resolve an uncertain effect. A verified
Designated receipt proves historical designation readback, not completed mining,
present terrain safety, game pause or native quiescence. An uncommitted original
preparation may be cancelled with the existing enablement; cancellation does not
roll back a designation. An opaque marker remains in place after any recovery.

## Evidence

The new regression suite was executed locally. Its connector upload was blocked
twice; the suite is provided separately with the local validation bundle and is
not included in these repository changes. The CLI suite shares that local fixture
and is likewise retained in the validation bundle. No existing suite is removed.

The initial 18 tests exercise complete ordinary modules, real private files and
locks, the existing native TCP client and a joined synthetic peer. They cover a
lost commit reply, query-only recovery, original-preparation cancellation,
every truncated marker prefix, corrupt original evidence, authority replacement,
malformed receipts, every small-inspection cancellation checkpoint and actual
stop publication against a locked batch. These are synthetic-source Python
results, not native DFHack, Rust, live-game or full-repository qualification.

## Final CLI acknowledgment

When the recovered inventory contains an unverified stop signal, the CLI
reopens the same original batch and effects store under the same operation
budget. It repeats the whole inventory and marker-witness checks before and
after serialization, then keeps root/effects/manifest/marker custody through
the one output attempt. Original query authority remains checked after receipt
retention. This step has no native connection and cannot renew the deadline.

Changed bytes, a replacement inode with identical bytes, missing original files,
expired work/deadline or replaced credentials prevent acknowledgment. A receipt
may already have been durably retained; a publication error never grants another
designation. A short write, failed flush, interruption or failed post-write check
returns failure without another query or output object. No file is repaired.
The original-room export remains the exact complete RoomPlan, without a wrapper
or extra newline; it is still an intention, not a current completion claim.

The final local run passes 61 warnings-denied cases: 18 stop-recovery API tests,
16 publication/CLI/room tests and 27 unchanged shared stop-writer tests. The actual
stop-writer process is killed after its first marker byte while a committed
synthetic designation loses its reply, then the CLI recovers only the original
key. Another case retains 512 targets and eight original receipts. Room-backed
initialization, lost-reply recovery and exact original-goal export use the real
handoff, terrain, room-plan, registry and receipt codecs and TCP client.

An independent import of the exact pre-change owner over the same files preserves
32 legacy inventories, 32 serialized responses and all 32 work-count comparisons.
These are component/Python results with synthetic native sources, not a DFHack
producer build, Rust qualification, physical drain or live-fortress campaign.
