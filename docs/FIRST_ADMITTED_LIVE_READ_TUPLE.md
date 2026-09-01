# First admitted live-read tuple

This runbook is the shortest honest path from the current empty registry to one experimental,
read-only, exact-tuple deployment. It is deliberately procedural: every step produces an artifact
consumed by the next step, and no later step repairs missing earlier evidence.

## Starting state

The checked-in registry currently contains no entries. Source presence is not admission. Begin from
an authoritative clean checkout and record:

```text
dwarf_fortress_mcp commit
DFHack commit
Dwarf Fortress version and distribution
host system and machine architecture
selected latest-nightly rustc -vV
Cargo.lock SHA-256
```

Do not start with an old plugin binary, an old acceptance receipt, or a registry entry from another
commit.

## 1. Full local qualification

```bash
./scripts/verify.sh
./scripts/qualify_local.sh
```

Required result:

```text
target/qualification/<run>/qualification-receipt.json
status = passed
source.dirty = false
all gates = passed
no static-only or missing-Rust escape
```

Pin the exact receipt path and SHA-256. Any source edit after this step invalidates it.

## 2. Source-bound release server

```bash
scripts/qualify_live_server_binary.sh \
  target/qualification/<run>/qualification-receipt.json \
  target/live-server-binary-qualification/<run>
```

Retain:

```text
live-server-binary-receipt.json
SHA256SUMS
exact target/release/dwarf-fortress-mcp bytes
```

The binary receipt proves only the server artifact. It does not prove DFHack compatibility or
admit a process.

## 3. Native plugin R1

Check out the exact intended DFHack source revision, then run:

```bash
scripts/qualify_dfhack_plugin.sh /absolute/path/to/dfhack-source
```

Required evidence includes:

- exact DFHack commit;
- exact `dwarf_fortress_mcp` commit expected by the bridge source;
- produced plugin SHA-256;
- successful native build;
- exact method inventory containing only `Handshake` and `ReadObservation`;
- symbol and string inventories completed, not skipped;
- empty mutation method set.

Install only the qualified plugin bytes into the disposable test installation.

## 4. Disposable-fort R2-R5 campaign

Use a disposable save and the exact plugin from R1. Capture the complete event stream expected by
`architecture/live_read_acceptance_v1.json`, then run:

```bash
scripts/qualify_live_read.sh \
  /absolute/path/to/events.jsonl \
  /absolute/path/to/dfhack-plugin-qualification.json
```

The campaign must establish:

```text
R2 authentication, rejection, and secret non-disclosure
R3 deterministic complete citizen reads and pagination independence
R4 restart, generation, version, cursor, and partial-publication fencing
R5 cold-agent briefing, attention, coverage, and safe-next-step semantics
```

A synthetic or development receipt cannot be promoted.

## 5. Propose the exact registry generation

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /absolute/path/to/live-read-acceptance-receipt.json \
  --native-receipt /absolute/path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --output /tmp/live_compatibility_registry_v1.json
```

Review all canonical entry fields, especially:

- source revisions and plugin digest;
- DF/DFHack/bridge/protocol versions;
- host platform;
- R1-R5 receipt identities;
- complete, conditional, and omitted domains;
- read-only capabilities and empty mutation set;
- limitations and evidence locator;
- reproduced `entry_id`.

## 6. Promote through registry compare-and-swap

```bash
REGISTRY_SHA256="$(sha256sum architecture/live_compatibility_registry_v1.json | awk '{print $1}')"
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /absolute/path/to/live-read-acceptance-receipt.json \
  --native-receipt /absolute/path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --expected-registry-sha256 "$REGISTRY_SHA256" \
  --in-place
```

Commit the reviewed registry generation and associated public-safe evidence references. Do not
commit bearer tokens, private saves, deployment floors, or process tickets.

## 7. Accept the generation into deployment custody

Create owner-private custody once:

```bash
install -d -m 0700 /private/dfmcp
python3 scripts/live_compatibility_floor.py init \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

For an existing floor, advance only after review:

```bash
FLOOR_SHA256="$(sha256sum /private/dfmcp/live-compatibility-floor.json | awk '{print $1}')"
python3 scripts/live_compatibility_floor.py advance \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --expected-floor-sha256 "$FLOOR_SHA256"
```

Verify exact equality:

```bash
python3 scripts/live_compatibility_floor.py verify \
  --floor /private/dfmcp/live-compatibility-floor.json \
  --registry architecture/live_compatibility_registry_v1.json
```

## 8. Write the exact deployment manifest

The manifest must identify the same:

```text
Dwarf Fortress version
DFHack version
bridge version
protocol version
system and machine
source commits
plugin SHA-256
```

Copy the promoted 64-hex `entry_id`; do not infer or search for “the closest” entry at launch time.

## 9. Run authority-free artifact preflight

```bash
python3 scripts/doctor_live_admission.py \
  /absolute/path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --require-entry-id <64-hex-entry-id> \
  --binary /absolute/path/to/dwarf-fortress-mcp \
  --server-receipt /absolute/path/to/live-server-binary-receipt.json \
  --local-qualification-receipt /absolute/path/to/qualification-receipt.json \
  --binary-contract architecture/live_server_binary_receipt_v1.json \
  --source-root /absolute/path/to/exact-clean-source \
  --expected-dfmcp-commit <40-hex-commit>
```

Required status:

```text
artifact_preflight_ready
```

This is diagnosis, not process authority.

## 10. Launch through the only admitted entrypoint

```bash
export DFMCP_BRIDGE_TOKEN='<32..256-byte loopback secret>'
python3 scripts/serve_admitted_live.py \
  /absolute/path/to/live-deployment-manifest.json \
  --registry architecture/live_compatibility_registry_v1.json \
  --compatibility-floor /private/dfmcp/live-compatibility-floor.json \
  --binary /absolute/path/to/dwarf-fortress-mcp \
  --server-receipt /absolute/path/to/live-server-binary-receipt.json \
  --local-qualification-receipt /absolute/path/to/qualification-receipt.json \
  --source-root /absolute/path/to/exact-clean-source \
  --expected-dfmcp-commit <40-hex-commit> \
  --require-entry-id <64-hex-entry-id> \
  --launch-record /private/dfmcp/admitted-live-launch.json
```

Do not invoke `dwarf-fortress-mcp serve-live` directly. Do not set `LD_*`, `DYLD_*`,
`GLIBC_TUNABLES`, or equivalent loader overrides.

## 11. Verify first live response provenance

The first successful live Agent Turn must report the exact:

```text
compatibility entry ID
registry digest
compatibility decision digest
floor file SHA-256
floor digest and sequence
server receipt digest
launch digest
ticket ID
server executable SHA-256
mutation_capabilities = []
```

Compare those fields with the retained registry, floor, receipt, and launch record. A successful
bridge read under different provenance is not the deployment you qualified.

## 12. Retain evidence and stop on drift

Retain the public-safe receipts, checksums, exact source commit, registry generation, and secret-free
launch record. Keep deployment floor and ticket custody private.

Stop and requalify when any exact identity changes. Do not patch the registry entry, recycle an old
receipt, lower floor custody, ignore a restart/reset, or broaden the compatibility statement to
adjacent versions.

## Terminal success statement

The strongest honest statement after this runbook is:

> One exact source/plugin/version/platform tuple is experimentally admitted for the bounded
> authenticated read-only V1 domains named by its R1-R5 evidence and was launched through the
> matching local monotonic floor and source-bound server receipt.

It is not a mutation claim, a neighboring-version claim, a hostile-host security claim, or a
production support window.
