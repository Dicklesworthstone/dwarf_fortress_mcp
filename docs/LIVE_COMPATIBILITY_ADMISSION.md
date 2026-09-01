# Live compatibility admission

A successful RPC, a Rust unit test, a native plugin build, a disposable-fort campaign, a qualified
server binary, and an admitted process launch are different kinds of evidence. No one rung
substitutes for another.

## Current registry status

The checked-in machine registry is:

```text
architecture/live_compatibility_registry_v1.json
```

It currently has status `no_admitted_live_tuples` and contains no entries. The live-read source is
substantial, but no current source/binary/version/platform tuple is admitted merely because the
code exists. Until an exact R1-R5 campaign is promoted, the runtime launcher must fail closed.

## Complete proof chain

An exact read-only tuple is admitted only through:

```text
clean dwarf_fortress_mcp source revision
  + exact DFHack source revision
  + exact native plugin binary digest
  + R1 native-build receipt
  + R2 authenticated handshake matrix
  + R3 deterministic complete-read matrix
  + R4 restart, drift, gap, and partial-publication fencing
  + R5 cold-agent semantic orientation
  → one experimental exact-tuple registry entry
```

Starting a process adds separate custody and executable proof:

```text
reviewed registry generation
  + owner-only monotonic floor matching those exact registry bytes
  + exact deployment manifest
  + required compatibility entry ID
  + deterministic compatibility decision
  + passing local qualification receipt
  + source-bound release-server receipt
  + opened executable device, inode, size, mode, owner, and SHA-256
  + sanitized dynamic-loader environment
  + single-use process- and executable-bound ticket
  → descriptor-only exec of authenticated read-only live MCP
```

Compatibility admission, local anti-rollback custody, artifact qualification, diagnosis, and
runtime admission remain separate. Passing one does not imply the others.

## Machine boundaries

```text
architecture/live_compatibility_registry_v1.json
architecture/live_compatibility_floor_v1.json
architecture/live_admission_doctor_v1.json
architecture/live_server_binary_receipt_v1.json
scripts/promote_live_compatibility.py
scripts/resolve_live_compatibility.py
scripts/live_compatibility_floor.py
scripts/doctor_live_admission.py
scripts/verify_live_server_binary_receipt.py
scripts/serve_admitted_live.py
crates/dfmcp-mcp/src/admission.rs
```

The raw Rust live-server runner is private to `dfmcp-mcp`. The public entrypoint consumes and
deletes a valid single-use ticket before it starts the MCP transport. Direct
`dwarf-fortress-mcp serve-live` invocation without that ticket fails closed.

## Exact tuple

A registry entry binds:

- Dwarf Fortress version;
- DFHack version;
- bridge implementation and protocol versions;
- host operating system and machine architecture;
- exact `dwarf_fortress_mcp` Git commit;
- exact DFHack Git commit;
- native plugin SHA-256;
- R1 native-build receipt SHA-256;
- R2-R5 live receipt file SHA-256 and canonical receipt digest;
- exact capabilities, coverage, omissions, limitations, and evidence locator.

Changing any component creates a different tuple. Compatibility never flows automatically across
patch releases, platforms, compilers, source revisions, plugin binaries, or registry generations.

A compatibility decision also binds the canonical digest of the complete registry object and the
explicitly required entry ID. Selecting the same entry from a different registry generation
therefore produces a different decision identity.

## Support and authority

R1-R5 promotion creates an `experimental` entry, never a `supported` or `production` entry. The V1
authority ceiling is:

```text
doctor
observe
query
wait
```

The mutation-capability set is empty. The native receipt must prove that the plugin exports exactly:

```text
Handshake
ReadObservation
```

No pause, command, Lua, keyboard, designation, filesystem, arbitrary RPC, or mutation path is
admitted.

The complete V1 observation domains are fortress identity, clock, pause state, and complete citizen
roster. Citizen names are conditional on the requested projection. Items, jobs, map state, economy,
detailed welfare, military, and history remain omitted. An empty result cannot prove absence in an
omitted domain.

## Promotion

Produce clean native and live receipts:

```bash
scripts/qualify_dfhack_plugin.sh /path/to/dfhack-source
scripts/qualify_live_read.sh \
  /path/to/events.jsonl \
  /path/to/dfhack-plugin-qualification.json
```

Create a proposed registry generation without mutating the checked-in root:

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /path/to/live-read-acceptance-receipt.json \
  --native-receipt /path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --output /tmp/live_compatibility_registry_v1.json
```

After reviewing the proposed entry and its evidence, an authoritative in-place promotion is a
compare-and-swap operation:

```bash
REGISTRY_SHA256="$(sha256sum architecture/live_compatibility_registry_v1.json | awk '{print $1}')"
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /path/to/live-read-acceptance-receipt.json \
  --native-receipt /path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --expected-registry-sha256 "$REGISTRY_SHA256" \
  --in-place
