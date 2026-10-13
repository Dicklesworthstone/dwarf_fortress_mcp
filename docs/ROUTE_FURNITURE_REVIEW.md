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

## Executable create/check workflow

`scripts/review_route_furniture.py` connects the retained contract to the existing
query-only reader. Both operations reuse the unchanged map/1.5 and operations/1.4
client, full-roster decoder, candidate projection and verified inventory release.
No existing mutation executor, native method, credential family or default
allocation command is modified.

Use the same isolated environment as `allocate_route_furniture.py`:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE=1
DFMCP_ROUTE_FURNITURE_ENDPOINT=127.0.0.1:5000
DFMCP_MAP_TOKEN=<operator-configured map read token>
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured operations read token>
```

Other `DFMCP_` variables are refused. Tokens must meet the existing read profile's
32..256-byte constraint and are never retained in artifacts or results.

```sh
python3 scripts/review_route_furniture.py create \
  --request-file furnishings-request.json \
  --origin 0 0 5 --size 64 64 1 \
  --max-route-steps 1000 --timeout-ms 10000
```

The coordinates are illustrative. `create` performs one complete read bracket,
allocates all targets, and exports `review` plus `review_digest`. An unknown target
or shortage still produces a valid report but no review and no executable plan.
An exported review is canonical JSON without a trailing newline; preserve it
exactly using `furniture_plan.canonical`. For example, from a saved successful
create response, with `PYTHONPATH=scripts`:

```python
import json
from pathlib import Path
from furniture_plan import canonical

packet = json.loads(Path('create-result.json').read_text())
assert packet['ok'] and packet['review'] is not None
with open('original-review.json', 'xb') as output:
    output.write(canonical(packet['review']))
# Retain packet['review_digest'] independently with the original operator handoff.
```

Later, including after process restart:

```sh
python3 scripts/review_route_furniture.py check \
  --review-file original-review.json \
  --expected-review <independently-retained-review-digest> \
  --timeout-ms 10000
```

`check` takes the request, region, selected items and route limit ONLY from that
original artifact. Request/region/route-limit overrides are refused before native
contact. A changed operator endpoint is refused before opening a socket; changed
source generations/software are refused immediately after the read handshakes.
One equal-paused-map bracket and a complete released inventory then feed original
selection revalidation, not the allocator. Native sources close before decoding,
projection and graph work. The original review is never rewritten or replaced.

The result includes every original slot's current status, unchanged original
plan identity, original and current route witnesses, and the source digests.
Exit 0 and `ok: true` mean the report was established, not that all constraints
passed. Check `result.status == "constraints_met"`; only then is `result.plan`
non-null. `result.original_plan` is historical intent even when current review
fails. Refused operations exit 2 with sanitized errors and null result/source.
An incomplete stdout write never prints a second object or repeats acquisition.

Each invocation shares the existing 1..60,000 ms deadline, 272 calls, 20 MiB
network limit and 20,000,000 work allowance. Artifact creation leaves 100,000 work
units aside before giving the remainder to allocation. Artifact loading and the
fresh review share the same budget; no per-page or per-phase renewal occurs.
The complete outer packet must fit the existing 96-KiB output ceiling. Budget or
output refusal yields no partial plan. Input files require a bounded regular
POSIX no-follow final-component open and stable descriptor/path metadata; this
is not all-parent private-file custody. No file or journal is written by the CLI.

### Integration evidence

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_route_furniture_review test_route_furniture_review_runtime -v
```

All **31 actual Python test functions pass**, with no skips: 17 core and 14 runtime
cases. Runtime cases use independently encoded, joined loopback traffic and real
CLI subprocesses. They include export/reopen/revalidation on one original endpoint,
better replacement stock that must not be substituted, a paged roster with a job
attachment at its end, changed native type/material/subtype, rejected flags,
source/nonce/tick/pause mismatch, lost or wrong release acknowledgment, capture
checksum failure, original endpoint and pre-contact budget refusal, source close
before projection, a real foreground timeout, revocation during review or final
rendering, immutable input files, sanitized CLI failures and short stdout writes.
The entire imported source closure was verified against its original Git blob
SHAs before testing; those seven existing modules remain unchanged.

The loopback peer is not DFHack. No live fortress, SDK/plugin build, Rust/MCP test,
full workspace qualification or production admission is established. This closes
the retained-selection-to-fresh-review workflow, not the separate atomic native
route/placement enforcement or durable batch-custody boundary.
