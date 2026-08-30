# Contributing

`dwarf_fortress_mcp` is design-first but evidence-driven. A proposal must identify the invariant
it serves, failure modes it introduces, evidence that could falsify it, and the acceptance gate
that closes the work.

## Before proposing a change

1. Read `AGENTS.md`, `IMPLEMENTATION_STATUS.md`, `FRANKENSTACK_DEEP_DIVE.md`, and the relevant
   sections of the comprehensive plan.
2. Identify affected stable IDs for invariants, errors, capabilities, effects, schemas,
   publication primitives, graph algorithms, work packages, and tests.
3. Update the relevant machine registry under `architecture/` or `design/registries/`.
4. Add deterministic tests for success, retry, cancellation, stale and negative witnesses,
   cursor/epoch discontinuity, partial publication, indeterminate effects, and recovery where
   applicable.
5. Run `./scripts/qualify_local.sh` on the latest nightly toolchain.

## Dependency changes

The dependency universe is closed. A new crate is not justified by convenience or popularity.
Any proposed dependency must either be an owned `asupersync`/Franken-suite crate, the owned
`fastmcp_rust` MCP plane within its admitted modern-only profile (ADR-013), or pass a separate
architecture decision that proves it is fundamental, deterministic under the lab, compatible with
safe Rust, and materially better than an owned implementation. Update
`architecture/dependency_allowlist.toml` and its evidence, not merely `Cargo.toml`.

## Design changes

Protocol and semantic changes require an ADR in `docs/DECISION_LOG.md` covering alternatives,
affected invariants, migration, security, authority, witness semantics, determinism, performance,
token economics, recovery, and evidence required to reverse the decision.

## Pull-request standard

Leave the tree in a coherent, locally qualifiable state. Do not merge placeholder implementations
that fabricate success. Unsupported behavior fails explicitly with a stable error. Do not add
ambient filesystem/network authority, native FFI, arbitrary DFHack command execution, detached
background work, hidden network downloads, or raw-byte merge of structured state.

GitHub workflow YAML is a portable local-build specification, not proof from GitHub-hosted
runners. Release evidence must come from the repository's local qualification and DSR path.
