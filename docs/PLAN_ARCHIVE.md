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
