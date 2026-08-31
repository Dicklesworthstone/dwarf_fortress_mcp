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
  scripts/qualify_dfhack_plugin.sh /path/to/dfhack-source [extra cmake configure args...]

Environment:
  DFMCP_DFHACK_REF             DFHack revision to build (default: source checkout HEAD)
  DFMCP_DFHACK_QUAL_DIR        Receipt/artifact root (default: target/dfhack-qualification/<run>)
  DFMCP_DFHACK_BUILD_JOBS      Parallel build jobs (default: detected CPU count or 2)
  DFMCP_DFHACK_SKIP_SUBMODULES Set to 1 only when every dependency is already usable
  DFMCP_DFHACK_KEEP_WORKTREE   Set to 1 to preserve the isolated worktree after the run

The script never installs into a live Dwarf Fortress/DFHack tree. It creates a detached git
worktree at an exact DFHack commit, stages bridge/dfhack-plugin under DFHack's documented
plugins/external/ seam, registers it with add_subdirectory(dfmcp_bridge), builds only the plugin
target, fingerprints the output, and writes a machine-readable receipt.
EOF
}

[[ $# -ge 1 ]] || { usage >&2; exit 2; }
DFHACK_SOURCE="$1"
shift
CMAKE_EXTRA_ARGS=("$@")

for command in git cmake python3; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done
[[ -d "$DFHACK_SOURCE" ]] || die "DFHack source directory does not exist: $DFHACK_SOURCE"
git -C "$DFHACK_SOURCE" rev-parse --is-inside-work-tree >/dev/null 2>&1 || \
  die "DFHack source must be a git worktree"

python3 scripts/check_dfhack_bridge.py

DFMCP_COMMIT="$(git rev-parse HEAD)"
DFMCP_DIRTY=false
[[ -z "$(git status --porcelain=v1)" ]] || DFMCP_DIRTY=true
if [[ "$DFMCP_DIRTY" == true && "${DFMCP_ALLOW_DIRTY:-0}" != 1 ]]; then
  die "bridge qualification requires a clean dwarf_fortress_mcp tree"
fi

DFHACK_REF="${DFMCP_DFHACK_REF:-HEAD}"
DFHACK_COMMIT="$(git -C "$DFHACK_SOURCE" rev-parse "$DFHACK_REF^{commit}")"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${DFMCP_COMMIT:0:12}-${DFHACK_COMMIT:0:12}"
OUT_DIR="${DFMCP_DFHACK_QUAL_DIR:-$ROOT/target/dfhack-qualification/$RUN_ID}"
WORKTREE="$OUT_DIR/dfhack-worktree"
BUILD_DIR="$OUT_DIR/build"
LOG_DIR="$OUT_DIR/logs"
EXTERNAL_DIR="$WORKTREE/plugins/external"
PLUGIN_DST="$EXTERNAL_DIR/dfmcp_bridge"
EXTERNAL_CMAKE="$EXTERNAL_DIR/CMakeLists.txt"
mkdir -p "$OUT_DIR" "$LOG_DIR"

cleanup() {
  local status=$?
  trap - EXIT
  if [[ "${DFMCP_DFHACK_KEEP_WORKTREE:-0}" != 1 && -d "$WORKTREE" ]]; then
    git -C "$DFHACK_SOURCE" worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true
  elif [[ -d "$WORKTREE" ]]; then
    warn "Preserving isolated DFHack worktree: $WORKTREE"
  fi
  exit "$status"
}
trap cleanup EXIT

info "Creating isolated DFHack worktree at $DFHACK_COMMIT"
git -C "$DFHACK_SOURCE" worktree add --detach "$WORKTREE" "$DFHACK_COMMIT" \
  >"$LOG_DIR/worktree.log" 2>&1

if [[ "${DFMCP_DFHACK_SKIP_SUBMODULES:-0}" != 1 ]]; then
  info "Initializing DFHack submodules in the isolated worktree"
  git -C "$WORKTREE" submodule update --init --recursive \
    >"$LOG_DIR/submodules.log" 2>&1
else
  warn "Submodule initialization explicitly skipped"
fi

info "Registering dfmcp_bridge through DFHack's external-plugin seam"
mkdir -p "$EXTERNAL_DIR"
[[ ! -e "$PLUGIN_DST" ]] || die "isolated DFHack tree already contains external/dfmcp_bridge"
mkdir -p "$PLUGIN_DST"
cp -R "$ROOT/bridge/dfhack-plugin/." "$PLUGIN_DST/"
if [[ -e "$EXTERNAL_CMAKE" ]]; then
  if grep -Eq '^[[:space:]]*add_subdirectory\([[:space:]]*dfmcp_bridge[[:space:]]*\)[[:space:]]*$' "$EXTERNAL_CMAKE"; then
    die "isolated DFHack external registry already contains dfmcp_bridge"
  fi
  printf '\n# Injected by dwarf_fortress_mcp native qualification.\nadd_subdirectory(dfmcp_bridge)\n' \
    >> "$EXTERNAL_CMAKE"
else
  cat > "$EXTERNAL_CMAKE" <<'EOF'
# Generated in an isolated DFHack worktree by dwarf_fortress_mcp qualification.
add_subdirectory(dfmcp_bridge)
EOF
fi

grep -Eq '^[[:space:]]*add_subdirectory\([[:space:]]*dfmcp_bridge[[:space:]]*\)[[:space:]]*$' \
  "$EXTERNAL_CMAKE" || die "dfmcp_bridge was not registered in external/CMakeLists.txt"

JOBS="${DFMCP_DFHACK_BUILD_JOBS:-}"
if [[ -z "$JOBS" ]]; then
  if command -v nproc >/dev/null 2>&1; then
    JOBS="$(nproc)"
  elif command -v sysctl >/dev/null 2>&1; then
    JOBS="$(sysctl -n hw.ncpu 2>/dev/null || printf 2)"
  else
    JOBS=2
  fi
fi
[[ "$JOBS" =~ ^[1-9][0-9]*$ ]] || die "DFMCP_DFHACK_BUILD_JOBS must be a positive integer"

info "Configuring the isolated DFHack plugin build"
cmake -S "$WORKTREE" -B "$BUILD_DIR" \
  -DBUILD_PLUGINS=ON \
  -DBUILD_DOCS=OFF \
  "${CMAKE_EXTRA_ARGS[@]}" \
  >"$LOG_DIR/configure.log" 2>&1

info "Building only the dfmcp_bridge target"
cmake --build "$BUILD_DIR" --target dfmcp_bridge --parallel "$JOBS" \
  >"$LOG_DIR/build.log" 2>&1

mapfile -t CANDIDATES < <(
  find "$BUILD_DIR" -type f \( \
    -name 'dfmcp_bridge.plug.so' -o \
    -name 'dfmcp_bridge.plug.dylib' -o \
    -name 'dfmcp_bridge.plug.dll' -o \
    -name 'dfmcp_bridge.so' -o \
    -name 'dfmcp_bridge.dylib' -o \
    -name 'dfmcp_bridge.dll' \
  \) -print | LC_ALL=C sort
)
[[ ${#CANDIDATES[@]} -eq 1 ]] || \
  die "expected exactly one dfmcp_bridge binary, found ${#CANDIDATES[@]} (see $BUILD_DIR)"
PLUGIN_BINARY="${CANDIDATES[0]}"

if command -v sha256sum >/dev/null 2>&1; then
  PLUGIN_SHA256="$(sha256sum "$PLUGIN_BINARY" | awk '{print $1}')"
else
  command -v shasum >/dev/null 2>&1 || die "sha256sum or shasum is required"
  PLUGIN_SHA256="$(shasum -a 256 "$PLUGIN_BINARY" | awk '{print $1}')"
fi
PLUGIN_SIZE="$(wc -c < "$PLUGIN_BINARY" | tr -d '[:space:]')"

STRINGS_LOG="$LOG_DIR/plugin-strings.txt"
STRINGS_STATUS=skipped
if command -v strings >/dev/null 2>&1; then
  strings "$PLUGIN_BINARY" | LC_ALL=C sort -u > "$STRINGS_LOG"
  for required in Handshake ReadObservation dfmcp_bridge; do
    grep -Fq "$required" "$STRINGS_LOG" || \
      die "plugin binary lacks required bridge marker: $required"
  done
  if grep -Eq 'dfmcp\.bridge\.v1\.(Pause|Resume|Dig|Teleport|RunCommand|RunLua|Mutate|ApplyEffect)' \
    "$STRINGS_LOG"; then
    die "plugin binary contains a forbidden dfmcp mutation descriptor"
  fi
  STRINGS_STATUS=passed
else
  warn "strings is unavailable; binary string inventory is recorded as skipped"
  : > "$STRINGS_LOG"
fi

SYMBOLS_LOG="$LOG_DIR/plugin-symbols.txt"
SYMBOLS_STATUS=skipped
if command -v nm >/dev/null 2>&1; then
  if nm -a "$PLUGIN_BINARY" > "$SYMBOLS_LOG" 2>&1; then
    SYMBOLS_STATUS=passed
  else
    warn "nm could not inspect the plugin binary"
  fi
else
  warn "nm is unavailable; symbol inventory is recorded as skipped"
  : > "$SYMBOLS_LOG"
fi

FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RECEIPT="$OUT_DIR/dfhack-plugin-qualification.json"
STARTED_AT="$STARTED_AT" FINISHED_AT="$FINISHED_AT" DFMCP_COMMIT="$DFMCP_COMMIT" \
DFMCP_DIRTY="$DFMCP_DIRTY" DFHACK_COMMIT="$DFHACK_COMMIT" PLUGIN_BINARY="$PLUGIN_BINARY" \
PLUGIN_SHA256="$PLUGIN_SHA256" PLUGIN_SIZE="$PLUGIN_SIZE" OUT_DIR="$OUT_DIR" \
STRINGS_STATUS="$STRINGS_STATUS" SYMBOLS_STATUS="$SYMBOLS_STATUS" \
EXTERNAL_CMAKE="$EXTERNAL_CMAKE" python3 - <<'PY'
from __future__ import annotations
import hashlib, json, os, platform, subprocess
from pathlib import Path
root=Path.cwd()
out=Path(os.environ['OUT_DIR'])
def digest(path: Path) -> str:
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()
def output(args: list[str]) -> str | None:
    try:
        return subprocess.run(args,check=True,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout.strip()
    except Exception:
        return None
receipt={
  'schema':'dfmcp.dfhack-plugin-qualification/1',
  'status':'native-build-passed',
  'started_at':os.environ['STARTED_AT'],
  'finished_at':os.environ['FINISHED_AT'],
  'source':{
    'dfmcp_commit':os.environ['DFMCP_COMMIT'],
    'dfmcp_dirty':os.environ['DFMCP_DIRTY']=='true',
    'dfhack_commit':os.environ['DFHACK_COMMIT'],
  },
  'host':{
    'system':platform.system(),
    'release':platform.release(),
    'machine':platform.machine(),
    'cmake':output(['cmake','--version']),
    'cxx':output([os.environ.get('CXX','c++'),'--version']),
  },
  'plugin':{
    'path':os.environ['PLUGIN_BINARY'],
    'bytes':int(os.environ['PLUGIN_SIZE']),
    'sha256':os.environ['PLUGIN_SHA256'],
    'registration':'plugins/external/CMakeLists.txt:add_subdirectory(dfmcp_bridge)',
    'rpc_methods':['Handshake','ReadObservation'],
    'mutation_rpc_methods':[],
    'strings_inventory':os.environ['STRINGS_STATUS'],
    'symbols_inventory':os.environ['SYMBOLS_STATUS'],
  },
  'source_digests':{
    'registry':digest(root/'architecture/dfhack_read_bridge_v1.json'),
    'cmake':digest(root/'bridge/dfhack-plugin/CMakeLists.txt'),
    'proto':digest(root/'bridge/dfhack-plugin/proto/DfmcpBridge.proto'),
    'cpp':digest(root/'bridge/dfhack-plugin/src/dfmcp_bridge.cpp'),
    'rust_wire':digest(root/'crates/dfmcp-adapter/src/dfhack_wire.rs'),
    'capsule':digest(root/'crates/dfmcp-adapter/src/live_observation.rs'),
    'page_driver':digest(root/'crates/dfmcp-adapter/src/live_session.rs'),
    'static_checker':digest(root/'scripts/check_dfhack_bridge.py'),
    'external_registration':digest(Path(os.environ['EXTERNAL_CMAKE'])),
  },
  'logs':{
    'worktree':str(out/'logs/worktree.log'),
    'submodules':str(out/'logs/submodules.log'),
    'configure':str(out/'logs/configure.log'),
    'build':str(out/'logs/build.log'),
    'symbols':str(out/'logs/plugin-symbols.txt'),
    'strings':str(out/'logs/plugin-strings.txt'),
  },
  'claims_not_established':[
    'successful handshake against a running DFHack process',
    'token rejection matrix',
    'read determinism against a disposable fortress',
    'pagination-invariant live capsule identity against a running game',
    'compatibility outside the exact built DFHack revision',
  ],
}
Path(os.environ['OUT_DIR'],'dfhack-plugin-qualification.json').write_text(
    json.dumps(receipt,indent=2,sort_keys=True)+'\n', encoding='utf-8')
PY

ok "Native DFHack plugin build passed"
printf 'Plugin:  %s\nSHA-256: %s\nReceipt: %s\n' "$PLUGIN_BINARY" "$PLUGIN_SHA256" "$RECEIPT"
