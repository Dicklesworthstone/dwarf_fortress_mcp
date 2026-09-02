# Live compatibility and process admission

A successful RPC, a Rust unit test, a native plugin build, a disposable-fort campaign, a source
qualification receipt, a server-binary receipt, and an admitted process launch are different kinds
of evidence. No rung substitutes for another.

## Current operational status

The checked-in compatibility registry is:

```text
architecture/live_compatibility_registry_v1.json
```

It currently has status `no_admitted_live_tuples` and contains no entries. The repository contains a
substantial protocol-1.0 read-only live stack and an implemented protocol-1.1 announcement
generation, but source presence does not admit either one. Until exact evidence is promoted, the
production launcher fails closed.

Protocol 1.1 is available only through the separately named, explicitly unadmitted development
binary. It cannot consume a production ticket or appear in production admission provenance.

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
  → one reviewed exact-tuple registry entry
```

Starting a production process adds separate custody and executable proof:

```text
reviewed registry generation
  + owner-only monotonic floor matching those exact registry bytes
  + exact deployment manifest
  + required compatibility entry ID
  + deterministic compatibility decision
  + exact bridge protocol selected by that decision
  + passing local qualification receipt
  + source-bound release-server receipt
  + opened executable device, inode, size, mode, owner, and SHA-256
  + sanitized dynamic-loader environment
  + protocol-bound V2 single-use process ticket
  → descriptor-only execution of the exact admitted runtime
```

Compatibility admission, local anti-rollback custody, artifact qualification, readiness diagnosis,
and runtime admission remain separate.

## Machine boundaries

```text
architecture/live_compatibility_registry_v1.json
architecture/live_compatibility_floor_v1.json
architecture/live_admission_doctor_v1.json
architecture/live_admission_ticket_v2.json
architecture/live_server_binary_receipt_v1.json
scripts/promote_live_compatibility.py
scripts/resolve_live_compatibility.py
scripts/live_compatibility_floor.py
scripts/doctor_live_admission.py
scripts/verify_live_server_binary_receipt.py
scripts/serve_admitted_live.py
crates/dfmcp-mcp/src/admission.rs
```

The raw production live-server implementation is private to `dfmcp-mcp`. The public production
entrypoint consumes a valid single-use ticket before starting MCP. Direct
`dwarf-fortress-mcp serve-live` invocation without the exact ticket and protocol environment fails
closed.

## Exact tuple identity

A registry entry binds:

- Dwarf Fortress version;
- DFHack version;
- bridge implementation and protocol versions;
- operating system and machine architecture;
- exact `dwarf_fortress_mcp` Git commit;
- exact DFHack Git commit;
- native plugin SHA-256;
- R1 native-build receipt SHA-256;
- R2-R5 live receipt file SHA-256 and canonical receipt digest;
- exact capabilities, coverage, omissions, limitations, and evidence locator.

Changing any component creates a different tuple. Compatibility never flows automatically across
patch releases, protocols, platforms, compilers, source revisions, plugin binaries, or registry
generations.

The resolver also binds the canonical digest of the complete registry object and the explicitly
required entry ID. Selecting the same-looking entry from different registry bytes produces a
different decision identity.

## Support and authority ceiling

R1-R5 promotion creates an `experimental` entry, never a `supported` or `production` entry. The
current read-only capability ceiling is:

```text
doctor
observe
query
wait
```

The mutation-capability set is empty. Protocol 1.0 and protocol 1.1 both retain a two-method native
waist:

```text
Handshake
ReadObservation
```

Protocol 1.1 adds bounded retained-announcement fields inside `ReadObservation`; it does not add a
standalone RPC or mutation path. No pause, command, Lua, keyboard, designation, filesystem,
arbitrary forwarding, or game mutation route is admitted.

## Compatibility promotion

Produce clean native and live receipts for one exact generation, then create a proposed registry
without mutating the checked-in root:

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /path/to/live-read-acceptance-receipt.json \
  --native-receipt /path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --output /tmp/live_compatibility_registry_v1.json
```

After independent review, authoritative in-place promotion is compare-and-swap fenced:

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

The versioned registry is source evidence. A deployment host additionally needs trusted local
custody so an older but structurally valid registry cannot silently replace a newer accepted
generation. The monotonic floor records the last accepted exact registry bytes and ordered entry
IDs.

Initialize it only in an owner-private location:

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

Advance only through compare-and-swap after reviewing a newer append-only generation:

```bash
FLOOR_SHA256="$(sha256sum /private/dfmcp/live-compatibility-floor.json | awk '{print $1}')"
python3 scripts/live_compatibility_floor.py advance \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --expected-floor-sha256 "$FLOOR_SHA256"
```

