#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ORIGINAL="$ROOT/scripts/qualify_dfhack_plugin.sh"
[[ -f "$ORIGINAL" ]] || { printf 'missing base native qualification harness: %s\n' "$ORIGINAL" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { printf 'python3 is required\n' >&2; exit 1; }

generated="$(mktemp "${TMPDIR:-/tmp}/dfmcp-qualify-v1-1.XXXXXX.sh")"
cleanup() { rm -f -- "$generated"; }
trap cleanup EXIT

python3 - "$ORIGINAL" "$generated" <<'PY'
from __future__ import annotations
import os, sys
from pathlib import Path
source=Path(sys.argv[1]).read_text(encoding='utf-8')
required=[
    'DfmcpBridge.proto',
    'dfmcp_bridge.cpp',
    'dfmcp_bridge',
    'qualify_dfhack_plugin.sh',
    'ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"',
]
for marker in required:
    if marker not in source:
        raise SystemExit(f'base native qualification harness drifted; missing {marker!r}')
source=source.replace(
    'ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"',
    'ROOT="${DFMCP_SOURCE_ROOT:?DFMCP_SOURCE_ROOT is required}"',
    1,
)
source=source.replace('DfmcpBridge.proto','DfmcpBridgeV1_1.proto')
source=source.replace('dfmcp_bridge.cpp','__DFMCP_V11_SOURCE__')
source=source.replace('dfmcp_bridge','dfmcp_bridge_v1_1')
source=source.replace('__DFMCP_V11_SOURCE__','dfmcp_bridge_v1_1.cpp')
source=source.replace('qualify_dfhack_plugin.sh','qualify_dfhack_plugin_v1_1.sh')
if 'v1_1_v1_1' in source:
    raise SystemExit('protocol-1.1 native qualification transformation doubled a generation suffix')
Path(sys.argv[2]).write_text(source,encoding='utf-8',newline='\n')
os.chmod(sys.argv[2],0o700)
PY

DFMCP_SOURCE_ROOT="$ROOT" bash "$generated" "$@"
