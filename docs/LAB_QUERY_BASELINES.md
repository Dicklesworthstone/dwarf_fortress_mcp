# Foreground query baselines in the laboratory

The default laboratory `fortress_query` tool can retain a small, complete
selection of the current world and later return only its selected endpoint
changes. An agent can capture citizens' labor settings before a plan, compare
them after commit, or inspect a bounded selection of jobs and furniture across
successive authorized observations. The retained handle and its next steps
remain discoverable through a session-owned catalogue.

The operations reuse the existing foreground baseline engine and its presence,
provenance, generation, pagination and atomic publication semantics. They read
the current laboratory reference world, make no native observation, and do not
advance simulation time. A baseline retains selected rows independently of the
ordinary recent-world window; it does not pin a complete world version.

## Request and workflow

Pass the following JSON **as a string in the existing `mode` argument** of the
default `fortress_query` tool. Supply the owning `session_id` separately as
usual. There is no additional top-level tool or native protocol generation.

```json
{
  "mode": "temporal",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "capture",
      "key": "labor-before-plan",
      "max_game_ticks": 100,
      "select": {
        "kind": "entities",
        "kinds": ["unit"],
        "fields": ["labor.MINING", "labor.BREW"]
      }
    }
  }
}
```

The returned `captured.baseline` is an opaque `qb1:` handle. The capture must
finish every page of the selection against the same current snapshot before
retaining anything. Fields absent from that source remain explicitly unknown;
requesting a field does not create an observation.

After an authorized plan/commit, observation or bounded simulation step, query
the current world against the baseline:

```json
{
  "mode": "temporal",
  "request": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "changes",
      "baseline": "<the complete qb1 handle>",
      "limit": 4
    }
  }
}
```

`basis` identifies the original captured anchor. `anchor`, `current_anchor`
and the Agent Turn anchor identify the exact current target. `changes` contains
whole before/after rows, while `change_count` describes the complete comparison
and `returned` describes this page. If `continuation` is non-null, copy it into
the same `changes` operation and finish the pages before changing the current
world. The token binds the baseline and complete target identity. A changed
target rejects the old token; it never combines pages from different worlds.

The lifecycle has four operations:

| Query kind | Required query fields | Result |
| --- | --- | --- |
| `capture` | `key`, `select`, `max_game_ticks` | Immutable selected-row baseline and its metadata |
| `changes` | `baseline` | Bounded endpoint differences; optional `limit` and `continuation` |
| `baselines` | None | This session's retained, expired and epoch-invalidated metadata |
| `release_baseline` | `baseline` | Explicit, repeatable release of retained rows |

For example, recover handles after losing conversational context by passing
`{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"baselines"}}}`
as the mode string. Release a handle by using
`{"kind":"release_baseline","baseline":"<the complete qb1 handle>"}` as the
query inside the same wrapper. Release changes private query storage only.

Every successful temporal Agent Turn includes the complete bounded catalogue in
`active_work.query_baselines`. Each descriptor includes its handle, key, basis,
deadline, row/byte counts, status, and typed `next_step` and `release_step`
arguments for `fortress.query`. Valid baselines suggest `changes`; expired or
epoch-invalidated baselines suggest release. Existing pending plans, action
receipts and objectives remain represented in the same packet.

## Identity and lifetime

Capture keys contain 1–64 ASCII letters, digits, dots, underscores or hyphens.
Repeating a capture reuses its handle only when the owning session, exact
anchor, normalized selection and lifetime are identical. Reusing the key with
a changed selection, anchor or lifetime returns `conflict`; the old baseline
is not replaced. A successful capture whose response was lost can therefore
be recovered by listing or an exact retry.

The deadline is `basis.game_tick + max_game_ticks`, with checked arithmetic.
The lifetime must be positive and fit the negotiated game-tick budget. The
deadline is exclusive: comparison is refused at or after it. Reading pages,
listing baselines, and ordinary tool calls do not move the basis or renew the
deadline. Queries themselves consume no game ticks.

An optional complete `request.expected_anchor` fences all four operations.
For a rolling comparison loop, finish the current changes, capture a fresh key
with the just-observed target as `expected_anchor`, and release the old handle.
An intervening world change refuses the new capture rather than silently
changing the handoff point.

Checkpoint restore creates a new observation epoch. Prior baselines remain
listed as `invalidated_epoch` so their state can be explained and explicitly
released; comparing across the restore is refused. Expired entries similarly
remain listed as `expired`. Neither condition silently reacquires selected
rows. Releasing a missing handle is successful with `released: false`.

Baselines belong to one session and remain process-local even when the world
itself uses the laboratory's durable storage. Process restart loses these
selected rows and changes the handle incarnation. Disposing the owning session
reclaims its query allocations; it does not affect another session's catalogue.
This feature adds no close-session tool, durable monitoring, condition watch,
background sampler or independent simulation loop.

