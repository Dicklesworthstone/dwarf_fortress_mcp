# Retained original route-furniture reviews

Beads: `df-dfhack-bridge-plane-c-pic.3/.4` and
`df-franken-storage-mvcc-54h.4`. This increment preserves an allocation's original
intent across re-observation instead of running allocation again and silently
selecting replacement items. It is a read-only review, not a placement gate.

## Retained contract

`scripts/route_furniture_review.py` exports a canonical
`dfmcp.route-furniture-review/1` object containing the complete original request,
all exact selected items, native type identities, material/subtype, original
positions and route witnesses, region, maximum route length, original allocation
and plan identities, and both source manifests and observation bounds.

`create` accepts only a complete allocated result with a matching decision digest.
`Review.decode` requires an independently retained expected review digest; never
obtain that expected identity from the untrusted input being checked. The digest
covers the complete canonical JSON, prefixed with `dfmcp-route-furniture-review/1`
and NUL. It is an integrity/selection check, not an operator signature. Unknown
fields/policies, duplicate keys, noncanonical ordering or encoding, excessive
nesting, partial selections and invalid original constraints refuse unchanged.
Only immutable bytes are retained; accessors produce independent parsed values.

## Fresh review, without reallocation

`revalidate` receives a full projected candidate inventory and its native type
map, complete unchanged map/1.5 bytes, and independently supplied current source
metadata. It never calls either assignment solver. Every original selection is
checked, even after an earlier slot fails. A new better item cannot replace an
original missing or ineligible item, even when the full new inventory could
satisfy the request by a different allocation.

The same fortress, region, map dimensions, endpoint, software and two distinct
native generations must remain. Current tick and native horizons must not
predate the original source. The two generations are separate namespaces and
need not equal each other. New map/inventory digests and item positions are
allowed: material, subtype, native type, original Manhattan/same-level constraints
and the original maximum route length are rechecked, not renewed or relaxed.

All slots produce a bounded status. Missing projected items and unestablished
target/item terrain remain `unestablished`; they are not global absence claims.
Changed item identity, violated constraints and disconnection in the captured
route model produce `constraints_not_met`. Only when every original selection
passes does `plan` contain the unchanged original exact-item plan. Otherwise it
is null; the complete original plan remains separately visible as historical
intent. Each passing route is independently recomputed with the original
stable-index, target-rooted BFS policy and receives a complete path digest.

## Scope and bounds

The artifact and complete pure result are each at most 49,152 bytes. Input JSON
has depth at most 12 and at most 16,384 aggregate nodes. Requests retain the
existing 32-slot, 16-KiB request and 65,536-candidate bounds; maps retain the
16,384-tile limit. One explicit work allowance covers artifact validation,
complete candidate scanning, graph construction, all BFSs and path witnesses.
Exhaustion or callback failure returns no partial result. Hashing, bounded JSON
serialization and legacy request normalization are not internally preempted.

A source record and pure API arguments do not attest native acquisition. The
operator must independently qualify the actual captures and retain the expected
review digest. This object is not a private-file journal, a replay-resistant
current-source clock, a reservation, or evidence of an action's outcome. Its
original source floor cannot detect every later rollback across separate calls.
It does not inspect placement receipts, discharge uncertain effects, prove
native movement/safety, or predict paths after preceding furniture is placed.

Most importantly, this review does not silently install a new check in the legacy
placement executor. Extracting only its exact-item plan still loses the route
contract at execution time. Fresh native placement review remains required; an
atomic native route/placement guard and durable route-aware batch integration
remain separate work. No protocol, permission, production admission or Rust MCP
surface is changed by this increment.

## Executed development checks — 2026-10-13

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts \
  python3 -m unittest test_route_furniture_review -v
```

Seventeen actual Python test functions pass with no skips. They cover full intent
round-trip, exact selections despite better replacements, material/subtype/type
changes, moved items, blocked paths, unknown endpoints, all original slots,
source/clock/horizon mismatches, every fixture truncation and single-byte
corruption, rechecksummed malformed artifacts, exact work limits, every callback
abort, all 512 small obstacle maps against independent distance relaxation, and
65,536 candidates with 32 original targets. The three reused source dependencies
were reconstructed byte-exactly and verified against their Git blob SHAs.
These tests do not execute DFHack, Rust, or a native placement operation.
