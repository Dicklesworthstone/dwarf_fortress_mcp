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

for command in git python3 cargo rustc; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done
[[ $# -le 1 ]] || die "usage: scripts/qualify_live_announcement_generation.sh [OUTPUT_DIR]"

COMMIT="$(git rev-parse HEAD)"
[[ -z "$(git status --porcelain=v1)" ]] || die "announcement source qualification requires a clean worktree"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${COMMIT:0:12}"
OUT_DIR="${1:-$ROOT/target/live-announcement-qualification/$RUN_ID}"
mkdir -p "$OUT_DIR/logs"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

run_gate() {
  local name="$1"; shift
  info "$name"
  "$@" >"$OUT_DIR/logs/$name.stdout" 2>"$OUT_DIR/logs/$name.stderr"
  ok "$name"
}

run_gate contract-check python3 scripts/check_live_announcement_contract.py
run_gate contract-tests python3 scripts/test_live_announcement_contract.py
run_gate rustfmt cargo fmt --all -- --check
run_gate adapter-tests cargo test --locked -p dfmcp-adapter live_announcements
run_gate adapter-clippy cargo clippy --locked -p dfmcp-adapter --all-targets --all-features -- -D warnings

FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
STARTED_AT="$STARTED_AT" FINISHED_AT="$FINISHED_AT" COMMIT="$COMMIT" OUT_DIR="$OUT_DIR" \
python3 - <<'PY'
from __future__ import annotations
import hashlib, json, os, platform, subprocess, tempfile
from pathlib import Path
root=Path.cwd()
def digest(path: Path) -> str:
    value=hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b''):
            value.update(chunk)
    return value.hexdigest()
def command(args: list[str]) -> str:
    return subprocess.run(args,check=True,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout.strip()
files={
    'contract':'architecture/live_announcement_read_v1.json',
    'design':'docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md',
    'agent_semantics':'docs/ANNOUNCEMENT_WINDOW_AGENT_SEMANTICS.md',
    'protobuf':'bridge/dfhack-plugin/proto/DfmcpBridge.proto',
    'assembler':'crates/dfmcp-adapter/src/live_announcements.rs',
    'checker':'scripts/check_live_announcement_contract.py',
    'checker_tests':'scripts/test_live_announcement_contract.py',
    'qualification_wrapper':'scripts/qualify_live_announcement_generation.sh',
}
receipt={
    'schema':'dfmcp.live-announcement-source-qualification/1',
    'status':'source-qualified-not-live-admitted',
    'started_at':os.environ['STARTED_AT'],
    'finished_at':os.environ['FINISHED_AT'],
    'source':{'commit':os.environ['COMMIT'],'dirty':False},
    'host':{
        'system':platform.system(),
        'machine':platform.machine(),
        'python':platform.python_version(),
    },
    'toolchain':{'rustc_vv':command(['rustc','-vV']),'cargo':command(['cargo','--version'])},
    'digests':{name:digest(root/relative) for name,relative in files.items()},
    'gates':['contract-check','contract-tests','rustfmt','adapter-tests','adapter-clippy'],
    'authority':{'effect':'read_only','mutation_capabilities':[]},
    'claims_not_established':[
        'native DFHack announcement extraction',
        'safe-Rust announcement protobuf decoding',
        'canonical world-event projection',
        'MCP announcement exposure',
        'disposable-fort live evidence',
        'compatibility admission',
    ],
}
destination=Path(os.environ['OUT_DIR'])/'live-announcement-source-qualification.json'
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
print(destination)
PY

ok "Announcement source qualification complete"
printf 'Receipt: %s\n' "$OUT_DIR/live-announcement-source-qualification.json"
