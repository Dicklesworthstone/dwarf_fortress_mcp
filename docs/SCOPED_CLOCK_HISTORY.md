# Discover original construction-clock history without transcript memory

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`.

The scoped construction-progress workflow previously required the operator to
retain every confirmation to select an old clock record. The legacy
`recover_bounded_runs.py` inventory does not accept the new `.scope.json`
associations: those files are not native intent capsules. That legacy reader is
unchanged. `route_furniture_clock_inventory.Inventory` instead inspects the
complete bounded scoped-clock directory while the caller holds original batch
custody. It performs no native calls or file writes.

## Exact evidence, useful recovery handles

Discovery reuses the actual version-2 `Scope`, original source checks, native
intent verifier and native receipt `OutcomeStore`. It checks every artifact,
including records outside the selected page. Original-batch records bind the
retained review, plan, original step and source. A same-endpoint foreign batch
cannot become this batch's clock history. Foreign records are fully decoded and
counted without exporting their per-batch details.

Each original row contains its confirmation (`clock_record_id`), original native
key, step, retained phase and receipt digest. `query_supported` is true only for
an original-batch scope with its exact native intent and no retained terminal
outcome. It points to existing query-only recovery, never a new start. A complete
scope with no native intent is visible as `scope_without_intent`, but is not proof
of nonapplication and does not authorize synthesizing an intent or retry. A
`source_lost` receipt remains unresolved even though native history is terminal.
An unassociated legacy intent is reported as unbound and never adopted. Outcome
files without native intents, corrupt bytes and invalid namespaces refuse the
complete inventory without repair.

Pagination is deterministic in confirmation order. The continuation binds the
original batch's current journal inventory, complete local directory identity,
all artifact identities and exact content hashes, selection and page size.
Changed source bytes, byte-identical file replacement, new outcomes, changed
membership or changed original batch inventory invalidate old continuations.
Every page carries complete counts and explicit foreign-detail coverage; an
empty filtered page is never a claim that the directory has no unresolved work.

## New-effect history admission

`admit_new(scope)` is a local precondition, not native authority. It requires all
prior records in the selected directory to have a fully associated `stopped` or
`refused` receipt and spare capacity for one new record. Unresolved original,
foreign, scope-only or unassociated records prevent another progress effect.
The current original batch and scope are checked separately; old consent does
not become reusable because its receipt is terminal.

The inventory can borrow the already-held clock-directory lock from
`bounded_run_outcomes.new_run`. After admission, only the selected new scope,
intent and outcome filenames may appear. All previous membership, identities and
bytes remain pinned and are rechecked before effects and publication. The
existing ScopeStore and OutcomeStore still validate the new files. The history
reader never creates, removes or repairs an artifact, nor does it issue a native
preparation/commit token.

The guarantee is limited to this operator-selected directory and cooperating
local owners. Selecting a different directory, malicious owner rollback and
other controllers are not globally fenced. Historical pause evidence remains
historical. Neither complete local history nor an empty directory establishes
current pause, route readiness, construction completion or production admission.

## Bounds and executed evidence

The directory has at most 256 records / 768 files, each at most 8 KiB. Names have
a fixed `progress-<64 lowercase hex>.run` namespace with only `.scope.json` and
`.outcome.json` suffixes. A page contains 1..64 requested rows and at most 64 KiB.
The caller's same deadline, work and custody-byte allowances span enumeration,
full decoding, all checks and rendering. Existing OutcomeStore I/O receives a
conservative whole-operation reservation; allowances are never replenished.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_clock_inventory -v
```

The initial 24 tests pass with real private files, existing Scope/Source and
native intent/receipt codecs, actual read-only locks and all 256 records paged.
They cover orphan scope, lost outcomes, SourceLost, foreign/unbound records,
corruption, source/plan substitutions, stale continuations, namespace/size/mode/
link/FIFO refusals, membership changes during reads, exact new-file membership,
borrowed-lock lifetime, resource exhaustion and descriptor cleanup. Tests forbid
native sockets, writes and fsync during offline discovery. The original batch
custody and budget are explicit test ports, not a live game or complete private
batch integration. No Rust, native plugin, live fortress or full-workspace
qualification is claimed.
