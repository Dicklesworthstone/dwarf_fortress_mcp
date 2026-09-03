#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
export PYTHONDONTWRITEBYTECODE=1

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

CONTRACT="$ROOT/architecture/live_server_binary_receipt_v1.json"
VERIFIER="$ROOT/scripts/verify_live_server_binary_receipt.py"
[[ -f "$CONTRACT" && -f "$VERIFIER" ]] || die "server artifact contract or verifier is missing"

LOCAL_RECEIPT_RAW="$1"
if [[ "$LOCAL_RECEIPT_RAW" != /* ]]; then
  LOCAL_RECEIPT_RAW="$PWD/$LOCAL_RECEIPT_RAW"
fi
LOCAL_RECEIPT_PARENT="$(dirname -- "$LOCAL_RECEIPT_RAW")"
LOCAL_RECEIPT_NAME="$(basename -- "$LOCAL_RECEIPT_RAW")"
[[ -n "$LOCAL_RECEIPT_NAME" && "$LOCAL_RECEIPT_NAME" != . && "$LOCAL_RECEIPT_NAME" != .. ]] || \
  die "local qualification receipt name is invalid"
LOCAL_RECEIPT_PARENT="$(cd -P -- "$LOCAL_RECEIPT_PARENT" && pwd)"
LOCAL_RECEIPT="$LOCAL_RECEIPT_PARENT/$LOCAL_RECEIPT_NAME"
[[ -e "$LOCAL_RECEIPT" || -L "$LOCAL_RECEIPT" ]] || \
  die "local qualification receipt does not exist: $LOCAL_RECEIPT"

COMMIT="$(python3 - "$VERIFIER" "$ROOT" <<'PY'
from __future__ import annotations
import importlib.util,sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt_commit',module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
print(module.git_text(Path(sys.argv[2]),['rev-parse','HEAD'],'Git HEAD'))
PY
)"
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "HEAD is not a full lowercase Git commit"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${COMMIT:0:12}"
RAW_OUT_DIR="${2:-$ROOT/target/live-server-binary-qualification/$RUN_ID}"
if [[ "$RAW_OUT_DIR" != /* ]]; then
  RAW_OUT_DIR="$ROOT/$RAW_OUT_DIR"
fi
OUT_PARENT="$(dirname -- "$RAW_OUT_DIR")"
RUN_NAME="$(basename -- "$RAW_OUT_DIR")"
[[ -n "$RUN_NAME" && "$RUN_NAME" != . && "$RUN_NAME" != .. ]] || \
  die "server qualification run directory name is invalid"
mkdir -p "$OUT_PARENT"
OUT_PARENT="$(cd -P -- "$OUT_PARENT" && pwd)"
OUT_DIR="$OUT_PARENT/$RUN_NAME"
[[ ! -e "$OUT_DIR" && ! -L "$OUT_DIR" ]] || \
  die "server qualification run directory already exists: $OUT_DIR"
mkdir -m 0700 "$OUT_DIR"
mkdir -m 0700 "$OUT_DIR/logs"

BINARY="$ROOT/target/release/dwarf-fortress-mcp"
CHECKS_TSV="$OUT_DIR/checks.tsv"
RECEIPT="$OUT_DIR/live-server-binary-receipt.json"
CHECKSUMS="$OUT_DIR/SHA256SUMS"
: > "$CHECKS_TSV"
chmod 0600 "$CHECKS_TSV"

cleanup_invalid_evidence() {
  rm -f -- "$RECEIPT" "$CHECKSUMS"
}
trap 'status=$?; if [[ $status -ne 0 ]]; then cleanup_invalid_evidence; fi' EXIT

validate_local_receipt() {
  python3 - "$VERIFIER" "$CONTRACT" "$LOCAL_RECEIPT" "$ROOT" "$COMMIT" <<'PY'
from __future__ import annotations
import importlib.util,sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt_prerequisite',module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
contract=module.load_contract(Path(sys.argv[2]))
receipt_path=Path(sys.argv[3])
source_root=Path(sys.argv[4])
commit=sys.argv[5]
sha=module.sha256_file(receipt_path)
module.validate_local_qualification_receipt(
    receipt_path,
    source_root,
    commit,
    sha,
    contract['source_binding']['required_local_qualification_gates'],
)
PY
}

info "Validating the prerequisite local qualification receipt and exact source inventory"
validate_local_receipt
ok "Prerequisite local qualification receipt"

info "Building the exact release server binary"
cargo build --locked --release --bin dwarf-fortress-mcp \
  >"$OUT_DIR/logs/build.stdout" 2>"$OUT_DIR/logs/build.stderr"
[[ -f "$BINARY" ]] || die "release build did not produce $BINARY"

sha256_path() {
  python3 - "$VERIFIER" "$1" <<'PY'
from __future__ import annotations
import importlib.util,sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt_digest',module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
print(module.sha256_file(Path(sys.argv[2])))
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

info "Revalidating source after build and executable checks"
validate_local_receipt
ok "Source remained identical to the prerequisite receipt"

FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUSTC_VV_RAW="$(rustc -vV)"
CARGO_VERSION_RAW="$(cargo --version)"
info "Issuing the create-only source-bound server binary receipt"
VERIFIER="$VERIFIER" CONTRACT="$CONTRACT" LOCAL_RECEIPT="$LOCAL_RECEIPT" \
COMMIT="$COMMIT" BINARY="$BINARY" CHECKS_TSV="$CHECKS_TSV" RECEIPT="$RECEIPT" \
STARTED_AT="$STARTED_AT" FINISHED_AT="$FINISHED_AT" RUSTC_VV_RAW="$RUSTC_VV_RAW" \
CARGO_VERSION_RAW="$CARGO_VERSION_RAW" python3 - <<'PY'
from __future__ import annotations
import importlib.util,json,os,platform,sys,tempfile
from pathlib import Path
module_path=Path(os.environ['VERIFIER'])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt_issue',module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
contract=module.load_contract(Path(os.environ['CONTRACT']))
local_receipt=Path(os.environ['LOCAL_RECEIPT'])
commit=os.environ['COMMIT']
binary=Path(os.environ['BINARY'])
root=module.ROOT
local_sha=module.sha256_file(local_receipt)
module.validate_local_qualification_receipt(
    local_receipt,
    root,
    commit,
    local_sha,
    contract['source_binding']['required_local_qualification_gates'],
)
maximum=module.require_positive_int(contract['binary']['maximum_bytes'],'contract.binary.maximum_bytes')
fd,metadata=module.open_stable_regular(binary,maximum,'release server binary')
try:
    module.validate_open_metadata(metadata)
    binary_sha=module.sha256_descriptor(fd)
finally:
    os.close(fd)
checks=[]
for line in Path(os.environ['CHECKS_TSV']).read_text(encoding='utf-8').splitlines():
    name,stdout_sha,stderr_sha=line.split('\t')
    checks.append({
        'name':name,
        'status':'passed',
        'stdout_sha256':module.require_hash(stdout_sha,f'check.{name}.stdout'),
        'stderr_sha256':module.require_hash(stderr_sha,f'check.{name}.stderr'),
    })
if [item['name'] for item in checks] != contract['required_executable_checks']:
    raise SystemExit('executable check order differs from the artifact contract')
source_digests={}
for name,relative in contract['source_binding']['required_source_digests'].items():
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
        'local_qualification_receipt_sha256':local_sha,
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
payload=(json.dumps(receipt,indent=2,sort_keys=True,ensure_ascii=False)+'\n').encode('utf-8')
descriptor,temporary=tempfile.mkstemp(prefix=f'.{destination.name}.',dir=destination.parent)
published=False
try:
    os.fchmod(descriptor,0o600)
    with os.fdopen(descriptor,'wb',closefd=True) as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())
    os.link(temporary,destination,follow_symlinks=False)
    published=True
    os.unlink(temporary)
    temporary=''
    directory=os.open(destination.parent,os.O_RDONLY | getattr(os,'O_DIRECTORY',0))
    try: os.fsync(directory)
    finally: os.close(directory)
    raw,digest,metadata=module.read_stable_bytes_with_metadata(
        destination,
        'published server receipt',
        module.MAX_JSON_BYTES,
    )
    if raw != payload or digest != module.sha256_bytes(payload):
        raise SystemExit('published server receipt bytes differ from the prepared payload')
    if os.name=='posix' and metadata.st_mode & 0o777 != 0o600:
        raise SystemExit('published server receipt does not have exact mode 0600')
except BaseException:
    if published:
        try: os.unlink(destination)
        except OSError: pass
    if temporary:
        try: os.unlink(temporary)
        except OSError: pass
    raise
PY

info "Independently re-verifying the receipt, source inventory, and opened binary inode"
python3 "$VERIFIER" "$RECEIPT" "$BINARY" \
  --contract "$CONTRACT" \
  --source-root "$ROOT" \
  --local-qualification-receipt "$LOCAL_RECEIPT" \
  --expected-dfmcp-commit "$COMMIT" \
  >"$OUT_DIR/logs/verify.stdout" 2>"$OUT_DIR/logs/verify.stderr"

python3 - "$VERIFIER" "$RECEIPT" "$LOCAL_RECEIPT" "$BINARY" "$CHECKSUMS" <<'PY'
from __future__ import annotations
import importlib.util,os,sys
from pathlib import Path
module_path=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('verify_live_server_binary_receipt_checksums',module_path)
if spec is None or spec.loader is None:
    raise SystemExit('cannot load server artifact verifier')
module=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=module
spec.loader.exec_module(module)
paths=[Path(value) for value in sys.argv[2:5]]
destination=Path(sys.argv[5])
payload=''.join(f'{module.sha256_file(path)}  {path.name}\n' for path in paths).encode('utf-8')
flags=os.O_WRONLY | os.O_CREAT | os.O_EXCL
if hasattr(os,'O_CLOEXEC'): flags |= os.O_CLOEXEC
if hasattr(os,'O_NOFOLLOW'): flags |= os.O_NOFOLLOW
descriptor=os.open(destination,flags,0o600)
try:
    with os.fdopen(descriptor,'wb',closefd=True) as handle:
        handle.write(payload)
        handle.flush()
        os.fchmod(handle.fileno(),0o600)
        os.fsync(handle.fileno())
    directory=os.open(destination.parent,os.O_RDONLY | getattr(os,'O_DIRECTORY',0))
    try: os.fsync(directory)
    finally: os.close(directory)
except BaseException:
    try: os.unlink(destination)
    except OSError: pass
    raise
PY

trap - EXIT
ok "Live server binary qualification complete"
printf 'Binary:    %s\nReceipt:   %s\nChecksums: %s\n' "$BINARY" "$RECEIPT" "$CHECKSUMS"
