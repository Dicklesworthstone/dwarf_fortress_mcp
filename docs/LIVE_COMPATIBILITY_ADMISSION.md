# Live compatibility and process admission

A successful RPC, Rust unit test, native plugin build, disposable-fort campaign, source
qualification receipt, server-binary receipt, registry mutation, and admitted process launch are
different kinds of evidence. No rung substitutes for another.

## Current operational status

The checked-in registry remains at the stable repository path:

```text
architecture/live_compatibility_registry_v1.json
```

The path is retained for operational compatibility, while the payload schema is now
`dfmcp.live-compatibility-registry/2`. The checked-in object has status
`no_admitted_live_tuples`, zero historical entries, zero active entries, and zero revocations.
Protocol 1.0 therefore has no currently admitted tuple. Protocol 1.1 remains an implemented,
explicitly unadmitted development generation and cannot consume production admission state.

## Evidence ladder

One exact read-only tuple requires:

```text
clean dwarf_fortress_mcp source revision
  + exact DFHack source revision
  + exact native plugin binary digest
  + exact bridge protocol and implementation version
  + R1 native build and method inventory
  + R2 authentication and non-disclosure matrix
  + R3 deterministic complete-read matrix
  + R4 restart, drift, gap, and partial-publication fencing
  + R5 cold-agent semantic orientation
  + generation-specific gates such as protocol-1.1 A1-A6
  → one reviewed content-addressed registry entry
```

Starting a production process adds separate custody and executable proof:

```text
reviewed registry generation
  + owner-only monotonic floor matching those exact registry bytes
  + exact deployment manifest
  + required compatibility entry ID
  + revocation-aware compatibility decision
  + exact bridge protocol selected by that decision
  + passing local qualification receipt
  + source-bound server binary receipt
  + opened executable device, inode, size, mode, owner, and SHA-256
  + sanitized dynamic loader environment
  + protocol-bound V2 single-use admission ticket
  → descriptor-only execution of the exact admitted runtime
```

Compatibility evidence, registry policy, local anti-rollback custody, artifact qualification,
authority-free diagnosis, and runtime admission remain separate.

## Exact registry identity

A historical entry binds game and DFHack versions, bridge implementation and protocol, operating
system and architecture, exact dfmcp and DFHack commits, native plugin SHA-256, R1 and live receipt
identities, capabilities, coverage, omissions, limitations, and an evidence locator. Changing any
component creates a different entry ID.

The registry is append-only at the semantic level. Historical entries remain present byte-for-byte
through their content-addressed identifiers. Active entries are historical entries without a
revocation. At most one active entry may exist for one exact deployment tuple.

The resolver binds the entire canonical registry digest and the explicitly required entry ID. Its
V2 decision reports:

```text
matching_entry_ids
matching_revocations
registry_historical_entry_count
registry_active_entry_count
registry_revocation_count
registry_revocations_digest
registry_digest
```

A required revoked entry must fail closed. It cannot fall through to a newer active replacement for
the same tuple because the caller fenced a different evidence identity.

## Evidence-bearing revocation

Silent deletion is not revocation. Registry V2 supports a separate, content-addressed,
**evidence-bearing revocation** record containing:

```text
revocation_id
entry_id
scope = runtime_admission
reason_code
bounded human reason
evidence locator + SHA-256
canonical limitations
```

Supported reason codes are:

```text
compatibility_regression
evidence_invalidated
operational_withdrawal
security_incident
```

A revocation preserves the historical entry and every prior decision identity. It denies future process admission for that exact entry. It does not delete evidence, prove a broader compromise,
grant mutation authority, or terminate a process that already consumed a valid single-use ticket.
Operators must separately stop and replace already-running processes when policy requires it.

Create a proposed revoked registry generation:

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --revoke-entry-id <64-hex-entry-id> \
  --reason-code security_incident \
  --reason 'Exact bounded explanation of the withdrawal decision.' \
  --evidence-locator qualification/revocation/<case>/report.json \
  --evidence-sha256 <64-hex-evidence-digest> \
  --output /tmp/live_compatibility_registry_v2.json
```

After independent review, authoritative in-place revocation requires the current registry file
digest and the same single-writer lock used by promotion:

```bash
REGISTRY_SHA256="$(sha256sum architecture/live_compatibility_registry_v1.json | awk '{print $1}')"
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --revoke-entry-id <64-hex-entry-id> \
  --reason-code compatibility_regression \
  --reason 'Exact bounded explanation of the failed regression.' \
  --evidence-locator qualification/revocation/<case>/report.json \
  --evidence-sha256 <64-hex-evidence-digest> \
  --expected-registry-sha256 "$REGISTRY_SHA256" \
  --in-place
```

Revocation rejects absent entries, duplicate revocations, unsupported reason codes, malformed or
traversing evidence locators, malformed evidence digests, stale compare-and-swap state, rewritten
historical entries, and noncanonical ordering.

A fresh requalification of the same deployment tuple is allowed only after the old entry is
revoked, and only when new evidence produces a different content-addressed entry ID. The old
revocation remains in history.

## Promotion

Promotion remains deterministic and evidence-bound:

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /path/to/live-read-acceptance-receipt.json \
  --native-receipt /path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --output /tmp/live_compatibility_registry_v2.json
```

