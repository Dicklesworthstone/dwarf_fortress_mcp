# Local qualification and release custody

GitHub-hosted Actions are not part of the release trust model. Workflow files remain useful as
portable job specifications for controlled local execution, `act`, and
`doodlestein_self_releaser`, but a badge or workflow result is not authoritative evidence.

The current release path separates four claims:

```text
tracked source qualified
≠ release server artifact qualified
≠ DFHack/native/live tuple qualified
≠ process admitted
```

A higher rung applies only to the exact bytes and identities it names.

## Source of truth

A release candidate starts from one exact Git commit and tree. For a clean passing local receipt,
the working tree must be **HEAD-equivalent**, not merely reported clean by ordinary Git status.
Qualification binds:

- the exact 40-hex commit and tree;
- the exact NUL-delimited porcelain status, including untracked files;
- every tracked regular `100644` or `100755` path in strict UTF-8 path-byte order;
- each tracked path’s Git blob object ID, working-tree mode, byte length, and SHA-256;
- executable-bit semantics on Unix;
- the exact canonical gate sequence;
- the host and toolchain identity reported by the run.

The issuer independently computes Git blob identities from working-tree bytes. Consequently an
`assume-unchanged` or `skip-worktree` index flag cannot make modified bytes HEAD-equivalent, and
`core.fileMode=false` cannot hide executable-bit drift. Empty tracked files are valid and remain in
the complete tracked-file digest inventory. Tracked symbolic links, gitlinks, traversal, and
unsupported modes fail closed.

Inherited `GIT_*` variables are removed from the issuer and verifier environment before Git
identity commands run. `GIT_OPTIONAL_LOCKS=0`, `GIT_CONFIG_NOSYSTEM=1`, and `LC_ALL=C` make the
inspection boundary more deterministic and prevent a caller-supplied `GIT_DIR` or work-tree
override from redirecting evidence.

This establishes exact tracked-source identity. It does not by itself make the build environment
hermetic: ignored build outputs and controlled-host toolchain configuration remain explicit
qualification assumptions until isolated build-root custody is implemented.

## Two-phase source snapshot

`architecture/local_qualification_receipt_v1.json` and
`scripts/write_local_qualification_receipt.py` define a two-phase source snapshot:

```text
collect exact commit, tree, status, and complete tracked inventory
→ publish source-snapshot.json create-only
→ reconstruct and compare the snapshot
→ run the canonical gates
→ reconstruct and compare before receipt publication
→ publish qualification-receipt.json create-only
→ reconstruct and compare after publication
```

Any source, mode, inventory, commit, tree, or status change during qualification prevents a clean
receipt from surviving. If a post-publication comparison fails, the invalid receipt is removed,
the parent directory is fsynced, and absence is verified.

The endpoint comparisons detect persistent drift. They do not prove that no transient modification
was introduced and reverted during a gate. Controlled-host custody remains part of the model.

## Receipt statuses

The local receipt schema is `dfmcp.qualification-receipt.v1`. Status is explicit:

| Status | Meaning | Release-admissible |
|---|---|---:|
| `passed` | Every canonical gate passed against one stable, clean, HEAD-equivalent snapshot. | Yes, as a source-qualification input only. |
| `development_dirty` | Every gate passed, but source was dirty or not HEAD-equivalent. | No. |
| `static_only` | Only a canonical passing gate prefix ran because Rust gates were explicitly unavailable or skipped. | No. |
| `failed` | Qualification ended before every canonical gate passed. | No. |

A dirty full run is downgraded automatically to `development_dirty`. It cannot emit a `passed`
receipt. Static-only execution does not append a fictitious gate outside the canonical server
contract; its shorter passing prefix and status express the limitation.

## Evidence custody

Qualification evidence uses private, create-only custody:

```text
real run directory: exact mode 0700
source snapshot:    exact mode 0600
canonical gates:    exact mode 0600
local receipt:      exact mode 0600
server receipt:     exact mode 0600
checksums:          exact mode 0600
```

When user identity is available, the run directory and evidence files must be owned by the effective
user. Final-component symbolic links are rejected. The snapshot, gate journal, and local receipt
must share one private run directory.

Receipt publication is atomic no-replace publication, not a check followed by `os.replace`:

```text
create same-directory temporary file
→ force exact mode 0600
→ write and fsync bytes
→ hard-link temporary inode to the absent destination
→ unlink temporary name
→ fsync parent directory
→ re-read and compare the published bytes
```

A destination that appears during publication wins; qualification fails without overwriting it.
This closes the check-to-publish race inherent in `exists()` followed by replacement.

## Canonical local gates

`scripts/qualify_local.sh` is the receipt-producing entrypoint. It:

1. creates a fresh exact-mode `0700` run directory;
2. captures the source snapshot before any gate runs;
3. records only the exact gate order in
   `architecture/live_server_binary_receipt_v1.json`;
4. runs repository, protocol, compatibility, custody, admission, Python, shell, Rust, and executable
   checks;
5. issues the final receipt only through the shared source-stable issuer.

The top-level non-receipt gate is:

```bash
./scripts/verify.sh
```

The source-bound receipt gate is:

