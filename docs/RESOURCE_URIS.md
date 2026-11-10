# Resource URI consumers

The laboratory `dwarf-fortress-mcp serve` runtime exposes capability-checked
resources through the same modern MCP connection as its eleven tools. This
document records the implemented consumer surface for
`dfmcp-reality-wiring-proof-4bf.11`; the broader resource roadmap remains open.
These resources describe the laboratory reference world. They do not establish
live DFHack compatibility or authorize game mutations.

## Discovery and authority

Call `resources/templates/list` to discover
`df://session/{session_id}/{view}` and `df://doctor/{session_id}`. The session
template dispatches all views: the pinned transport router cannot register
multiple templates with overlapping `df://session/` prefixes. `resources/list`
alone is therefore not an enumeration of every instantiated view.

Open a session with `fortress_open_session`, then supply its exact returned
identifier in resource URIs. Possession of a URI is not authority. State reads
resolve a currently valid session, check its durable-owner fence, and authorize
the negotiated `observe` grant before resolving any world version. Historical
reads check expiry at the **current** world tick. Revoked, expired, or narrowly
scoped grants cannot gain whole-world access through an old hash. The resource
handler runs under the same session/shared-world lock as the semantic tools.

Resource inspection does not advance objective history, record adapter
observation events, consume semantic request IDs, retain another world version,
or publish durable custody. Replay and capabilities inspection use the same
current shared-world view and durable-owner fence. State pages use one wall
deadline through projection and escaped-response size accounting. Task resources
fetch their retained candidate without world/session locks, then reauthorize
Observe and render under the current owner-fenced view. Expiry or owner
replacement during the fetch refuses publication, including candidate errors.

Session views explicitly return private `ttlMs: 0` cache hints. This overrides
the pinned transport's default one-hour legacy promotion: current aliases,
grants, active work and historical/current continuity must be checked afresh.
Immutable source records do not make the surrounding authorization and
continuity metadata immutable.

## Implemented views

| URI after `df://session/{session_id}/` | Contents |
| --- | --- |
| `summary` | Bounded current snapshot summary |
| `capabilities` | The session's negotiated grants and version record; self-description only |
| `handoff` | Resumable current packet with pending plan, open actions, obligations and resume protocol |
| `replay` | Recorded calls in a deterministic replay bundle |
| `anchor` | Current source anchor and complete version tuple |
| `entity-{id}~{generation}~{anchor_digest}` | One Query-authorized entity at the exact current complete anchor, discovered through entity query rows |
| `tasks`, `tasks-{offset}` | Bounded modern task discovery |
| `task-{task_id}` | Original plan progress, cancellation drain, and final evidence |
| `task-{task_id}~evidence-{offset}` | Digest-checked UTF-8 pages of retained task evidence |
| `state` | Current immutable-world manifest, including section counts and exact-version page URIs |
| `state-{state_hash}` | Manifest of precisely that retained world version |
| `state-{state_hash}~{section}~{offset}` | Bounded canonical records from precisely that retained version |
| `plans` | Pending and retained original prepared-plan catalog |
| `plans-{catalog_digest}~{offset}` | Continuation bound to the same complete catalog and current anchor |
| `plan-{plan_digest}~{object_digest}` | Exact original prepared-plan manifest |
| `plan-{plan_digest}~{object_digest}~steps~{offset}` | Bounded readable projections of original steps and proof requirements |
| `plan-{plan_digest}~{object_digest}~archive~{offset}` | Lossless bounded pages of the original typed plan archive |

`df://doctor/{session_id}` returns the session's diagnostic bundle under its
negotiated `doctor` capability. State pages require `observe`, as do the existing
summary, anchor, handoff and replay reads. The capabilities view describes its
own valid session and grants no additional authority. Task views retain their
task/session ownership and capability checks.

Entity references require the current anchor, matching entity generation and
current negotiated `query` authority. Retained `state` pages instead use the
source hash and `observe` authority described below; they explicitly mark
historical continuity and cannot supply fresh mutation preconditions.

## Reading an immutable world

