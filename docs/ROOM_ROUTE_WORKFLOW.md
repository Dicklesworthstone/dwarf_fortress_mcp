# Create and recheck complete room-backed route plans

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` and
`dfmcp-wp-pln01-spatial-blueprint-layout-pwd` (remain open).

`review_room_routes.py` joins the original room compiler, fixed-order route
allocator, retained route review and combined room/item/delivery condition.
It exposes four operations without changing the existing native clients, room
or route child codecs, placement journals, batch formats or other commands.
The complete room plan stays attached rather than lowering silently to furniture.

## Create from one map/inventory acquisition

Supply an existing canonical RoomPlan and its independently selected digest:

```sh
python3 scripts/review_room_routes.py create \
  --room-plan-file room-plan.json --expected-room-plan ORIGINAL_ROOM_DIGEST \
  --origin 0 0 5 --size 64 64 1 --max-route-steps 1000 \
  --timeout-ms 10000 > room-route-report.json
```

Paths, digest and coordinates are illustrative. The region must include every
original floor, doorway, corridor and required bedroom wall as well as candidate
item routes. All room geometry is validated before native acquisition; capturing
only the furniture targets is rejected. Exact source identity and map dimensions
remain checked after acquisition.

Use the existing isolated read-only environment:

```text
DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE=1
DFMCP_ROUTE_FURNITURE_ENDPOINT=127.0.0.1:5000
DFMCP_MAP_TOKEN=<operator-configured map read token>
DFMCP_OPERATIONS_PAGED_TOKEN=<operator-configured inventory read token>
```

The default acquisition adapter delegates to the unchanged
`route_furniture_review_runtime.acquire`: two equal paused map/1.5 endpoints
bracket one completely decoded, released operations/1.4 roster on one connection.
That connection closes before allocation or checking. There is no new native
method, credential, protocol selector, retry or automatic game effect. Equality
of paused endpoints is not an atomic cross-profile snapshot.

Creation uses the existing polynomial fixed-order allocator. It chooses new
items but preserves the room's EXACT furnishing request and dependencies. A
single structural allowance covers allocation, review sealing, combined-handoff
validation, raw room-terrain assessment and independent delivery replay; each
phase receives only its predecessor's remaining allowance. Artifact export has
a separate reservation that the solver cannot borrow. Authority and wall time
are rechecked after serialization and immediately before the final output attempt.

## Interpret the whole result

Exit zero / `ok: true` means a report, not readiness. Use `result.status` and
`result.plan`, not the nested allocation's successful item selection alone.
The combined status is ready only when the room terrain, all original selections
and the entire original delivery order satisfy their captured-model conditions.
Known contradiction is blocked; missing evidence is unestablished.

A complete allocation may emit a historical `handoff` even when the room is
blocked or unknown. This permits later checks without reselecting items. Its
presence is NOT a ready or executable-plan assertion. Shortage emits no handoff,
no partial review and no top-level plan. Reports always retain complete room
intent, including in shortage cases. Historical nested plan fields retain their
own narrower scope; only the combined top-level plan is readiness-conditioned.

To export canonical artifact bytes directly, add `--emit handoff`. For CREATE,
this narrow export refuses unless the whole combined condition is ready. Artifact
exports have no trailing newline; reports/error objects end in one newline.
Retain the report's `result.handoff_digest` independently. The digest is not a
signature, custody proof, observation issuer or authority grant.

## Check original intent, without reallocation

```sh
python3 scripts/review_room_routes.py check \
  --handoff-file room-route.json --expected-handoff ORIGINAL_HANDOFF_DIGEST \
  --timeout-ms 10000
```

The complete artifact determines the request, original items, route limit and
region. Creation overrides are not accepted by check. Reacquisition uses the
original source binding; a changed endpoint is refused before contact. The
checker never calls an allocator, strengthens dependencies, replaces a missing
item, or renews a constraint. Each current map must cover all original geometry.
The report returns the existing handoff identity rather than a replacement artifact.

## Offline bind and inspect

Existing canonical room/route artifacts can be paired without network access:

```sh
python3 scripts/review_room_routes.py bind \
  --room-plan-file room-plan.json --expected-room-plan ORIGINAL_ROOM_DIGEST \
  --review-file route-review.json --expected-review ORIGINAL_ROUTE_REVIEW_DIGEST \
  --emit handoff > room-route.json

python3 scripts/review_room_routes.py inspect \
  --handoff-file room-route.json --expected-handoff ORIGINAL_HANDOFF_DIGEST
```

Both child codecs are fully revalidated. A sequencer-strengthened request that
no longer equals its original room request is rejected. Inspect and bind remain
historical intent only and need no credentials or native module imports. Their
`--emit handoff` explicitly exports historical canonical bytes, not fresh readiness.

Inputs are bounded regular files opened without following the final symlink;
file/path identities and metadata must remain stable during reading. This is
ordinary input stability, NOT all-parent private-journal custody. No files are
created or replaced by the program. Shell output redirection above is explicit
operator file handling. The command emits one complete response of at most
256 KiB, never silently omits rooms to fit, and never emits a second object after
a short stdout write. Refusal exits two, omits cached intent/source facts and
sanitizes errors. There are no background tasks.

## Executed evidence and remaining boundaries

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error -m unittest \
  test_room_route_handoff test_room_route_workflow -v
```

33 actual Python tests pass, zero skips: 15 core and 18 workflow/CLI tests. They
execute the complete original room compiler, route solver, fixed-order matching,
review codecs, full delivery replay, new workflow, file readers and CLI main.
All 12 reused pure modules match upstream Git blobs and remain unchanged.
Real subprocess bind/inspect execution also installs an audit hook rejecting
sockets and child-process creation after startup. Cases cover 32 furnishings,
rotated geometry, blocked room despite allocated items, missing original item,
precontact scope/option rejection, shortage without partial artifacts, output
and work bounds, failed reads, source substitution and late authority loss. Four deliberately weakened
implementations fail regression assertions: ignoring parent-request equality,
ignoring room-terrain readiness, ignoring complete sequential delivery, and
omitting authorization after serialization.

Live create/check tests inject the acquisition port and use test authority and
foreground-budget objects. They do NOT exercise the default native imports,
network budget, full operations projection, native TCP/paging/release or a real
DFHack process. The delegation's source contract was inspected, not qualified
as a complete real-network command pipeline here. Basic geometry and rotations
are exercised; cost-aware corridor fallback combinations are not newly qualified.

This is an unstarted-plan read workflow, not durable room-backed route-batch
custody or enforcement inside native preparation/commit. The existing route
batch cannot implicitly adopt this new parent artifact. No placed-prefix
reauthentication, room assignment, completed-construction proof, clock effect,
current usability, Rust/MCP, native SDK/plugin, full-workspace or production
admission is established. Those broader integration beads remain open.
