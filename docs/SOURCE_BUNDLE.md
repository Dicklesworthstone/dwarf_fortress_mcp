# Canonical source bundles

The release source bundle is a deterministic, manifest-sealed projection of one exact clean Git commit. It contains tracked regular files only, preserves executable mode, fixes archive metadata, rejects symlinks and submodules, and is independently verified without extracting archive members or executing project code.

The machine contract is `architecture/source_bundle_v1.json`. Creation and verification are performed by `scripts/create_source_bundle.py` and `scripts/verify_source_bundle.py`; `scripts/create_source_bundle.sh` is a strict wrapper.

A verified source bundle proves exact source content and archive integrity. It does not prove compilation, tests, DFHack compatibility, registry admission, binary reproducibility, signature authenticity, or hostile-host resistance.
