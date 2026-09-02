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
[[ -f architecture/live_admission_ticket_v2.json ]] || die "Protocol-bound V2 admission ticket contract is missing"
[[ -f crates/dfmcp-mcp/src/admission.rs ]] || die "Rust live admission boundary is missing"
[[ -f crates/dwarf-fortress-mcp/tests/live_admission.rs ]] || die "Binary live admission tests are missing"

info "Rejecting corrupted source, symlinks, local-path placeholders, and probe debris"
python3 scripts/check_repository_integrity.py
ok "Repository integrity"

info "Validating deterministic clean-commit source bundles"
python3 scripts/check_source_bundle.py
ok "Source bundle contract"

info "Validating repository contracts"
python3 scripts/validate_repo.py
ok "Repository contracts"

info "Validating the agent operating-model contract"
python3 scripts/check_agent_contract.py
ok "Agent operating-model contract"

info "Validating the authenticated protocol-1.0 read-only DFHack bridge"
python3 scripts/check_dfhack_bridge.py
ok "DFHack protocol-1.0 bridge contract"

info "Validating isolated protocol-1.1 announcement reads"
python3 scripts/check_live_announcements.py
ok "Protocol-1.1 announcement contract"

info "Validating native bridge authentication ordering"
python3 scripts/check_bridge_auth_order.py
ok "Bridge authentication ordering"

info "Validating the authenticated live MCP mode"
python3 scripts/check_live_mcp.py
ok "Live MCP contract"

info "Validating the compiled authenticated live-read stack"
python3 scripts/check_live_read_stack.py
ok "Compiled live-read stack contract"

info "Validating the R2-R5 probe, scanner, journal, and evidence contract"
python3 scripts/check_live_acceptance_contract.py
ok "Live-read acceptance contract"

info "Validating the agent-facing live capture plan"
python3 scripts/check_live_capture_plan.py
ok "Live capture plan"

info "Validating exact compatibility promotion"
python3 scripts/check_live_compatibility_registry.py
ok "Exact compatibility promotion"

info "Validating registry-generation-bound compatibility resolution"
python3 scripts/check_live_compatibility_resolution.py
ok "Exact compatibility resolution"

info "Validating owner-private monotonic registry custody"
python3 scripts/check_live_compatibility_floor.py
ok "Live compatibility floor"

info "Validating authority-free live-admission diagnosis"
python3 scripts/check_live_admission_doctor.py
ok "Live admission doctor"

info "Validating protocol-, floor-, receipt-, ticket-, and descriptor-bound live execution"
python3 scripts/check_live_server_artifact.py
ok "Live server artifact admission"

info "Enforcing closed dependency universe"
python3 scripts/check_dependency_policy.py
ok "Dependency policy"

info "Running Python contract tests"
python3 scripts/test_repository_integrity.py
python3 scripts/test_read_stable_repository_file.py
python3 scripts/test_read_stable_repository_file_loader.py
python3 scripts/test_source_bundle.py
python3 scripts/test_source_bundle_output_location.py
python3 scripts/test_live_announcement_contract.py
python3 scripts/test_live_mcp_v1_1.py
python3 scripts/test_live_announcement_acceptance.py
python3 scripts/test_live_announcement_evidence_journal.py
python3 scripts/test_dfhack_plugin_receipt_v1_1.py
python3 scripts/test_live_read_acceptance.py
python3 scripts/test_live_read_evidence_journal.py
python3 scripts/test_scan_live_read_secrets.py
python3 scripts/test_live_read_capture_guidance.py
python3 scripts/test_live_compatibility_registry.py
python3 scripts/test_live_compatibility_resolution.py
python3 scripts/test_live_compatibility_floor.py
python3 scripts/test_doctor_live_admission.py
python3 scripts/test_qualify_live_server_binary.py
python3 scripts/test_live_server_binary_receipt.py
python3 scripts/test_admitted_live_launcher.py
python3 scripts/test_live_admission_ticket.py
ok "Python contract tests"

info "Checking script syntax"
python3 -m py_compile \
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
bash -n \
  scripts/bootstrap_github_repo.sh \
  scripts/create_source_bundle.sh \
  scripts/qualify_dfhack_plugin.sh \
  scripts/qualify_dfhack_plugin_v1_1.sh \
  scripts/qualify_live_read.sh \
  scripts/qualify_live_announcements.sh \
  scripts/qualify_live_announcement_source.sh \
  scripts/qualify_live_server_binary.sh \
  scripts/qualify_local.sh \
  scripts/verify.sh
ok "Script syntax"

if ! command -v cargo >/dev/null 2>&1; then
  if [[ "${DFMCP_ALLOW_MISSING_RUST:-0}" == "1" ]]; then
    warn "Rust toolchain unavailable; static validation passed, Rust gates were explicitly skipped"
    exit 0
  fi
  die "latest nightly cargo is required (set DFMCP_ALLOW_MISSING_RUST=1 only for non-release static checks)"
fi

info "Checking Rust formatting"
cargo fmt --all -- --check
ok "Rust formatting"

info "Running Clippy with warnings denied"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
ok "Clippy"

info "Running workspace tests"
cargo test --locked --workspace --all-targets --all-features
ok "Workspace tests"

info "Running release-mode workspace tests"
cargo test --locked --release --workspace --all-targets --all-features
ok "Release workspace tests"

info "Building warning-free API documentation"
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --all-features --no-deps
ok "API documentation"

info "Running executable contract checks"
cargo run --locked --quiet --bin dwarf-fortress-mcp -- contract >/dev/null
cargo run --locked --quiet --bin dwarf-fortress-mcp -- doctor >/dev/null
cargo run --locked --quiet --bin dwarf-fortress-mcp -- demo >/dev/null
cargo run --locked --quiet --bin dfmcp-live-probe -- help >/dev/null
cargo run --locked --quiet --bin dfmcp-live-announcement-probe -- help >/dev/null
ok "Executable contract checks"

printf '%bAll verification gates passed.%b\n' "$GREEN" "$RESET"
