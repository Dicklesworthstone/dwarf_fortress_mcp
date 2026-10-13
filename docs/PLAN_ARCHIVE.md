# Exact original-plan archives

Recovery increment for `df-franken-storage-mvcc-54h.1` and native handoff bead
`df-dfhack-bridge-plane-c-pic.4`. Previously the semantic/native association
stored plan identity but required the embedding caller to retain and resupply
all original `PreparedPlan` content. Recompiling a remembered request can change
a goal, expiry, compensation, dependency or obligation under newer code.

## Additive custody decision

`dfmcp_intent::plan_archive` is a bounded lossless archive of an already sealed
plan. It does not change the existing semantic plan digest, idempotency scheme,
planner, bridge protocol, journal or native coordinator. Existing stores are not
migrated. The archive never supplies policy, credentials, current authority,
checkpoint proof, a native receipt, current observation or evidence of completion.

The closed binary format is `DFMPAR01`, a big-endian u32 body length, the typed
body, and a SHA-256 trailer. The checksum covers the domain
`dfmcp-exact-plan-archive/1` followed by NUL and the complete header/body.
All body integers are big-endian; counts/string lengths are u32; text is UTF-8
without NUL. Strings are not executable instructions. Every enum is explicitly
tagged; unknown tags and boolean values other than zero/one refuse.

`encode(plan, limits)` returns bytes and a `PlanArchiveSeal` containing both the
unchanged semantic digest and the archive-object digest. `decode(bytes, seal,
limits)` requires that externally selected pair, checks the entire object before
parsing, validates the recovered original plan with `validate_structure`, and
requires byte-exact re-encoding. No automatic normalization, rebase, expiry renewal
or key regeneration occurs on read. Retain the seal in the original handoff; do
not derive the expected identity from an untrusted file's own trailer.

Archive tags preserve `Other`/`Custom` variants even when they reuse a reserved
name. This matters because the older semantic kind encoding is name-based.
Two such typed plans may share a semantic digest but have different archive
objects. The separate object identity must never be discarded in favor of the
semantic digest alone. This increment preserves, rather than migrates, that
older identity scheme.

## Bounds and cancellation

The profile accepts at most 4 MiB per framed archive, 256 plan steps, 65,536
aggregate members/nodes, combined predicate/value depth 64, 4,096 members in
other collections, 64 predicates per step section, and 64 KiB per general
string/blob. Summaries, predicate fields and kind names retain tighter bounds.
This is intentionally narrower than the general 4,096-step planner limit.
`ArchiveLimits` may narrow but not widen the hard profile.

A bounded writer visits the input before the legacy validator may clone,
normalize or hash it. The reader checks counts against both the hard profile
and remaining bytes before allocation, checks depth before descending, and
rejects duplicate or unordered map/set entries. Encode/decode `_with_check`
variants accept caller-owned cancellation/deadline checks throughout visits
and before return. Legacy structural validation, final hashing and canonical
normalization are bounded but not internally cooperatively interrupted.

## Evidence, 2026-10-13

Thirteen Rust tests are present, covering every action family, all building/dig
variants, literal values, composite predicates, dependency/obligation custody,
reserved-name aliases, original expiry/key/goal preservation, every truncation
and single-byte corruption of a fixture, rechecksummed invalid data, limits and
cooperative aborts. They have not been compiled or executed in this environment.

`python scripts/check_plan_archive_vector.py` executes an independent byte
construction of the 324-byte pause archive. The persisted fixture, length,
object checksum and unchanged semantic plan digest agree. This is fixture/design
evidence, not execution of the Rust codec or native recovery.

Required Rust checks:

```text
cargo test --locked -p dfmcp-intent --test plan_archive
./scripts/verify.sh
```

No full qualification, native/live success, production admission, or broader
storage-bead completion is claimed.

## Durable owner and original work-order reattachment

`dfmcp_adapter::plan_archive::PlanArchiveStore` holds exactly one immutable
archive under injected `EffectJournalStorage` custody. `create` requires current
whole-fortress Query and Plan authority; it validates the entire source before
storage contact, refuses nonempty backing, writes the frame, flushes and syncs,
and independently reads every byte back before returning the store and seal.
It never truncates or overwrites an existing complete or partial archive.

