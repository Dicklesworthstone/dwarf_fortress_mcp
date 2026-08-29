#!/usr/bin/env bash
set -Eeuo pipefail

REPOSITORY="${1:-Dicklesworthstone/dwarf_fortress_mcp}"
VISIBILITY="${2:-public}"
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

[[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "repository must be OWNER/NAME"
case "$VISIBILITY" in public|private|internal) ;; *) die "visibility must be public, private, or internal" ;; esac
command -v git >/dev/null 2>&1 || die "git is required"
command -v gh >/dev/null 2>&1 || die "GitHub CLI (gh) is required"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated"

info "Running static repository validation"
DFMCP_ALLOW_MISSING_RUST=1 scripts/verify.sh

if gh repo view "$REPOSITORY" >/dev/null 2>&1; then
  die "repository already exists: $REPOSITORY"
fi

if [[ ! -d .git ]]; then
  info "Initializing local Git repository"
  git init -b main
fi

current_branch="$(git branch --show-current)"
[[ "$current_branch" == "main" ]] || die "expected branch main, found ${current_branch:-detached}"
if git remote get-url origin >/dev/null 2>&1; then
  die "origin already exists; refusing to retarget it"
fi

if ! git config user.name >/dev/null; then
  login="$(gh api user --jq .login)"
  git config user.name "$login"
  warn "configured repository-local git user.name as $login"
fi
if ! git config user.email >/dev/null; then
  login="$(gh api user --jq .login)"
  git config user.email "${login}@users.noreply.github.com"
  warn "configured repository-local no-reply git email"
fi

git add --all
if git diff --cached --quiet; then
  if ! git rev-parse --verify HEAD >/dev/null 2>&1; then
    die "no files are staged for the initial commit"
  fi
  warn "working tree already committed; using existing HEAD"
else
  info "Creating initial design-contract commit"
  git commit -m "Initial semantic architecture and executable contract"
fi

info "Creating $VISIBILITY GitHub repository $REPOSITORY and pushing main"
gh repo create "$REPOSITORY" "--$VISIBILITY" --source=. --remote=origin --push
ok "Published https://github.com/$REPOSITORY"
