# Live compatibility admission

A successful call, a unit test, a native plugin build, and a real disposable-fort campaign are four
different kinds of evidence. None of them alone authorizes a broad compatibility claim.

The live-read adapter admits an exact tuple only through this evidence chain:

```text
clean dfmcp source revision
  + exact DFHack source revision
  + exact native plugin binary digest
  + R1 native-build receipt
  + R2 authenticated handshake matrix
  + R3 paused-world deterministic read matrix
  + R4 restart, drift, and partial-publication fencing
  + R5 cold-agent semantic orientation
  → one experimental exact-tuple registry entry
```

Starting the live MCP process adds a distinct executable-admission chain:

```text
exact deployment manifest
  + one exact registry generation
  + required compatibility entry ID
  + passing local qualification receipt
  + qualified release-server binary receipt
  + opened release-server device, inode, size, mode, owner, and SHA-256
  + sanitized dynamic-loader environment
  + single-use admission ticket bound to the process ID and executable inode
  → descriptor-only exec of authenticated read-only live MCP
```

Compatibility admission, server-artifact qualification, and runtime admission are deliberately
separate. Passing one does not imply either of the others.

## Machine boundaries

The registry, promotion boundary, deterministic resolver, artifact verifier, and admitted launcher
are:

```text
architecture/live_compatibility_registry_v1.json
scripts/promote_live_compatibility.py
scripts/resolve_live_compatibility.py
scripts/verify_live_server_binary_receipt.py
scripts/serve_admitted_live.py
```

The raw Rust live server runner is private to `dfmcp-mcp`. The public `run_live_stdio` entrypoint
first consumes the launcher-issued ticket. Direct `dwarf-fortress-mcp serve-live` invocation
without that ticket fails closed.

## Exact tuple

A compatibility entry is bound to all of the following:

- Dwarf Fortress version;
- DFHack version;
- bridge implementation version;
- bridge protocol version;
- host operating system and machine architecture;
- `dwarf_fortress_mcp` Git commit;
- DFHack Git commit;
- native plugin SHA-256;
- native-build receipt SHA-256;
- live acceptance receipt SHA-256 and canonical receipt digest.

Changing any component creates a different tuple. Compatibility does not flow automatically from
one patch release, platform, compiler output, bridge binary, or source revision to another.

A compatibility decision additionally binds the canonical digest of the complete registry
object, not merely its entry count. Two registry generations containing the same selected entry
therefore produce different decision digests.

## Support level

R1-R5 promotion creates an `experimental` entry. It does not create a `supported` or `production`
entry.

`experimental` means only that the exact tuple passed the bounded read-only acceptance campaign
represented by its receipts. It does not establish:

- mutation correctness;
- compatibility with adjacent DF or DFHack versions;
- protection against a compromised local host or account;
- durable production custody or crash recovery;
- release packaging or end-user installation quality;
- strategic completeness outside the observed domains.

A future support-level transition requires a separate registry generation and separate acceptance
contract. Editing the word in an existing entry is not an admissible promotion.

## Read-only capability envelope

Every V1 registry entry has the same authority ceiling:

```text
doctor
observe
query
wait
```

The mutation-capability set must remain empty. The native R1 receipt must show exactly:

```text
Handshake
ReadObservation
```

and no mutation RPCs. Both binary string and symbol inventories must have passed. A skipped
inventory is sufficient for development diagnosis but insufficient for compatibility promotion.

## Epistemic coverage

The admitted complete domains are limited to the V1 slice:

- fortress identity;
- fortress clock;
- pause state;
- complete citizen roster.

Citizen names are conditional on the declared projection. The following domains remain explicitly
omitted and unknown:

- items;
- jobs;
- map state;
- economy;
- detailed welfare;
- military;
- history.

An admitted tuple cannot prove absence in an omitted domain.

## Promotion procedure

First produce a clean R1 native-build receipt:

```bash
scripts/qualify_dfhack_plugin.sh /path/to/dfhack-source
```

Then capture and verify the real R2-R5 campaign:

