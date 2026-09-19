# Bounded work-order creation — development protocol 1.10

## Implemented scope

`bridge/dfhack-work-orders-v1_10` implements a separate native creation path for
finite wooden furniture orders: WoodenBed=1, WoodenDoor=2, WoodenTable=3, and
WoodenChair=4. Quantity is 1..100, never an unlimited order. The fixed template
uses OneTime frequency, wood material category, unrestricted workshop ID (-1),
maximum one workshop, no conditions/reaction/item/art specification, and no
forced validation or activation. An observed new order is not proof of manager
approval, material availability, workshop feasibility, or completed production.

The existing native generations, read-only runtimes, job-suspension/1.9 MCP
server, dependencies, compatibility registry and production runner map are
unchanged. Protocol 1.10 has no production runner and is not admitted. This
increment does NOT expose creation through MCP or provide durable restart
coordination. Direct users of the native commit method must supply an external
durable, authorized coordinator before dispatch; native process-local retention
alone is insufficient for restart-safe creation.

The fixed native methods are Handshake, ReadOrders, PrepareOrder, CommitOrder,
and QueryOrder, all registered with zero flags for DFHack-owned suspension.
There is no timer, onUpdate work, shell/Lua surface, generic native command,
worker interruption, order deletion, existing-order edit, or clock advancement.

## Operator enablement and bounded transaction

All methods require exactly `DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10=1` and a
matching `DFMCP_WORK_ORDERS_TOKEN` of 32..256 bytes. Client nonce is 16..64 bytes.
Prepare and commit additionally check `DFMCP_WORK_ORDERS_ALLOW_PRODUCTION=1` on
every call. Revoking production enablement leaves read/query available; this
operator switch is not a substitute for ConfigureProduction authority in a future
Rust coordinator. Unknown protobuf fields, missing required fields, mismatched
protocols, wrong optional-field shapes and unbounded values are refused.

ReadOrders captures world folder/site, game tick, paused state, bridge incarnation,
intervention sequence, next order ID, and the complete sorted unique set of at
most 4,096 existing native order IDs. It does not claim coverage of existing
order configurations, requirements or resources. No native pointer is retained.
The witness hashes exact canonical bytes, not an incidental JSON encoding.

Prepare binds the witness, recipe, finite amount, ASCII idempotency key and
server-checked plan digest. It requires a paused loaded fortress, spare queue
capacity and an unexhausted native ID/sequence. Retention is bounded to 256 plans;
there is no eviction. Same-key replay returns original content and does not renew
the 60-second monotonic lifetime. A changed-content key conflicts.

Commit rechecks lifetime, incarnation, sequence and the complete current queue
witness before insertion. Changed or expired preconditions retire the key as
Refused without writing the game. The engine marks Unknown and fences competing
preparations before the only insertion callback. Native object/vector allocations
are owned and reserved before the two semantic writes (append pointer, increment
next ID); allocation failure cannot leak the object or fabricate creation.

Created requires exact expected queue readback: only the new ID, next-ID horizon
and local sequence may change. The actual handler independently verifies every
fixed-template field of the new native manager order before issuing a configuration
witness. Receipt construction completes before publishing terminal state. Any
exception or mismatched readback retains Unknown, never a fabricated negative
receipt. An unresolved insertion blocks new keys and other prepared commits.
Created, Refused and Unknown replay never invoke insertion again.

Pause/unpause events retire earlier preparations through sequence advancement.
World/map load/unload resets native retention and advances incarnation without
wrapping. Missing native records after reset are NOT proof of non-creation, an
undo, or permission to retry. Hashes are identity checksums, not signatures or
proof against a malicious same-user controller/plugin.

## Wire identity

Unsigned integers are big-endian and strings use unsigned 16-bit UTF-8 byte lengths
without NUL. Native ID fields stay inside the nonnegative signed-32-bit domain.

Observation (`DFMWO010`, maximum 17 KiB): generation u64, sequence u64, tick u64,
next_order u32, site u32, paused u8, folder text (1..512), count u32, strictly
increasing order IDs (u32, each below next_order). Recipe specification is recipe
u8 plus amount u32. Configuration (`DFMWOC10`) is newly allocated ID u32 and spec;
it is emitted only after full native field verification, not merely type matching.

Plan = SHA-256(`dfmcp-work-order-plan/1` + NUL + spec + observation witness).
Token = first 16 bytes of SHA-256(`dfmcp-work-order-token/1` + NUL + generation
u64 + key text + plan). Keys are 1..128 ASCII alphanumeric, period, underscore or
hyphen. Native enum values or arbitrary reaction names are not request arguments.

Effect (`DFMWOE10`, maximum 357 bytes): generation u64, sequence u64, original
tick u64, allocated-ID candidate u32, spec, original witness (32), plan (32),
token (16), state u8, after_known u8, after_tick u64, after_queue_witness (32),
configuration_witness (32), receipt (32), key text. States are Prepared=0,
Unknown=1, Created=2, Refused=4; value 3 is not an implicit not-applied outcome.
Only Created carries readback. All absent backing fields are canonical zero.

Receipt = SHA-256(`dfmcp-work-order-receipt/1` + NUL + generation u64 + key text
+ plan + token + state u8 + after_known u8 + after_tick u64 + after_queue_witness
+ configuration_witness). Prepared/Unknown have zero receipt bytes. Created and
Refused carry terminal checksums with different meanings; neither proves goods
were produced. Old generation vectors and evidence do not qualify this protocol.

## Executed checks and remaining integration

Run:

```sh
python scripts/test_work_orders_native.py --ubsan --mutations
python scripts/test_work_orders_native.py --compiler clang++ --ubsan --mutations
```

Both GCC 14.2 and Clang 17 passed 729 engine assertions in six groups and 348
actual-handler assertions in five groups, using C++17, warning-denied compilation
and nonrecovering UBSan. Five emitted native vectors match independent Python
hashlib/struct reconstruction and checked-in fixtures. Four separately compiled
mutants (commit witness, replay fence, queue readback and uncertainty fence removed)
were rejected by executed assertions under each compiler. Reports with source
SHA-256 are retained in `docs/evidence/work-orders-native-{gcc,clang}.json`.

The handler tests compile the complete actual plugin source against explicit
DFHack/protobuf doubles, including lost reply allocation, production revocation,
all four recipes, queue corruption, vector allocation failure, 24 template-field
corruptions, lifecycle events and exact method registration. They do not exercise
real DFHack fields, generated protobuf serialization, CoreSuspender/event delivery,
a Rust client, a filesystem coordinator, MCP or a live fortress. No full repository,
native-plugin or production qualification is established.

Native field/API references reviewed: DFHack/df-structures revision
`3bfa5aa5ae3fdbe4e77d22f8125b1823c5847ccc`, `df.workquota.xml` blob
`09a58a4e651e0ab4d83c1656a8332e3c24f99bc1` and `df.job.xml` blob
`3160dcfd3d508ef82a2e08d34196b3b0ffd5515b`; DFHack `plugins/orders.cpp`
blob `174c6454541a2a84bbe9864ad9f38d177b14d864`. These are API inspection
references, not evidence that this plugin builds against those SDK revisions.
