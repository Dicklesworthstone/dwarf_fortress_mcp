# Bounded normal-mining designation — development protocol 1.16

`dfmcp_dig_v1_16` adds a separate native designation transaction. It sets normal
mining (`Default`) on a rectangular 1..8 by 1..8 selection at one z-level, at
priority 4000. It does not excavate terrain, create jobs directly, advance time,
remove existing jobs/designations, channel, make stairs, or remove constructions.
No existing bridge generation or production runner is changed. No tuple is admitted.

This is native development source, tested against explicit SDK/protobuf doubles,
not a real DFHack installation. The Rust evidence/client integration is a separate
step. A production-authorized, durable external coordinator remains mandatory
before dispatch; native in-memory idempotency alone cannot survive process death.

## Observation and eligibility

A complete capture includes the target rectangle and its one-tile 3D halo, at most
300 cells. It binds world folder/site, dimensions, game tick, paused state, native
incarnation and intervention sequence. Cells are missing, hidden, or visible.
Hidden cells carry only a presence tag. Their type, material, designation, priority,
temperatures and occupancy are not interpreted or serialized. Missing map blocks
are not allocated. World/map events change incarnation; pause events fence plans.

Visible cells retain exact tile type, non-dig designation bits, occupancy, priority,
block scheduling bits/cooldown, temperatures, and bounded policy classifications.
A full scan of at most 65,536 jobs is required before claiming no job at a target.
Null jobs, repeated IDs/cycles and an oversized list refuse acquisition, including
malformation after a relevant job was found. Block-event scans are bounded to
4,096 entries in at most twelve halo blocks; duplicate priority events are refused.

Every target must be a visible natural soil/stone/mineral WALL, with no existing
dig/smoothing designation, occupying building/unit/item, or existing job. The game
must be paused. Missing halo blocks and known visible liquid, aquifer, feature, or
temperature-above-10080 evidence anywhere in the halo block preparation.

The default `allow_hidden_neighbors=false` also refuses a hidden halo cell. Setting
it true explicitly acknowledges unobserved neighbor risks, and is sealed into the
plan. It NEVER permits a hidden target, a missing block, or a known hazard. No
hidden tile is revealed or inspected. This switch is intent, not capability or
confirmation authority. A caller must independently authorize the guarded action,
its target write scope and halo read scope, and honor required checkpoint/protected-
area policy. Passing these local checks does NOT certify excavation safety,
structural support, absence of latent water/magma, miner reachability, material
value, or safe future unpausing. Temperature is a fixed conservative native-unit
filter, not a complete thermodynamic or geological model.

## Fixed native surface and operator gates

Six zero-flag RPC methods are registered under `dfmcp_dig_v1_16`:
`Handshake`, `ReadDesignation`, `PrepareDesignation`, `CommitDesignation`,
`QueryDesignation`, and `CancelDesignation`. DFHack's suspended RPC dispatch owns
all native access. There is no onUpdate work, timer, thread, Lua, arbitrary command,
or pointer retained outside the call.

All methods require `DFMCP_ALLOW_UNADMITTED_DIG_V1_16=1` and a matching
`DFMCP_DIG_TOKEN` of 32..256 bytes. Nonces are 16..64 bytes. Prepare, commit and
preparation cancellation also require `DFMCP_DIG_ALLOW_DESIGNATE=1` on every call.
Revocation leaves reads and receipt lookup available. Unknown protobuf fields,
missing required fields, incorrect optional-field shapes and wrong profiles fail.
No request selects a command, raw native address, enum, priority, path or endpoint.

## Transaction and uncertainty

Preparation seals the complete observation witness, region, hidden-neighbor policy,
key, plan digest and token. It never writes the game. Retention is capped at 128
keys with no eviction. Exact-key replay neither rereads the map nor extends the
60-second monotonic lifetime; changed-content reuse conflicts.

Commit rechecks lifetime, incarnation, intervention sequence, every captured field,
and current eligibility under the same native suspension. Failed revalidation
retires the preparation as Refused without any write. The exact allowed post-state
is constructed before dispatch. Unknown is published and competing preparations
are fenced BEFORE the native writer runs.

The writer preallocates optional priority events and reserves their block-vector
slots before semantic writes. New priority events are zero-initialized; only target
priorities become 4000. It writes only target dig bits and priorities plus the
`designated` flag and zero designation-check cooldown on touched blocks. It avoids
MapCache::WriteAll, which can remove existing designation jobs. Existing job lists,
non-target priorities, and unrelated captured bits remain untouched.

