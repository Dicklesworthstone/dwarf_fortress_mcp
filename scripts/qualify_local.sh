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

command -v python3 >/dev/null 2>&1 || die "python3 is required"
command -v git >/dev/null 2>&1 || die "git is required"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
COMMIT="$(git rev-parse HEAD)"
DIRTY=false
if [[ -n "$(git status --porcelain=v1)" ]]; then DIRTY=true; fi
if [[ "$DIRTY" == true && "${DFMCP_ALLOW_DIRTY:-0}" != 1 ]]; then
  die "release qualification requires a clean worktree (set DFMCP_ALLOW_DIRTY=1 only for development evidence)"
fi

RUN_ID="${DFMCP_QUALIFICATION_RUN_ID:-${STARTED_AT//[:]/-}-${COMMIT:0:12}}"
OUT_DIR="${DFMCP_QUALIFICATION_DIR:-$ROOT/target/qualification/$RUN_ID}"
mkdir -p "$OUT_DIR"
GATES_FILE="$OUT_DIR/gates.tsv"
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
  local final_status="$1"
  local finished_at
  finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  STARTED_AT="$STARTED_AT" FINISHED_AT="$finished_at" COMMIT="$COMMIT" DIRTY="$DIRTY" \
  FINAL_STATUS="$final_status" GATES_FILE="$GATES_FILE" OUT_DIR="$OUT_DIR" python3 - <<'PY'
from __future__ import annotations
import hashlib, json, os, platform, subprocess
from pathlib import Path
root=Path.cwd()
def digest(path: Path) -> str | None:
    if not path.is_file(): return None
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()
def command_output(args: list[str]) -> str | None:
    try: return subprocess.run(args,check=True,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout.strip()
    except Exception: return None
gates=[]
for line in Path(os.environ['GATES_FILE']).read_text().splitlines():
    name,state,detail=(line.split('\t',2)+['',''])[:3]
    gates.append({'name':name,'state':state,'detail':detail or None})
receipt={
 'schema':'dfmcp.qualification-receipt.v1',
 'status':os.environ['FINAL_STATUS'],
 'started_at':os.environ['STARTED_AT'],
 'finished_at':os.environ['FINISHED_AT'],
 'source':{'commit':os.environ['COMMIT'],'dirty':os.environ['DIRTY']=='true'},
 'host':{'system':platform.system(),'release':platform.release(),'machine':platform.machine(),'python':platform.python_version()},
 'toolchain':{'rustc_vv':command_output(['rustc','-vV']),'cargo':command_output(['cargo','--version'])},
 'digests':{
   'Cargo.lock':digest(root/'Cargo.lock'),
   'dependency_allowlist':digest(root/'architecture/dependency_allowlist.toml'),
   'franken_imports':digest(root/'architecture/franken_imports.json'),
   'publication_primitives':digest(root/'architecture/publication_primitives.json'),
   'graph_algorithms':digest(root/'architecture/graph_algorithms.json'),
   'agent_turn_contract':digest(root/'architecture/agent_turn_contract.json'),
   'agent_operating_model':digest(root/'docs/AGENT_OPERATING_MODEL.md'),
   'agent_contract_checker':digest(root/'scripts/check_agent_contract.py'),
   'repository_integrity_checker':digest(root/'scripts/check_repository_integrity.py'),
   'repository_integrity_tests':digest(root/'scripts/test_repository_integrity.py'),
   'dfhack_read_bridge_contract':digest(root/'architecture/dfhack_read_bridge_v1.json'),
   'dfhack_bridge_proto':digest(root/'bridge/dfhack-plugin/proto/DfmcpBridge.proto'),
   'dfhack_bridge_plugin':digest(root/'bridge/dfhack-plugin/src/dfmcp_bridge.cpp'),
   'dfhack_wire_client':digest(root/'crates/dfmcp-adapter/src/dfhack_wire.rs'),
   'dfhack_acceptance_probe':digest(root/'crates/dfmcp-adapter/src/dfhack_probe.rs'),
   'bridge_auth_order_checker':digest(root/'scripts/check_bridge_auth_order.py'),
   'live_connection_admission':digest(root/'crates/dfmcp-adapter/src/live_connect.rs'),
   'live_source_fence':digest(root/'crates/dfmcp-adapter/src/fenced_live_source.rs'),
   'live_fortress_identity':digest(root/'crates/dfmcp-adapter/src/live_identity.rs'),
   'live_observation_capsule':digest(root/'crates/dfmcp-adapter/src/live_observation.rs'),
   'live_observation_driver':digest(root/'crates/dfmcp-adapter/src/live_session.rs'),
   'live_adapter_bootstrap':digest(root/'crates/dfmcp-adapter/src/live_bootstrap.rs'),
   'live_world_projection':digest(root/'crates/dfmcp-adapter/src/live_projection.rs'),
   'live_read_adapter':digest(root/'crates/dfmcp-adapter/src/live_adapter.rs'),
   'live_mcp_server':digest(root/'crates/dfmcp-mcp/src/live_server.rs'),
   'live_mcp_checker':digest(root/'scripts/check_live_mcp.py'),
   'live_read_stack_checker':digest(root/'scripts/check_live_read_stack.py'),
   'live_acceptance_probe_binary':digest(root/'crates/dwarf-fortress-mcp/src/bin/dfmcp-live-probe.rs'),
   'live_acceptance_probe_manifest':digest(root/'crates/dwarf-fortress-mcp/Cargo.toml'),
   'live_acceptance_contract':digest(root/'architecture/live_read_acceptance_v1.json'),
   'live_acceptance_contract_checker':digest(root/'scripts/check_live_acceptance_contract.py'),
   'live_acceptance_verifier':digest(root/'scripts/verify_live_read_acceptance.py'),
   'live_acceptance_tests':digest(root/'scripts/test_live_read_acceptance.py'),
   'live_acceptance_journal':digest(root/'scripts/live_read_evidence_journal.py'),
   'live_acceptance_journal_tests':digest(root/'scripts/test_live_read_evidence_journal.py'),
   'live_acceptance_secret_scanner':digest(root/'scripts/scan_live_read_secrets.py'),
   'live_acceptance_secret_scanner_tests':digest(root/'scripts/test_scan_live_read_secrets.py'),
   'live_acceptance_wrapper':digest(root/'scripts/qualify_live_read.sh'),
   'dfhack_bridge_checker':digest(root/'scripts/check_dfhack_bridge.py'),
   'dfhack_native_build_harness':digest(root/'scripts/qualify_dfhack_plugin.sh')
 },
 'gates':gates
}
out=Path(os.environ['OUT_DIR'])/'qualification-receipt.json'
out.write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
print(out)
PY
}
trap 'status=$?; if [[ $status -ne 0 ]]; then write_receipt failed >/dev/null 2>&1 || true; fi' EXIT