Read `df://session/{session_id}/state` first. Its
`dfmcp.lab-state-manifest/1` result supplies the exact source hash, source anchor,
complete `anchor_v2` tuple, and these four section links:

| Section | Records | Completeness boundary |
| --- | --- | --- |
| `entities` | Entities and their typed fields/presence states | Entity record membership in the retained laboratory snapshot |
| `relations` | Directed edges, endpoints, kind and typed fields/presence | Relation record membership in that same snapshot |
| `chunks` | Chunk coordinates, dimensions, terrain runs, overlays and hashes | Retained map chunks, not unseen map coverage |
| `events` | Event identity, tick, subject, summary and fields/presence | Retained event records, not an exhaustive history of the game |

For example, a manifest can point to
`df://session/{session_id}/state-{state_hash}~entities~0`. Read that URI and
follow the returned `next` URI until it is null. Every continuation carries
the **source** hash and the next record offset. Never substitute the current
hash partway through a traversal. The projected profile has its own
`profile_digest` and `projected_anchor`; neither is a history address.

The state page schema is `dfmcp.lab-state-page/1`. Pages reuse the existing
`research-full` canonical observation projection rather than introduce a
second record renderer. Unknown, unsupported, omitted, redacted and stale
fields retain explicit presence states. A retained compatibility value in an
unknown fact is not exposed as established evidence. Binary values retain the
projection's explicit length/omission representation.

Read `complete`, `complete_domain`, `absence_proven`, `truncated` and
`observation_coverage` together. `complete: true` means this page reached the
end of the section. `complete_domain: true` means this one page covers the
entire section from offset zero. A final nonzero-offset page cannot by itself
prove full-domain absence. A zero-record complete section proves absence only
within its named retained record domain. Manifest counts are navigation
metadata, not field-completeness or live-game proofs.

Each success includes an `agent_turn` anchored to the source version. If the
world has changed, `historical` is true, the current anchor is separate, and
continuity is `stale`. The records still come from the exact requested version;
they are not fresh action preconditions. Re-read current state before planning
an effect. Active-work coverage is explicitly omitted from a state page; use
the handoff reference to inspect it. Neither an empty active-work collection
nor a state page constitutes proof that no work is pending.

## Inspecting an original prepared plan

Read `df://session/{session_id}/plans` to discover normal prepared plans,
including the current pending plan and original plans retained by the objective
book after commit. This does not require creating an MCP Task. The catalog
returns `dfmcp.lab-plan-catalog/1`, with each plan's summary, compiler source
kind, retention class, semantic digest, exact object digest, manifest URI,
`step_details` URI and direct archive URI. A retained objective whose exact
verified plan is unavailable
has `status: "exact_plan_unavailable"` and null links; the consumer does not
reconstruct a replacement from the current world.

All plan views require current, whole-fortress `observe` **and** `query`
grants. Both are checked under the authoritative session/shared-world view and
durable-owner fence before revealing plan existence. Historical plan expiry
does not block inspection, but an expired current capability does. Resources
never acquire commit authority, authorize a new pursuit, or revive old handles.

A plan manifest (`dfmcp.lab-plan-manifest/1`) reports its original IDs,
summary, terminal condition, step count, required capabilities, risk, checkpoint
requirement and expiry. Its `inspection_projection: true` fields are readable
summaries, not an exact typed reconstruction. Its source anchor remains separate
from `current_anchor`; historical reads carry stale continuity. Current goal
truth is explicitly `not_evaluated`. Active work is explicitly omitted and
points to the handoff resource. Neither a plan seal nor a completed byte download
proves a goal, action effect, lack of work, or live-game compatibility.

Follow `step_details` for readable `dfmcp.lab-plan-steps-page/1` pages. These
reuse the laboratory's existing step renderer for action summaries, dependency
and capability requirements, planned postconditions and obligation details.
Each page contains at most 25 whole rows and shrinks to fit the escaped output
budget. `offset`, `returned`, `total` and `next` describe the traversal;
`complete_projection` is true only when one page covers all rows from zero.
Every page retains both exact identities and the original anchor, even after
the current world or pending plan changes.

