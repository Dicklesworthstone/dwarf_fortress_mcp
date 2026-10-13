# Route-aware whole-plan furniture allocation

Beads: `df-franken-storage-mvcc-54h.4` and `df-dfhack-bridge-plane-c-pic.3/.4`.
This adds an obstacle-aware alternative to the existing Manhattan-distance
furniture allocator. It does not change the legacy allocator or placement path.

`route_furniture_allocation.allocate_routes(request, candidates, terrain_raw)`
validates the complete unchanged map/1.5 capture, preserves all original slot
material, subtype, same-z, exclusion and maximum-Manhattan constraints, then
minimizes total shortest candidate-route steps. Equal totals use the existing
exact lexical slot-name/item-ID tie break. A separate `max_route_steps` limit
narrows route eligibility; it does not reinterpret `Slot.max_distance`.

The movement model matches `observed-dry-cardinal-floor-stairs/1`: visible dry
floor/stair cells with no observed building/unit occupancy and nonzero native
walkability; vertical transitions require complementary stair shapes. Ramps,
diagonals, doors, hidden cells and routes outside the captured region are not
modeled. Item/target endpoints must share a z level, but a modeled route may use
stairs to another level and return. This is NOT native unit pathfinding, danger
assessment, throughput or a forecast of routes after earlier furnishings.

For N targets, a full target-rooted BFS provides shortest distances before
keeping the N best eligible items for that target. That reduction preserves a
global optimum: the other N-1 targets cannot consume every one of those N better
items. The existing maximum-matching and exact integer Hungarian solver choose
the whole assignment. A deficient Hall set has fewer than N neighboring items,
so no member's eligible list was trimmed: the witness covers the full supplied
candidate graph within this bounded route model.

Unobserved/unsupported targets return `target_unestablished`, null maximum
assignment counts and no shortage/plan. Established targets may yield a checked
model shortage; no partial executable subset is emitted. Unknown terrain never
becomes complete-world absence. Successful proposals use the existing exact-item
`dfmcp.furniture-plan/1` format. Material constraints and route evidence are not
automatically enforced by that legacy plan format at future placement time.

A second bounded BFS rechecks each selected route and creates a streaming digest
of every flattened path index. The path policy is target-rooted BFS with stable
flattened-index ties, not necessarily the same tie as a BFS rooted at the item.
The decision digest binds the normalized request, all supplied candidates (even
unselected ones), exact raw map digest, policies, route limit, work budget and
complete result. Source/order changes cannot silently reuse earlier evidence.
Pure callers supply candidate facts; these APIs do not attest their acquisition.

Bounds: 32 slots, 65,536 candidates, 16,384 terrain tiles, 128 cells per region
axis, 20,000,000 shared work units and 49,152 result bytes. Space is
O(tiles + candidates + N*N); no slot-by-inventory or slot-by-terrain matrix is
retained. The same cooperative allowance covers decoding, routing, full candidate
scans, matching, optimization and route witnesses. Refusal returns no partial
result. Final serialization and hashing are bounded but not internally preempted.

## Executed development validation — 2026-10-13

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 -m unittest test_route_furniture_allocation -v
```

All 16 test functions passed. They execute the actual Python implementation,
not a mirror: all 512 three-by-three obstacle masks (unestablished targets remain
unknown), 120 random inventories against independent Floyd-Warshall/unpruned
permutation and cardinality oracles, material starvation, complete Hall witnesses,
legacy constraints, stair detours, unknown targets, source/order invalidation,
exact work boundaries, every cooperative callback refusal, every small-fixture
truncation, maximum candidate/slot counts and an 8,255-tile snake in a maximum-size
map. Both reused upstream Python modules were verified against their pinned Git
blob hashes. No Rust, native DFHack, full repository or production qualification
is implied, and the broader beads remain open.

## Executable map/inventory read workflow

`allocate_route_furniture.py` now drives the allocator from the unchanged
`dfmcp_map_v1_5` and `dfmcp_operations_v1_4` plugins. It does not load plugins or
change the game. The operator supplies a bounded map region and the existing
`dfmcp.furniture-request/1` request file. In an isolated environment, configure:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE=1
DFMCP_ROUTE_FURNITURE_ENDPOINT=127.0.0.1:5000
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured operations read token>
DFMCP_MAP_TOKEN=<operator-configured map read token>
```

The endpoint is optional (the value shown is its default); each token must be
32..256 UTF-8 bytes without NUL. All other `DFMCP_` variables, including placement
credentials and production-admission state, are refused. No read credential is
copied into a result, error, plan or log.

```sh
python3 scripts/allocate_route_furniture.py \
  --request-file furnishings-request.json \
  --origin 0 0 5 --size 64 64 1 \
  --max-route-steps 1000 --timeout-ms 10000
```

