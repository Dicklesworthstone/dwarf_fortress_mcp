#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/dist}"
mkdir -p "$OUT"
NAME="dwarf_fortress_mcp"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cp -a "$ROOT" "$TMP/$NAME"
rm -rf "$TMP/$NAME/.git" "$TMP/$NAME/target" "$TMP/$NAME/dist"
(
  cd "$TMP"
  if command -v zip >/dev/null 2>&1; then
    zip -q -9 -r "$OUT/${NAME}.zip" "$NAME"
  else
    python3 -m zipfile -c "$OUT/${NAME}.zip" "$NAME"
  fi
)
printf 'created %s\n' "$OUT/${NAME}.zip"