These rows explicitly identify their omitted contract fields, including full
typed action and predicate representations, preconditions, compensation action, obligation failure
predicate and idempotency key. Rendered predicates are sealed requirements,
and created entity IDs are predictions from the sealed steps; neither is
current dispatch evidence. The page links the exact archive for complete typed
inspection. Step pages never evaluate predicates, dispatch work or advance
semantic request IDs.

The exact prepared plan is available as the existing **DFMPAR01** archive
documented in [PLAN_ARCHIVE.md](PLAN_ARCHIVE.md). Follow the manifest's archive
link, or the catalog's direct archive link if manifest metadata exceeds the
output allowance. Each `dfmcp.lab-plan-archive-page/1` response carries:

- The original semantic `plan_digest` and exact typed `object_digest`.
- `format: "DFMPAR01"`, `encoding: "hex"`, and `data_hex`.
- An original-byte `offset`, `returned_bytes`, and `total_bytes`.
- The SHA-256 of the complete framed archive and the next exact-object URI.
- `complete` when the page reaches the object's end, and `complete_object`
  only when this one page covers the entire object from offset zero.

Concatenate decoded hex bytes at their declared offsets, verify the complete
archive SHA-256, then use the checked archive decoder with **both** externally
selected digests. The semantic plan digest alone can alias typed Other/Custom
variants; it is insufficient to select an exact object. The manifest's object
digest uses the archive codec's domain-separated identity; it differs from the
plain SHA-256 of the complete framed bytes. Pages never substitute a new pending
plan, rebase an old plan, rewrite expiry, or regenerate idempotency keys.

The archive contains every original `PreparedPlan` field, including the full
typed actions, preconditions, postconditions, compensation, dependency and
obligation contracts. `source_kind` classifies the compiler request; this
resource does not export the separate original `PlanSource` request or replace
the original-goal history in handoff. Readable manifests and step pages do not
replace this complete typed contract.

The catalog has at most 25 rows per page. Its continuation binds the entire
sorted retained catalog, session identity and current anchor. Pending
replacement, commit, collection or a world change returns `cursor_gap`; refresh
`plans` instead of joining different catalog generations. A catalog with zero
rows describes only the retained plan book, not the absence of fortress work.
Exact plan links do not pin retention: replaced uncommitted plans and collected
original objectives return `stale_anchor`. Neither failure falls back to a
similar digest or the current pending plan.

Plan inspection charges the retained-record scan plus every encoded step and
the complete archive bytes before publishing a page. Catalog discovery encodes
the available original plans to obtain their exact object identities; their
**cumulative** bytes must fit the negotiated allowance. A large catalog can
therefore return `budget_exceeded`, even when the requested output page is
small. Selected-plan reads encode only matching semantic-digest candidates.
The existing codec additionally bounds each object to 4 MiB, 256 steps, 65,536
nodes and nesting depth 64. One wall deadline covers lookup, checked encoding,
projection and escaped publication.

Archive pages begin with at most 4,096 original bytes and reduce their byte
count until the complete escaped resource response fits the session byte/token
budget and 60,000-byte transport ceiling. The shared maximum numeric JSON-RPC
ID reserve is counted once. Catalog rows, projected step rows and manifest
fields are indivisible;
a metadata or source-budget refusal does not return a partial trusted object.
All addresses require lowercase 64-digit hashes and canonical decimal offsets,
with no query strings, extra segments, signs or leading zeroes.

## Bounds, retention, and refusals

A page starts with at most 25 records and reduces its row count to fit the
minimum of the session byte budget, four bytes per output token, and a 60,000
byte resource transport ceiling. Measurement includes the escaped inner JSON,
resource wrapper, and reserved JSON-RPC envelope space, including the pinned
protocol's largest 4,096-byte exact integer request ID. The resource context
does not expose that original ID, so this allowance is conservative even for
ordinary short IDs. Very small output budgets can therefore refuse metadata
before returning a page. Rows are indivisible;
if metadata or one record cannot fit, the consumer returns `budget_exceeded`
instead of dropping fields or returning a malformed fragment. Whenever row
clipping occurs, offsets, continuations and coverage claims are recomputed.