```bash
./scripts/qualify_local.sh
```

Both run the machine-checked implementation-status contract and the release-source-custody contract.
The latter is defined by `architecture/release_source_custody_v1.json` and checked by
`scripts/check_release_source_custody.py` plus hostile mutation tests.

`PYTHONDONTWRITEBYTECODE=1` prevents Python imports from creating untracked bytecode while the source
snapshot is active.

## Server artifact qualification

After a clean `passed` local receipt exists, qualify the release server separately:

```bash
scripts/qualify_live_server_binary.sh \
  /absolute/private/run/qualification-receipt.json \
  /path/to/new/server-qualification-run
```

The local receipt must itself reside in a real exact-mode `0700` directory as an exact-mode `0600`
regular file. Before accepting it, the independent verifier:

- verifies receipt SHA-256 and exact schema;
- requires `status = passed`, `dirty = false`, and `head_equivalent = true`;
- checks the exact commit, tree, and source-snapshot digest fields;
- re-enumerates every current HEAD path;
- re-hashes every current working-tree file;
- compares the complete inventory exactly, including missing, extra, and changed paths;
- validates every canonical gate and its order.

The server qualifier performs that complete replay before the build, again after the build and
executable checks, again while issuing the receipt, and through the independent verifier after
publication. Source changed during qualification or server build yields no qualified server
receipt.

The server receipt additionally binds:

- the local receipt file SHA-256;
- exact source commit and platform;
- the curated load-bearing source map;
- the complete source inventory inherited from the local receipt and replayed independently;
- warning-free release executable checks;
- executable size and SHA-256;
- an empty mutation-capability set;
- explicit claims not established.

The server qualification run directory and logs are private and create-only. Its receipt uses the
same hard-link no-replace publication discipline. Failed runs remove any newly published receipt or
checksum evidence.

## Qualification ladder

### Q0 — static contracts

- machine registries parse and cross-reference;
- repository integrity and source-bundle checks pass;
- local-receipt and release-source-custody contracts pass;
- implementation-status claims match executable machine state;
- protocol, compatibility, floor, doctor, and process-admission contracts agree;
- dependency and shell/Python syntax policies pass.

### Q1 — Rust semantic gates

- `cargo metadata --locked --offline`;
- `cargo fmt --check`;
- warning-denied Clippy on workspace, all targets, and all features;
- debug and release workspace tests;
- warning-denied rustdoc;
- executable contract, doctor, deterministic demo, and probe help.

### Q2 — deterministic campaigns

- fixed-seed replay suites;
- schedule and fault corpora;
- snapshot/delta, rebase, idempotency, and cancellation campaigns;
- golden result and decision-witness comparison.

### Q3 — native and live compatibility

Against one exact Dwarf Fortress, DFHack, plugin, source, protocol, and platform tuple:

- R1 native plugin build and method inventory;
- R2 authentication and non-disclosure;
- R3 deterministic complete reads;
- R4 restart, drift, gap, and partial-publication fencing;
- R5 cold-agent semantic orientation;
- generation-specific campaigns such as protocol-1.1 A1-A6.

### Q4 — registry, floor, and process admission

- reviewed exact tuple promoted into the compatibility registry;
- owner-private monotonic floor advanced to those exact registry bytes;
- source-bound server artifact accepted;
- exact bridge protocol present in the reviewed production runner map;
- protocol-bound single-use ticket consumed by the exact executable process.

### Q5 — performance, recovery, and release assets

- same-binary A/A and A/B receipts;
- latency and memory accounting;
- crash and restart matrices;
- checkpoint and ATP reconstruction;
- long-horizon soak;
- exact archives, checksums, signatures, SBOMs, install, upgrade, rollback, and uninstall evidence.

Only earned gates appear in status. Q1 success is never live-game verification. MCP transport
conformance is also a separate rung and never substitutes for DFHack or game evidence.

## Workflow and DSR policy

`.github/workflows/*.yml` describes jobs for controlled local or self-hosted execution. Workflows
should call repository scripts rather than reproduce gate logic in YAML. DSR may stage source,
execute native target builds, retain logs, and withhold publication until all requested artifacts
succeed, but its manifest seals existing receipts; it does not rewrite them.

A strict release eventually contains one primary archive per target plus checksums, signatures when
enabled, an SPDX JSON SBOM, exact source archive, qualification receipts, compatibility evidence,
and install/rollback records. Unexpected assets cannot substitute for missing expected assets.

## Reproducibility and current limits

Bit-identical binaries across different native toolchains are a target where feasible, not assumed.
The current baseline is exact tracked-source provenance plus named host, toolchain, features, profile,
and artifact hash.

A local qualification receipt does not establish:

- a native DFHack plugin build;
- disposable-fort behavior;
- compatibility-registry admission;
- deployment-floor acceptance;
- bridge connectivity;
- a server artifact, unless the separate server qualifier passed;
- process admission;
- mutation authority;
- hostile-host or hostile-root resistance;
- signed release provenance.

The checked-in compatibility registry remains the authority for tuple admission. An empty registry
means no live tuple is admitted, regardless of how many source or server qualification gates pass.
