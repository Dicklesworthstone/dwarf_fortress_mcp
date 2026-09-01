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

info "Rejecting local-path placeholders and probe debris"
python3 scripts/check_repository_integrity.py
ok "Repository integrity"

info "Validating repository contracts"
python3 scripts/validate_repo.py
ok "Repository contracts"

info "Validating the agent operating-model contract"
python3 scripts/check_agent_contract.py
ok "Agent operating-model contract"

info "Validating the authenticated read-only DFHack bridge"
python3 scripts/check_dfhack_bridge.py
ok "DFHack read-only bridge contract"

info "Validating native bridge authentication ordering"
python3 scripts/check_bridge_auth_order.py
ok "Bridge authentication ordering"

info "Validating the authenticated live MCP mode"
python3 scripts/check_live_mcp.py
ok "Live MCP contract"

info "Validating the compiled authenticated live-read stack"
python3 scripts/check_live_read_stack.py
ok "Compiled live-read stack contract"

info "Validating the R2-R5 probe, journal, and evidence contract"
python3 scripts/check_live_acceptance_contract.py
ok "Live-read acceptance contract"

info "Enforcing closed dependency universe"
python3 scripts/check_dependency_policy.py
ok "Dependency policy"

info "Running Python contract tests"
python3 scripts/test_repository_integrity.py
python3 scripts/test_live_read_acceptance.py
python3 scripts/test_live_read_evidence_journal.py
ok "Python contract tests"

info "Checking script syntax"
python3 -m py_compile \
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
  scripts/check_dependency_policy.py
bash -n \
  scripts/bootstrap_github_repo.sh \
  scripts/create_source_bundle.sh \
  scripts/qualify_dfhack_plugin.sh \
  scripts/qualify_live_read.sh \
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

info "Building warning-free API documentation"
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --all-features --no-deps
ok "API documentation"

info "Running executable contract checks"
cargo run --locked --quiet --bin dwarf-fortress-mcp -- contract >/dev/null
cargo run --locked --quiet --bin dwarf-fortress-mcp -- doctor >/dev/null
cargo run --locked --quiet --bin dwarf-fortress-mcp -- demo >/dev/null
cargo run --locked --quiet --bin dfmcp-live-probe -- help >/dev/null
ok "Executable contract checks"

printf '%bAll verification gates passed.%b\n' "$GREEN" "$RESET"
