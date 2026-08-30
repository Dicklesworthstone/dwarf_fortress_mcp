# Local Qualification and Release with `doodlestein_self_releaser`

GitHub-hosted Actions are not part of the release trust model. Workflow files are retained because `dsr` and `act` can execute them locally and because they provide a portable description of jobs. The authoritative evidence is produced on controlled local machines.

## 1. Source of truth

The release candidate is identified by:

- repository URL;
- clean Git commit;
- annotated version tag when releasing;
- `Cargo.lock` digest;
- exact nightly toolchain identity;
- exact sibling-crate revisions;
- qualification-policy version;
- target triple and host identity.

A build from an uncommitted worktree is development output, not a release candidate.

## 2. Qualification ladder

### Q0 — static contracts

- machine registries parse and cross-reference;
- schemas and examples validate;
- Markdown links resolve;
- protocol and invariant registries agree;
- dependency policy passes;
- shell syntax passes;
- prohibited Rust constructs scan passes.

### Q1 — Rust semantic gates

- `cargo metadata --locked --offline`;
- `cargo fmt --check`;
- Clippy on workspace/all targets/all features with warnings denied;
- tests on workspace/all targets/all features;
- warning-free rustdoc;
- executable contract, doctor, and deterministic demo.

### Q2 — deterministic campaigns

- fixed-seed replay suites;
- schedule/fault corpus through the lab runtime;
- snapshot/delta, rebase, idempotency, and cancellation campaigns;
- golden result and decision-witness comparison.

### Q3 — platform qualification

Run the same clean source on the supported Linux, macOS, and Windows hosts. Each host emits a target receipt. Cross-platform semantic digests must agree where platform-independent; expected binary differences are named.

### Q4 — bridge compatibility

Against each supported DF/DFHack pair:

- handshake and capability probe;
- read-only differential snapshots;
- malformed/oversized bridge messages;
- disconnect/restart/reconnect;
- effect-family live tests in disposable saves;
- checkpoint/restore and stale-epoch invalidation.

### Q5 — performance and recovery

- same-binary A/A and A/B receipts;
- latency distributions and memory accounting;
- crash matrix;
- checkpoint and ATP reconstruction;
- long-horizon soak with active compaction/indexing.

Only earned gates appear in the release status. Q1 success must never be phrased as live-game verification.

Protocol conformance is a separate, transport-independent rung (WP-21): the MCP 2026-07-28
conformance suite runs against the pinned `fastmcp_rust` revision on every pin bump, and results
are recorded in `docs/DOGFOODING_FASTMCP.md`. MCP conformance evidence never implies live-game
verification.

## 3. Qualification receipt

`scripts/qualify_local.sh` writes a machine-readable receipt containing:

```text
source commit and dirty state
toolchain identity
host and target
policy version
commands and exit states
start/end timestamps
Cargo.lock and registry digests
binary and documentation digests
skipped gates with explicit reasons
```

Receipts are immutable evidence inputs. The release manifest seals them; it does not edit them.

## 4. Workflow policy

`.github/workflows/*.yml` is labeled as a **local execution specification**. It may be run by `act` or DSR on controlled hosts. Repository policy does not require push or pull-request triggers. No badge is treated as evidence.

The workflow should call repository scripts rather than reimplement qualification logic in YAML. This keeps local shell execution, `act`, and native-host execution equivalent.

## 5. DSR repository configuration

`release/dsr/dwarf_fortress_mcp.yaml.example` is copied into the operator’s DSR `repos.d` directory and adjusted only for local paths/hosts and final asset keys. It defines:

- Rust language and binary;
- supported targets and native hosts;
- release workflow path;
- pre-build qualification command;
- exact archive naming;
- required companion files;
- checksum/signature/SBOM contract;
- sibling-crate revisions when integrations become live.

DSR snapshots to disk-backed staging, builds targets in isolation, retains per-attempt logs, and withholds the authoritative manifest until all requested targets succeed.

## 6. Exact asset contract

A strict release contains one primary archive per target plus:

- `.sha256` sidecar for each primary;
- `.minisig` sidecar when signing is enabled;
- SPDX JSON SBOM;
- `SHA256SUMS.txt`;
- source archive;
- qualification manifest and receipts;
- compatibility matrix;
- public signing key pinned by the tag, when used.

Unexpected assets do not satisfy missing expected assets. Symlinks, path traversal, name collisions, and untracked companion files fail packaging.

## 7. Reproducibility

Bit-identical binaries across different native toolchains are a target where feasible, not assumed. The required baseline is **semantic reproducibility** and exact provenance:

- same source and lockfile;
- exact toolchain identity;
- same feature set and build profile;
- deterministic generated sources;
- declared environment inputs;
- artifact hash per host;
- executable self-report of build identity.

A reproducibility campaign rebuilds on a second host of the same target and classifies any difference before publication.

## 8. Release admission

Release is fail-closed when:

- the worktree or sibling crate is dirty;
- lockfile or dependency graph differs from preflight;
- a target receipt is missing;
- source/tag identity changes between build and upload;
- signatures or checksums fail;
- exact assets differ from contract;
- a required gate is skipped;
- compatibility evidence is stale;
- the release tag policy cannot be proven.

## 9. No hidden remote dependency

Local qualification must work without GitHub-hosted runners. Network access may be required to fetch already pinned source or publish a release, but compilation and testing use the retained source snapshot and locked dependencies. `cargo metadata --locked --offline` is the dependency-resolution check.
