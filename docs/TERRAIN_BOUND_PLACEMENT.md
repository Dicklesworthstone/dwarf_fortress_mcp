# Retained terrain history through durable furnishing placement

The existing `furniture_batch.py` CLI now accepts the closed terrain/furniture
handoff from `bind_room_terrain.py`. It creates batch/4; existing exact-item,
allocation-backed and room-backed batch/1, /2 and /3 bytes and keys are unchanged.
This is an isolated development workflow under the existing furniture/1.19
permission model. Beads `df-dfhack-bridge-plane-c-pic.3/.4/.5` remain open.

## Import and place

After binding the completed original terrain journal to the exact room
allocation, switch from the map-read environment to the existing isolated
furniture environment. Remove unrelated DFMCP profile variables. The original
terrain journal is read offline; no map credential is needed for this stage.

```sh
mkdir -m 700 "$HOME/terrain-furnishings"
python3 scripts/furniture_batch.py init --directory "$HOME/terrain-furnishings" \
  --terrain-handoff terrain-furniture-handoff.json
```

The handoff supplies the original fortress, so explicit folder/site overrides
are rejected. Initialization performs one native placement observation but no
placement. Retain the returned batch ID. Existing `review`, single-attempt
`advance`, `query`, `cancel`, `inspect`, and `stop` commands are unchanged.
Every advance still needs its freshly reviewed native plan and review seal,
plus the separate explicit placement permission. No command unpauses the game.

Batch/4 embeds the entire handoff, including fresh raw map bytes, original room
geometry, exact inventory selection, and original terrain journal identity.
Its manifest is bounded to 384 KiB, with a bounded nesting profile; batch/1-/3
retain their prior ceilings. Responses remain bounded to the existing 192 KiB
room profile and show compact terrain references rather than repeatedly dumping
raw map bytes. The complete batch digest binds every room-backed effect key, so
changed terrain history cannot adopt old registered placement keys.

## Original terrain custody is required for new work

Initialization, review and advance reopen the exact original private terrain
journal, replay its complete history under the same operation deadline/work
allowance, and verify the retained goal, source, path/inode identities and full
bytes. The read-only journal owner remains held through the operation. Native
preparation, commit and final publication retain the existing batch checks and
add original terrain byte/path checks. The CLI verifies original terrain again
after final serialization and holds it through its single stdout write.

All placement captures must match the retained endpoint, fortress, dimensions
and software and cannot predate the retained map. Map, inventory and placement
generation numbers remain separate namespaces. Existing native eligibility,
original inventory constraints, predecessor horizons, stable keys, confirmation,
pending-effect fences, and one-attempt dispatch rules are not weakened.

## Recovery must remain available after terrain-file loss

`query`, `cancel`, `inspect`, offline original-room export and `stop` do **not**
require the terrain journal. They validate the immutable batch and child effect
journals and retain the terrain reference without claiming to have verified its
current custody. An unknown original effect must remain reconcilable even when
the terrain file is gone. None of these commands can dispatch a replacement
placement, repair the terrain store, reallocate an item or renew a deadline.

In such responses `terrain_history_verified_this_call` and `advance_allowed`
are false. A missing original terrain journal blocks new review/advance before
native contact. A lost commit reply followed by terrain-file loss still allows
original-key query; lost preparation still allows original-key cancellation.
Loss after preparation but before commit blocks dispatch and preserves the
original preparation for cancellation. Losing custody after a durable placement
may refuse the response, but never causes another placement or key.

The original journal remains necessary. Moving/copying it or replacing it with
identical bytes is not custody recovery. Plain batch inspection establishes
neither new placement authority nor current terrain. A historical terrain
completion and a placement receipt do not prove room completion, continuous
wall preservation, safety, native room assignment or construction completion.

## Executed development evidence

Seventeen new methods passed on Python 3.13.5 using real private terrain and
placement journals, the real CLI and a joined fragmented furniture/1.19 peer.
The maximum case imported 32 slots and 646 exclusions, deleted the temporary
input, and executed all 32 exact synthetic native placements. Tests also cover
source/clock/item drift, substituted/rehashed evidence, custody loss at effect
boundaries, shared budgets, revoked authority and short stdout.

Thirty additional comparisons matched the exact prior batch implementation
(blob `553481212fe205df34bedfe273d1af972ca52ccb`) byte-for-byte across existing
batch/1, /2 and /3 inspection, review, per-step receipt and stop results.
The binding producer's sixteen tests were executed separately in its increment.

These are focused tests against a byte-verified local source subset. Terrain
history and inventory inputs in the new placement suite are fixture-generated;
placement peers use production codecs. This is not DFHack/SDK/live-game,
Rust/MCP or full-repository qualification. The construction-monitor consumer of
batch/4 is a separate integration increment. Production admission is unchanged.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests \
  python3 -m unittest test_terrain_furniture_batch -v
```