`open` is read-only. It requires an independently retained `PlanArchiveSeal`,
verifies the complete frame and recovered original plan, and compares the backing
bytes again before accepting custody. `recover` rechecks the whole backing before
and after decoding. Changed bytes, extent or storage identity fence that owner;
putting old bytes back does not unfence it. A fresh verified reopen is required.
Invalid credentials or insufficient budgets do not repair bytes or grant access.

`create_private_plan_archive` and `open_private_plan_archive` use the existing
Linux x86_64/aarch64 descriptor-pinned private-file backend: normalized absolute
operator paths, no symbolic links, a 0700 parent and a single-link 0600 regular
file, exclusive locking, and file plus directory synchronization on publication.
The creation wrapper validates and reserves the source before it can create a
file. An existing file is never silently adopted by create. Unsupported platforms
refuse instead of substituting weaker file custody. Paths are embedding/operator
configuration, not MCP tool arguments.

The immutable archive is not a new native effect journal. It does not migrate
old associations or recover original content that was never archived. Retain
both archive identities in the operator's original handoff before relying on
restart recovery; deriving expected identity from a file's own trailer defeats
that selection check. A digest is not an operator signature or policy grant.

### Workflow

Before native preparation, archive the exact `PreparedPlan` already inspected
by the operator and retain the returned seal. After restart, open the existing
native coordinator and its association journal under their existing recovery
rules, then open the original archive using that retained seal:

```text
let mut archive = open_private_plan_archive(
    &operator_path, retained_archive_seal, archive_limits, &current_context,
)?;
let review = archive.reattach_work_order(&mut recovered_work_order_session, &current_context)?;
```

This is an embedding sketch, not a runnable standalone example. The helper
loads the exact original plan, then calls the **existing** semantic work-order
`reattach` path using the remaining byte allowance and wall deadline. It cannot
prepare, insert, reconcile, renew expiry, start a goal monitor or issue policy.
The coordinator still requires the original native key, association, exact plan
and source binding. An archive cannot adopt unrelated/unassociated native work.
Unknown native effects remain Unknown; recovering the plan does not prove an
insertion or goal. Labor and other embeddings can explicitly pass `recover()`'s
original value to their own existing review/recovery APIs; no automatic generic
dispatch was introduced.

### Authority, budgets and failure scope

Every load uses current Query authority, not archived grants or the original
anchor's tick. The store is bound to the opening session and fortress. Contexts
older than the original epoch/tick/sequence or inconsistent at the same cursor
refuse. Within an open owner, accepted current-context floors cannot regress.
A later independently supplied restore epoch may inspect the old plan as history,
but this does not make that plan valid for dispatch in the restored fortress.
The trusted embedding remains responsible for issuing fresh current contexts
on reopen; this immutable file does not persist a global authorization clock.

The total foreground deadline is capped at 60 seconds and never replenished
between file open, codec, verification and native reattachment. Before codec
work, the store reserves four times the selected `ArchiveLimits.max_bytes` in
byte-budget units, plus actual read/write/verification bytes. This is a fixed
policy reservation, not a count of every internal copy or a peak-memory estimate.
The core's independent byte/node/depth limits remain in force. Narrow the archive
profile for small plans rather than granting a larger operation by default.
The step count is also bounded by the current entity allowance during decoding.
File reads and writes are checked between chunks; a blocking injected I/O call
cannot be preempted internally by this synchronous owner.

A short write, failed sync, cancellation/deadline loss, or failed readback returns
no published owner. Surviving partial bytes refuse unchanged. A complete frame
whose sync acknowledgment was lost can be explicitly opened read-only under its
known seal. That proves readable original bytes only: it does not retroactively
certify the lost durability acknowledgment or authorize a new native attempt.

Sixteen additional Rust store regressions are present, covering private-file
custody, exact publication/reopen, no-write recovery, denied authority, expired
grants, session and context floors, exact budgets, every fixture truncation and
byte corruption, partial writes, lost sync acknowledgment and permanently fenced
owners. All **29 Rust codec/store tests remain uncompiled and unexecuted here**.
The native reattachment helper is source-reviewed, not runtime-qualified.

The independent Python fixture/framing check now also rejects **651** truncation,
byte corruption, trailing-byte and changed external-identity cases. These are
Python framing checks; they do not execute the Rust decoder, store or private-file
backend and do not cover typed plan validation.

```text
python scripts/check_plan_archive_vector.py
cargo test --locked -p dfmcp-adapter --test plan_archive_store
```
