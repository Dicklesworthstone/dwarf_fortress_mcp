# Original placement recovery: custody and retained source history

This increment extends the original-query reconciliation path described in
`BUILD_PLACEMENT_CLIENT_RECOVERY.md`. It preserves its explicit `reconciled`
frame, original-indeterminate receipt reference, query-only transport transition,
exact original manifest requirement and immutable normal receipt handling.
No generic successor, native protocol, dispatch permit or production admission
is broadened. Beads: df-dfhack-bridge-plane-c-pic.4/.5.

## Recovery boundaries

Online recovery now requires writable private journal custody before opening a
native connection. Query may complete a native proof and clear the native pending
fence; a read-only owner must not contact that path and only then discover that
it cannot retain the proof. Resolved receipt inspection and the existing offline
cancellation no-op for an attempted placement remain offline and read-only.

`State.source_generation` exposes the generation of the last durably retained
receipt, or the original intent generation before a receipt exists. Handshake,
reply binding and normal receipt retention refuse regression below this floor.
This does not claim durable retention of every transiently observed manifest.
The separate exact-manifest reconciliation check is unchanged: a known source
change still cannot authorize original-frontier positive reconciliation.

An absent native lookup no longer replaces a known uncertain receipt's output
with a null effect or an unknown-absent status. The result retains its historical
receipt and `indeterminate` state, sets `native_record_present: false`, and stays
pending. Absence is neither nonapplication nor permission to retry. Repeated
uncertainty and absent replies append no frames.

Before an indeterminate receipt is appended, the store reserves one complete
bounded frame for its later exact positive proof. The maximum remains 32 KiB per
journal and 16 KiB per frame, with at most five semantic transitions. Positive
reconciliation preserves the uncertain prefix and original dispatch history.
File sync, directory sync and readback still precede acknowledgement. Partial
writes remain unrepaired and fenced. Complete bytes surviving a failed sync may
be inspectable on reopen, but do not claim that the failed call acknowledged them.

## Executed checks

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_build_placement_wire test_build_placement_store \
  test_build_placement_receipt_recovery \
  test_build_placement_recovery_transport -v
```

All 53 tests pass on Python 3.13.5: 19 existing wire cases, 15 existing store cases,
12 new receipt/custody cases and seven new real TCP/CLI cases. The original wire
and store test files are unchanged. Tests use the unchanged eight-value native
fixture corpus, actual codecs, real private files and finite joined test peers.
The main-branch predecessor at 5271476 fails the new regressions for absent-reply
status loss, native contact through read-only custody, retained source rollback
and missing positive-frame reservation.

The tests exercise start -> uncertain receipt -> reopen -> revoked placement
permission -> explicit query -> durable original Placed receipt, with one peer
placement call total. They also exercise lost positive replies, immutable normal
receipts, original endpoint/selection binding, same-connection permit refusal,
credential revocation, all prefixes and byte corruptions of a positive frame,
maximal text bounds, failed sync and torn append.

A source-bound local mutation run rejects six weakened implementations: cached
receipt short-circuit, omitted file sync, retained source-floor rollback, removed
read-only contact guard, absent-reply status erasure and omitted positive-receipt
reservation. Original input hashes are checked before and after execution.

This is Python component and loopback qualification, not a live DFHack game,
real SDK/protobuf-runtime build, Rust/MCP coordinator execution, complete route
parent suite or full-workspace qualification. Concurrent commits b2fede7/5271476
are preserved; their separate test counts are not added to this run. Native RPC
recovery evidence is separately scoped in `BUILD_PLACEMENT_RECOVERY.md`.