```bash
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

Review the proposed entry and evidence bundle. An authoritative in-place promotion is a
compare-and-swap operation against an explicitly selected registry generation:

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

The promotion lock prevents concurrent in-place writers. A stale expected digest fails rather
than overwriting a newer registry generation.

Promotion fails closed when:

- the live receipt is synthetic, dirty, development-only, malformed, or digest-inconsistent;
- any R2-R5 gate is absent, reordered, or not passed;
- the native receipt is absent, dirty, malformed, or bound to different commits or binary bytes;
- string or symbol inventory was skipped;
- a mutation RPC appears;
- the protocol is not exactly V1;
- an entry identifier does not reproduce its canonical fields;
- the exact source/binary/version/platform tuple already has a canonical entry;
- the evidence locator is unbounded or traversal-bearing;
- the selected registry generation changed before publication.

## Deployment resolution

A deployment manifest names the exact version, platform, source revisions, and plugin digest to be
started. Resolve it before artifact execution and fence the expected entry ID:

```bash
python3 scripts/resolve_live_compatibility.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --require-entry-id <64-hex-entry-id> \
  --output /path/to/live-compatibility-decision.json
```

A miss returns no capability. A matching tuple under a different entry ID also fails closed. The
canonical decision digest covers the complete registry-generation digest and requested entry ID.

## Server binary receipt

Run local qualification for the exact clean source revision, then build and qualify the release
server artifact:

```bash
./scripts/qualify_local.sh
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

The server binary receipt binds the clean source commit, complete passing local receipt, source
files, executable checks, platform, binary size, and binary SHA-256. It grants no bridge or game
authority and does not substitute for R1-R5 compatibility evidence.

## Admitted runtime launch

Start the exact binary only through the admitted launcher:

```bash
export DFMCP_BRIDGE_TOKEN='<32..256-byte loopback secret>'
python3 scripts/serve_admitted_live.py \
  /path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --binary /path/to/qualified/dwarf-fortress-mcp \
  --server-receipt /path/to/live-server-binary-receipt.json \
  --local-qualification-receipt /path/to/qualification-receipt.json \
  --source-root /path/to/exact/source \
  --expected-dfmcp-commit <40-hex-source-commit> \
  --require-entry-id <64-hex-entry-id> \
  --launch-record /private/path/admitted-live-launch.json
```

The launcher verifies the registry decision, source commit, server receipt, and already-opened
binary descriptor. It rejects dynamic loader override variables and refuses a path-based exec
fallback.

Immediately before descriptor exec, it creates an owner-only `.dfmcp-admission` directory and an
owner-read/write ticket. The ticket contains no bridge token. It binds:

- one random ticket ID;
- the current process ID, which survives `exec`;
- creation and short expiry times;
- compatibility entry ID;
- registry-generation, decision, server-receipt, launch, and binary digests;
- executable device, inode, byte length, full mode, and owner UID;
- the exact read-only capability list;
- an empty mutation-capability list.

The Rust process opens the ticket, rejects symbolic links and permissive custody, verifies stable
metadata and canonical digest, compares the current executable inode, deletes the ticket, proves
that deletion, and only then starts MCP. The ticket is single-use: restart requires a fresh exact
launcher decision and a new ticket.

This is an accidental-bypass, stale-ticket, and cross-process fence inside the documented local
bearer-token threat model. It is not a cryptographic defense against compromise of the same user
account, launcher source, and executable together.

## Registry determinism

Entries are sorted by their SHA-256 `entry_id`. The identifier is the digest of every canonical
entry field except the identifier itself. Reordering keys or formatting JSON does not change the
identifier; changing any semantic field does.

The promotion tool writes root-last through an atomic same-directory replacement. It does not
launch Dwarf Fortress, execute shell commands, download artifacts, discover receipts implicitly,
or weaken any source acceptance gate.

## Revocation and supersession

Protocol V1 intentionally has no automated revocation mutation. A bad tuple must be removed or
marked under a future explicit revocation schema in a reviewed commit, with the evidence and
reason retained elsewhere. Silent replacement would erase operational history and create an ABA
hazard for agents caching compatibility decisions.

Until a revocation schema lands, consumers must treat absence from the current registry as not
admitted and presence as experimental only for the exact entry bytes.
