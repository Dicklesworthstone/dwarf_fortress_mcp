# Work-order approval and progress — unadmitted read-only protocol 1.12

This batched profile is separate from the single-order progress/1.11 native
reader added on upstream main in `33e9abf0f875948e0ec79472c0625661651f3c09`.
It leaves `dfmcp_order_progress_v1_11`, `DFMOP011`, its two methods and its evidence
unchanged. Batched `DFMWP012` has its own 1.12 package, method binding and session
namespace; the codecs are not interchangeable and admission does not transfer.

This separate profile observes the state of 1..32 selected native manager-order
IDs. It closes the read-side gap after finite work-order insertion: an agent can
inspect validation/activity flags and remaining-work counters without treating
an insertion receipt as completed production. No existing native generation,
creation journal, production runner or compatibility entry is modified.

## Native contract

`dfmcp_work_order_progress_v1_12` exposes exactly `Handshake` and
`ReadObservation`, both with DFHack suspension flags zero. It contains no setter,
create/delete path, onUpdate callback, timer or retained native pointer. Calls
require exact `DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12=1`, the separate
32..256-byte `DFMCP_WORK_ORDER_PROGRESS_TOKEN`, a 16..64-byte nonce and protocol
1.12. Unknown protobuf fields and incorrect request shapes are refused.

Selection is 1..32 strictly increasing unique nonnegative signed-32-bit IDs.
Every read scans and validates the entire bounded queue (at most 4,096 orders):
null pointers, duplicate IDs, invalid horizons and oversized queues invalidate
all selected absence claims, even when the bad entry is outside the selection.
Exactly one present/absent row is returned for every requested ID. An absent row
means only that the ID was not in the observed queue, not why it disappeared.

The `DFMWP012` observation carries generation, capture sequence, game tick,
folder/site, pause state, next-ID horizon, total queue count and selected rows.
Present rows include job type/key, reaction, remaining and total counters, raw
status, frequency, workshop restrictions, next-check time, condition counts and
an optional recognized finite-wood recipe. Wire integers are big-endian; text
has unsigned 16-bit byte lengths, valid UTF-8, no NUL, and explicit bounds.
The complete observation is at most 16 KiB. Absent rows carry no hidden backing
fields. The machine layout is `architecture/work_order_progress_v1_12.json`.

Recipe recognition checks the complete *static* finite-wood template from 1.10:
job/item/material/art selectors, workshop restrictions, absence of conditions,
finite total of 1..100, and OneTime frequency. Dynamic status, remaining count
and next-check time are intentionally excluded from static template identity.
Unknown status bits or inconsistent counters disable recognition. Recognition
is not linkage to a historical creation receipt and does not grant authority.
Existing arbitrary orders still expose the selected bounded fields with recipe
0; their entire configuration is NOT claimed to have been serialized.

World/map lifecycle events advance the progress-plugin incarnation. Each capture
consumes a monotonic sequence without wrapping. A failed serialization can leave
a sequence gap; no missing capture is invented. Game clocks may be unpaused;
DFHack's per-call suspension provides one coherent capture, not ongoing control
of the game clock. An order's disappearance, changed configuration, decreased
allocation horizon, clock regression or changed incarnation cannot certify
production completion. Game counters can also be modified by other controllers.

## Evidence

`python scripts/test_work_order_progress_native.py --ubsan` and the same command
with `--compiler clang++` each passed **1,358 assertions in five groups**. Both
compile the complete actual plugin source against explicit SDK/protobuf doubles,
with C++17, warning-denied compilation and nonrecovering UBSan. The emitted native
fixture matches an independent Python struct reconstruction byte for byte.

Tests exercise approval and counter progression, observed zero and disappearance,
all four templates and all 100 finite totals, 23 static-template perturbations,
complete-queue corruption outside selection, maximum queue/selection, authentication,
request shape, lifecycle reset, malformed text, allocation/serialization failure,
immutable owned captures and exactly two zero-flag methods. Reports retain exact
source SHA-256. This is not a real DFHack SDK build, protobuf runtime test,
CoreSuspender/event-delivery test or a live fortress campaign.

Native field semantics were inspected in `DFHack/df-structures` revision
`3bfa5aa5ae3fdbe4e77d22f8125b1823c5847ccc`, `df.workquota.xml` (manager order,
status and frequency) and `df.job.xml` (recipe type names), and in the existing
1.10 creation implementation. Those source references do not establish plugin
build compatibility or lifecycle correctness against a particular game release.

Three separately compiled mutants removing complete-queue uniqueness, static
material recognition or unknown-request-field rejection are rejected by executed
assertions under both compilers. Use `--mutations` to reproduce those checks.
The Rust/MCP integration and its independent evidence limits are documented in
`docs/WORK_ORDER_PROGRESS_MCP.md`.