The laboratory call log still records successful temporal operations. After a
successful capture, its exported transcript is marked `replayable: false` with
an explicit process-local-handle reason: a new session cannot replay the
original `qb1:` identities. Releasing the baseline does not make those earlier
calls replayable. A refused capture does not change transcript replayability.

## What an endpoint difference establishes

Rows are compared by numeric entity ID and generation, in that order:

| Change | Meaning |
| --- | --- |
| `entered_result` | This generation matches at the current endpoint only. |
| `left_result` | It matches at the baseline endpoint only. |
| `changed_in_result` | The same generation matches both endpoints, but its label or selected fact semantics differ. |

Reused IDs produce a departure for the old generation and an arrival for the
new one. Leaving a filtered selection does not establish death, deletion or
global absence. Selection predicates follow the existing three-valued query
rules; unavailable facts do not become false values or satisfy typed filters.

Presence, source-kind and epistemic-class changes are material even when a
stored value is equal. Revisions, observation ticks and source digests alone
are counted as `provenance_only_refreshes`. Unknown and redacted values remain
unavailable in before/after results. Unselected facts are outside the comparison.

The result establishes selected endpoint differences only. An entity can
change and change back between them without appearing in the result. An empty
comparison does not prove continuous stability or successful execution of a
mutation. An advanced comparison has partial temporal continuity; a comparison
against the same complete anchor is a heartbeat. Coverage remains explicit,
and `mutation_authority` is false.

## Bounds, authority and complete publication

The closed wrapper accepts `mode` and `request`. The request accepts `schema`,
optional `expected_anchor`, and `query`. Capture selections accept only the
existing `entities` kind, `kinds`, typed `where`, `fields` and `order`; capture
owns its pagination. Unknown and duplicate wrapper, envelope, operation and
selection fields are rejected from the original request bytes. Historical
`at` selectors and watch operations are outside this wrapper.

| Bound | Ceiling |
| --- | ---: |
| Original request UTF-8 bytes | 65,536 |
| Complete rows in a baseline | 256 |
| Serialized selected rows per baseline | 256 KiB |
| Baselines per session / process | 8 / 128 |
| Underlying acquisition query calls | 64 |
| Source-row visits across one acquisition | 2,000,000 |
| Change rows per page | 256, further narrowed by the session budget |
| Change continuation bytes | 128 |

Current whole-fortress Query authority is checked before source access and
again before publication. Handles, keys and continuation checksums grant no
authority. Revoked, expired, exhausted, scoped or cancelled authority is not
revived by retaining an older selection. Another session cannot list or
compare these rows; its release request cannot release them.

The complete canonical current source is admitted against the current record
and byte-work budget, even for a one-row selection. The operation shares one
foreground wall-time allowance. Source work and output allowance remain
separate: a large authorized source may yield a small selected baseline under
a smaller response cap. `source_work` reports canonical source bytes and
records, and no repeated query or temporal operation creates another source.

Before acquiring result rows, the runtime reserves the complete Agent Turn,
existing pending game work, and retained baseline catalogue. Capture and release
prepare their final encoded packet while the session is owned and before the
existing engine commits the retained-state change. The publisher checks the
exact anchor, public result fields and complete catalogue. A rendering error,
elapsed deadline, or packet too large for the negotiated byte/token budget
leaves retained state unchanged. Whole result rows and their continuation are
published together; they are not clipped after a baseline mutation commits.

## Implementation and validation scope

`lab_temporal_query.rs` validates and admits the current session operation,
projects the catalogue, and invokes the existing `query_history` transaction
boundary through a baseline-only semantic dispatcher. It does not copy the
store or comparison algorithm. `lab_temporal_response.rs` renders the complete
laboratory Agent Turn without locks, source reads or presentation-cache access.
The ordinary native semantic dispatcher is unchanged.

Helper tests exercise closed parsing, generation and presence changes,
continuation identity, exact retries, capacity, current authorization,
deadline/epoch invalidation, ownership cleanup and rejected publication.
Renderer tests exercise canonical affordances, uncertainty and preserved game
work. Public facade tests exercise capture through real plan/commit and
checkpoint/restore paths, catalogue recovery, pending-plan preservation,
session teardown, current grants, packet-budget rollback and large-source /
small-output selection. They also cover expired admission time before capture
and honest transcript replayability after allocating a process-local handle.
These tests concern the laboratory reference world;
they do not establish native or live-game qualification.

See [LAB_QUERY_COMPARISON.md](LAB_QUERY_COMPARISON.md) when both exact world
hashes are still retained and no baseline allocation is needed. The decoded
mode-string schema is
[`lab_query_baselines.schema.json`](../schemas/lab_query_baselines.schema.json).
