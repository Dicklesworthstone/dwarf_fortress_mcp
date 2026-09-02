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
  scripts/qualify_live_announcement_source.sh [OUTPUT_DIR]

Produces a source-only protocol-1.1 qualification receipt for the exact clean
Git revision. This proves static and Rust gates, including the separately named
unadmitted development MCP runtime. It does not prove a native DFHack build,
live A1-A6 behavior, compatibility admission, server-artifact qualification,
or admitted runtime launch.
EOF
}

[[ $# -le 1 ]] || { usage >&2; exit 2; }
for command in python3 git cargo rustc; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

CONTRACT="$ROOT/architecture/live_announcement_source_qualification_v1_1.json"
[[ -f "$CONTRACT" ]] || die "source qualification contract is missing"
COMMIT="$(git rev-parse HEAD)"
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "HEAD is not a full lowercase Git commit"
[[ -z "$(git status --porcelain=v1)" ]] || die "announcement source qualification requires a clean worktree"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${COMMIT:0:12}"
OUT_DIR="${1:-$ROOT/target/live-announcement-source-qualification/$RUN_ID}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
GATES_FILE="$OUT_DIR/gates.tsv"
RECEIPT="$OUT_DIR/live-announcement-source-qualification.json"
: > "$GATES_FILE"

record() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$GATES_FILE"; }
run_gate() {
  local gate="$1"; shift
  info "$gate"
  if "$@"; then
    record "$gate" passed ""
    ok "$gate"
  else
    local status=$?
    record "$gate" failed "exit=$status"
    return "$status"
  fi
}

write_receipt() {
  local status="$1"
  local finished_at
  finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  CONTRACT="$CONTRACT" COMMIT="$COMMIT" STARTED_AT="$STARTED_AT" \
  FINISHED_AT="$finished_at" STATUS="$status" GATES_FILE="$GATES_FILE" \
  RECEIPT="$RECEIPT" python3 - <<'PY'
from __future__ import annotations
import hashlib,json,os,platform,subprocess,tempfile
from pathlib import Path
root=Path.cwd()
contract=json.loads(Path(os.environ['CONTRACT']).read_text(encoding='utf-8'))
def digest(path: Path) -> str:
    if not path.is_file():
        raise SystemExit(f'missing source-bound file: {path.relative_to(root)}')
    value=hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b''):
            value.update(chunk)
    return value.hexdigest()
def command_output(args: list[str]) -> str:
    return subprocess.run(args,check=True,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout.strip()
def canonical(value: object) -> bytes:
    return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode('utf-8')
gates=[]
for line in Path(os.environ['GATES_FILE']).read_text(encoding='utf-8').splitlines():
    name,state,detail=(line.split('\t',2)+['',''])[:3]
    gates.append({'name':name,'state':state,'detail':detail or None})
required=contract['required_gates']
if [gate['name'] for gate in gates] != required[:len(gates)]:
    raise SystemExit('qualification gate order differs from the source contract')
if os.environ['STATUS']=='passed':
    if [gate['name'] for gate in gates] != required or any(gate['state']!='passed' for gate in gates):
        raise SystemExit('passing receipt requires every exact source gate to pass')
digests={
    name:digest(root/relative)
    for name,relative in contract['required_source_digests'].items()
}
unsigned={
    'schema':contract['receipt_schema'],
    'status':os.environ['STATUS'],
    'started_at':os.environ['STARTED_AT'],
    'finished_at':os.environ['FINISHED_AT'],
    'source':{'commit':os.environ['COMMIT'],'dirty':False},
    'bridge':contract['bridge'],
    'host':{
        'system':platform.system(),
        'release':platform.release(),
        'machine':platform.machine(),
        'python':platform.python_version(),
    },
    'toolchain':{
        'rustc_vv':command_output(['rustc','-vV']),
        'cargo':command_output(['cargo','--version']),
    },
    'source_digests':digests,
    'gates':gates,
    'capabilities_granted':[],
    'mutation_capabilities':[],
    'claims_established':contract['claims_established'] if os.environ['STATUS']=='passed' else [],
    'claims_not_established':contract['claims_not_established'],
}
receipt={**unsigned,'receipt_digest':hashlib.sha256(canonical(unsigned)).hexdigest()}
destination=Path(os.environ['RECEIPT'])
payload=json.dumps(receipt,indent=2,sort_keys=True)+'\n'
fd,temporary=tempfile.mkstemp(prefix=f'.{destination.name}.',dir=destination.parent)
try:
    with os.fdopen(fd,'w',encoding='utf-8',newline='\n') as handle:
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
}
trap 'status=$?; if [[ $status -ne 0 ]]; then write_receipt failed >/dev/null 2>&1 || true; fi' EXIT

run_gate repository-integrity python3 scripts/check_repository_integrity.py
run_gate announcement-contract python3 scripts/check_live_announcements.py
run_gate announcement-contract-tests python3 scripts/test_live_announcement_contract.py
run_gate announcement-acceptance-tests python3 scripts/test_live_announcement_acceptance.py
run_gate announcement-mcp-contract python3 scripts/check_live_mcp_v1_1.py
run_gate announcement-mcp-contract-tests python3 scripts/test_live_mcp_v1_1.py
run_gate python-syntax python3 -m py_compile \
  scripts/check_live_announcements.py \
  scripts/check_live_announcements_core.py \
  scripts/check_live_announcement_publication.py \
  scripts/test_live_announcement_contract.py \
  scripts/verify_live_announcement_acceptance.py \
  scripts/test_live_announcement_acceptance.py \
  scripts/check_live_mcp_v1_1.py \
  scripts/test_live_mcp_v1_1.py
run_gate shell-syntax bash -n \
  scripts/qualify_dfhack_plugin_v1_1.sh \
  scripts/qualify_live_announcements.sh \
  scripts/qualify_live_announcement_source.sh
run_gate cargo-metadata cargo metadata --locked --offline --format-version 1
run_gate rustfmt cargo fmt --all -- --check
run_gate clippy cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_gate adapter-tests cargo test --locked -p dfmcp-adapter
run_gate announcement-mcp-process-tests cargo test --locked -p dwarf-fortress-mcp --test live_v1_1_development_admission
run_gate workspace-tests cargo test --locked --workspace --all-targets --all-features
run_gate rustdoc env RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --all-features --no-deps
run_gate announcement-probe-help cargo run --locked --quiet --bin dfmcp-live-announcement-probe -- help

write_receipt passed
trap - EXIT
python3 - "$RECEIPT" "$OUT_DIR/SHA256SUMS" <<'PY'
from __future__ import annotations
import hashlib,sys
from pathlib import Path
path=Path(sys.argv[1])
value=hashlib.sha256(path.read_bytes()).hexdigest()
Path(sys.argv[2]).write_text(f'{value}  {path.name}\n',encoding='utf-8')
PY
ok "Protocol-1.1 source qualification complete"
printf 'Receipt:   %s\nChecksums: %s\n' "$RECEIPT" "$OUT_DIR/SHA256SUMS"
