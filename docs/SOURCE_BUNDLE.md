# Canonical source bundles

A release source bundle is a deterministic, manifest-sealed projection of one **exact clean Git
commit**. It is not a snapshot of whatever happens to be on disk and it is not produced by copying
the worktree. Tracked Git blobs and their executable bits are the authority.

The machine contract is `architecture/source_bundle_v1.json`. The authoritative implementation is:

```text
scripts/read_stable_repository_file.py
scripts/create_source_bundle.py
scripts/verify_source_bundle.py
scripts/check_source_bundle.py
scripts/test_source_bundle.py
```

`scripts/create_source_bundle.sh` is only a strict command-line wrapper. It deliberately contains no
second archive, manifest, or publication implementation.

## Creation model

Creation requires a clean checkout including untracked files and resolves one exact `HEAD` commit
and tree. It rejects tracked symbolic links, gitlinks/submodules, and any tracked entry other than a
regular `100644` or `100755` blob.

The creator runs Git with `tar.umask=0022`, yielding canonical archive modes:

```text
directories       0755
regular files     0644
executable files  0755
```

It writes into a **sibling staging directory** of the final destination. The archive and manifest
are independently verified against the exact Git object database and the still-clean checkout
before any final path becomes visible. Only after all checks pass is the complete staging directory
**published atomically** with one directory rename and a parent-directory `fsync`.

A failed build removes the staging directory and leaves no partial published bundle. An existing
output directory is never overwritten.

## Archive identity

The archive is an uncompressed Git tar stream with one prefix:

```text
dwarf_fortress_mcp-<40-hex-commit>/
```

Its semantic member sequence is deterministic UTF-8 byte order over the root directory, required
parent directories, and tracked files. The verifier rejects:

- an absolute, parent-traversing, noncanonical, or out-of-prefix path;
- a symbolic link, hard link, device, FIFO, socket, or unsupported member type;
- a **semantic duplicate**, including directory spellings that normalize to the same path;
- a missing, extra, or reordered member;
- mode, owner, group, size, content, or device-metadata drift;
- mixed member timestamps;
- unsupported PAX metadata or a PAX commit comment that does not equal the manifest commit;
- nonzero trailing payload after the canonical tar end marker.

Every regular member is hashed while read directly from the archive. The verifier never extracts
members and never executes project code.

## Manifest and verification receipt

The manifest binds:

```text
commit + tree
archive basename + size + SHA-256 + prefix
ordered file paths + Git modes + sizes + SHA-256 digests
entries digest
claims-not-established
manifest digest
```

No timestamp, absolute path, machine-local path, token, or mutable output location enters canonical
manifest identity. The verification receipt separately records the manifest file digest, archive
facts, optional checkout reconciliation, explicit `clean` and `clean_required` state, authority, and
its own canonical digest. Receipt publication is create-only.

The source-bundle contract and adversarial suite are part of normal repository verification and
local qualification. Their source digests are recorded by `qualify_local.sh`. They remain a release
packaging boundary, however, and are intentionally not folded into the narrower live-server binary
receipt, whose job is to qualify one executable and its admission machinery.

## Commands

From a clean checkout:

```bash
scripts/create_source_bundle.sh
```

Or choose a new absolute destination:

```bash
scripts/create_source_bundle.sh /private/release/source-bundle-<commit>
```

Independent verification can be repeated without extraction:

```bash
python3 scripts/verify_source_bundle.py \
  /path/to/source-bundle-manifest.json \
  /path/to/dwarf_fortress_mcp-<commit>.tar \
  --source-root /path/to/exact/checkout \
  --require-clean-source
```

## Evidence boundary

A verified source bundle proves exact source content, archive structure, canonical metadata,
manifest integrity, and, when requested, agreement with one clean checkout. It **does not prove
compilation** and does not prove tests, latest-nightly qualification, DFHack compatibility,
compatibility-registry admission, binary reproducibility, release-signature authenticity, live
operation, mutation correctness, or hostile-host resistance.
