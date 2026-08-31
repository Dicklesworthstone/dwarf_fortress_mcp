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
  scripts/qualify_live_read.sh EVENTS.jsonl NATIVE_BUILD_RECEIPT.json [OUTPUT_DIR]

This command verifies externally captured R2-R5 evidence. It does not launch Dwarf Fortress,
manufacture evidence, or turn a static fixture into a live qualification. The event stream must
follow architecture/live_read_acceptance_v1.json and must bind the current clean source revision
to the supplied passing R1 native-build receipt.

Environment:
  DFMCP_ALLOW_DIRTY=1  Admit development-only evidence from a dirty source tree.
EOF
}

[[ $# -ge 2 && $# -le 3 ]] || { usage >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || die "python3 is required"
command -v git >/dev/null 2>&1 || die "git is required"

EVIDENCE="$1"
NATIVE_RECEIPT="$2"
[[ -f "$EVIDENCE" ]] || die "evidence stream does not exist: $EVIDENCE"
[[ -f "$NATIVE_RECEIPT" ]] || die "native build receipt does not exist: $NATIVE_RECEIPT"

COMMIT="$(git rev-parse HEAD)"
DIRTY=false
[[ -z "$(git status --porcelain=v1)" ]] || DIRTY=true
if [[ "$DIRTY" == true && "${DFMCP_ALLOW_DIRTY:-0}" != 1 ]]; then
  die "live qualification requires a clean source tree (set DFMCP_ALLOW_DIRTY=1 only for development evidence)"
fi

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${STARTED_AT//[:]/-}-${COMMIT:0:12}"
OUT_DIR="${3:-$ROOT/target/live-read-qualification/$RUN_ID}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
EVIDENCE_COPY="$OUT_DIR/evidence.jsonl"
NATIVE_COPY="$OUT_DIR/native-build-receipt.json"
RECEIPT="$OUT_DIR/live-read-acceptance-receipt.json"

cp -- "$EVIDENCE" "$EVIDENCE_COPY"
cp -- "$NATIVE_RECEIPT" "$NATIVE_COPY"

ARGS=(
  "$EVIDENCE_COPY"
  --contract "$ROOT/architecture/live_read_acceptance_v1.json"
  --source-root "$ROOT"
  --native-build-receipt "$NATIVE_COPY"
  --expected-dfmcp-commit "$COMMIT"
  --receipt "$RECEIPT"
)
if [[ "$DIRTY" == true ]]; then
  ARGS+=(--allow-dirty-development)
  warn "Producing development-only evidence from a dirty tree"
fi

info "Verifying bounded R2-R5 live-read evidence"
python3 scripts/verify_live_read_acceptance.py "${ARGS[@]}" | tee "$OUT_DIR/verifier.log"

EXPECTED_STATUS=qualified
[[ "$DIRTY" == false ]] || EXPECTED_STATUS=development-evidence
python3 - "$RECEIPT" "$EXPECTED_STATUS" <<'PY'
from __future__ import annotations
import json, sys
from pathlib import Path
path=Path(sys.argv[1])
expected=sys.argv[2]
value=json.loads(path.read_text(encoding='utf-8'))
if value.get('status') != expected:
    raise SystemExit(f"receipt status {value.get('status')!r} does not match {expected!r}")
if value.get('source', {}).get('dfmcp_commit') is None:
    raise SystemExit('receipt lost its source commit')
PY

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUT_DIR" && sha256sum evidence.jsonl native-build-receipt.json live-read-acceptance-receipt.json > SHA256SUMS)
elif command -v shasum >/dev/null 2>&1; then
  (cd "$OUT_DIR" && shasum -a 256 evidence.jsonl native-build-receipt.json live-read-acceptance-receipt.json > SHA256SUMS)
else
  die "sha256sum or shasum is required"
fi

ok "Live-read evidence verified"
printf 'Receipt: %s\nChecksums: %s\n' "$RECEIPT" "$OUT_DIR/SHA256SUMS"