Readback must match the entire predicted capture, including scheduling fields in
visible halo cells sharing a touched block, not merely the number of dig bits set.
Only then can Designated evidence and its checksum be published. Allocation errors,
partial writes, no-op writes, wrong readback or post-write exceptions retain Unknown.
This is NOT a rollback-capable atomic game batch. Unknown blocks new keys and other
prepared commits. Replaying Unknown, Designated or Refused never dispatches again.

CancelDesignation retires a still-prepared key as Refused/Cancelled. It cannot clear
a dig flag, undo excavation, reclassify Unknown, or erase a record. A lost reply can
be recovered with exact key/digest lookup while native retention exists. Missing
records after unload/restart are not evidence of nonapplication or permission to
retry. Hashes are integrity/identity checksums, not signatures or authority.

## Canonical formats

All integers are big-endian. Strings have unsigned 16-bit UTF-8 byte lengths and
no NUL. Region is x/y/z/width/height as five u32 values. Coordinates and full halo
must fit the observed map and nonnegative signed-16-bit tile space.

Observation `DFMDG016`: generation/sequence/tick u64; site/map-x/map-y/map-z u32;
region; paused u8; folder text (1..512); cell-count u16; cells in z/y/x halo order.
Each cell starts with presence u8 (0=missing, 1=hidden, 2=visible). Visible cells
then carry six u32s (tile type, non-dig designation, occupancy, priority, cooldown,
non-designated block flags), two u16 temperatures, dig u8, hazard-mask u8 and flags
u8. Flags: natural-wall=1, smoothing=2, occupied=4, existing-job=8, block-designated=16.
Hazards: liquid=1, aquifer=2, feature=4, temperature=8. Redacted cells have no payload.
Maximum observation is 16 KiB; actual maximum by this fixed shape is below 11 KiB.

Plan = SHA256(`dfmcp-dig-designation-plan/1` + NUL + region + allow-hidden u8 + witness).
Token = first 16 bytes of SHA256(`dfmcp-dig-designation-token/1` + NUL + generation
u64 + key text + plan). Keys are 1..128 ASCII alphanumeric, period, underscore or hyphen.

Effect `DFMDGE16`: original generation/sequence/tick u64; region; allow-hidden u8;
witness/plan (32 bytes each); token (16); state/reason/after-known u8; designated-count
u32; after-witness/receipt (32 bytes each); key text. Maximum is 334 bytes.
States: Prepared=0, Unknown=1, Designated=2, Refused=4. Reasons: None=0, Stale=1,
Cancelled=2. Only Designated has after evidence and count. Prepared/Unknown have
zero receipts. Refused has no after fields and a terminal checksum. The receipt
hash covers domain `dfmcp-dig-designation-receipt/1` + NUL + generation + key text
+ plan + token + state + reason + after-known + count + after-witness.

## Executed checks and remaining integration

Run `python scripts/test_dig_designation_native.py --mutations`, then the same with
`--compiler clang++`. Both compilers pass 1,319 engine and 4,499 actual-handler
assertions with warning-denied C++17 and nonrecovering UBSan. Four actual C++ encoder
vectors match independent Python hashlib/struct reconstruction. Six independently
compiled mutants (witness, replay, readback priority, cross-key uncertainty, hidden
redaction, scheduling) fail actual assertions. Reports include exact source hashes.

These tests do not establish generated-protobuf behavior, real DFHack fields/ABI,
CoreSuspender/plugin events, filesystem crash safety, Rust/MCP execution, miner
behavior, finished excavation, or production admission. Native field/API references
inspected: DFHack Maps.h blob 93d506e2734efea591cd72a4fc6bf7a11c97b8a5 and
MapCache.cpp blob a14d81288ee0645496c5dc5b270719dbe516b948. References are API inspection,
not an SDK build claim. The mutation bead's old closed status is not new qualification.

## Coexistence and executable client

Concurrent main commits introduced `bridge/common/dig_designation.h` with
DFMDG015/DFMDGE15 and the separate order-run/1.14 RPC. This implementation was
moved to **1.16** with `dig_designation_v1_16.h` and namespace
`dfmcp_dig_v1_16`, preserving both additions. It does not reinterpret their
captures, proofs or policy. The native fixtures/reports were regenerated and
both compilers rerun against the exact 1.16 bytes.

`DIG_DESIGNATION_CLIENT.md` documents the executable one-shot Python developer
client. Its immutable, fsynced intent prevents redispatch through an existing
capsule but is not the missing Rust durable coordinator or a terminal journal.
MCP remains unchanged. Read the hidden-neighbor policy and evidence limits before
using the explicitly gated native effect on a disposable fortress.
