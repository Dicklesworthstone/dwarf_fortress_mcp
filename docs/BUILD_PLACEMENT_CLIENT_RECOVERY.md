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
