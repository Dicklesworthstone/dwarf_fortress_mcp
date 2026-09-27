# Durable exact furnishing plans in Rust

`dfmcp_adapter::furniture_batch` retains one complete furnishing request across
placement and restart. It is the Rust counterpart of the existing
[`dfmcp.furniture-plan/1` compiler](FURNITURE_PLANS.md), with a separate durable
parent format for the Rust single-placement coordinator. It does not import the
Python batch directory or rewrite existing placement journals.

## Complete intent and ordering

`FurniturePlan::decode` accepts at most 16 KiB of closed JSON with 1..32 exact
ordinary bed, chair or table selections. Step names have at most 48 ASCII
letters, digits, dots, underscores or hyphens. Items and target tiles must be
unique; dependencies must name other steps without duplicates, self references
or cycles. Unknown fields, duplicate JSON keys (including escaped aliases),
incomplete values and values outside the native bounds fail before producing a
plan. The bounded schema-specific parser adds no dependency to the adapter.

Canonical JSON sorts steps and dependency names and includes every empty `after`
array. The domain-separated SHA-256 matches `scripts/furniture_plan.py`.
Execution uses deterministic Kahn ordering, choosing the lexically first ready
name at each step. Dependencies order placement registration; they do not prove
construction completion or control the game's job scheduler.

`BatchDefinition` binds the full plan, exact `BuildBinding` (endpoint, fortress,
generation, dimensions and software identities) and original child-journal ID.
Its canonical representation is at most 17,456 bytes. A child key is
`fb-<64-character batch ID>-<step name>`, at most 116 ASCII characters. Callers
must compare the definition's complete binding with the actual opened journal;
the inventory alone does not expose endpoint or software fields.

## Progress and recovery

The audit reads every retained child against every original ordered step. It
rejects foreign keys, missing predecessors, altered selections, changed source
identity and regressed tick, sequence, building-ID or job-ID horizons. Only a
validated native `placed` record in the coordinator's synchronized `terminal`
state advances the prefix. A tracking receipt alone cannot unlock another item.

Progress always includes all original steps, their keys, native plan digests,
states and outcomes. A ready plan exposes one next step. Pending preparation or
uncertainty exposes its original recovery identity. Refusal and cancellation
halt the remaining plan. `all_placed` means all original stage-zero building/job
registrations were retained; it does not mean any building has completed or is
currently usable. A permanent stop removes the next step while preserving any
pending, refused, cancelled or already placed evidence.

`validate_next` adds exact next-selection and original-capture checks around
the existing coordinator. It is an additional constraint, never a permission
source. Current capabilities, source custody, lease, policy, fresh observation,
local review and the original native preparation connection remain the effect
boundary. Reopening stored preparation does not recreate that connection or
permit a second placement attempt.

## Private parent custody

The `DFMFBJ01` file contains an immutable complete definition, digest and end
marker. At most one `DFMFBST1` stop frame follows, binding the complete header
and original batch ID. The maximum file size is 64 KiB. Existing empty, damaged,
truncated, noncanonical or substituted files are refused without repair.

The parent uses the same safe Linux descriptor custody as the placement journal:
normalized absolute paths, exact owned `0700` parent directories, exact `0600`
regular files, no symlink traversal, exclusive file locks, identity rechecks,
file/parent synchronization and complete readback. A batch parent and its original
placement journal can be open simultaneously in the same private directory.
The existing placement header, nonce and append format remain unchanged.

Creation requires Control mode, current whole-fortress Query and Plan authority,
an explicit complete definition and an exclusively created empty file. Stop
needs Query authority in Control or Recover mode and performs no native call.
Offline mode never publishes a stop. Any failed authorized stop fences local
advancement until reopen; `durable_stopped()` distinguishes a verified permanent
stop from that fence. No failure truncates or repairs retained bytes.

## Executed scope

```sh
cargo test --offline --locked -p dfmcp-adapter --lib furniture_batch -- --test-threads=1
cargo test --offline --locked -p dfmcp-adapter --lib build_placement -- --test-threads=1
```

All 24 new core/store tests and 45 existing placement regressions passed on the
repository's exact pinned nightly. Tests use independent Python canonical
vectors, all 4,096 four-node graphs against a permutation oracle, every prefix
of a 32-step batch through the actual journal decoder, native receipt goldens,
fault-injected storage and real Linux custody. The source-bound
[execution report](evidence/furniture-batch-rust.json) identifies the tested inputs.

This evidence establishes focused Rust development behavior. It does not qualify
the whole workspace, a real DFHack plugin, a live fortress or a production
runtime. Beads `df-dfhack-bridge-plane-c-pic.4` and `.5` remain open for their
broader mutation and recovery scope.