The reused canonical projection verifies the entire source. Its total source
record count and canonical byte size must fit the session's work budget before
projection; a small requested output page does not bypass that work bound.
These source costs are reported in `source_work`.

Bounded entity, task and state consumers share the same complete numeric-ID
reserve. State pages do not add it twice. The entity boundary regression checks
refusal below the actual escaped maximum-ID response and acceptance with the
complete reserve. This regression passed on exact qualified `4ed1091`.

Resource reads do not pin history. The normal retention rules apply: recent
session versions and versions retained by active roots such as pending plans
or checkpoints. A collected or never-observed hash returns `stale_anchor`,
with the reason and an Agent Turn gap. It never resolves to a newer snapshot
or another session's private history. References may therefore expire even
while a client is traversing them. Start a fresh traversal from `state` when
that occurs; do not concatenate pages from different source hashes.

Addresses use a closed grammar: exactly 64 lowercase hexadecimal hash digits,
one of the four section names, and a canonical nonnegative decimal record
offset (no leading zeroes except `0`, signs, query strings or extra segments).
Out-of-range offsets are refused. State-resource errors include
`dfmcp.resource-error/1` in JSON-RPC error data, the semantic error code,
`effect_may_have_occurred: false`, and an Agent Turn with no claimed complete
domains. Authorization refusals do not disclose a current anchor.

## Planned resource families that remain unavailable

The family inventory in `MCP_SURFACE.md` also names fortress-scoped entities,
map chunks, events, objectives, plans, actions, obligations, checkpoints,
attention, surprises and compatibility, plus knowledge and memory resources.
Those literal URI families are not registered by the laboratory resource
router. Fortress-only URIs lack an explicit session authority binding;
registering them without that design would introduce ambient authority.

Session `state` pages now provide the canonical entity/relation/chunk/event
consumer path. Session `plans` now supplies exact original prepared-plan discovery and archive
retrieval. Session `anchor`, `handoff`, and task evidence provide useful views of
identity and active work, but do not claim implementation of
every planned independently addressable receipt, checkpoint or cognition
resource. Knowledge and memory URIs still have no registered resource consumer.
The `.11` roadmap bead consequently remains open.

## Verification

The original-plan resource slice adds focused tests for exact codec round trips,
immutable traversal, catalog gaps, current capability scopes and expiry, source
and output budgets, and inspection side effects. Execution results for this
slice are recorded separately in the current implementation status; the older
qualification receipt below does not cover these new files.

**Execution status:** exact clean `4ed1091` passed 26 resource tests and all
11 objective-lifecycle tests, including the 30 process/durable allowed/denied
resource-read cases. The five-case actual modern-handshake subprocess suite
also passed. Both mandatory scripts and all 43 qualification gates passed,
with 5255 tests in each debug/release profile. Receipt SHA-256:
`6e2da780f6e02faa460888324f0e24b01820305f894b58f5ad36ae3bf3898e0d`.
These are laboratory consumer results, not live DFHack qualification or admission.
Earlier container OOM and refused intermediate builds remain historical failed
attempts. A later documentation commit does not inherit the source receipt.

`resources::state::tests` exercises canonical grammar, source projection reuse,
every section, immutable traversal across state changes, escaped byte budgets,
unknown field handling, empty-domain proofs, retention collection, foreign
hashes, revoked/expired/narrow grants and insufficient source/output budgets.

`test_immutable_world_resource_pages_over_modern_stdio` in
`modern_handshake_golden.rs` drives the real `serve` binary through discovery,
template listing, current and retained resources, all four paginated sections,
an intervening semantic plan/commit, malformed/unknown addresses, and
capability denial. Its assertions check actual JSON-RPC response sizes,
maximum-length string/integer request IDs, private zero-TTL cache hints, and
Agent Turn identity. The passing run establishes this laboratory consumer
contract; it is not live bridge admission evidence.