Custody requires an absolute path, a real exact-mode `0700` parent directory, a regular
non-symbolic-link exact-mode `0600` file, and ownership by root or the effective user.
Initialization is exclusive. Advancement is locked, atomic, fsynced, digest-chained, and
compare-and-swap fenced. Every prior entry ID must remain present.

The floor is local anti-rollback custody. It is not compatibility evidence, distributed consensus,
revocation, or protection against simultaneous compromise of the owner/root account and all local
artifacts.

## Authority-free admission doctor

Before handling a bridge secret or attempting execution, run the deterministic admission doctor:

```bash
python3 scripts/doctor_live_admission.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --require-entry-id <64-hex-entry-id>
```

The fixed stage order is:

```text
registry
compatibility_floor
exact_tuple_resolution
server_artifact
```

Without artifact inputs, success is `compatibility_ready`. Supplying the complete artifact set can
produce `artifact_preflight_ready`. The doctor does not execute the server, connect to DFHack, read
the bearer token, alter the registry or floor, or grant capabilities. A doctor report is diagnosis,
not authority.

## Source-bound server qualification

Qualify the exact clean source revision:

```bash
./scripts/qualify_local.sh
```

Then qualify the release executable separately:

```bash
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

The server receipt binds the clean source commit, canonical local gate order, source-file digests,
platform, toolchain, executable checks, binary size, and binary SHA-256. The source map includes the
V2 admission-ticket contract, launcher, Rust ticket consumer, Agent Turn projection, and focused
launcher/ticket tests.

A server receipt qualifies one executable. It does not admit a Dwarf Fortress/DFHack/plugin tuple,
prove a trusted floor exists, connect to DFHack, or grant game authority.

## Protocol-bound V2 process admission

The normative contract is:

```text
architecture/live_admission_ticket_v2.json
```

The bridge protocol is not inferred after launch. It is copied from the exact deployment manifest
and must agree across:

```text
compatibility decision
→ launch record bridge_protocol
→ ticket bridge_protocol
→ DFMCP_ADMITTED_BRIDGE_PROTOCOL
→ Rust admission context and retained provenance
→ final private server runner
```

Both the launch digest and ticket digest cover `bridge_protocol`. The Rust consumer rejects the
legacy `dfmcp.live-admission-ticket/1` schema, a protocol mismatch, an unknown protocol, or a
protocol without a reviewed production runner.

The production protocol map currently contains only:

```text
protocol 1.0 → dwarf-fortress-mcp serve-live → private protocol-1.0 runner
```

Protocol 1.1 remains `implemented_unadmitted_development_only`. Even if a malformed or prematurely
promoted manifest names protocol 1.1, the launcher and Rust dispatcher refuse it before live-server
startup. Admitting protocol 1.1 requires its complete independent source, native, A1-A6, baseline
R2-R5, registry, floor, server-artifact, and runtime-dispatch evidence.

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

The launcher:

1. validates secret shape and rejects dynamic-loader overrides;
2. reads the registry and owner-private floor independently and requires exact generation equality;
3. resolves the manifest under the required entry fence;
4. rejects any bridge protocol absent from the production map;
5. verifies the source-bound server receipt and opens the executable without following a symlink;
6. checks executable owner, mode, device, inode, length, and SHA-256;
7. re-reads the registry and floor after artifact verification;
8. writes a protocol-bound launch record;
9. re-hashes the already-open descriptor before issuing the ticket;
10. creates a real exact-mode `0700` ticket directory and exact-mode `0600` single-use ticket;
11. re-reads the floor and registry immediately before execution;
12. verifies protocol equality and re-hashes the descriptor immediately before descriptor-only
    `execve`.

The ticket contains no bridge token. It binds process ID, expiry, bridge protocol, exact entry,
registry, decision, monotonic-floor file/content/sequence, server receipt, launch record, executable
identity, read-only capability list, and an empty mutation list.

The Rust process verifies canonical ticket digest, process, expiry, protocol, capabilities,
registry/decision/floor/receipt/launch identities, executable inode and SHA-256, then deletes the
ticket and proves its absence before starting the selected private server. A restart requires a new
decision and ticket. No path-based execution fallback exists.

Every admitted Agent Turn exposes the retained bridge protocol alongside ticket, entry, registry,
decision, floor, receipt, launch, and executable identities. Presentation explains authority; it
cannot create or widen it.

## Failure posture

Absence from the current registry means not admitted. Presence means experimental only for the
exact bytes and evidence named by the entry. Missing floor custody, stale floor bytes, a different
entry ID, dirty source, source-receipt mismatch, platform drift, protocol mismatch, permissive
files, symbolic links, loader injection, executable replacement, legacy tickets, unknown protocol,
or direct `serve-live` invocation all fail closed.
