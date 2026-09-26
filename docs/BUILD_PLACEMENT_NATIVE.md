# Native furniture placement — development protocol 1.19

`bridge/dfhack-build-v1_19` connects the existing guarded furniture engine to
DFHack's supported building and item APIs. It can register one ordinary bed,
chair or table at an exact floor tile using one exact existing item. The engine's
capture, plan, token and retained record bytes remain as specified in
`BUILD_PLACEMENT.md`. The separate machine contract is
`architecture/build_placement_v1_19.json`.

This is unadmitted development source. Actual handler execution with explicit
DFHack/protobuf doubles establishes only the tested control flow and byte checks.
It does not establish a real SDK build, generated protobuf behavior, a live
fortress campaign, physical crash safety, complete Rust qualification, production
admission or a completed usable building. All existing native generations and
the production runner map retain their existing semantics.

## Native acquisition and eligibility

A suspended RPC dispatch captures the exact fortress identity and clock, native
incarnation and intervention sequence, map dimensions, building/job ID horizons,
complete bounded building count, selected furniture item and a 3x3 same-level
terrain context. Building registry acquisition checks all entries for nulls,
duplicate IDs, invalid ID horizons and invalid bounds, even after finding a
relevant building. It does not retain native pointers outside the call.

Missing and hidden map cells have no attribute payload. A selected hidden item,
or an item on hidden terrain, also has no attribute payload. Capturing an item
does not traverse inventory/container graphs or inspect terrain beneath a hidden
cell. Helpers that inspect support and free space run only after every context
cell is known visible.

Placement requires the engine's paused, dry, visible, free floor
selection with a neighboring free floor and an available exact-kind ground item.
The handler additionally refuses a target inside any registered building's
bounding rectangle, including zones, stockpiles and their extent holes. It also
checks the separately bounded `ANY_ZONE` registry and refuses target membership
in a zone's `room` rectangle, which can differ from its base building bounds.
Zone pointers must belong to the complete building registry, IDs must be unique,
and room bounds must be valid. Extent holes are conservatively refused without
reading native extent arrays. This is deliberately conservative: DFHack
automatically associates new furniture with
containing zones. Refusing those overlaps prevents that ancillary relation write.
Target pile and smoothing designations also make the tile ineligible. No target
or neighbor is uncovered, allocated, excavated or cleared during acquisition.

The selected item's first flag word remains witnessed exactly, separating
`on_ground`, `in_job` and all other flags. The engine permits the two native
bookkeeping flags `temps_computed` (bit 28) and `weight_computed` (bit 29); changing
either bit still invalidates a prepared witness. Every other residual flag makes
the item unavailable. A nonzero second item flag word is
refused because this engine generation has no field that could represent it
without losing information. All item reference collections are bounded; unexpected
general/specific references make the item unavailable. No material search,
replacement-item choice, hauling reachability, structural safety or cost
optimization is claimed.

## Fixed RPC and operator gates

Six methods are registered with zero DFHack RPC flags:

| Method | Optional request fields | Result payload |
|---|---|---|
| `Handshake` | none | native identity and retained-work summary |
| `ReadPlacement` | kind, item ID, x, y, z | complete capture |
| `PreparePlacement` | selection, key, witness, plan digest | retained record and replay flag |
| `CommitPlacement` | key, plan digest, prepare token | retained record |
| `QueryPlacement` | key, plan digest | retained record if known |
| `CancelPlacement` | key, plan digest, prepare token | retained record |

