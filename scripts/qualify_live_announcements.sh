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
  scripts/qualify_live_announcements.sh EVENTS.jsonl NATIVE_RECEIPT.json [OUTPUT_DIR]

Verifies one exact protocol-1.1 A1-A6 evidence stream against one native
plugin receipt and writes an immutable, secret-free announcement acceptance
receipt plus SHA256SUMS. This does not promote compatibility or start MCP.
EOF
}

[[ $# -ge 2 && $# -le 3 ]] || { usage >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || die "python3 is required"

EVENTS="$1"
NATIVE_RECEIPT="$2"
[[ -f "$EVENTS" ]] || die "event stream does not exist: $EVENTS"
[[ -f "$NATIVE_RECEIPT" ]] || die "native receipt does not exist: $NATIVE_RECEIPT"
EVENTS="$(cd "$(dirname "$EVENTS")" && pwd)/$(basename "$EVENTS")"
NATIVE_RECEIPT="$(cd "$(dirname "$NATIVE_RECEIPT")" && pwd)/$(basename "$NATIVE_RECEIPT")"

RUN_ID="$(date -u +%Y-%m-%dT%H-%M-%SZ)-$(python3 - "$EVENTS" <<'PY'
from __future__ import annotations
import hashlib,sys
from pathlib import Path
print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()[:12])
PY
)"
OUT_DIR="${3:-$ROOT/target/live-announcement-qualification/$RUN_ID}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
RECEIPT="$OUT_DIR/live-announcement-acceptance-receipt.json"

info "Verifying exact A1-A6 announcement evidence"
python3 scripts/verify_live_announcement_acceptance.py \
  "$EVENTS" \
  "$NATIVE_RECEIPT" \
  --output "$RECEIPT"

python3 - "$EVENTS" "$NATIVE_RECEIPT" "$RECEIPT" "$OUT_DIR/SHA256SUMS" <<'PY'
from __future__ import annotations
import hashlib,sys
from pathlib import Path
paths=[Path(value) for value in sys.argv[1:4]]
def digest(path: Path) -> str:
    value=hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b''):
            value.update(chunk)
    return value.hexdigest()
Path(sys.argv[4]).write_text(
    ''.join(f'{digest(path)}  {path.name}\n' for path in paths),
    encoding='utf-8',
)
PY

ok "Protocol-1.1 announcement qualification complete"
printf 'Receipt:   %s\nChecksums: %s\n' "$RECEIPT" "$OUT_DIR/SHA256SUMS"