In-place promotion likewise requires `--expected-registry-sha256`. Promotion rejects dirty,
synthetic, malformed, digest-inconsistent, incomplete, reordered, mutation-bearing, duplicate,
traversal-bearing, or stale evidence. An active exact tuple cannot be duplicated. A revoked
historical tuple may be requalified only with distinct evidence identity.

## Monotonic floor and anti-rollback custody

A deployment host maintains an owner-private monotonic floor for accepted registry generations.
Floor V2 binds:

```text
registry file SHA-256
canonical registry digest
ordered historical entry IDs
ordered revocation IDs
ordered revoked entry IDs
ordered active entry IDs
monotonic sequence
previous floor digest
```

Active and revoked entry IDs are disjoint and exactly partition historical entries. Every prior
historical entry ID and every prior revocation ID must remain present. Consequently an old registry
cannot silently restore a revoked entry, erase admission history, or erase revocation history.

Initialize in a private location:

```bash
install -d -m 0700 /private/dfmcp
python3 scripts/live_compatibility_floor.py init \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

Verify exact equality:

```bash
python3 scripts/live_compatibility_floor.py verify \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

Advance after reviewing a newer promotion or revocation generation:

```bash
FLOOR_SHA256="$(sha256sum /private/dfmcp/live-compatibility-floor.json | awk '{print $1}')"
python3 scripts/live_compatibility_floor.py advance \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --expected-floor-sha256 "$FLOOR_SHA256"
```

Custody requires an absolute path, a real exact-mode `0700` parent, a regular non-symlink exact-mode
`0600` file, root/effective-user ownership, no-follow reads, exclusive initialization, locking,
compare-and-swap, atomic replacement, and directory fsync.

A verified legacy floor using `dfmcp.live-compatibility-floor/1` may migrate only through an explicit
compare-and-swap advance. It cannot verify a registry containing revocations because V1 did not bind
revocation identity. The floor is local custody, not distributed consensus, compatibility evidence,
process termination, or hostile-root protection.

## Authority-free admission doctor

Before reading a bridge token or executing a binary, run the admission doctor:

```bash
python3 scripts/doctor_live_admission.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --require-entry-id <64-hex-entry-id>
```

Its fixed stages are registry, compatibility floor, exact tuple resolution, and optional server
artifact. Successful states are `compatibility_ready` and `artifact_preflight_ready`. A revoked
required entry produces `not_ready` with matching revocations and a bounded requalification or
policy-review recovery step.

The doctor is authority-free. It does not execute the server, connect to DFHack, read a bearer
token, alter registry or floor state, terminate a process, or grant capabilities.

## Protocol-bound V2 process admission

The normative process contract is `architecture/live_admission_ticket_v2.json`. Bridge protocol is
copied from the exact deployment manifest and must agree across compatibility decision, launch
record, single-use ticket, `DFMCP_ADMITTED_BRIDGE_PROTOCOL`, Rust admission context and provenance,
and final private runner lookup. Launch and ticket digests cover it.

The production map currently contains only:

```text
protocol 1.0 → dwarf-fortress-mcp serve-live → private protocol-1.0 runner
```

Protocol 1.1 remains `implemented_unadmitted_development_only`. Unknown protocols, protocol 1.1
production attempts, mismatches, revoked entries, and legacy V1 tickets fail before live-server
startup.

## Source-bound server qualification

Qualify the exact clean source, then the release executable:

```bash
./scripts/qualify_local.sh
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

The server binary receipt binds the local qualification receipt, exact source digests, platform,
toolchain, executable checks, size, and SHA-256. It grants no bridge or game authority and does not
substitute for registry or floor policy.

## Admitted launch

Only after every prior rung passes:

```bash
export DFMCP_BRIDGE_TOKEN='<32..256-byte loopback secret>'
python3 scripts/serve_admitted_live.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --binary /path/to/qualified/dwarf-fortress-mcp \
  --server-receipt /path/to/live-server-binary-receipt.json \
  --local-qualification-receipt /path/to/qualification-receipt.json \
  --source-root /path/to/exact/source \
  --expected-dfmcp-commit <40-hex-source-commit> \
  --require-entry-id <64-hex-entry-id> \
  --launch-record /private/dfmcp/admitted-live-launch.json
```

The launcher validates token shape and dynamic loader hygiene; verifies registry and floor; resolves
the explicitly required active entry; validates protocol; verifies the source-bound server receipt;
opens the executable without following a symlink; checks owner, mode, device, executable inode,
length, and SHA-256; re-reads registry and floor; writes a secret-free launch record; re-hashes the
open descriptor; issues a single-use admission ticket bound to process ID; and performs descriptor
execution with no path fallback.

The Rust consumer repeats ticket, protocol, capability, custody, and executable checks, deletes the
ticket, proves its absence, and only then starts the selected private runtime. A revocation committed
after ticket consumption does not terminate that process; a fresh launch must resolve the current
registry and will refuse the revoked entry.

## Failure posture

Absence from the registry means not admitted. A historical entry with a revocation is not active.
Presence without revocation means experimental only for the exact evidence named by the entry.
Missing floor custody, stale generation, rewritten history, erased revocation ID, wrong required
entry, protocol mismatch, source-receipt mismatch, loader injection, permissive files, symbolic
links, executable replacement, unknown protocol, legacy ticket, or direct `serve-live` invocation
all fail closed.