Every call requires exact protocol 1.19, `DFMCP_ALLOW_UNADMITTED_BUILD_V1_19=1`,
and a bearer secret matching `DFMCP_BUILD_TOKEN` (32..256 bytes). Nonces are
16..64 bytes. Any present `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, including an empty
value, refuses this development profile. Prepare and commit additionally require `DFMCP_BUILD_ALLOW_PLACE=1`.
Query and cancellation remain available when the placement permission is revoked.
Authorization and exact optional-field shape checks precede any game access.
Malformed, unknown-field, oversized and wrong-profile requests fail closed.

All successful replies report native generation and versions, `unresolved` and
`retained_records`. Error replies contain only the eight base fields, zero
generation and empty versions. The bounds are 2048 bytes per parsed request,
8192 bytes per reply, 2048 bytes per capture and 6144 bytes per record. These
handler bounds do not replace DFHack's outer transport frame limits.

## One native attempt and independent readback

Preparation remains effect-free and expires after 60 monotonic seconds; replay
does not renew it. Commit compares the entire capture and current eligibility
under the same native suspension. The engine retains `Indeterminate` and fences
competing effects before entering the writer.

The writer allocates an unregistered building with `Buildings::allocInstance`,
sets its private one-tile dimensions with `Buildings::setSize`, and allocates the
one-item selection vector. It then repeats full capture equality and eligibility
checks, validates native incarnation/intervention sequence, and rechecks both
operator gates and absence of the production marker immediately before
`Buildings::constructWithItems`.

The DFHack call can partially link the building, set planned tile occupancy,
schedule building checks, create a `ConstructBuilding` job and attach the exact
item before throwing. The handler therefore relinquishes private pointer
ownership at the call boundary, never deletes or deconstructs an object after
entry, and does not interpret a false return as nonapplication. A partial or
unverifiable attempt stays indeterminate. At most one attempt is possible per key;
the global native uncertainty fence also blocks changing the key to retry.

Publishing `Placed` requires the engine's exact expected-after capture and a
separate verifier of the newly registered building's exact type, one-tile
footprint, material and stage zero. Native target occupancy must be specifically
`Planned`. The new building must have exactly one construction job, no unexpected
references or zone relations, and a valid bounded maximum build stage. The job
must occur once in a completely checked bounded native job list, with consistent
back-links and ID horizon, exact position/material, no flags or unexpected
references, the exact building holder and one exact `Hauled` attachment. The
selected item must have the reverse reference to that very job. A final complete
capture must still equal the expected state.

The writer's boolean return, a transport acknowledgment, ID count changes, or a
matching furniture type alone cannot satisfy that proof. `Placed` means
historical stage-zero construction registration; it does not assert later
construction completion, accessibility, hauling success or present state.

## Recovery and retention

The 256-record native table never evicts. Query and duplicate commit return
retained history without game reads or repeated effect dispatch. Preparation
cancellation only retires an unattempted record; it cannot detach an item, cancel
a construction job or remove a placed building. Source-change callbacks preserve
all records and unresolved fences while advancing native generation; pause
changes advance the intervention sequence.

Records are native-lifetime history. Unload/process loss destroys that custody;
an absent key after restart proves no nonapplication. A durable external intent
journal must exist before dispatch, and uncertain effects must be reconciled
without changing keys or blindly retrying. This handler introduces no background
timer, update worker, independent clock authority, checkpoint or automatic
rollback.

## Executable checks and upstream references

```sh
python3 scripts/test_build_placement_engine.py --mutations
python3 scripts/test_build_placement_native.py --mutations
python3 scripts/test_build_placement_native.py --compiler clang++
```

The native test runner compiles the actual handler against explicit SDK and
protobuf doubles with C++17, warnings denied and nonrecovering UBSan. Its report
identifies tested source hashes, assertion counts and rejected compiled mutants.
The current GCC 13.3 run passes 52 scenarios / 27,429 assertions, including
exhaustive optional-field-shape rejection, production isolation, hidden-field
redaction, full revalidation, exact native link verification, partial effects,
uncertainty fences, immutable replay and retention. Five actual RPC-produced
capture/plan/token/prepared/placed byte values pass independent Python decoding,
reencoding and commitment verification. Four separately compiled weakened
implementations fail native-handler regressions for missing full revalidation,
uncertainty fencing, immutable replay and final readback. The separate engine suite passes 23
scenarios / 1,133 assertions and checks its eight unchanged complete byte fixtures.
These are separate evidence scopes; neither is a real DFHack or live campaign
qualification. Clang is unavailable in the current environment.

API inspection used DFHack commit
`7e4d6190861ea9aaadd648534cd6c1089a1958ae` and its exact structures submodule
`d7a93414744ce230fe2397a6eb8682fd58a8c2fa`. Allocation, support checks, construction
order and implicit zone side effects were inspected in
[Buildings.h](https://github.com/DFHack/dfhack/blob/7e4d6190861ea9aaadd648534cd6c1089a1958ae/library/include/modules/Buildings.h)
and
[Buildings.cpp](https://github.com/DFHack/dfhack/blob/7e4d6190861ea9aaadd648534cd6c1089a1958ae/library/modules/Buildings.cpp).
Job attachment semantics came from
[Job.cpp](https://github.com/DFHack/dfhack/blob/7e4d6190861ea9aaadd648534cd6c1089a1958ae/library/modules/Job.cpp).
Native fields were checked against pinned
[item](https://github.com/DFHack/df-structures/blob/d7a93414744ce230fe2397a6eb8682fd58a8c2fa/df.item.xml),
[building](https://github.com/DFHack/df-structures/blob/d7a93414744ce230fe2397a6eb8682fd58a8c2fa/df.building.xml),
[job](https://github.com/DFHack/df-structures/blob/d7a93414744ce230fe2397a6eb8682fd58a8c2fa/df.job.xml)
and
[reference](https://github.com/DFHack/df-structures/blob/d7a93414744ce230fe2397a6eb8682fd58a8c2fa/df.reference.xml)
definitions. Inspection of these source signatures is not an ABI or compatibility
admission claim. Beads `df-dfhack-bridge-plane-c-pic.4` and
`df-dfhack-bridge-plane-c-pic.5` retain their broader acceptance work.
