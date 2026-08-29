# Closed Dependency Universe

`dwarf_fortress_mcp` is a safe-Rust, latest-nightly project. Dependency minimization is not aesthetic; it is required for deterministic scheduling, auditable semantics, constrained supply-chain risk, and fleet-wide reuse.

## 1. Default rule

A crate is forbidden unless it belongs to one of these classes:

1. `core`, `alloc`, or `std`;
2. an owned `asupersync` crate;
3. an owned Franken-suite crate with an admitted semantic contract;
4. a tiny fundamental serialization/data-format crate explicitly listed in the allowlist;
5. a build-only tool whose output is reproducible, checked in where appropriate, and absent from the runtime trust domain.

The current phase-zero workspace intentionally has only path dependencies among its own crates.

## 2. Fundamental exceptions

The initial prospective exceptions are `serde` and `serde_json`, with default features minimized. They are not automatically added; the direct MCP/protocol implementation must first demonstrate that using them is preferable to fleet-owned equivalents. Any additional exception requires an ADR containing:

- exact need and rejected owned alternatives;
- feature-level dependency graph;
- runtime threads, I/O, allocation, and nondeterminism behavior;
- unsafe-code and build-script audit;
- canonical-format implications;
- deterministic-lab implications;
- removal plan if the Franken equivalent becomes available.

## 3. Explicitly prohibited convenience dependencies

Without a constitutional amendment, the runtime may not depend on:

- Tokio, async-std, smol, Rayon, crossbeam executors, or hidden thread pools;
- petgraph, graph-tool bindings, NetworkX/Python, or opaque graph engines;
- rusqlite, sqlx, Diesel, RocksDB, LMDB, SQLite C FFI, or external databases;
- reqwest, hyper, axum, tonic, tower, gRPC frameworks, or general web stacks;
- prost/codegen as an excuse for an unbounded wire surface;
- Tantivy, Lucene, HNSW libraries, vector databases, or external search services;
- mmap wrappers or native libraries that introduce unsafe/FFI into the core process;
- dynamic plugin loading;
- system OpenSSL or other C cryptography bindings.

A prohibited dependency may appear in an offline differential test tool only if isolated from shipping crates and recorded in the test-tool ledger.

## 4. Crate layering

The target dependency direction is:

```text
dfmcp-types
  ↓
dfmcp-world + dfmcp-protocol + dfmcp-evidence
  ↓
dfmcp-query + dfmcp-graph + dfmcp-intent
  ↓
dfmcp-ledger + dfmcp-transfer + dfmcp-bridge
  ↓
dfmcp-runtime + dfmcp-policy
  ↓
dwarf-fortress-mcp
  ↓
dfmcp-lab / conformance / bench (test-only edges inward)
```

The pure semantic core does not depend on filesystem, sockets, clocks, environment variables, process globals, or the production runtime. Adapter crates depend inward through traits. Test and benchmark crates may depend broadly; production crates may not depend on them.

## 5. `asupersync` integration

When admitted, `asupersync` replaces custom runtime scaffolding rather than coexisting with it. All blocking/effectful APIs accept `&Cx` or a project wrapper that preserves authority and budget semantics. There is one timer system, one cancellation tree, one task ownership model, and one deterministic lab.

## 6. Franken-suite integration

A sibling dependency is not admitted merely because it is owned. The adapter must pin:

- source revision;
- exact crates and features;
- semantic contract version;
- failure and cancellation behavior;
- persistent format compatibility;
- deterministic reference tests;
- performance acceptance evidence.

`doodlestein_self_releaser` strict releases should pin clean sibling revisions in the DSR repository configuration and release manifest.

## 7. Wire protocols and DFHack

The Rust server remains free of C/C++ FFI. Dwarf Fortress integration is out of process through a bounded, versioned bridge. The preferred initial transport is authenticated loopback with a fixed framed protocol and operation lookup. A minimal Rust codec is implemented for the exact supported schema or uses an approved owned codec. The bridge may include an unavoidable DFHack-side Lua component, but it is treated as an external adapter artifact, not linked into the Rust trust domain.

Arbitrary Lua, shell, or DFHack command execution is never exposed through MCP.

## 8. Unsafe code

Workspace policy is `unsafe_code = "forbid"`. A future optimization that truly requires unsafe code must live outside the default trusted workspace, carry a ledger row, have a bit-identical safe implementation, and remain default-off until independent evidence shows a material benefit. The project’s baseline does not depend on such an island.

## 9. Nightly policy

`rust-toolchain.toml` tracks `nightly` with required components. Release receipts record the exact resolved toolchain commit and `rustc -vV`. Nightly movement is qualified locally before release. Reproducibility comes from receipts and retained toolchain identity, not pretending the moving channel is a fixed version.

## 10. Enforcement

Repository validation checks:

- every production dependency against `architecture/dependency_allowlist.toml`;
- absence of prohibited crate names;
- local path integrity;
- workspace crate direction;
- `#![forbid(unsafe_code)]` in roots;
- no `unwrap`, `expect`, panic, todo, or unimplemented in shipping Rust;
- toolchain and edition policy;
- lockfile presence for release qualification.

Changing the allowlist requires a reviewed ADR and a machine-readable registry update in the same commit.
