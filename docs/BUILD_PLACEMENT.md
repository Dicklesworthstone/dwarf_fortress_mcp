# Guarded furniture placement engine

`bridge/common/build_placement.h` implements one-shot placement of one ordinary
bed, chair or table using one exact existing item. The engine is native-independent
C++17; callbacks must execute inside one suspended DFHack dispatch. The isolated
native RPC integration is described in `BUILD_PLACEMENT_NATIVE.md`, and the
durable Python development client in `BUILD_PLACEMENT_CLIENT.md`. Neither adds
a Rust/MCP production control path or live admission.

## Plan and evidence

The complete capture binds native generation/sequence, fortress folder/site, tick,
map dimensions, building/job ID horizons, building count, one selected item and a
3x3 same-level terrain context. Hidden/missing cells and items have no attribute
payload. Placement requires a paused, visible, dry, supported, free floor target,
at least one adjacent free floor, and an unworn unclaimed ground item of the exact
kind. Native `temps_computed` and `weight_computed` flags (bits 28 and 29) are
permitted bookkeeping; every flag remains witnessed exactly, and a change still
invalidates preparation. All other residual item flags make the item unavailable.
This narrow filter is not pathfinding, structural safety or a game checkpoint.

Preparation is effect-free, expires after 60 monotonic seconds, and cannot be
renewed by replay. Commit rereads the entire capture before any writer invocation.
The engine records Indeterminate and blocks further placements before calling the
writer once. An exception, false writer result, unchanged terrain or missing
readback cannot prove nonapplication. Exact expected-after capture, native building
and ConstructBuilding job verification, the exact Hauled item and reverse job link,
and a final capture are all required to publish Placed. Placed means historical
construction-job registration at stage zero, not a completed or usable building.

Duplicate commit/query returns retained history without another native callback.
Cancellation retires preparation only; it never deconstructs a building or detaches
an item/job. Source changes preserve all records and the unresolved-work fence.
Retention is bounded at 256 keys without eviction. Plugin/process loss still needs
a durable external intent journal; native lifetime retention is not crash recovery,
a global controller fence, an anti-rollback root or permission to retry.

## Canonical bytes

Capture, insertion proof and record markers are DFMBC019, DFMBI019 and DFMBR019.
Integers are big-endian; variable fields are u16-length-prefixed; booleans and
presence tags are strict bytes. Signed material fields use their u32 bit patterns.
`Capture::encode`, `Insertion::encode` and `Record::encode` are the field-order
contract; eight independently constructed complete fixtures are retained in
`bridge/common/tests/fixtures/build_placement_v1_19.json`.

Plan SHA-256 covers `dfmcp-build-plan/1` + NUL + selection bytes + capture SHA-256.
The token is the first 16 bytes of SHA-256(`dfmcp-build-token/1` + NUL + framed key
+ plan digest). Record SHA-256 covers `dfmcp-build-receipt/1` + NUL + all preceding
record bytes. Hashes are commitments, not signatures or authority.

## Executed checks

```sh
python3 scripts/test_build_placement_engine.py --mutations
python3 scripts/test_build_placement_engine.py --compiler clang++
```

The current computed-flag increment passes 23 groups / 1,133 actual C++ assertions
with GCC 13.3, warnings denied and nonrecovering UBSan. Eight C++ byte outputs
still match the unchanged independently constructed Python corpus. Five separately
compiled weakened engines fail their regressions: missing item-flag filtering,
full revalidation, uncertainty fencing, immutable replay and final readback.
`--mutation NAME` runs one mutation separately; `--evidence PATH` retains
source-bound JSON. The earlier engine revision had GCC 14.2 and Clang 17 evidence;
Clang is unavailable in the current environment, so that earlier evidence does not
qualify the modified source. These checks are not real DFHack SDK, live-fortress,
Rust/MCP, power-loss or full repository qualification. Beads
df-dfhack-bridge-plane-c-pic.4/.5 remain open; all other protocols and production
admission are unchanged.
