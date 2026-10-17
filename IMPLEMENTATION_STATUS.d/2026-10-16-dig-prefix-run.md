## Bounded original dig-prefix execution

Beads: df-dfhack-bridge-plane-c-pic.4/.5; broad work remains open.

Implemented an exact 1..8-step original-prefix review and transition contract.
It uses the existing batch inventory digest, original key sequence and original
native rectangle geometry. Every earlier terminal receipt and room identity is
preserved; uncertain/refused outcomes never authorize the next original step.

Executed 22 dependency-free Python tests against the actual new policy module.
The tests are model inputs, not native/file-custody evidence. No effect-shell,
CLI, Rust/native or live execution is claimed in this first increment.

### Existing native effect-shell integration

Implemented the executable `run_dig_blueprint.py review|run` path. It rebuilds the
review from original locked custody, then consumes only that exact consecutive
prefix under current operator authority. Fresh per-step native plans use the
existing original write-ahead capsule/registry, prepare and one-shot commit;
terminal receipt publication and full-history replay precede continuation.
Connections are distinct NEW-step connections, never failure retries. Unknown or
refused work stops the prefix; interrupted calls preserve the original keys.
The response keeps the complete original blueprint and batch, with one initial
wall/work/RPC/byte allowance and pre-contact full-output reservation.

Executed 52 tests on final local source (22 pure + 30 legacy execution cases).
The latter run actual existing dig source, private files, fsync/locking, client
framing and joined synthetic TCP peers, including subprocess CLI execution and
SIGKILL after the first synthetic effect. Tests cover 512 exact target tiles in
eight one-shot steps, lost second acknowledgment and explicit query recovery,
replayed preparation, source/clock/authority loss, sync faults, immutable prior
records, budgets, final authorization and short output without repeat dispatch.

The existing source Git blob identities used in this component run are:
- dig_blueprint.py: eb16a43e3df6f6453fdadfe5386ec7faf3b80315
- dig_blueprint_client.py: d9e49c0c740fd055493e1a4941b637787851efa5
- dig_designation_client.py: 0f26e5cd6af55c64537f69a4aee0610bb8dc9edc
- dig_designation_store.py: e82497d39842f294f77e9e576647f19d2a1d594c

The eager room dependency boundary used a validation-only adapter supplying its
unchanged bounded JSON reader and refusing room-handoff use. All executed
end-to-end cases are legacy floor-blueprint batches, not room-backed batches;
the latter remain source-reviewed delegation through the original owner. The
adapter is not shipped and the committed tests use ordinary repository imports.
No native DFHack producer/SDK, Rust workspace, live-game, full-repository or
production-admission qualification is claimed. Broader live mutation work stays
open. No existing production/native/journal file is changed by this increment.
