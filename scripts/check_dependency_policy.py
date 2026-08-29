#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parents[1]
POLICY_PATH = ROOT / "architecture/dependency_allowlist.toml"
OWNED_GIT_PREFIX = "https://github.com/Dicklesworthstone/"
FULL_GIT_REVISION = re.compile(r"^[0-9a-fA-F]{40}$")


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def normalized(name: str) -> str:
    return name.replace("_", "-").lower()


def dependency_name(specification: Any, fallback: str) -> str:
    if isinstance(specification, dict):
        package = specification.get("package")
        if isinstance(package, str):
            return package
    return fallback


def dependency_tables(manifest: dict[str, Any]) -> Iterable[tuple[str, dict[str, Any]]]:
    for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
        table = manifest.get(table_name, {})
        if isinstance(table, dict):
            yield table_name, table
    workspace = manifest.get("workspace", {})
    if isinstance(workspace, dict):
        table = workspace.get("dependencies", {})
        if isinstance(table, dict):
            yield "workspace.dependencies", table


def main() -> int:
    policy = load_toml(POLICY_PATH)
    classes = policy["classes"]
    phase_zero = {normalized(x) for x in policy["phase_zero"]["external_runtime_dependencies"]}
    fundamental = {normalized(x) for x in classes["fundamental_external"]}
    prefixes = tuple(normalized(x) for x in classes["owned_runtime_prefixes"] + classes["owned_franken_prefixes"])
    prohibited = {normalized(x) for x in policy["prohibited"]["crates"]}
    failures: list[str] = []
    checked = 0

    manifests = [ROOT / "Cargo.toml", *sorted((ROOT / "crates").glob("*/Cargo.toml"))]
    for manifest_path in manifests:
        manifest = load_toml(manifest_path)
        for table_name, table in dependency_tables(manifest):
            for declared_name, specification in table.items():
                checked += 1
                package_name = dependency_name(specification, declared_name)
                package = normalized(package_name)
                owned_name = package.startswith(prefixes)
                location = f"{manifest_path.relative_to(ROOT)} [{table_name}]"

                if package in prohibited:
                    failures.append(f"{location}: prohibited dependency {package_name}")
                    continue

                if isinstance(specification, dict) and specification.get("workspace") is True:
                    continue

                if isinstance(specification, dict) and "path" in specification:
                    path = (manifest_path.parent / str(specification["path"])).resolve()
                    if not (path / "Cargo.toml").is_file():
                        failures.append(f"{location}: path dependency {declared_name} has no Cargo.toml at {path}")
                        continue
                    try:
                        path.relative_to(ROOT)
                        inside_repository = True
                    except ValueError:
                        inside_repository = False
                    if inside_repository:
                        continue
                    if not owned_name:
                        failures.append(f"{location}: external local dependency {package_name} is not owned")
                        continue
                    sibling_manifest = load_toml(path / "Cargo.toml")
                    sibling_package = sibling_manifest.get("package", {})
                    repository = sibling_package.get("repository", "") if isinstance(sibling_package, dict) else ""
                    if not isinstance(repository, str) or not repository.startswith(OWNED_GIT_PREFIX):
                        failures.append(f"{location}: sibling {package_name} lacks owned repository provenance")
                    continue

                if isinstance(specification, dict) and "git" in specification:
                    git = specification.get("git")
                    revision = specification.get("rev")
                    if not owned_name or not isinstance(git, str) or not git.startswith(OWNED_GIT_PREFIX):
                        failures.append(f"{location}: git dependency {package_name} is outside the owned universe")
                    if not isinstance(revision, str) or FULL_GIT_REVISION.fullmatch(revision) is None:
                        failures.append(f"{location}: owned git dependency {package_name} must pin an exact 40-hex rev")
                    continue

                if owned_name:
                    failures.append(
                        f"{location}: owned dependency {package_name} must use a verified local path or exact owned git rev"
                    )
                    continue
                if package not in phase_zero and package not in fundamental:
                    failures.append(f"{location}: dependency {package_name} is outside the closed universe")

    root_manifest = load_toml(ROOT / "Cargo.toml")
    edition = root_manifest.get("workspace", {}).get("package", {}).get("edition")
    if edition != "2024":
        failures.append(f"workspace edition must be 2024, got {edition!r}")
    toolchain = load_toml(ROOT / "rust-toolchain.toml").get("toolchain", {})
    if toolchain.get("channel") != "nightly":
        failures.append("rust-toolchain.toml must track the latest nightly channel")
    lint = root_manifest.get("workspace", {}).get("lints", {}).get("rust", {}).get("unsafe_code")
    if lint != "forbid":
        failures.append('workspace must set unsafe_code = "forbid"')

    if failures:
        print(f"dependency policy failed: {len(failures)} failure(s)", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(f"dependency policy passed: {checked} dependency declarations checked")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
