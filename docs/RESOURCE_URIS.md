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
| `tasks`, `tasks-{offset}` | Bounded modern task discovery |
| `task-{task_id}` | Original plan progress, cancellation drain, and final evidence |
| `task-{task_id}~evidence-{offset}` | Digest-checked UTF-8 pages of retained task evidence |
| `state` | Current immutable-world manifest, including section counts and exact-version page URIs |
| `state-{state_hash}` | Manifest of precisely that retained world version |
| `state-{state_hash}~{section}~{offset}` | Bounded canonical records from precisely that retained version |

`df://doctor/{session_id}` returns the session's diagnostic bundle under its
negotiated `doctor` capability. State pages require `observe`, as do the existing
summary, anchor, handoff and replay reads. The capabilities view describes its
own valid session and grants no additional authority. Task views retain their
task/session ownership and capability checks.

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
Out-of-range offsets are refused. Resource errors include
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
consumer path. Session `anchor`, `handoff`, and task evidence provide useful
existing views of identity and active work, but do not claim implementation of
every planned independently addressable receipt, checkpoint or cognition
resource. Knowledge and memory URIs still have no registered resource consumer.
The `.11` roadmap bead consequently remains open.

## Verification

**Execution status for this change:** the tests below are registered regression
coverage, not a claim of a passing run. The full production
`cargo check -p dfmcp-mcp --lib --locked --offline` attempt was terminated by
the shared container's out-of-memory limit while compiling the pinned
`asupersync` dependency, before project-source diagnostics. The new Rust unit
and stdio tests consequently remain unexecuted in this session. Formatting and
`git diff --check` passed; no dependency, runtime module or source was replaced
or excluded to work around the compilation limit.

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
Agent Turn identity. On a successful run these checks would establish the
laboratory consumer contract; they are not live bridge admission evidence.
