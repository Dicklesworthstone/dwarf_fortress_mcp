# Canonical World Model

## Purpose

The world model is the stable semantic boundary between a version-sensitive DFHack source and
agents that need compact, reliable state. It must support planning, verification, explanation,
replay, and compatibility without mirroring every native structure.

## Canonical anchor

```text
StateAnchor {
  fortress_id
  observation_cursor { epoch, sequence }
  game_tick
  state_hash
}
```

An anchor names one canonical state. Queries and plans either use an exact anchor or explicitly
request a refreshed one. Mutations always use an exact anchor.

## Three representations

1. **Source representation:** raw or bridge-oriented DFHack data.
2. **Canonical representation:** versioned typed facts, graph, chunks, and events.
3. **Projection representation:** bounded MCP response for one capability and token budget.

Only 2 defines the source world state. A projection can have canonical bytes of its own while
remaining bound to that separate source anchor.

## Identity

An entity key is logically:

```text
(fortress lineage, entity kind, source identity, generation)
```

Generation prevents an old reference from silently resolving to a different later object that
reused the same source ID. Labels and coordinates are attributes.

## Presence

Production schemas must encode:

```text
Known(value)
Absent
Unknown(reason)
Unsupported(manifest)
Omitted(projection)
Redacted(policy)
Stale(last_anchor)
```

The Rust `FactPresence` type implements all seven states. A legacy `Fact` without an explicit
presence marker retains its known-value meaning. `Fact::known_value()` exposes a value only for
that legacy representation or a consistent `Known(value)` record. Known null remains distinct
from `Absent`; unavailable states and inconsistent known-value representations expose no known
value. This accessor does not establish source authenticity or mutation eligibility.

## Provenance

Every fact should carry:

- source/derivation ID;
- source schema and compatibility manifest;
- observed game tick/cursor;
- source digest;
- confidence/consistency class;
- taint;
- evidence parents.

A derived fact such as “drink runway” cites quantities and its formula version.

## Graph

Entities are typed records; edges are typed relationships. Ordered maps give deterministic
canonical traversal. Edges cannot dangle in a complete snapshot.

Representative entities:

```text
unit item building job work_order stockpile zone burrow squad
military_order syndrome historical_figure civilization announcement
plan action obligation lease checkpoint evidence
```

Representative edges:

```text
located_at contained_in assigned_to member_of performs requires
produces uses blocks threatens caused_by evidenced_by reserved_by
```

## Map chunks

The map is chunked, not tile-node-expanded. A chunk has:

- coordinate and revision;
- dimensions;
- terrain RLE;
- flag bitplanes in the production schema;
- sparse overlays;
- content digest.

Plans identify exact affected masks or cuboids. The lease and plan digest cover geometry.

## Events

Events are immutable, deduplicated observations with source identity, tick, type, subjects,
fields, source text, and evidence. Repeated polling may redeliver source events; ingestion must be
idempotent.

## Revisions

Content changes advance revision. Same generation/revision with different content is a conflict.
A removal names expected generation/revision. This catches stale deltas and bridge inconsistencies.

## Canonical hashing

Canonical state uses SHA-256 over explicit framed, ordered encoding. The existing
`dfmcp-world-snapshot-v1` and `dfmcp-state-delta-v1` bytes remain unchanged. Strict decoders enforce
bounded framing, ordered maps, value nesting, complete consumption, and exact canonical
re-encoding. Hashes exclude presentation, cache state, and unordered iteration.

Profile envelopes have separate versioned encodings and digests. A projected snapshot has its
own recomputed state hash and retains the original source anchor separately; it cannot label
changed projected bytes with the source state hash.

## Deltas

A complete delta names exact base and target anchors. Applying it must reconstruct the target
hash. Partial pages cannot be applied as a complete transition. Epoch changes require a full
snapshot.

