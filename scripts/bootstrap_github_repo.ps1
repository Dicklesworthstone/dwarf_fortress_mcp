param(
    [string]$Repository = "Dicklesworthstone/dwarf_fortress_mcp",
    [ValidateSet("public", "private", "internal")][string]$Visibility = "public"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $Root

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) { throw "$Name is required" }
}

if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw "Repository must be OWNER/NAME" }
Require-Command git
Require-Command gh
& gh auth status *> $null
if ($LASTEXITCODE -ne 0) { throw "gh is not authenticated" }

$env:DFMCP_ALLOW_MISSING_RUST = "1"
& python3 scripts/validate_repo.py
if ($LASTEXITCODE -ne 0) { throw "Repository validation failed" }

& gh repo view $Repository *> $null
if ($LASTEXITCODE -eq 0) { throw "Repository already exists: $Repository" }

if (-not (Test-Path .git)) { & git init -b main }
$Branch = (& git branch --show-current).Trim()
if ($Branch -ne "main") { throw "Expected branch main, found $Branch" }
& git remote get-url origin *> $null
if ($LASTEXITCODE -eq 0) { throw "origin already exists; refusing to retarget it" }

& git config user.name *> $null
if ($LASTEXITCODE -ne 0) {
    $Login = (& gh api user --jq .login).Trim()
    & git config user.name $Login
}
& git config user.email *> $null
if ($LASTEXITCODE -ne 0) {
    $Login = (& gh api user --jq .login).Trim()
    & git config user.email "$Login@users.noreply.github.com"
}

& git add --all
& git diff --cached --quiet
if ($LASTEXITCODE -ne 0) { & git commit -m "Initial semantic architecture and executable contract" }

& gh repo create $Repository "--$Visibility" --source=. --remote=origin --push
if ($LASTEXITCODE -ne 0) { throw "GitHub repository creation failed" }
Write-Host "Published https://github.com/$Repository" -ForegroundColor Green