```

Promotion rejects dirty, synthetic, development-only, malformed, digest-inconsistent, incomplete,
reordered, mutation-bearing, duplicate, traversal-bearing, or stale-generation evidence.

## Monotonic floor and anti-rollback custody

The registry is versioned source evidence. A deployment additionally needs trusted local custody so
an older, still-well-formed registry cannot silently replace a newer accepted generation. The
**monotonic floor** is an owner-only local record of the last accepted registry bytes and ordered
entry IDs.

Create a private directory and initialize the floor exactly once:

```bash
install -d -m 0700 /private/dfmcp
python3 scripts/live_compatibility_floor.py init \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

Verify custody and exact registry equality:

```bash
python3 scripts/live_compatibility_floor.py verify \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

After reviewing a newer append-only registry generation, advance through compare-and-swap:

```bash
FLOOR_SHA256="$(sha256sum /private/dfmcp/live-compatibility-floor.json | awk '{print $1}')"
python3 scripts/live_compatibility_floor.py advance \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --expected-floor-sha256 "$FLOOR_SHA256"
```

Custody requires an absolute path, a real owner-only `0700` parent directory, a regular
non-symbolic-link `0600` file, and ownership by root or the effective user. Initialization is
exclusive. Advancement is locked, atomic, fsynced, digest-chained, and compare-and-swap fenced.
Every prior entry ID must remain present. Formatting-only byte changes are explicit generations
because the floor binds both file SHA-256 and canonical registry digest.

The floor is local anti-rollback custody. It does not admit a tuple, create compatibility evidence,
grant authority, implement distributed consensus, or defend against compromise of the owner/root
account. Silent entry removal is forbidden; an explicit evidence-bearing revocation schema is a
future separate design problem.

## Authority-free admission doctor

Before handling a bridge secret or attempting execution, run the deterministic **admission doctor**:

```bash
python3 scripts/doctor_live_admission.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --require-entry-id <64-hex-entry-id>
```

The doctor checks, in fixed order:

```text
registry
compatibility_floor
exact_tuple_resolution
server_artifact
```

Without artifact inputs, a successful report is `compatibility_ready`. Supplying the complete
artifact input set can produce `artifact_preflight_ready`:

```bash
python3 scripts/doctor_live_admission.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --require-entry-id <64-hex-entry-id> \
  --binary /path/to/qualified/dwarf-fortress-mcp \
  --server-receipt /path/to/live-server-binary-receipt.json \
  --local-qualification-receipt /path/to/qualification-receipt.json \
  --binary-contract architecture/live_server_binary_receipt_v1.json \
  --source-root /path/to/exact/source \
  --expected-dfmcp-commit <40-hex-source-commit>
```

The doctor is authority-free. It does not execute the server, connect to DFHack, read the bridge
token, alter the registry or floor, or grant capabilities. Reports contain no runtime timestamps;
identical input bytes produce the same canonical report digest. `artifact_preflight_ready` remains
preflight, not launch evidence.

## Local and server-artifact qualification

Qualify the exact clean source revision:

```bash
./scripts/qualify_local.sh
```

Then build and qualify the release server artifact:

```bash
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

The server receipt binds the clean source commit, exact passing gate order, floor and doctor source,
Rust admission consumer, Agent Turn provenance projection, executable checks, platform, source-file
digests, binary size, and binary SHA-256. It grants no bridge or game authority and does not prove
that a trusted floor exists on the deployment host.

## Admitted launch

Only after all prior rungs pass:

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

The launcher:

1. validates the secret shape and rejects dynamic-loader override variables;
2. reads the registry and owner-only floor independently and requires exact generation equality;
3. resolves the manifest under the required entry fence;
4. verifies the source-bound server receipt and opens the exact executable without following a
   symlink;
5. checks executable owner, mode, device, inode, length, and SHA-256;
6. re-reads the registry and floor after artifact verification;
7. writes the launch record;
8. re-hashes the already-open descriptor before issuing the ticket;
9. creates an owner-only `.dfmcp-admission` directory and `0600` single-use ticket;
10. re-reads the floor and registry immediately before execution;
11. re-hashes the opened descriptor immediately before descriptor-only `execve`.

The ticket contains no bridge token. It binds the process ID, expiry, exact entry, registry,
decision, monotonic-floor file/content/sequence, server receipt, launch record, executable identity,
read-only capability list, and an empty mutation list.

The Rust process rejects permissive or symbolic custody, verifies the canonical ticket digest,
process and expiry, reopens and hashes the current executable bytes, deletes the ticket, proves its
absence, retains the admitted provenance for every live Agent Turn, and only then starts MCP. A
restart requires a fresh launch decision and fresh ticket. No path-based execution fallback exists.

This protects against accidental bypass, stale tickets, registry rollback relative to the trusted
floor, cross-process ticket reuse, and same-inode executable byte substitution inside the stated
local threat model. It is not protection against simultaneous compromise of the owning account,
launcher, floor, source, executable, and process.

## Determinism and failure posture

Registry entry IDs are SHA-256 digests of all canonical entry fields except the identifier itself.
The promotion tool publishes atomically and never launches Dwarf Fortress, executes shell commands,
downloads artifacts, or weakens source gates.

Absence from the current registry means not admitted. Presence means experimental only for the
exact bytes and evidence named by the entry. Missing floor custody, stale floor bytes, a different
entry ID, dirty source, source-receipt mismatch, platform drift, permissive files, symlinks, loader
injection, executable replacement, or direct `serve-live` invocation all fail closed.
