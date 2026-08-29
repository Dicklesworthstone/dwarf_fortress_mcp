# ATP State, Checkpoint, and Evidence Plane

ATP is used to move immutable, verified object graphs. It is not the command channel for game mutation.

## 1. Object classes

Domain-separated object classes include:

- world anchors and observation capsule runs;
- canonical snapshot chunks;
- graph/search/knowledge generations;
- Dwarf Fortress save/checkpoint files;
- replay traces and deterministic crashpacks;
- qualification receipts, benchmark artifacts, and evidence ledgers;
- compatibility bundles and bridge schemas.

Object identity includes class, schema version, canonical length, and content digest. Equal bytes in different semantic classes do not collide at the protocol level.

## 2. Object graph

A transfer root names a manifest. A manifest contains sorted child descriptors:

```text
ObjectDescriptor {
    object_id,
    class,
    schema,
    length,
    encoding,
    source_symbols,
    repair_symbols,
    child_root,
}
```

Large objects are chunked deterministically. Optional compression is applied before fountain coding when measurements justify it. Encryption, when enabled, occurs under a versioned encrypt-then-code policy so repair operates on authenticated ciphertext.

## 3. Transfer lifecycle

```text
Created
→ Negotiating
→ FetchingManifest
→ FetchingObjects
→ VerifyingObjects
→ VerifyingClosure
→ PersistingRoot
→ Published
```

Any failure before `Published` leaves the prior root authoritative. A resume journal records verified objects and path receipts. Resumption never trusts an unverified partial chunk.

## 4. Path graph and racing

Candidate paths can include local disk, loopback bridge, LAN peer, remote build host, removable archive, or object store adapter. Paths declare cost, expected throughput, latency, trust, authorization, and supported object classes. Selection is a graph problem: one object may be sourced from multiple donors and reach the receiver through different relays.

The transfer controller may race paths for the manifest or initial symbols. Losers are cancelled through structured drain. Partial state is retained only if it is content-addressed, verified, and useful for resume.

## 5. RaptorQ policy

Fountain coding is appropriate for large durable artifacts that may be partially lost or fetched from multiple donors. It is not automatic for tiny control records. Policy chooses source symbol size and repair overhead by object class, failure model, and measured economics.

A decode success is followed by content verification. Decoder output is never accepted solely because enough symbols were present.

## 6. Generation and anti-rollback

Published remote roots are generation-monotone for a named lineage. A receiver rejects:

- a lower generation without an explicit rollback capability;
- a different root for an already sealed `(lineage, generation)`;
- a manifest whose parent chain is inconsistent;
- an object class outside the path capability;
- a root whose schema or policy is unsupported.

Intentional restore creates a new observation epoch rather than rewriting history.

## 7. Checkpoint graph

A checkpoint root names:

- fortress lineage and source anchor;
- bridge/DF/DFHack compatibility identity;
- ordered save-file descriptors;
- server ledger cutoff;
- required observation capsules;
- action/obligation/effect reconciliation state;
- graph/search generations if retained;
- restore instructions and policy;
- evidence and signatures.

The checkpoint becomes discoverable only after graph closure and durability verification. Derived generations may be omitted because they are rebuildable; the manifest says so explicitly.

## 8. Proof of retrievability

Retained roots may be periodically sampled. Challenges are unpredictable under the audit seed, responses are verified against object identity, and results are appended to the evidence ledger. Audit failure schedules repair or re-replication and marks durability degraded. Sampling is evidence, not proof that every byte is currently readable; policy states the statistical guarantee.

## 9. Remote workers

Remote workers may receive immutable snapshots and produce derived graph/search/benchmark artifacts. Their output is untrusted until verified. They do not receive mutation authority merely because they possess a state root. Capability tokens bind object class, lineage, generation range, operation, expiry, and byte budget.

## 10. Mutation exclusion

Non-idempotent game effects use the bridge operation protocol, not ATP. The reasons are fundamental:

- ATP intentionally tolerates duplication and reordering of symbols;
- a completed object transfer says nothing about game-thread execution;
- effect lookup and reconciliation need operation identity, not content reachability;
- eventual delivery can be dangerous after plan expiry;
- mutation authority must remain single-coordinator and lease-fenced.

ATP may transport an immutable plan or evidence package, but a separate authorized commit converts it into an effect.

## 11. Fault campaign

Required tests include corrupt symbols, valid symbols for the wrong object, manifest truncation, child omission, duplicate donors, path loss, crash after child persistence but before root publication, journal truncation, stale generation, malicious rollback, wrong capability, decode with insufficient symbols, disk-full during root commit, and successful reconstruction from independent donor subsets.
