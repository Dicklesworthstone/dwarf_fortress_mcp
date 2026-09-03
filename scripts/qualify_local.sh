#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -t 1 ]]; then
  BLUE='\033[1;34m'; GREEN='\033[1;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; RESET='\033[0m'
else
  BLUE=''; GREEN=''; YELLOW=''; RED=''; RESET=''
fi
info() { printf '%b==>%b %s\n' "$BLUE" "$RESET" "$*"; }
ok() { printf '%bOK%b  %s\n' "$GREEN" "$RESET" "$*"; }
warn() { printf '%bWARN%b %s\n' "$YELLOW" "$RESET" "$*"; }
die() { printf '%bERROR%b %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

command -v python3 >/dev/null 2>&1 || die "python3 is required"
command -v git >/dev/null 2>&1 || die "git is required"
[[ -f architecture/live_admission_ticket_v2.json ]] || die "Protocol-bound V2 admission ticket contract is missing"
[[ -f crates/dfmcp-mcp/src/admission.rs ]] || die "Rust live admission boundary is missing"
[[ -f crates/dwarf-fortress-mcp/tests/live_admission.rs ]] || die "Binary live admission tests are missing"

RECEIPT_WRITER="$ROOT/scripts/write_local_qualification_receipt.py"
LOCAL_RECEIPT_CONTRACT="$ROOT/architecture/local_qualification_receipt_v1.json"
GATE_CONTRACT="$ROOT/architecture/live_server_binary_receipt_v1.json"
[[ -f "$RECEIPT_WRITER" && -f "$LOCAL_RECEIPT_CONTRACT" && -f "$GATE_CONTRACT" ]] || \
  die "local qualification receipt machinery is incomplete"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
COMMIT="$(git rev-parse HEAD)"
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "HEAD is not a full lowercase Git commit"
ALLOW_DIRTY=false
if [[ "${DFMCP_ALLOW_DIRTY:-0}" == 1 ]]; then
  ALLOW_DIRTY=true
elif [[ -n "$(git status --porcelain=v1 --untracked-files=all)" ]]; then
  die "release qualification requires a clean worktree (set DFMCP_ALLOW_DIRTY=1 only for development evidence)"
fi

RUN_ID="${DFMCP_QUALIFICATION_RUN_ID:-${STARTED_AT//[:]/-}-${COMMIT:0:12}}"
RAW_OUT_DIR="${DFMCP_QUALIFICATION_DIR:-$ROOT/target/qualification/$RUN_ID}"
if [[ "$RAW_OUT_DIR" != /* ]]; then
  RAW_OUT_DIR="$ROOT/$RAW_OUT_DIR"
fi
OUT_PARENT="$(dirname -- "$RAW_OUT_DIR")"
RUN_NAME="$(basename -- "$RAW_OUT_DIR")"
[[ -n "$RUN_NAME" && "$RUN_NAME" != . && "$RUN_NAME" != .. ]] || die "qualification run directory name is invalid"
mkdir -p "$OUT_PARENT"
OUT_PARENT="$(cd -P -- "$OUT_PARENT" && pwd)"
OUT_DIR="$OUT_PARENT/$RUN_NAME"
[[ ! -e "$OUT_DIR" && ! -L "$OUT_DIR" ]] || die "qualification run directory already exists: $OUT_DIR"
mkdir -m 0700 "$OUT_DIR"

GATES_FILE="$OUT_DIR/gates.tsv"
SOURCE_SNAPSHOT="$OUT_DIR/source-snapshot.json"
RECEIPT="$OUT_DIR/qualification-receipt.json"
: > "$GATES_FILE"
chmod 0600 "$GATES_FILE"

begin_arguments=(
  --source-root "$ROOT"
  --contract "$LOCAL_RECEIPT_CONTRACT"
  --snapshot "$SOURCE_SNAPSHOT"
  --expected-commit "$COMMIT"
)
if [[ "$ALLOW_DIRTY" == true ]]; then
  begin_arguments+=(--allow-dirty)
fi
python3 "$RECEIPT_WRITER" begin "${begin_arguments[@]}" >/dev/null

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
  python3 "$RECEIPT_WRITER" finish \
    --source-root "$ROOT" \
    --contract "$LOCAL_RECEIPT_CONTRACT" \
    --gate-contract "$GATE_CONTRACT" \
    --snapshot "$SOURCE_SNAPSHOT" \
    --gates "$GATES_FILE" \
    --output "$RECEIPT" \
    --expected-commit "$COMMIT" \
    --started-at "$STARTED_AT" \
    "--requested-status" "$final_status" \
    >/dev/null
}
trap 'status=$?; if [[ $status -ne 0 && ! -e "$RECEIPT" ]]; then write_receipt failed >/dev/null 2>&1 || true; fi' EXIT

run_gate repository-integrity python3 scripts/check_repository_integrity.py
run_gate local-qualification-receipt python3 scripts/check_local_qualification_receipt.py
run_gate implementation-status python3 scripts/check_implementation_status.py
run_gate static-contracts bash -c \
  'python3 scripts/validate_repo.py && python3 scripts/check_source_bundle.py && python3 scripts/check_live_announcements.py'
run_gate agent-contract python3 scripts/check_agent_contract.py
run_gate dfhack-read-bridge-contract python3 scripts/check_dfhack_bridge.py
run_gate bridge-auth-order python3 scripts/check_bridge_auth_order.py
run_gate live-mcp-contract python3 scripts/check_live_mcp.py
run_gate compiled-live-read-stack-contract python3 scripts/check_live_read_stack.py
run_gate live-acceptance-contract python3 scripts/check_live_acceptance_contract.py
run_gate live-capture-plan python3 scripts/check_live_capture_plan.py
run_gate live-compatibility-registry python3 scripts/check_live_compatibility_registry.py
run_gate live-compatibility-resolution python3 scripts/check_live_compatibility_resolution.py
run_gate live-compatibility-floor python3 scripts/check_live_compatibility_floor.py
run_gate live-admission-doctor python3 scripts/check_live_admission_doctor.py
run_gate live-server-artifact-admission python3 scripts/check_live_server_artifact.py
run_gate dependency-policy python3 scripts/check_dependency_policy.py
run_gate repository-integrity-tests bash -c \
  'python3 scripts/test_repository_integrity.py && python3 scripts/test_read_stable_repository_file.py && python3 scripts/test_source_bundle.py && python3 scripts/test_source_bundle_output_location.py && python3 scripts/test_read_stable_repository_file_loader.py'
run_gate local-qualification-receipt-tests python3 scripts/test_local_qualification_receipt.py
run_gate implementation-status-tests python3 scripts/test_implementation_status.py
run_gate live-acceptance-tests bash -c \
  'python3 scripts/test_live_read_acceptance.py && python3 scripts/test_live_announcement_contract.py && python3 scripts/test_live_mcp_v1_1.py && python3 scripts/test_live_announcement_acceptance.py && python3 scripts/test_dfhack_plugin_receipt_v1_1.py'
run_gate live-acceptance-journal-tests bash -c \
  'python3 scripts/test_live_read_evidence_journal.py && python3 scripts/test_live_announcement_evidence_journal.py'
run_gate live-acceptance-secret-scanner-tests python3 scripts/test_scan_live_read_secrets.py
run_gate live-capture-guidance-tests python3 scripts/test_live_read_capture_guidance.py
run_gate live-compatibility-promotion-tests python3 scripts/test_live_compatibility_registry.py
run_gate live-compatibility-resolution-tests python3 scripts/test_live_compatibility_resolution.py
run_gate live-compatibility-floor-tests python3 scripts/test_live_compatibility_floor.py
run_gate live-admission-doctor-tests python3 scripts/test_doctor_live_admission.py
run_gate live-server-binary-qualification-tests python3 scripts/test_qualify_live_server_binary.py
run_gate live-server-binary-receipt-tests python3 scripts/test_live_server_binary_receipt.py
run_gate admitted-live-launcher-tests bash -c \
  'python3 scripts/test_admitted_live_launcher.py && python3 scripts/test_live_admission_ticket.py'
run_gate python-syntax python3 -m py_compile \
  scripts/validate_repo.py \
  scripts/read_stable_repository_file.py \
  scripts/check_repository_integrity.py \
  scripts/test_repository_integrity.py \
  scripts/test_read_stable_repository_file.py \
  scripts/test_read_stable_repository_file_loader.py \
  scripts/create_source_bundle.py \
  scripts/verify_source_bundle.py \
  scripts/check_source_bundle.py \
  scripts/test_source_bundle.py \
  scripts/test_source_bundle_output_location.py \
  scripts/write_local_qualification_receipt.py \
  scripts/check_local_qualification_receipt.py \
  scripts/test_local_qualification_receipt.py \
  scripts/check_implementation_status.py \
  scripts/test_implementation_status.py \
  scripts/check_agent_contract.py \
  scripts/check_dfhack_bridge.py \
  scripts/check_live_announcements.py \
  scripts/check_live_announcements_core.py \
  scripts/check_live_announcement_publication.py \
  scripts/check_live_announcement_bootstrap.py \
  scripts/test_live_announcement_contract.py \
  scripts/check_live_mcp_v1_1.py \
  scripts/test_live_mcp_v1_1.py \
  scripts/verify_live_announcement_acceptance.py \
  scripts/test_live_announcement_acceptance.py \
  scripts/live_announcement_evidence_journal.py \
  scripts/test_live_announcement_evidence_journal.py \
  scripts/issue_dfhack_plugin_receipt_v1_1.py \
  scripts/test_dfhack_plugin_receipt_v1_1.py \
  scripts/check_bridge_auth_order.py \
  scripts/check_live_mcp.py \
  scripts/check_live_read_stack.py \
  scripts/check_live_acceptance_contract.py \
  scripts/check_live_capture_plan.py \
  scripts/verify_live_read_acceptance.py \
  scripts/test_live_read_acceptance.py \
  scripts/live_read_evidence_journal.py \
  scripts/test_live_read_evidence_journal.py \
  scripts/scan_live_read_secrets.py \
  scripts/test_scan_live_read_secrets.py \
  scripts/live_read_capture_guidance.py \
  scripts/test_live_read_capture_guidance.py \
  scripts/promote_live_compatibility.py \
  scripts/check_live_compatibility_registry.py \
  scripts/test_live_compatibility_registry.py \
  scripts/resolve_live_compatibility.py \
  scripts/check_live_compatibility_resolution.py \
  scripts/test_live_compatibility_resolution.py \
  scripts/live_compatibility_floor.py \
  scripts/check_live_compatibility_floor.py \
  scripts/test_live_compatibility_floor.py \
  scripts/doctor_live_admission.py \
  scripts/check_live_admission_doctor.py \
  scripts/test_doctor_live_admission.py \
  scripts/verify_live_server_binary_receipt.py \
  scripts/test_qualify_live_server_binary.py \
  scripts/test_live_server_binary_receipt.py \
  scripts/serve_admitted_live.py \
  scripts/check_live_server_artifact.py \
  scripts/test_admitted_live_launcher.py \
  scripts/test_live_admission_ticket.py \
  scripts/check_dependency_policy.py
run_gate shell-syntax bash -n \
  scripts/bootstrap_github_repo.sh \
  scripts/create_source_bundle.sh \
  scripts/qualify_dfhack_plugin.sh \
  scripts/qualify_dfhack_plugin_v1_1.sh \
  scripts/qualify_live_read.sh \
  scripts/qualify_live_announcements.sh \
  scripts/qualify_live_announcement_source.sh \
  scripts/qualify_live_server_binary.sh \
  scripts/verify.sh \
  scripts/qualify_local.sh

if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
  if [[ "${DFMCP_STATIC_ONLY:-0}" == 1 ]]; then
    warn "Rust gates explicitly skipped; this receipt is not release-admissible"
    write_receipt static_only
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
run_gate live-probe-help bash -c \
  'cargo run --locked --quiet --bin dfmcp-live-probe -- help && cargo run --locked --quiet --bin dfmcp-live-announcement-probe -- help'

write_receipt passed
trap - EXIT
ok "Local qualification complete: $RECEIPT"
