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

## Inventory export

`allocate_furniture.py --request-file request.json --with-handoff` performs one
complete operations/1.4 capture using the existing read-only credential/profile.
It binds only Handshake and ReadObservation. Full paging, digest, source identity,
strict decoding and verified capture release precede handoff derivation. The same
decoded inventory supplies native selected-item facts; no second read or decode
is introduced. One shared foreground budget and final operator check apply.

The opt-in output profile is `furniture-allocation-handoff/1`; its result schema
is `dfmcp.furniture-allocation-handoff-result/1`. Successful output retains the
exact plan and `result.handoff`, with original request/selections stored once in
that handoff. A shortage returns `handoff: null`, no executable plan and no partial
assignment. Default allocation output remains unchanged without the option.

The handoff is canonical compact sorted ASCII JSON without a trailing newline.
For an already-reviewed successful export, extract it with the existing codec:

```sh
python3 scripts/allocate_furniture.py --request-file request.json --with-handoff > allocation.json
python3 - <<'PYTHON'
import json, sys
sys.path.insert(0, 'scripts')
from furniture_handoff import Handoff
from furniture_plan import canonical
with open('allocation.json', 'rb') as source:
    result = json.load(source)
if not result['ok'] or result['result']['status'] != 'allocated':
    raise SystemExit('No complete allocation; do not initialize a batch.')
artifact = Handoff.from_json(result['result']['handoff'])
with open('handoff.json', 'xb') as target:
    target.write(canonical(artifact.json()))
PYTHON
```

Run the allocator in its isolated inventory environment. Switch to the separately
authorized furniture environment for batch initialization/execution; do not combine
inventory and placement credentials. The two commands intentionally reject a union
of their DFMCP environments. This export is operator data, not a signed acquisition
receipt, reservation or native mutation permission.

## Durable execution and recovery

`furniture_batch.py init --handoff handoff.json --directory /absolute/private/batch
--world-folder FOLDER --site SITE` accepts the complete canonical handoff instead
of `--plan`. The directory must already exist, be empty and private. Initialization
checks the exact derived plan, fortress, endpoint, software, map dimensions and
fresh first-item observation before retaining the handoff in `batch.json`.

The new `dfmcp.furniture-batch/2` definition is bounded at 64 KiB. Legacy batch/1
retains its original shape and 32 KiB bound. Child journals, index frames, stop
markers and native wire bytes are unchanged. Full definition/output reservations
precede publication; operator read authority is rechecked around publication.

Every review, before-intent, before-prepare, before-commit and retained-child
audit enforces the original handoff. An independently valid replacement native
plan cannot bypass original item or request constraints. Reopening performs no
native bootstrap and restores no preparation/dispatch permission. Original-key
query and cancellation retain their existing recovery semantics. A lost commit
reply never permits another placement attempt or automatic reallocation.

Normal results retain the complete plan and compact allocation identity.
`inspect --allocation` returns the full original request/source/selected items
under the existing 64 KiB output bound; it cannot be combined with the separate
child-receipt view. Local stopping does not erase uncertain effects.

## Whole-original-plan completion

`furniture_completion.Origin` accepts allocation-backed batch/2 after every
original child has a registered Placed receipt. It independently replays all
original child journals and revalidates their BEFORE captures against the
retained request. The complete handoff, plan, original bytes and inode identities
remain part of the origin; file replacement cannot substitute that evidence.

Allocation-backed origins use `DFMFCO02` and digest domain
`dfmcp.furniture-completion-origin/2` plus NUL. Their goals use `DFMFCG02` and
`dfmcp.furniture-completion-goal/2` plus NUL. Legacy `DFMFCO01`/`DFMFCG01` bytes
and digest domains remain unchanged; cross-generation header substitution fails.
The existing completion journal delegates to Goal.decode and needs no frame-format
change. Native placement and observation formats are unchanged.

Later samples must match the original operations generation/software separately
from the original furniture generation. The original allocation tick and all
three job/building/item identity horizons are lower bounds. All original targets
must satisfy the existing construction condition together over its global
stability streak. Placement alone, partial-target success, repeated same-tick
samples, missing receipts or regressed source identity cannot prove completion.
Sampled satisfaction is not current usability, continuous stability or discharge
of the original placement effect.

## Authority and evidence

An imported handoff is historical operator data, not independently attested
native acquisition. Initialization does not infer a network read merely from
successful decoding: it acquires fresh native BEFORE evidence through the existing
isolated furniture connection. Retained constraints only narrow that path; they
never grant authority, reserve items, establish paths or restore a connection
permit. No new native method, credential or production runner is introduced.

## Focused validation

```sh
for suite in test_furniture_handoff.py test_furniture_handoff_batch.py \
             test_furniture_handoff_inventory.py test_furniture_handoff_completion.py; do
  PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p "$suite" -v || exit
done
```

All 59 methods passed in four separate final invocations: 18 semantic core,
20 real TCP/private-journal batch, 10 inventory/CLI integration and 11 completion
codec/evaluator methods. The aggregate invocation exceeded the executor's time
limit; the separate invocations completed without failures or skips.

The actual allocator and batch CLIs executed through separate subprocesses and
credentials. Tests covered complete paging, missing/corrupt release, source and
item drift, final-boundary refusal, revocation, original-file loss, lost commit
reply, original-key recovery, restart and legacy formats. A complete 32-slot
inventory was allocated, exported, initialized and placed against the joined
protocol peer, then its full original completion goal accepted two later
all-target construction fixtures. Observed fixture sizes were 34,099 bytes for
allocation export, 48,371 for full batch inspection and 347,309 for completion
origin. They are fixture observations, not performance or universal size claims.

The peer is a protocol test double, not a DFHack plugin. Later completion samples
are explicit wire fixtures; the full completion-monitor CLI/journal process suite
was not executed here. No Rust toolchain/full checkout was available. Therefore
this is focused Python development evidence only, not Rust/MCP, native DFHack SDK,
live-game, full-workspace qualification, registry admission or production support.
The existing broader beads remain open. See `docs/evidence/furniture-handoff-python.json`.
