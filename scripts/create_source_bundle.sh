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

Creates one deterministic, manifest-sealed source bundle from the exact clean
Git commit. The Python creator builds all artifacts in a sibling staging
directory, independently verifies the archive, manifest, and checkout, then
publishes the complete directory atomically. OUTPUT_DIR, when supplied, must be
an absolute path and must not already exist.
EOF
}

[[ $# -le 1 ]] || { usage >&2; exit 2; }
for command in git python3; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

CREATOR="$ROOT/scripts/create_source_bundle.py"
CONTRACT="$ROOT/architecture/source_bundle_v1.json"
VERIFIER="$ROOT/scripts/verify_source_bundle.py"
STABLE_READER="$ROOT/scripts/read_stable_repository_file.py"
for path in "$CREATOR" "$CONTRACT" "$VERIFIER" "$STABLE_READER"; do
  [[ -f "$path" && ! -L "$path" ]] || die "required source-bundle file is missing or symbolic: $path"
done

if [[ $# -eq 1 ]]; then
  [[ "$1" = /* ]] || die "OUTPUT_DIR must be an absolute path"
  info "Creating and verifying source bundle in $1"
  python3 "$CREATOR" "$1" --source-root "$ROOT" --contract "$CONTRACT"
else
  info "Creating and verifying source bundle under target/source-bundle/<commit>"
  python3 "$CREATOR" --source-root "$ROOT" --contract "$CONTRACT"
fi

ok "Source bundle created through the atomic verified creator"
