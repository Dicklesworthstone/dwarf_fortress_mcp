# Discover construction-clock recovery work after a context reset

The existing recovery CLI can now discover retained construction-clock records
without a remembered batch ID or progress confirmation:

```sh
python3 scripts/recover_route_furniture_clock.py inventory \
  --clock-directory /absolute/private/clock-records \
  --selection pending --limit 16
```

This operation is fully offline and requires no native credentials. The directory
must contain only original scoped-clock files: `progress-<confirmation>.run`, its
`.scope.json`, and an optional `.outcome.json`. Both unchanged intermediate Scope
and final-construction FinalScope codecs are accepted. Discovery reads their
historical batch association; it does not reconstruct or open a furniture batch.

## Complete evidence, bounded presentation

The entire selected directory is validated before returning any page. Missing
intent/scope pairs, orphan outcomes, foreign names, corrupt or noncanonical bytes,
symlinks, hard links, special files and changed custody refuse the entire census.
Nothing is repaired, deleted, rewritten or silently skipped. The original selected
`inspect`, `query` and explicit `cancel` commands remain available separately.

Results include the directory fingerprint, all-record count, filtered counts,
compact rows, an explicit truncated flag and a continuation. Optional `--batch-id`
filters presentation but never skips validation of other records. Selections are:

- `all`: every selected record;
- `pending`: records lacking retained terminal evidence and needing native query;
- `terminal`: every retained terminal receipt, including source loss;
- `attention`: retained source-loss records, which remain unresolved.

An absent receipt is not evidence of nonapplication. A stopped receipt proves a
historical native pause, not current pause. A source-lost terminal receipt still
requires attention and is not counted as resolved.

Continue with the returned `--continuation`, preserving selection, batch filter
and limit. The checked cursor binds these parameters to exact directory identity,
file identities and bytes. Publication, replacement, removal or addition invalidates
it. It is a local integrity cursor, not authenticated authority or proof that prior
pages were consumed. A fresh inventory restarts presentation after a change.

## Ownership and limits

One exclusive private-directory lock spans the scan, revalidation and single output
attempt. Per-record readers borrow that same lock and close sequentially; descriptor
use does not grow with the record count. Original intent/scope/receipt fingerprints
and content digests are rechecked before publication. Cooperative locks and hashes
do not defend against a malicious owner rolling back all independently held evidence.

Limits: 256 records, 768 directory entries, 32 rows per page, 64 KiB output, unchanged
8 KiB file bounds, 256 MiB conservative disk allowance, 100,000 custody work units and
one nonrenewable 1..60,000 ms deadline. No network request occurs. Filesystem calls
and synchronization are not hard-real-time preemptible.

`complete_clock_inventory_verified` means only the selected private directory.
It does not mean all clocks in a fortress, original furniture-batch custody,
construction completion, current readiness, current pause or mutation permission.

## Executed component evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONWARNINGS=error \
  python3 scripts/check_scoped_clock_recovery.py --mutations
```

All 50 tests and 13 assertion-based mutation checks pass on the uploaded source.
The 16 new inventory tests cover mixed scopes/batches/outcomes, stable pagination,
replacement and membership races, orphan/corrupt records, borrowed locking, budget
and path refusals, actual CLI subprocesses, single output, and all 256 terminal
records with bounded descriptor use. The preceding 34 tests retain real TCP client
coverage against joined independent test peers. The completed restored-source run
measures all 14 production/test/runner files before and after execution.

No real DFHack process, plugin build, Rust/MCP or full-workspace qualification was
executed. Production admission, native methods and execution owners are unchanged.
Beads df-dfhack-bridge-plane-c-pic.4/.5 remain in progress.
