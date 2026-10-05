# Retaining inventory intent through Python furnishing execution

Owning beads: `df-dfhack-bridge-plane-c-pic.4` and
`df-dfhack-bridge-plane-c-pic.5`. Both retain their broader acceptance scope.

## Immutable handoff core

`scripts/furniture_handoff.py` retains one complete normalized
`dfmcp.furniture-request/1`, all exact selected items and native type numbers,
and an operations/1.4 capture reference. It derives the existing exact-item
`dfmcp.furniture-plan/1` rather than accepting a second independently editable
plan. The new `dfmcp.furniture-handoff-python/1` representation is bounded at
32 KiB and uses its own SHA-256 domain. It is not the Rust binary handoff format.

A complete assignment must satisfy every slot, exclusion, material/subtype,
same-level distance, unique-item and dependency constraint. The selected native
item type/key mapping must be consistent. A shortage cannot become a handoff.
The retained source covers the endpoint, software, inventory generation, exact
capture digest/size, game tick and job/building/item ID horizons.

Later native BEFORE observations must use the original item, kind, native type,
material and subtype. Position may change only inside the original level and
distance constraint. Source software/endpoint/fortress must still agree and
clocks and effect ID horizons cannot predate allocation. The operations plugin's
generation is not compared with a furniture-plugin generation: the batch must
independently fence the latter. Construction post-state is not an item-selection
precondition and must not be passed to the BEFORE validator.

## Durable execution and recovery

`furniture_batch.py init --handoff FILE --world-folder FOLDER --site SITE`
accepts the complete canonical handoff instead of `--plan`. The operator must
select an existing empty private batch directory and the separate furniture
read configuration. Initialization checks the exact derived plan, fortress,
endpoint, software, map dimensions and fresh first-item observation before
publishing the original handoff inside `batch.json`.

The new `dfmcp.furniture-batch/2` definition is bounded at 64 KiB. Legacy
`dfmcp.furniture-batch/1` definitions retain their 32 KiB bound and original
shape. Child journals, index frames, stop markers and native wire bytes are
unchanged. Full definition/output reservations precede publication.

Every review, before-intent, before-prepare, before-commit and retained-child
audit enforces the original handoff. An independently valid replacement native
plan cannot bypass original item or request constraints. Reopening performs no
native bootstrap and restores no preparation/dispatch permission. Original-key
query and cancellation retain their existing recovery semantics. A lost commit
reply never permits another placement attempt or automatic reallocation.

Normal results retain the complete plan and a compact allocation identity.
`inspect --allocation` returns the complete original request, source and selected
items under the existing 64 KiB output bound; it cannot be combined with the
separate child-receipt view. Local stopping does not erase uncertain effects.

## Authority and evidence

A handoff file is operator-supplied historical data, not independently
attested native acquisition. The initializer does not infer that a network read
occurred merely because a handoff decodes. It rechecks native BEFORE evidence
using the existing isolated furniture connection. Imported constraints only
narrow that path; they do not create authority, reserve items, establish paths,
restore a connection permit, authorize reallocation or prove construction.

An inventory-export command and allocation-backed completion origins are not
part of this execution increment. The Python handoff format remains separate
from the existing Rust binary handoff; no Rust qualification is implied.

## Focused validation

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tests -p test_furniture_handoff.py -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tests -p test_furniture_handoff_batch.py -v
```

All 38 methods pass: 18 semantic core tests and 20 batch tests using the real
Python TCP client, binary codecs and private POSIX journals with a joined
protocol test peer. Coverage includes scarcity, source/item drift, last-boundary
revocation, missing original custody, lost commit replies, indeterminate outcomes,
legacy execution, offline restart and complete 32-item execution. The largest
complete result in the 32-item fixture is 48,369 bytes.

This is focused Python development execution, not a native DFHack plugin build,
live-game campaign, Rust/MCP, full-workspace qualification or production admission.
