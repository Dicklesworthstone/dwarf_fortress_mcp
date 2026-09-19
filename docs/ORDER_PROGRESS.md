# Selected work-order progress — unadmitted read protocol 1.11

The separate native `dfmcp_order_progress_v1_11` plugin reads current manager-order
counters and approval/activity flags. It complements creation/1.10 but never
changes that generation, its journal, or its evidence. No production runner or
compatibility admission is added. Current order counters are not produced-item
counts, native disappearance is not completion, and a zero remaining counter
alone is not a goods-production proof.

## Native read boundary

The two fixed methods are Handshake and ReadOrderProgress. Both require exactly
DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11=1 and the independent
DFMCP_ORDER_PROGRESS_TOKEN (32..256 bytes). Nonces are 16..64 bytes. Unknown fields,
missing required fields, wrong shapes, and non-1.11 protocols are refused.
Zero-flag RPC registration leaves all game access inside DFHack suspension.
There is no setter, creation, activation, validation override, command/Lua surface,
onUpdate callback, timer, job interruption, or retained native pointer.

ReadOrderProgress selects a native manager-order ID and scans the entire bounded
queue (at most 4,096 IDs), rejecting nulls, duplicates and IDs at/above the
allocation horizon even when the selected order was already found. Absent means
not in that complete current queue. It does not distinguish deletion, completion,
restore, or a never-allocated requested ID. World/map events change the reader's
incarnation. Sampling sequence is not a canonical world revision or game tick.

Present records preserve native job type, frequency, remaining/total counters and
raw status bits. Recipe codes 1..4 are emitted ONLY for the full finite wooden
bed/door/table/chair template, including no conditions/customization, wood-only
materials, OneTime frequency, global workshop scope and max_workshops=1. The
counter total must be 1..100 and remaining must be no greater than total. Unknown
configurations, infinite/repeating orders, and unexpected status bits retain their
raw fields but receive recipe code zero. Scheduling timestamps, validation and
activity are dynamic, not immutable template fields. Recognized configuration is
not evidence of feasibility, cause of inactivity, or correlation to a prior
creation from an independently incarnated plugin.

## Canonical observation

All integers are big-endian; booleans are exactly zero or one. DFMOP011 encodes:
magic[8], generation u64, sample sequence u64, game tick u64, selected order u32,
next-order horizon u32, site u32, paused u8, present u8, folder text (u16 byte length,
1..512 valid UTF-8 bytes without NUL). Only a present record then carries native
job type i32, remaining u32, total u32, raw status u32, frequency i32, recipe u8.
The maximum record is 1,024 bytes. An absent record has no synthetic payload.
Native IDs fit nonnegative signed 32-bit values. Counters fit nonnegative signed
16-bit values. Raw unknown type/frequency values are preserved within bounds.
The generation and successful sample sequence never wrap. The sequence may skip
on failed serialization; this does not imply lost game history.

## Executed native evidence

`python scripts/test_order_progress_native.py --mutations` and the same command
with `--compiler clang++` each pass 1,456 assertions in four groups under C++17,
warning-denied compilation and nonrecovering UBSan. They compile the actual full
handler with explicit checked-in DFHack/protobuf doubles. Four native vectors
match independent Python struct packing and checked-in hex fixtures. Three
separately compiled mutants remove duplicate validation, template-condition checks,
or correct counter acquisition; all are rejected by actual test assertions.
Reports retain source hashes in docs/evidence/order-progress-native-{gcc,clang}.json.
This is NOT a real SDK build, generated-protobuf run, real CoreSuspender/event test,
Rust execution, live fortress campaign, or qualification/admission evidence.

Field references: DFHack/df-structures 3bfa5aa5ae3fdbe4e77d22f8125b1823c5847ccc,
df.workquota.xml (09a58a4e651e0ab4d83c1656a8332e3c24f99bc1); the existing
creation/1.10 fixed-template verifier. These are field inspection references, not
proof that this plugin builds against that SDK revision.