run_gate repository-integrity python3 scripts/check_repository_integrity.py
run_gate static-contracts python3 scripts/validate_repo.py
run_gate agent-contract python3 scripts/check_agent_contract.py
run_gate dfhack-read-bridge-contract python3 scripts/check_dfhack_bridge.py
run_gate bridge-auth-order python3 scripts/check_bridge_auth_order.py
run_gate live-mcp-contract python3 scripts/check_live_mcp.py
run_gate compiled-live-read-stack-contract python3 scripts/check_live_read_stack.py
run_gate live-acceptance-contract python3 scripts/check_live_acceptance_contract.py
run_gate dependency-policy python3 scripts/check_dependency_policy.py
run_gate repository-integrity-tests python3 scripts/test_repository_integrity.py
run_gate live-acceptance-tests python3 scripts/test_live_read_acceptance.py
run_gate live-acceptance-journal-tests python3 scripts/test_live_read_evidence_journal.py
run_gate live-acceptance-secret-scanner-tests python3 scripts/test_scan_live_read_secrets.py
run_gate python-syntax python3 -m py_compile \
  scripts/validate_repo.py \
  scripts/check_repository_integrity.py \
  scripts/test_repository_integrity.py \
  scripts/check_agent_contract.py \
  scripts/check_dfhack_bridge.py \
  scripts/check_bridge_auth_order.py \
  scripts/check_live_mcp.py \
  scripts/check_live_read_stack.py \
  scripts/check_live_acceptance_contract.py \
  scripts/verify_live_read_acceptance.py \
  scripts/test_live_read_acceptance.py \
  scripts/live_read_evidence_journal.py \
  scripts/test_live_read_evidence_journal.py \
  scripts/scan_live_read_secrets.py \
  scripts/test_scan_live_read_secrets.py \
  scripts/check_dependency_policy.py
run_gate shell-syntax bash -n \
  scripts/bootstrap_github_repo.sh \
  scripts/create_source_bundle.sh \
  scripts/qualify_dfhack_plugin.sh \
  scripts/qualify_live_read.sh \
  scripts/verify.sh \
  scripts/qualify_local.sh

if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
  if [[ "${DFMCP_STATIC_ONLY:-0}" == 1 ]]; then
    record rust-toolchain skipped "cargo/rustc unavailable; DFMCP_STATIC_ONLY=1"
    warn "Rust gates explicitly skipped; this receipt is not release-admissible"
    write_receipt static-only
    trap - EXIT
    exit 0
  fi
  die "latest nightly cargo/rustc are required (DFMCP_STATIC_ONLY=1 creates non-release static evidence only)"
fi

run_gate cargo-metadata cargo metadata --locked --offline --format-version 1
run_gate rustfmt cargo fmt --all -- --check
run_gate clippy cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_gate tests cargo test --locked --workspace --all-targets --all-features
run_gate release-tests cargo test --locked --release --workspace --all-targets --all-features
run_gate rustdoc env RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --all-features --no-deps
run_gate contract cargo run --locked --quiet --bin dwarf-fortress-mcp -- contract
run_gate doctor cargo run --locked --quiet --bin dwarf-fortress-mcp -- doctor
run_gate demo cargo run --locked --quiet --bin dwarf-fortress-mcp -- demo
run_gate live-probe-help cargo run --locked --quiet --bin dfmcp-live-probe -- help

write_receipt passed
trap - EXIT
ok "Local qualification complete: $OUT_DIR/qualification-receipt.json"
