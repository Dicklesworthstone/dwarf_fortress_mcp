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

Only 2 is canonical.

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

The phase-zero Rust `Fact` type currently models known facts; presence algebra is the next schema
work item.

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

The phase-zero scaffold uses SHA-256 and explicit framed, ordered encoding. Production encoding
will move into a bounded schema crate. Hashes exclude presentation, cache state, and unordered
iteration.

## Deltas

A complete delta names exact base and target anchors. Applying it must reconstruct target hash.
Partial pages cannot be applied as a complete transition. Epoch changes require a full snapshot.

## Completeness profiles

Profiles describe what a canonical snapshot promises:

- `control-minimum`;
- `operations`;
- `spatial`;
- `historical`;
- `research-full`.

A client projection can omit fields while still referencing a fuller canonical profile.

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
