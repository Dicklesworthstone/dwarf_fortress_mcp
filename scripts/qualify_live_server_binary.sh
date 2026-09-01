#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -t 1 ]]; then
  BLUE='\033[1;34m'; GREEN='\033[1;32m'; YELLOW='\033[1;33m'; RED='\033[1;31m'; RESET='\033[0m'
else
  BLUE=''; GREEN=''; YELLOW=''; RED=''; RESET=''
fi
info() { printf '%b==>%b %s\n' "$BLUE" "$RESET" "$*"; }
ok() { printf '%bOK%b  %s\n' "$GREEN" "$RESET" "$*"; }
warn() { printf '%bWARN%b %s\n' "$YELLOW" "$RESET" "$*"; }
die() { printf '%bERROR%b %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage:
  scripts/qualify_live_server_binary.sh LOCAL_QUALIFICATION_RECEIPT.json [OUTPUT_DIR]

Builds the exact clean source revision's release `dwarf-fortress-mcp` binary,
runs the contract/doctor/demo executable checks, writes a source-bound binary
qualification receipt, and independently re-verifies the receipt plus opened
binary inode. This establishes only the Rust server artifact. It does not
establish DFHack R1 or live R2-R5 compatibility.
EOF
}

[[ $# -ge 1 && $# -le 2 ]] || { usage >&2; exit 2; }
for command in git cargo rustc python3; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

LOCAL_RECEIPT="$1"
[[ -f "$LOCAL_RECEIPT" ]] || die "local qualification receipt does not exist: $LOCAL_RECEIPT"
LOCAL_RECEIPT="$(cd "$(dirname "$LOCAL_RECEIPT")" && pwd)/$(basename "$LOCAL_RECEIPT")"
CONTRACT="$ROOT/architecture/live_server_binary_receipt_v1.json"
VERIFIER="$ROOT/scripts/verify_live_server_binary_receipt.py"
[[ -f "$CONTRACT" && -f "$VERIFIER" ]] || die "server artifact contract or verifier is missing"

COMMIT="$(git rev-parse HEAD)"
[[ -z "$(git status --porcelain=v1)" ]] || die "server artifact qualification requires a clean source tree"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${COMMIT:0:12}"
OUT_DIR="${2:-$ROOT/target/live-server-binary-qualification/$RUN_ID}"
mkdir -p "$OUT_DIR/logs"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
BINARY="$ROOT/target/release/dwarf-fortress-mcp"
CHECKS_TSV="$OUT_DIR/checks.tsv"
RECEIPT="$OUT_DIR/live-server-binary-receipt.json"
: > "$CHECKS_TSV"

info "Validating the prerequisite local qualification receipt"
python3 - "$VERIFIER" "$CONTRACT" "$LOCAL_RECEIPT" "$COMMIT" <<'PY'
from __future__ import annotations
import importlib.util, sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt', module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
contract=module.load_contract(Path(sys.argv[2]))
receipt_path=Path(sys.argv[3])
commit=sys.argv[4]
sha=module.sha256_file(receipt_path)
module.validate_local_qualification_receipt(
    receipt_path,
    commit,
    sha,
    contract['source_binding']['required_local_qualification_gates'],
)
PY
ok "Prerequisite local qualification receipt"

info "Building the exact release server binary"
cargo build --locked --release --bin dwarf-fortress-mcp \
  >"$OUT_DIR/logs/build.stdout" 2>"$OUT_DIR/logs/build.stderr"
[[ -f "$BINARY" ]] || die "release build did not produce $BINARY"

sha256_path() {
  python3 - "$1" <<'PY'
from __future__ import annotations
import hashlib, sys
from pathlib import Path
path=Path(sys.argv[1])
digest=hashlib.sha256()
with path.open('rb') as handle:
    for chunk in iter(lambda:handle.read(1024*1024),b''):
        digest.update(chunk)
print(digest.hexdigest())
PY
}

for check in contract doctor demo; do
  info "Running release binary check: $check"
  stdout="$OUT_DIR/logs/$check.stdout"
  stderr="$OUT_DIR/logs/$check.stderr"
  if ! "$BINARY" "$check" >"$stdout" 2>"$stderr"; then
    die "release binary check failed: $check"
  fi
  printf '%s\t%s\t%s\n' "$check" "$(sha256_path "$stdout")" "$(sha256_path "$stderr")" \
    >> "$CHECKS_TSV"
done

FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUSTC_VV_RAW="$(rustc -vV)"
CARGO_VERSION_RAW="$(cargo --version)"
info "Issuing the source-bound server binary receipt"
VERIFIER="$VERIFIER" CONTRACT="$CONTRACT" LOCAL_RECEIPT="$LOCAL_RECEIPT" \
COMMIT="$COMMIT" BINARY="$BINARY" CHECKS_TSV="$CHECKS_TSV" RECEIPT="$RECEIPT" \
STARTED_AT="$STARTED_AT" FINISHED_AT="$FINISHED_AT" RUSTC_VV_RAW="$RUSTC_VV_RAW" \
CARGO_VERSION_RAW="$CARGO_VERSION_RAW" python3 - <<'PY'
from __future__ import annotations
import importlib.util, json, os, platform, sys, tempfile
from pathlib import Path
module_path=Path(os.environ['VERIFIER'])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt', module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
contract=module.load_contract(Path(os.environ['CONTRACT']))
local_receipt=Path(os.environ['LOCAL_RECEIPT'])
commit=os.environ['COMMIT']
binary=Path(os.environ['BINARY'])
maximum=module.require_positive_int(contract['binary']['maximum_bytes'], 'contract.binary.maximum_bytes')
fd, metadata=module._open_stable_regular(binary, maximum, 'release server binary')
try:
    module.validate_open_metadata(metadata)
    binary_sha=module.sha256_descriptor(fd)
finally:
    os.close(fd)
checks=[]
for line in Path(os.environ['CHECKS_TSV']).read_text(encoding='utf-8').splitlines():
    name, stdout_sha, stderr_sha=line.split('\t')
    checks.append({
        'name':name,
        'status':'passed',
        'stdout_sha256':module.require_hash(stdout_sha, f'check.{name}.stdout'),
        'stderr_sha256':module.require_hash(stderr_sha, f'check.{name}.stderr'),
    })
if [item['name'] for item in checks] != contract['required_executable_checks']:
    raise SystemExit('executable check order differs from the artifact contract')
source_digests={}
root=module.ROOT
for name, relative in contract['source_binding']['required_source_digests'].items():
    path=root/relative
    if not path.is_file():
        raise SystemExit(f'missing source-bound file: {relative}')
    source_digests[name]=module.sha256_file(path)
unsigned={
    'schema':module.RECEIPT_SCHEMA,
    'status':'qualified',
    'source':{
        'dfmcp_commit':commit,
        'dfmcp_dirty':False,
        'local_qualification_receipt_sha256':module.sha256_file(local_receipt),
    },
    'platform':{'system':platform.system(),'machine':platform.machine()},
    'toolchain':{
        'rustc_vv':' | '.join(os.environ['RUSTC_VV_RAW'].splitlines()),
        'cargo':' | '.join(os.environ['CARGO_VERSION_RAW'].splitlines()),
    },
    'binary':{
        'name':contract['binary']['name'],
        'profile':'release',
        'relative_path':'target/release/dwarf-fortress-mcp',
        'bytes':metadata.st_size,
        'sha256':binary_sha,
    },
    'executable_checks':checks,
    'source_digests':source_digests,
    'mutation_capabilities':[],
    'claims_not_established':contract['claims_not_established'],
}
receipt={**unsigned,'receipt_digest':module.sha256_bytes(module.canonical_json(unsigned))}
destination=Path(os.environ['RECEIPT'])
payload=json.dumps(receipt,indent=2,sort_keys=True)+'\n'
descriptor, temporary=tempfile.mkstemp(prefix=f'.{destination.name}.',dir=destination.parent)
try:
    with os.fdopen(descriptor,'w',encoding='utf-8',newline='\n') as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary,destination)
    directory=os.open(destination.parent,os.O_RDONLY | getattr(os,'O_DIRECTORY',0))
    try: os.fsync(directory)
    finally: os.close(directory)
except BaseException:
    try: os.unlink(temporary)
    except OSError: pass
    raise
PY

info "Independently re-verifying the receipt and opened binary inode"
python3 "$VERIFIER" "$RECEIPT" "$BINARY" \
  --contract "$CONTRACT" \
  --source-root "$ROOT" \
  --local-qualification-receipt "$LOCAL_RECEIPT" \
  --expected-dfmcp-commit "$COMMIT" \
  >"$OUT_DIR/logs/verify.stdout" 2>"$OUT_DIR/logs/verify.stderr"

python3 - "$VERIFIER" "$RECEIPT" "$LOCAL_RECEIPT" "$BINARY" "$OUT_DIR/SHA256SUMS" <<'PY'
from __future__ import annotations
import importlib.util, sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt', module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
paths=[Path(value) for value in sys.argv[2:5]]
Path(sys.argv[5]).write_text(
    ''.join(f'{module.sha256_file(path)}  {path.name}\n' for path in paths),
    encoding='utf-8',
)
PY

ok "Live server binary qualification complete"
printf 'Binary:  %s\nReceipt: %s\nChecksums: %s\n' "$BINARY" "$RECEIPT" "$OUT_DIR/SHA256SUMS"