`StateDelta::from_canonical_bytes` decodes all seven existing change types using the original v1
encoding. Structural decoding and canonical re-encoding do not prove a valid transition:
`apply_delta` still requires the exact base and verifies the reconstructed target. Snapshot and
delta input decoders have a 256 MiB frame bound; profiled envelopes impose the tighter bound below.

## Completeness profiles

`CompletenessProfile`, `ProfiledSnapshot`, and `ProfiledObservationCapsule` implement typed,
immutable projections in `crates/dfmcp-world/src/completeness.rs`. Their contracts describe which
parts of the supplied source are included. Inclusion alone does not establish that an entire game
domain was observed, that an action's preconditions are known, or that a live capability is admitted.
These five profiles are separate from agent presentation profiles such as `pulse` or `briefing`.

| Profile | Included source content |
|---|---|
| `control-minimum` | All entity and relation fields and map chunks; events omitted. This conservatively retains current state without an action-field allowlist. |
| `operations` | Operational entity fields for units, items, buildings, jobs, orders, stockpiles, zones, burrows, squads, military orders, announcements, and syndromes; reference `stock_ledger` and `dig_designation` fields; retained events. |
| `spatial` | Fields for units, items, buildings, stockpiles, zones, burrows, tile features, plants, creatures, and reference dig designations; map chunks. |
| `historical` | Historical figure, civilization, and announcement fields; retained events. |
| `research-full` | All supported normalized source content, including unregistered kind names and optional fields. Its canonical snapshot bytes and hash must equal the source. |

Every profile retains entity identities and labels, relation identities and endpoints, and
fortress/control-plane fields for plans, actions, obligations, leases, checkpoints, and evidence.
Relation fields are included when both endpoint domains are included. Excluded fields become
`Omitted(profile)` with a null compatibility placeholder and no known value; included facts retain
their original presence and provenance. Profile flags distinguish excluded map chunks or events from an observed empty domain.
A historical profile includes the supplied retained window, without claiming complete history.

### Envelope identity and provenance

A `ProfiledSnapshot` binds its profile, original source anchor, source schema, nonzero source
manifest digest, projected canonical snapshot, and ordered optional extensions. The original
source anchor and projected anchor remain distinct. The envelope digest covers all these fields.
Source schema names are bounded identity claims; a manifest digest by itself is not compatibility
admission. `verify_against_source` checks the projection against an independently retained canonical
source. Decoding an envelope and checking its digest establish internal consistency, not the
authenticity of a reduced projection's source claim.

`ProfiledObservationCapsule` binds exact base and successor envelope identities around the existing
observation capsule. Construction and application require unchanged profile, schema, manifest,
and optional extensions, together with exact projected anchors and contiguous same-epoch cursors.
Decoding requires the exact profiled base and verifies the reconstructed target envelope digest.
A profile or provenance change, restore epoch, or transition the existing delta cannot represent
requires a full projection.

### Bounds and evolution

The envelope domains are `dfmcp-profiled-snapshot-v1` and `dfmcp-profiled-capsule-v1`. Encoding is
preflight-bounded before recursive output allocation or capsule hashing; decoding is bounded too.
Each envelope is limited to 16 MiB, source schema names to 256 bytes, and optional extensions to
64 entries and 128 KiB total name/payload bytes. Values retain the existing 64-level nesting bound.
Unknown optional extension payloads are opaque, retained byte-for-byte, and included in identity;
unknown profile names or envelope versions are rejected. The codecs also reject inconsistent fact
presence and invalid source/projection identity. Existing unprofiled v1 encoding is not migrated
or relabeled by these wrappers.

## Derived projections

Aggregates, search indexes, embeddings, attention scores, and summaries are derived. They retain
source anchor and can be rebuilt. They never overwrite canonical facts.

## Invariants to test first

- canonical hash independent of insertion order;
- snapshot/delta equivalence;
- stale base refusal;
- generation reuse;
- same revision/different content conflict;
- edge endpoint integrity;
- chunk coverage;
- event dedupe;
- unknown versus absent;
- full rescan versus incremental state.
