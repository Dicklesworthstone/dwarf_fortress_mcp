# Work-orders/1.10: Rust evidence and fixed transport

The native producer and its exact scope are documented in WORK_ORDER_CREATION.md.
The `dfmcp_adapter::work_orders` module now connects that separate profile to Rust
observations, sealed finite plans, validated effects and a fixed-method RPC client.
It does not add a durable creation coordinator, MCP route, production runner,
capability grant or admission. No dependency or existing native generation changes.

## Complete selected evidence

`WorkOrderObservation::decode` retains and validates the entire bounded native
DFMWO010 membership record. The sorted unique IDs are below the signed-32-bit
allocation horizon; folder/site, paused state, tick and incarnation/sequence are
validated. Unknown counts, partial/trailing bytes and malformed UTF-8 fail closed.
This observation describes queue membership, not existing order configuration,
resources, manager availability or a canonical world snapshot. Its fortress ID
uses the same folder/site hash domain as existing live profiles.

`WorkOrderSpec` admits only four semantic wooden-furniture recipes and a finite
1..100 quantity. `WorkOrderPlan` requires an eligible paused observation with spare
queue capacity and unexhausted ID/sequence, validates its ASCII key, and constructs
the plan digest and native token locally. Callers cannot mutate a sealed plan.
Changing a key keeps the semantic plan digest but changes the native token; exact
key binding remains part of receipt verification. Eligibility is not authority.

`WorkOrderEffect::decode` requires that complete retained plan, not just a digest.
Every original identity field, recipe/amount, key, witness and token must match.
Created evidence additionally requires the exact reconstructed queue transition,
unchanged game tick, and the fixed created-order configuration witness. A
self-consistent checksum for another order/template/queue/tick is insufficient.
Prepared, Unknown, Created and Refused remain distinct. Unknown and Prepared
carry neither readback nor a receipt; Refused has a terminal checksum without a
created ID. Absent fields require canonical zero backing. Hashes are checksums,
not signatures or protection against a malicious trusted producer.

## Native transport

`work_orders::rpc::WorkOrderRpcClient` binds exactly Handshake, ReadOrders,
PrepareOrder, CommitOrder and QueryOrder in `dfmcp_work_orders_v1_10`. It checks
protocol 1.10, nonce, required/optional reply shape, minimal protobuf varints,
unknown/duplicate fields and distinct method IDs. Generation and DF/DFHack version
strings must remain the negotiated values for the connection.

Frames are bounded to 32 KiB, allowing complete 17-KiB membership observations
instead of accidentally inheriting job-control/1.9's 8-KiB ceiling. Text
notifications are bounded to eight, at most 64 KiB each and 256 KiB in total.
Every call has an explicit 1..60000-millisecond wall-time allowance. Concrete TCP
accepts only numeric loopback endpoints with nonzero ports, includes connect and
negotiation in one deadline, and applies remaining time before every blocking
read/write. Injected streams must honor the same absolute deadline contract.
There are no detached tasks, implicit reconnects or automatic retries.

ReadOrders evidence must match the manifest generation. Prepare distinguishes
fresh Prepared evidence from replayed historical native records and never treats
a replayed Created response as a fresh preparation. Commit requires exact
Prepared evidence. After entering commit I/O, all errors—including native error
envelopes, malformed effects and lost replies—become EffectIndeterminate and fence
the connection. An explicit Unknown record is returned as typed nonterminal
evidence, never a Created result. Local invalid arguments write nothing. Every
failed wire/evidence validation fences subsequent calls.

Query returns a complete plan-bound record or absence. Absence is not evidence
that an insertion never happened; a new connection does not authorize another
commit. A durable coordinator must persist dispatch intent BEFORE invoking
`commit_prepared`, retain ambiguity across restart and block blind/new-key retries.
The transport supplies no such persistence or capability authority by itself.

## Evidence and remaining work

Sixteen Rust test groups are registered (eight evidence, eight RPC), covering:
actual native vectors; all four recipes and all 100 finite amounts; every bit of
a Created record; rehashed forged receipts; state/presence and queue bounds;
UTF-8 and fortress lineage; fragmented reads/writes; lost replies; replayed
terminal preparations; source changes; invalid bindings; large observations;
frame/notification ceilings; and local validation with zero writes.

**These Rust tests were not compiled or executed.** Rust, Cargo and rustfmt are
unavailable in the implementation environment. No Rust formatting, Clippy,
workspace or runtime qualification is claimed. Source review is not compilation.

The executable Python reference checker:

```sh
python scripts/check_work_order_vectors.py
```

independently checked the five native fixtures and rejected 1,904 single-bit
Created-record corruptions, 952 incomplete effect prefixes, four trailing-byte
records and 30 rehashed/noncanonical outcomes. Its report and source hashes are
in `docs/evidence/work-orders-rust-reference.json`. These are fixed-plan Python
codec/checksum tests, NOT execution of Rust parsing, TCP, MCP or filesystem
coordination. The preceding native GCC/Clang reports remain separately scoped to
explicit SDK/protobuf doubles, not real DFHack field access or a game.

The next functional integration is a creation-specific durable coordinator and
session-bound intent/commit/reconciliation route. Do not reuse the job-suspension
journal as though its sealed records represented creation, dispatch directly from
MCP, widen existing read-only profiles, or claim completed production from a
successful native insertion. Live SDK build/transport testing, disposable-fort
campaigns and the full compatibility/admission chain remain required.
