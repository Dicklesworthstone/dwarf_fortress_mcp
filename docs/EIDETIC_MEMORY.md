# Agent Memory Layer — Eidetic Engine (`ee`)

**Status:** adopted as the recommended campaign-memory layer for fortress stewardship
(2026-08-30). `ee` is the owned sibling
[`eidetic_engine_cli`](https://github.com/Dicklesworthstone/eidetic_engine_cli): a durable,
local-first, explainable memory CLI for coding agents (Rust 2024, asupersync runtime, same
closed-stack discipline as the rest of the Franken suite).

## 1. The one constitutional rule

The canonical world model is deliberately distinct from **agent memory, which may be stale or
speculative** (README, "Canonical world model"). Eidetic Engine implements that memory layer, and
this project adopts it under exactly one inviolable rule:

> Memory is advisory context for agents. It is never canonical state, never authority, never a
> substitute for `fortress.observe`. When memory and a live observation disagree, the observation
> wins — always.

Everything else follows: memory entries reference canonical reality (anchors, evidence digests)
as *provenance pointers*, never as state themselves. A memory claiming "the aquifer is at z=-12"
is a hypothesis to re-verify, not a fact.

## 2. Why the fit is exact

| dfmcp doctrine | ee doctrine |
|---|---|
| Derived cognition is never authoritative; rebuild from capsules | "Search indexes are derived assets" |
| Evidence-backed completion; receipts before success | "Evidence before promotion" |
| No silent state mutation; indeterminate is first-class | "No silent memory mutation" |
| Deterministic replay by default | "Deterministic by default" |
| Explainable decisions (fortress.explain) | "Explainable retrieval" (`ee why`) |

The two systems also hand off cleanly: this project *produces* exactly the artifacts `ee` wants
as curated input — doctor bundles, obligation outcomes with terminal-predicate proofs,
qualification receipts, lab transcripts, fault-campaign reports.

## 3. Campaign workflow

One `ee` workspace per fortress campaign (`--workspace` scoping keeps campaigns isolated; `ee
team` sharing is gated on WP-15 multi-agent leases — see bead
`dfmcp-ee-team-memory-policy`).

**Session start**
```bash
ee resume --workspace forts/7-brokenfurnace --json        # "where was I?"
ee pack "continue the magma forge project" --workspace forts/7-brokenfurnace \
    --read-only --max-tokens 4000 --format markdown
```

**During a session**
- Provenance rule: when recording anything about fort state, include the dfmcp anchor
  (`epoch`, `sequence`, `state_hash`) and, where applicable, the evidence bundle digest — the
  memory points at canonical evidence; it does not restate it as truth.
- Risky-operation hesitation: `ee preflight check --cmd "<plated bridge designation>"` for
  advisory "this went badly last time" lookups. Advisory only; never a gate (that is what
  capabilities, leases, and plan sealing are for).

**Session end**
```bash
ee remember "Digging the aquifer pocket at z=-12 without a plated bridge flooded the
workshop level; see evidence bundle eb:01J… and anchor epoch 3 seq 18422." \
  --workspace forts/7-brokenfurnace --level procedural --kind anti_pattern --json
ee remember "Fort convention: food Overhaul order stays below 30 units until the
second harvest." --workspace forts/7-brokenfurnace --level procedural --kind rule --json
```
Batch curation from structured artifacts (doctor bundles, obligation outcomes) via
`ee remember --batch --stdin`; revive-with conditions (`--revive-when path_exists:…`) map neatly
onto "retry this idea when X exists" lessons (e.g., revive when a magma-safe material reserve
exists).

**Retrieval with provenance**
```bash
ee search "has anyone bridged the aquifer before" --workspace forts/7-brokenfurnace \
    --limit 20 --explain --json
ee why <memory-id> --workspace forts/7-brokenfurnace --json
```

## 4. Layering

```text
agent harness (Claude Code, Codex, …)
    ├── dfmcp plane: fortress.observe / plan / commit / wait / explain   ← canonical, authoritative
    └── ee plane: pack / resume / remember / search / preflight          ← advisory campaign memory
              └── provenance pointers INTO dfmcp anchors + evidence digests (one-way)
```

`ee` is **not** a dependency of the dfmcp Rust workspace and must never become one: the closed
universe governs the server's trust domain, and campaign memory lives with the agent harness,
outside it. The integration is operational (this doctrine + workflow) plus, later, export
templates from dfmcp artifacts into `ee remember --batch` form (see bead
`dfmcp-ee-evidence-curation`).

## 5. Anti-patterns (each one is a review failure)

- Treating a memory as state: acting on "the bridge is built" without `fortress.observe`.
- Letting memory expand authority: "the last agent had restore rights" grants nothing.
- Canonical leakage: never write anchors/state hashes INTO the canonical store from memory;
  memory cites evidence, evidence never cites memory.
- Skipping verification because "memory says it worked last time" — that is what
  postcondition proofs are for.
- Per-agent private memory used for coordination (that is WP-15 leases + `ee team`, only after
  multi-agent authority lands).

## 6. Rollback

This layer is documentation and workflow; deleting `docs/EIDETIC_MEMORY.md` and the associated
beads fully reverts it. No code, schema, or dependency surface changes.
