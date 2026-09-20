# Bounded ordinary-mining designation — development foundation

`bridge/common/dig_designation.h` adds the missing native transaction engine for
`designation.dig`, limited to ordinary mining of a single-z rectangle 1..8 by
1..8 tiles. It never excavates terrain, advances the clock, clears a designation,
sets stairs/channels/ramps, changes priorities, or enables automatic mining.
This first increment has no RPC or MCP route and is not production admission.

## Conservative selection and exact witness

The complete one-tile, three-dimensional halo is captured in z/y/x order (at
most 300 cells). All halo cells must be allocated and revealed, natural stone,
mineral or soil wall/floor, without a dig designation, liquid, aquifer/feature or
auto-dig marker, and both observed temperatures must be below 10075. Each target
must additionally be a rough natural wall without smoothing, occupancy or a
non-normal special. An unknown/hidden cell cannot be treated as an empty safe
cell. Hidden attributes have no wire payload. The capture includes folder/site,
map dimensions, game tick, pause state, bridge incarnation/intervention sequence,
raw tile types, non-dig designation bits, occupancy, non-designated block flags,
block designation markers, classifications and temperatures.

These are conservative local guards, NOT structural-support analysis, hazard-free
mining certification, global pathfinding, miner availability, protected-region
policy or excavation completion. In particular, no cave-in safety claim follows
from visible dry rock. The eventual caller still requires guarded Designate
authority and explicit operator enablement. Use disposable forts pending live
qualification. Native fixture classifications are not a real SDK compatibility
certificate.

## Prepare, commit and uncertain outcomes

A preparation seals the exact entire capture witness, small target rectangle,
ASCII key and plan digest. The game must be paused. Key replay is content-bound
and never extends the 60-second monotonic lifetime. The engine retains at most
128 keys with no eviction. Commit rechecks the original whole-halo witness,
incarnation, intervention sequence, paused eligibility and lifetime.

A stale preparation is permanently Refused before receipt allocation. Before
calling the setter, the engine constructs expected readback, advances its
intervention sequence and records Unknown. The setter may only set target dig
flags to Default and affected blocks' designated flag. Exact readback permits
only those changes and the sequence advance: an unrelated changed field, missing
block marker, partial write, allocation error or exception remains Unknown.
There is no rollback and no blind retry. Any Unknown blocks other keys as well
as the original one. Designated/Refused/Unknown replays never call the setter.
A Designated receipt establishes exact immediate designation readback, not a job
being assigned, mining being safe, or excavation having happened.

The engine is process-local. Lifecycle reset advances incarnation and clears
retention; a missing key is not a negative effect receipt. A durable authorized
coordinator must retain dispatch intent before native commit and never use a new
key or new store to bypass uncertainty. Native hashes are integrity checksums,
not signatures or authority against a malicious trusted plugin/controller.

## Canonical bytes

Unsigned scalars are big-endian and text has a u16 byte length. Observation magic
is DFMDG015; effect magic is DFMDGE15. The engine's encode functions are the fixed
canonical definition. Observation bytes are at most 16 KiB, keys 1..128 ASCII
alphanumeric/period/underscore/hyphen, folders 1..512 valid UTF-8 bytes without
NUL. Presence 0=unallocated, 1=hidden, 2=visible; only visible cells carry data.
States are Prepared=0, Unknown=1, Designated=2 and Refused=4; there is no inferred
NotApplied state. Only Designated carries a nonzero count and after-witness.
Only terminal states carry a receipt; absent fields are zero.

Plan = SHA256("dfmcp-dig-plan/1" + NUL + observation_witness).
Token = first16(SHA256("dfmcp-dig-token/1" + NUL + generation_u64 + key_text + plan)).
Receipt = SHA256("dfmcp-dig-receipt/1" + NUL + complete_effect_prefix_without_receipt).

## Executable evidence

Run `python3 scripts/test_bounded_dig_engine.py --mutations`, also with
`--compiler clang++`. Both C++17 warning-denied UBSan executions pass 4,774
assertions across five groups. Four actual C++ vectors agree with independent
Python struct/hashlib encoding; four separately compiled gate-removal mutants
are rejected by actual assertions. The exact unmodified retained_snapshot.h
hash implementation is used. Reports live in docs/evidence/bounded-dig-engine-*.
This is execution of the native pure engine, not Rust, a DFHack handler/SDK,
MCP, filesystem durability, a live fort or repository qualification.
