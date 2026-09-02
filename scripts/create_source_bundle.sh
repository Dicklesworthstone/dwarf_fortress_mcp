#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -t 1 ]]; then
  BLUE='\033[1;34m'; GREEN='\033[1;32m'; RED='\033[1;31m'; RESET='\033[0m'
else
  BLUE=''; GREEN=''; RED=''; RESET=''
fi
info() { printf '%b==>%b %s\n' "$BLUE" "$RESET" "$*"; }
ok() { printf '%bOK%b  %s\n' "$GREEN" "$RESET" "$*"; }
die() { printf '%bERROR%b %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage:
  scripts/create_source_bundle.sh [OUTPUT_DIR]

Creates a deterministic tar archive from the exact clean Git commit, writes a
canonical content manifest root-last, and independently verifies the archive,
manifest, and checkout. The bundle contains tracked regular files only and
rejects symlinks, submodules, special entries, dirty source, traversal, and
unmanifested archive content.
EOF
}

[[ $# -le 1 ]] || { usage >&2; exit 2; }
for command in git python3; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

CONTRACT="$ROOT/architecture/source_bundle_v1.json"
VERIFIER="$ROOT/scripts/verify_source_bundle.py"
STABLE_READER="$ROOT/scripts/read_stable_repository_file.py"
[[ -f "$CONTRACT" && -f "$VERIFIER" && -f "$STABLE_READER" ]] || \
  die "source bundle contract, verifier, or stable reader is missing"

COMMIT="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "HEAD is not a canonical 40-hex Git commit"
[[ "$TREE" =~ ^[0-9a-f]{40}$ ]] || die "HEAD tree is not a canonical 40-hex Git object"
[[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]] || \
  die "source bundle creation requires a clean worktree including untracked files"

OUT_DIR="${1:-$ROOT/target/source-bundle/$COMMIT}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
ARCHIVE_NAME="dwarf_fortress_mcp-$COMMIT.tar"
ARCHIVE="$OUT_DIR/$ARCHIVE_NAME"
MANIFEST="$OUT_DIR/source-bundle-manifest.json"
VERIFICATION="$OUT_DIR/source-bundle-verification.json"
ARCHIVE_TMP="$OUT_DIR/.$ARCHIVE_NAME.$$.tmp"
MANIFEST_TMP="$OUT_DIR/.source-bundle-manifest.$$.tmp"

cleanup() {
  rm -f -- "$ARCHIVE_TMP" "$MANIFEST_TMP"
}
trap cleanup EXIT

[[ ! -e "$ARCHIVE" && ! -e "$MANIFEST" && ! -e "$VERIFICATION" ]] || \
  die "source bundle destination already contains a published artifact"

info "Creating deterministic archive from commit $COMMIT"
git archive --format=tar --prefix="dwarf_fortress_mcp-$COMMIT/" "$COMMIT" > "$ARCHIVE_TMP"
python3 - "$ARCHIVE_TMP" <<'PY'
from __future__ import annotations
import os, sys
from pathlib import Path
path=Path(sys.argv[1])
with path.open('rb') as handle:
    os.fsync(handle.fileno())
PY

info "Building canonical source manifest"
ROOT="$ROOT" CONTRACT="$CONTRACT" VERIFIER="$VERIFIER" ARCHIVE_TMP="$ARCHIVE_TMP" \
ARCHIVE_NAME="$ARCHIVE_NAME" COMMIT="$COMMIT" TREE="$TREE" MANIFEST_TMP="$MANIFEST_TMP" \
python3 - <<'PY'
from __future__ import annotations
import importlib.util, json, os, sys
from pathlib import Path
module_path=Path(os.environ['VERIFIER'])
spec=importlib.util.spec_from_file_location('verify_source_bundle', module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load source bundle verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
root=Path(os.environ['ROOT'])
contract=module.load_contract(Path(os.environ['CONTRACT']))
commit=module.require_commit(os.environ['COMMIT'],'commit')
tree=module.require_commit(os.environ['TREE'],'tree')
actual_tree, entries=module.git_entries(root,commit)
if actual_tree != tree:
    raise SystemExit('Git tree changed during source bundle creation')
archive=module.read_stable_regular_file(
    Path(os.environ['ARCHIVE_TMP']),
    module.require_positive_int(contract['archive']['maximum_bytes'],'contract.archive.maximum_bytes'),
    'candidate source archive',
)
unsigned={
    'schema':module.MANIFEST_SCHEMA,
    'status':'created',
    'repository':'dwarf_fortress_mcp',
    'commit':commit,
    'tree':tree,
    'archive':{
        'name':module.validate_basename(os.environ['ARCHIVE_NAME'],'archive.name'),
        'format':'tar',
        'prefix':f'dwarf_fortress_mcp-{commit}/',
        'bytes':archive.size,
        'sha256':archive.sha256,
    },
    'entries':entries,
    'entries_digest':module.sha256_bytes(module.canonical_json(entries)),
    'claims_not_established':contract['claims_not_established'],
}
manifest={**unsigned,'manifest_digest':module.sha256_bytes(module.canonical_json(unsigned))}
payload=json.dumps(manifest,indent=2,sort_keys=True,ensure_ascii=False)+'\n'
path=Path(os.environ['MANIFEST_TMP'])
flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
if hasattr(os,'O_CLOEXEC'): flags|=os.O_CLOEXEC
fd=os.open(path,flags,0o600)
with os.fdopen(fd,'w',encoding='utf-8',newline='\n') as handle:
    handle.write(payload)
    handle.flush()
    os.fsync(handle.fileno())
PY

info "Publishing archive, then manifest root"
mv -- "$ARCHIVE_TMP" "$ARCHIVE"
mv -- "$MANIFEST_TMP" "$MANIFEST"
python3 - "$OUT_DIR" <<'PY'
from __future__ import annotations
import os, sys
from pathlib import Path
path=Path(sys.argv[1])
flags=os.O_RDONLY|getattr(os,'O_DIRECTORY',0)
fd=os.open(path,flags)
try: os.fsync(fd)
finally: os.close(fd)
PY

info "Independently verifying archive, manifest, and exact checkout"
python3 "$VERIFIER" "$MANIFEST" "$ARCHIVE" \
  --contract "$CONTRACT" \
  --source-root "$ROOT" \
  --output "$VERIFICATION"

[[ -z "$(git status --porcelain=v1 --untracked-files=no)" ]] || \
  die "tracked source changed during source bundle creation"

trap - EXIT
cleanup
ok "Source bundle created and verified"
printf 'Archive:      %s\nManifest:     %s\nVerification: %s\n' \
  "$ARCHIVE" "$MANIFEST" "$VERIFICATION"
