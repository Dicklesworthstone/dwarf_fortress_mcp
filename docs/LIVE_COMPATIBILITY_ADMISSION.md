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

The machine registry is:

```text
architecture/live_compatibility_registry_v1.json
```

The promotion boundary is:

```text
scripts/promote_live_compatibility.py
```

The registry deliberately begins empty. Source presence, static validation, laboratory tests, and
an R1 build receipt do not populate it.

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

## Support level

R1-R5 promotion creates an `experimental` entry. It does not create a `supported` or `production`
entry.

`experimental` means only that the exact tuple passed the bounded read-only acceptance campaign
represented by its receipts. It does not establish:

- mutation correctness;
- compatibility with adjacent DF or DFHack versions;
- protection against a compromised local host;
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

Finally create a proposed registry generation:

```bash
python3 scripts/promote_live_compatibility.py \
  --registry architecture/live_compatibility_registry_v1.json \
  --live-receipt /path/to/live-read-acceptance-receipt.json \
  --native-receipt /path/to/dfhack-plugin-qualification.json \
  --evidence-locator qualification/<run>/live-read-acceptance-receipt.json \
  --output /tmp/live_compatibility_registry_v1.json
```

Review the proposed entry and its evidence bundle before replacing the checked-in registry. Use
`--in-place` only from the clean authoritative checkout after that review.

Promotion fails closed when:

- the live receipt is synthetic, dirty, development-only, malformed, or digest-inconsistent;
- any R2-R5 gate is absent, reordered, or not passed;
- the native receipt is absent, dirty, malformed, or bound to different commits or binary bytes;
- string or symbol inventory was skipped;
- a mutation RPC appears;
- the protocol is not exactly V1;
- an entry identifier does not reproduce its canonical fields;
- the exact source/binary/version/platform tuple already has a canonical entry;
- the evidence locator is unbounded or traversal-bearing.

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
