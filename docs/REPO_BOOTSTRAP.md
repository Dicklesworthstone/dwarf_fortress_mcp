# Creating the Public Repository

The source archive includes a complete local repository tree but no GitHub remote.

## One command with GitHub CLI

From the extracted `dwarf_fortress_mcp` directory:

```bash
./scripts/bootstrap_github_repo.sh Dicklesworthstone/dwarf_fortress_mcp public
```

The script:

1. checks `git` and `gh`;
2. verifies GitHub authentication;
3. refuses to overwrite an existing repository;
4. initializes `main` if needed;
5. commits all source;
6. creates a public repository;
7. pushes `main`;
8. prints the URL.

PowerShell:

```powershell
./scripts/bootstrap_github_repo.ps1 -Repository Dicklesworthstone/dwarf_fortress_mcp -Visibility public
```

## Manual equivalent

```bash
git init -b main
git add .
git commit -m "Initial architecture and executable contract scaffold"
gh repo create Dicklesworthstone/dwarf_fortress_mcp --public --source=. --remote=origin --push
```

## First checks after push

```bash
gh repo view Dicklesworthstone/dwarf_fortress_mcp --web
./scripts/qualify_local.sh
```

Local qualification on the latest nightly toolchain is part of phase-zero acceptance. Workflow
YAML is available for `doodlestein_self_releaser`, `act`, and controlled self-hosted machines, but
GitHub-hosted Actions are not required evidence. Correct any compiler or lint issue without
weakening invariants or deleting tests.
