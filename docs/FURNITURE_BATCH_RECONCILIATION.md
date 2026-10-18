# Recover an uncertain original furnishing placement

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`.

An Indeterminate receipt is evidence of an attempted operation with an unresolved
outcome. It is not a resolved terminal result merely because a receipt file
exists. The batch and its existing placement owner now use one shared decision
for whether original-key recovery needs a native query.

```sh
python3 scripts/furniture_batch.py query \
  --directory "$ORIGINAL_BATCH" --batch-id "$ORIGINAL_BATCH_ID" \
  --step "$ORIGINAL_STEP_NAME"
```

This uses the existing furniture/1.19 development opt-in, token and original
endpoint. `DFMCP_BUILD_ALLOW_PLACE=0` is sufficient for a query; no placement
permission is acquired. The command cannot select another item, target or key.
It appends only a fully checked same-incarnation Placed receipt to the original
journal, preserving the prior uncertain receipt. Only after that evidence is
retained can a new review consider the next original item. Nothing is advanced
automatically by query, and a stopped batch remains stopped after reconciliation.

Absent native records, repeated uncertainty, malformed or contradictory replies,
and source changes never erase the original uncertainty or grant retry authority.
A lost query or synchronization acknowledgment may leave a complete receipt on
disk: reopen the original owner, never manufacture another placement. A torn
journal is refused unchanged; this command does not repair it.

Resolved receipts stay offline for repeated query/cancel, even without credentials.
Cancellation of a retained Indeterminate attempt also stays offline and pending:
it cannot prove nonapplication or remove a building/job. A missing receipt still
uses the original native query/cancel semantics. Current authority is checked
after query and final result formatting, including the exact-item batch profile.

The native clients, one-shot commit rules, receipt codec, journal framing, batch
identity and key sequence are unchanged. Exact-item custody no longer imports
unused room/terrain code. Selecting a handoff-backed batch still invokes its
original complete validators; missing or invalid dependencies do not downgrade
that batch to an exact-item definition.

Run the real-file/TCP component regressions:

```sh
python3 -W error -m unittest discover -s tests \
  -p 'test_furniture_batch_reconciliation_tcp.py' -v
```

The peer supplies synthetic native bytes. These tests are not DFHack or live-game
qualification, nor proof of construction completion or current usability.
