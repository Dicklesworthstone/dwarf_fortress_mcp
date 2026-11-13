# Durable original action and reserve evidence

Bead: `df-lab-selected-plan-wait-s4q`. Machine contract:
[`laboratory_durable_original_evidence_v1.json`](../architecture/laboratory_durable_original_evidence_v1.json).

The default laboratory can retain an original project's exact final action
receipts and its first selected reserve endpoint after action retirement,
compaction and process restart. The caller continues to select the original
plan digest through `fortress.wait`; no tool or action handle is added.

These records solve two different losses of evidence. Retiring a completed
action commit previously discarded terminal receipt rows. A selected future
reserve endpoint previously survived only inside the originating process.
Both kinds of evidence now have bounded durable custody when the fortress
uses the operator-configured durable laboratory store.

## What a resumed wait can claim

A recovered selected wait checks the unchanged original request, reconstructed
sealed plan, complete expected action census, exact archived receipt anchors
and the current verified recovery lineage. Terminal receipt history can
establish what the original monitor recorded. Current physical work and
current project truth are evaluated separately. An archived receipt does not
recreate a dispatch handle, a cancellation handle or a grant.

For an original future reserve, completion also requires the first selected
observation at the original absolute game tick. Recovery loads that exact
snapshot and derives the complete original output predicate using the stock
ledger identity captured from the sealed source. The journal stores the anchor
and snapshot bytes; it stores no Boolean conclusion to trust on its own.

The selected result reports:

| Field in `reserve_frontier` | Meaning |
| --- | --- |
| `observed_anchor` | The exact first selected endpoint, potentially in a prior verified process epoch. |
| `truth` | Original output truth at those immutable endpoint bytes. |
| `available` | A selected endpoint was actually observed and retained. |
| `durable` and `survives_restart` | The exact endpoint was durably published or independently recovered from the durable record. |
| `retention` | `durable_original_exact_snapshot` for durable custody; otherwise `owning_session_exact_snapshot`. |
| `survives_restore` | Always false for active pursuit evidence. |
| `reused_from_prior_wait` | This call inherited the original's first exact sample. |
| `continuous_maintenance_proven` | Always false: one endpoint does not establish uninterrupted stock maintenance. |

An ordinary wait that advances beyond the horizon without selected observation
does not create endpoint evidence. Later abundant stock cannot fill that gap.
Likewise, a first `false` or `unknown` endpoint remains unchanged when later
stock improves, including a second observation in the same game tick. A
historically true endpoint does not certify currently sufficient stock.

## Publication and failure behavior

The world progression owner first publishes the complete current frontier,
including action receipts and goal transitions. Selected inspection then
captures an endpoint only when the current anchor is the original absolute
horizon and the unchanged source and output ledger derive the sampled truth.
The store accepts only an exact already-published head for first capture.

The immutable snapshot object is synced before the hash-chained endpoint record.
The process-local cache is installed only after publication succeeds. A failed
write returns a refusal, retains no reusable in-memory sample, and stops this
selected request before another progression slice. An uncertain store write
fences later effects until reopen and replay. A known failure before publication
can be retried while the same exact endpoint is still current. If the caller
later advances without successfully recording it, the missing endpoint remains
unavailable; it cannot be repaired from a later world.

Read grants and the one foreground request deadline remain mandatory. Recovered
endpoint bytes enter the ordinary bounded version book only after complete
source admission, and selected waits count both the current snapshot and a
different archived endpoint against their negotiated source budget. Canonical
state resources can read that retained version under current read authority.

## Durable records and retention

The existing journal domain and original source record encodings are preserved.
Two additive record tags retain independent history:

| Tag | Fields and meaning |
| --- | --- |
| `E` | Fortress, original plan digest and the complete exact reserve anchor: state hash, tick, epoch and sequence. One immutable record per retained original. |
| `T` | Fortress, original plan digest, bounded count, then strictly ordered step IDs with explicit final state and complete exact anchor. The original source identity comes from the separately retained original record. |

New `W` progress or `R` retirement retains all recorded final action rows only
when every row has an atomic anchor and the map is nonempty. The semantic
recovery verifier additionally checks that the map covers every expected step
of the exact original plan. A missing step, independent unanchored legacy step,
or still-dispatched action cannot become complete action proof.

The compacted `T` map must agree exactly with the original's existing `N`
never-dispatched subset. A final state is not automatically a successful state:
failed, cancelled, compensated and never-dispatched dispositions remain distinct.
Archives never appear in the store's unfinished-commit iterator and cannot
resurrect work.

The store allows at most one terminal archive and one endpoint per retained
original, using the existing bound of 64 originals per fortress and 256 recorded
steps per commit. Each new record stays within the existing 64 KiB ordinary
record bound; source request text remains in its own original record. Exact
receipt and endpoint objects are compaction roots. Eligible original-history
eviction releases only that original's archive and endpoint roots.

On reopen, every retained object is content-verified and every metadata anchor
must equal its decoded snapshot anchor. Receipt frontiers must follow the
original seal, except an explicit abandoned disposition at the original's exact
stored restore-abandonment frontier, where game time may move backwards. The
semantic owner separately checks active recovery continuity before using any
history. Duplicate, malformed, missing-object or mismatched-anchor records fail
closed; only an incomplete final journal line is discarded as a torn append.

## Compatibility and restore

Older stores remain readable. If a pre-upgrade journal still contains complete
anchored `W`/`R` receipt rows, replay can preserve those existing rows. A legacy
compacted store that already discarded terminal history has no inferred receipt
map. Older `D`, `F` and `V` retirement keeps its prior weaker custody semantics.
There is no inferred endpoint for a store that never recorded `E`.

Older executables that do not recognize the new tags refuse to open a store
containing them. They cannot silently ignore the records and later erase the
evidence through compaction.

Restore clears active in-memory endpoint custody and marks the original pursuit
abandoned. The store may keep historical endpoint and action objects, but a
reopened original cannot adopt them as a new active pursuit across restore.
Any new work still requires a separately reviewed plan with current grants.

## Evidence and scope

Focused storage tests cover exact endpoints, immutable first samples, source
admission, torn and failed publication, repeated reopen, terminal receipt
retirement, complete-map bounds, corruption, compaction and eligible eviction.
Consumer tests cover failed endpoint publication and wrong-horizon rejection;
public facade regressions cover recovered outcome selection and endpoint reuse.
See `IMPLEMENTATION_STATUS.md` for the exact checks executed on this revision.
Test source presence alone is not a full workspace qualification claim.

This change applies to the deterministic laboratory. It does not expand a live
DFHack protocol, production runner, compatibility registry or mutation grant.