Coordinates and route limits above are illustrative, not a known fortress.
The request file is read completely under a no-follow final-component open;
nonregular, oversized, duplicate-field, changed and malformed input is refused
before native contact. Parent path components have ordinary filesystem semantics;
this input file is not private journal custody. The input is never rewritten.
No output plan file, journal, reservation or native effect is created.

One TCP connection binds only the map and operations `Handshake` and
`ReadObservation` methods. It obtains map-before, one complete operations roster,
its verified original-token release, and map-after. Both map captures must be
byte-identical and retain the same map manifest. Map and inventory must have the
same fortress folder/site, paused tick and software versions. Their native
incarnation numbers remain separate namespaces. The selected region must match
exactly. The socket closes before roster projection and route optimization.

The existing operations decoder and retained-page reader are reused, including
full relation validation, whole-capture SHA-256, exact offsets, retained token,
manifest consistency and release acknowledgment. A small additive parameter to
the shared protobuf decoder allows the fixed map profile's larger reply ceiling;
all existing callers keep their unchanged default page limit. The transport
bounds the declared frame before reading, and the decoder applies the same bound.
No generic method or caller-selected wire profile is exposed.

Candidate projection uses the existing furniture inventory policy: only supported
ground singleton furniture, with known material, valid coordinates, no other
projected flags, no holder/container relation, no contained item, no job-item
attachment and no explicit exclusion. Every item receives one classification.
The complete roster is validated before selection, so an attachment on the last
page cannot be skipped to make an attractive item appear available.

### Output and handoff limits

Successful acquisition emits one bounded JSON/Agent Turn object with `ok: true`.
Its `result.status` is `allocated`, `shortage`, or `target_unestablished`; callers
must inspect that status, not just the exit code. Only `allocated` has a complete
`result.plan` in the existing exact-item plan format. Shortage and unknown-target
reports exit zero but never contain an executable subset. Refused operations
exit two with `ok: false`, null result/request/source and a sanitized stable error.
A partial stdout write cannot start another acquisition or print a second object.

The pure result and its decision digest remain unchanged inside the outer
acquisition packet. Its `native_acquisition_proven: false` describes the pure
allocator; the outer `native_capture_established` records acquisition by this
invocation. The separate source record binds both raw capture digests, manifests,
region, horizons, classification counts and pure allocation digest. These are
local evidence identities, not signatures or independent network attestation.
The Agent Turn canonical world anchor remains null: this Python workflow does not
issue a Rust world generation.

The plan can be separately reviewed with the existing furniture-batch workflow,
but this command does NOT produce a durable route-aware placement handoff.
Neither legacy exact-item plans nor this report enforce future route freshness,
maximum route length or material restrictions at later placement time. Earlier
furnishings may block a route for a later target. Reobserve and review instead of
inferring future accessibility or silently substituting another item. Existing
placement receipts and unresolved effects are not inspected or discharged.

Equal paused endpoints around inventory do not establish an atomic cross-profile
snapshot, continuous stability, native movement rules, global absence, danger or
placement eligibility. These limits remain explicit in every successful packet.

### Shared budgets and validation

The full invocation shares one 1..60,000 ms wall deadline (default 10,000), 272 RPC
calls, 20 MiB network allowance, and 20,000,000 work units. Four bindings, two
handshakes, two map reads, at least one inventory page and its release are admitted
before connection. Longer inventories consume additional calls from that same
allowance. The solver reserves the remaining work allowance after decoding and
projection; its internal counter charges every structural unit. The outer callback
rechecks deadline and current authority, without refilling either allowance.
CPU checks and synchronous I/O deadlines are cooperative, not hard real-time
preemption. Complete output is at most 96 KiB, never a truncated executable plan.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_allocation test_route_furniture_inventory -v
```

All **35 actual Python test functions passed**: 16 core tests and 19 integration
functions. Integration tests use an independently encoded, joined loopback peer
and real CLI subprocesses. They cover three-page inventory with a final attachment,
map replies larger than an operations page, all 512 item-flag words, holder and
containment exclusions, wrong fortress/tick/pause/region, source/nonce drift,
malformed frames and protobuf, lost release replies, wrong release tokens,
notification limits, single-use acquisition, no-write CLI behavior, sanitized
failures, pre-contact budgets, post-acquisition deadline loss, solver revocation
and final-render revocation. The peer asserts literal fixed bindings and request
fields, not just constants imported from the client. Existing source dependencies
were reconstructed byte-exactly and checked against their pinned Git blob hashes;
only the shared decoder's explicit bounded-size parameter was changed.

The peer is not DFHack. No real SDK/plugin build, live fortress, Rust/MCP route,
full repository qualification or production admission is established. This adds
an executable native-protocol client; it does not claim a native-game test pass.
