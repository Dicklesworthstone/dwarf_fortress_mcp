# Append-only original placement reconciliation

Bead `df-dfhack-bridge-plane-c-pic.4`; broader qualification remains `.5`.

The native engine can now reconstruct its original Placed receipt at the exact
post-attempt frontier. The Python journal gains a matching explicit recovery
transition, without modifying the existing native wire encoding or permitting a
second placement. This first increment provides the codec/store boundary; native
RPC and CLI recovery integration are separate work.

`build_placement_reconciliation.verify_reconciliation` accepts only an exact
Indeterminate-to-Placed pair with the same complete original plan. It reuses the
strict receipt decoder: the full expected-after capture and stage-zero insertion
links must verify. Similar IDs, counters, completed buildings, empty queries,
refusals and cancellation are not substitutes for original-attempt evidence.
Source acquisition and custody are independently checked by the caller.

`Journal.retain_reconciliation` appends a `reconciled` frame naming the previous
uncertain receipt digest, complete Placed record and original manifest. The
original uncertain frame is retained byte-for-byte in the hash chain. Native
generation and software must equal both the original intent and the uncertain
receipt source. State exposes `reconciled_from` and both receipt identities.
Ordinary `retain` keeps its existing immutable-receipt semantics.

Existing journals remain readable. An extended journal has at most five frames,
within the unchanged 32-KiB file and 16-KiB frame limits. Older readers refuse a
new reconciliation frame rather than silently ignoring it. No automatic migration,
pruning, partial-tail repair, key replacement or overwrite occurs. A completed
resolution is idempotent; a second differing outcome is refused. Reopened handles
never regain preparation/dispatch permission. New independent work is possible
only after every pending local attempt is resolved and normal native checks pass.

The existing private descriptor ownership, full-directory pending fence, file
and directory synchronization and full readback apply to the append. Ambiguous
writes fence the current handle. A complete surviving frame can replay after a
lost sync acknowledgment, but read-only replay does not retroactively certify
that acknowledgment. Partial frames refuse unchanged and remain operator-visible.

Executed on 2026-10-13:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error::ResourceWarning \
  -m unittest test_build_placement_reconciliation -v
```

All 14 actual Python tests pass, including all three furniture kinds, strict
transition rejection, source/plan mismatches, every new-frame truncation and byte
mutation, private reopen, no retry, lost sync acknowledgments, torn writes,
readback failure and maximum-width source fields. The imported original wire and
transport bytes were verified against Git blob identities. No DFHack, native SDK,
Rust, full-workspace or production qualification is claimed.

## Existing query and route-batch integration

The native QueryPlacement integration landed independently in `bb13666`. This
client increment builds on it; it does not modify native engine or plugin code.
`build_placement_client.recover` no longer treats a stored Indeterminate receipt
as a resolved offline result. An explicit query acquires the original key under
current build-read authority, validates the complete receipt, appends the recovery
frame, and rechecks authority/deadline/custody before returning success.

A repeated Indeterminate or absent native record leaves local uncertainty and
its original bytes unchanged. Refusal, cancellation, a different plan, or a
cross-incarnation Placed record cannot resolve the attempted effect. Already
resolved receipts remain offline. Cancellation of a retained uncertain receipt
also remains an offline no-op: it cannot turn an attempt into nonapplication.

The actual transport permits an Indeterminate-to-Placed successor only during
QueryPlacement, with the original native incarnation and full receipt checks.
Neither querying nor accepting resolution creates a preparation permit. The
original native key remains reserved forever in the local journal inventory.
Client output is completely serialized before final current-authority and custody
checks. The CLI keeps the directory owner open through its one output attempt;
short writes, failed flushes and late close errors never emit a second object.
A persisted resolution can be inspected after a subsequent publication refusal;
that does not retroactively acknowledge a failed call.

For an original standalone placement, in the existing build-only environment:

```sh
python3 scripts/build_placement_client.py query \
  --directory /absolute/private/placements --key ORIGINAL_KEY
```

For an original route-backed batch, use its existing query operation:

```sh
python3 scripts/route_furniture_batch.py query \
  --directory /absolute/private/route-batch --batch-id ORIGINAL_BATCH_ID --step ORIGINAL_SLOT
```

Both use only `DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1`, `DFMCP_BUILD_ENDPOINT`
(optional numeric loopback endpoint), and `DFMCP_BUILD_TOKEN` under the existing
strict environment. Placement permission and map/inventory credentials are not
required. Do not use the combined route-placement environment for recovery.
Route-batch query now uses the shared `recovery_requires_native` predicate; its
parent/child locks, original review, association checks and final publication
custody remain unchanged. Future advances still require separate fresh route
review and explicit confirmation. Recovering Placed does not complete construction.

Scope: the standalone and route-backed query entry points are integrated here.
The older `furniture_batch.py` wrapper still treats a recorded receipt as offline;
its retained child can be explicitly recovered with the standalone command over
its original `effects` directory and original key. No Python migration changes
Rust/MCP journal formats; those consumers retain their existing rejection rules.

Additional executed tests:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error::ResourceWarning \
  -m unittest test_build_placement_reconciliation \
  test_build_placement_reconciliation_runtime test_build_placement_batch_recovery -v
```

All 31 focused tests pass: 14 codec/private-journal tests, 14 runtime tests, and
3 route recovery-handler tests. Runtime cases use the actual Python client,
real private journals, an independently framed joined TCP peer, and CLI
subprocesses. They cover original receipt resolution, no-repeat dispatch,
malformed/lost replies, absent or still-uncertain records, wrong incarnations,
revocation during sync, offline reopen, and normal placement dispatch ordering.
The peer uses the strict receipt fixture codec, not a live DFHack instance.

The three route tests compile selected actual production handlers via AST and
inject the parent batch, while executing real child-journal custody and TCP
recovery. They verify the changed credential selector and query/cancel behavior,
not the full parent/route pipeline. Full pre-existing Python suites, real SDK,
native game, Rust/MCP and full-workspace qualification have not run here. No
registry, production runner, authority issuer or admission is widened.
