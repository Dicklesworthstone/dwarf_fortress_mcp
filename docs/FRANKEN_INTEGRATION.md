# Franken Substrate Integration

The detailed repository-by-repository analysis is in [`../FRANKENSTACK_DEEP_DIVE.md`](../FRANKENSTACK_DEEP_DIVE.md). The machine-readable import ledger is [`../architecture/franken_imports.json`](../architecture/franken_imports.json).

## Doctrine

A sibling mechanism is admitted only when it has:

- a named semantic owner;
- a deterministic reference implementation;
- an invariant and explicit failure boundary;
- cancellation, recovery, and replay behavior;
- a replacement prohibition explaining what weaker shortcut is forbidden;
- differential and fault evidence;
- a measured reason to enter the production path.

Owned code is not automatically trusted. Integration is an earned status.

## Composition

```text
asupersync
  regions · Cx authority/budgets · cancellation · lab · ATP
       │
       ▼
authoritative capsule/MVCC/effect/evidence plane
  ├── FrankenSQLite doctrine: versions, witnesses, rebase, commit combining
  ├── FrankenFS doctrine: custody, root-last publication, repair, A/B receipts
  ├── FrankenGraphDB doctrine: one version universe, tiered graph, factorization
  ├── FrankenNetworkX doctrine: algorithms, canonical tie-breaks, witnesses
  ├── FrankenSearch doctrine: immutable generations, progressive cognition
  └── FrankenMarkdown doctrine: exact spans, bounded protocol, sibling publish
```

## `asupersync`

It is the exclusive runtime, not an optional accelerator. Every task is region-owned; every blocking/effectful operation carries `Cx`; authority and budgets narrow monotonically; cancellation drains with progress evidence; the same code runs under deterministic virtual time. ATP moves immutable verified object graphs, never live mutation authority.

## FrankenSQLite

The project imports semantic MVCC rather than merely SQL storage. World reads pin immutable anchors. Plans carry positive, negative, aggregate, spatial, relation, and epoch witnesses. Coarse witnesses are no-false-negative; fine refinement only improves concurrency. Rebase recompiles intent, and accepted merges carry canonical proof certificates. Commit combining is brief and deterministic.

## FrankenFS

Checkpoint, evidence, compatibility, graph, and search generations use staged root-last publication. Ownership has incarnation/generation fences. Repair plans are sealed to the inspected root and revalidated. Performance experiments use same-binary A/A and A/B receipts. Retained roots support corruption localization and optional retrievability auditing.

## FrankenSearch

One request pins one immutable generation. Retrieval progresses from exact filters through lexical, graph, structured, and optional semantic refinement under budgets. Results retain score components, source spans, freshness, and completeness. Adaptation is clamped and cannot weaken hard safety.

## FrankenMarkdown

Knowledge preserves exact bytes, spans, transformation lineage, and taint. Parser and protocol structures are nonrecursive/bounded. Human report, citation map, index, and diagnostics publish coherently. A direct local MCP implementation avoids a general web stack and second runtime.

## FrankenGraphDB

Observation capsules form one version universe for history, projections, subscriptions, branches, and replicas. Graph adjacency evolves from simple ordered reference storage toward inline micro-adjacency, sorted deltas, and sealed runs. Factorized/worst-case-aware operators avoid explosive path materialization. Branches are counterfactual planning views that emit live intents; they never merge fabricated state into the game.

## FrankenNetworkX

Planning-relevant graph algorithms are imported or reimplemented without Python/PyO3. Every non-unique result has a closed tie-break policy. Decision paths and observed operation counts are witnessed. Immutable snapshot views share generation-owned storage. Differential behavior, ordering, and failure semantics are contracts.

## Admission matrix

The architecture registry names the required evidence for each imported primitive. No adapter can change public MCP semantics, bypass authoritative roots, create background work outside the runtime, or turn derived cognition into mutation authority.
