#!/usr/bin/env python3
from __future__ import annotations

import json
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
FAILURES: list[str] = []
CHECKS = 0


def check(condition: bool, message: str) -> None:
    global CHECKS
    CHECKS += 1
    if not condition:
        FAILURES.append(message)


def read(path: str | Path) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def json_pointer(document: Any, pointer: str) -> Any:
    if pointer in ("", "#"):
        return document
    if pointer.startswith("#"):
        pointer = pointer[1:]
    if not pointer.startswith("/"):
        raise ValueError(f"unsupported JSON pointer {pointer!r}")
    current = document
    for raw in pointer[1:].split("/"):
        key = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(current, list):
            current = current[int(key)]
        else:
            current = current[key]
    return current


def is_type(instance: Any, expected: str) -> bool:
    if expected == "null":
        return instance is None
    if expected == "boolean":
        return isinstance(instance, bool)
    if expected == "integer":
        return isinstance(instance, int) and not isinstance(instance, bool)
    if expected == "number":
        return isinstance(instance, (int, float)) and not isinstance(instance, bool)
    if expected == "string":
        return isinstance(instance, str)
    if expected == "array":
        return isinstance(instance, list)
    if expected == "object":
        return isinstance(instance, dict)
    return False


@dataclass
class ValidationError(Exception):
    path: str
    message: str

    def __str__(self) -> str:
        return f"{self.path}: {self.message}"


def validate_instance(instance: Any, schema: dict[str, Any], root: dict[str, Any], path: str = "$") -> None:
    if "$ref" in schema:
        reference = schema["$ref"]
        if reference.startswith("#"):
            target = json_pointer(root, reference)
        elif "#" in reference:
            file_name, pointer = reference.split("#", 1)
            target_root = json.loads((ROOT / "schemas" / Path(file_name).name).read_text(encoding="utf-8"))
            target = json_pointer(target_root, "#" + pointer)
            root = target_root
        else:
            target = json.loads((ROOT / "schemas" / Path(reference).name).read_text(encoding="utf-8"))
            root = target
        validate_instance(instance, target, root, path)
        return

    for branch in schema.get("allOf", []):
        validate_instance(instance, branch, root, path)

    if "oneOf" in schema:
        successes = 0
        branch_errors: list[str] = []
        for branch in schema["oneOf"]:
            try:
                validate_instance(instance, branch, root, path)
                successes += 1
            except ValidationError as error:
                branch_errors.append(str(error))
        if successes != 1:
            sample = "; ".join(branch_errors[:3])
            raise ValidationError(path, f"expected exactly one matching oneOf branch, got {successes}; {sample}")

    if "if" in schema:
        try:
            validate_instance(instance, schema["if"], root, path)
            condition = True
        except ValidationError:
            condition = False
        selected = schema.get("then") if condition else schema.get("else")
        if selected is not None:
            validate_instance(instance, selected, root, path)

    expected = schema.get("type")
    if expected is not None:
        expected_types = [expected] if isinstance(expected, str) else expected
        if not any(is_type(instance, candidate) for candidate in expected_types):
            raise ValidationError(path, f"expected type {expected_types}, got {type(instance).__name__}")

    if "const" in schema and instance != schema["const"]:
        raise ValidationError(path, f"expected constant {schema['const']!r}")
    if "enum" in schema and instance not in schema["enum"]:
        raise ValidationError(path, f"value {instance!r} is not in enum")

    if isinstance(instance, str):
        if len(instance) < schema.get("minLength", 0):
            raise ValidationError(path, "string is shorter than minLength")
        if "maxLength" in schema and len(instance) > schema["maxLength"]:
            raise ValidationError(path, "string is longer than maxLength")
        if "pattern" in schema and re.search(schema["pattern"], instance) is None:
            raise ValidationError(path, f"string does not match {schema['pattern']!r}")

    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in schema and instance < schema["minimum"]:
            raise ValidationError(path, f"number is below minimum {schema['minimum']}")
        if "maximum" in schema and instance > schema["maximum"]:
            raise ValidationError(path, f"number is above maximum {schema['maximum']}")

    if isinstance(instance, list):
        if len(instance) < schema.get("minItems", 0):
            raise ValidationError(path, "array is shorter than minItems")
        if "maxItems" in schema and len(instance) > schema["maxItems"]:
            raise ValidationError(path, "array is longer than maxItems")
        if schema.get("uniqueItems"):
            encoded = [json.dumps(item, sort_keys=True, separators=(",", ":")) for item in instance]
            if len(encoded) != len(set(encoded)):
                raise ValidationError(path, "array items are not unique")
        if "items" in schema:
            for index, item in enumerate(instance):
                validate_instance(item, schema["items"], root, f"{path}[{index}]")

    if isinstance(instance, dict):
        required = schema.get("required", [])
        for key in required:
            if key not in instance:
                raise ValidationError(path, f"missing required property {key!r}")
        if "maxProperties" in schema and len(instance) > schema["maxProperties"]:
            raise ValidationError(path, "object exceeds maxProperties")
        properties = schema.get("properties", {})
        for key, value in instance.items():
            if key in properties:
                validate_instance(value, properties[key], root, f"{path}.{key}")
                continue
            additional = schema.get("additionalProperties", True)
            if additional is False:
                raise ValidationError(path, f"unknown property {key!r}")
            if isinstance(additional, dict):
                validate_instance(value, additional, root, f"{path}.{key}")


def extract_balanced_block(text: str, opening_brace: int) -> str:
    depth = 0
    for index in range(opening_brace, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[opening_brace + 1 : index]
    raise ValueError("unbalanced braces")


def validate_proto() -> None:
    path = ROOT / "proto/dfmcp.proto"
    text = path.read_text(encoding="utf-8")
    without_comments = re.sub(r"//[^\n]*", "", text)
    check(without_comments.count("{") == without_comments.count("}"), "proto has unbalanced braces")
    check('syntax = "proto3";' in text, "proto3 syntax declaration missing")
    check("package dfmcp.bridge.v1;" in text, "bridge package is not versioned")
    required_rpcs = {
        "Handshake", "Health", "ProbeCompatibility", "ReadSnapshot", "ReadDelta",
        "PrepareMutation", "CommitMutation", "LookupOperation", "CancelOperation",
        "CreateCheckpoint", "RestoreCheckpoint",
    }
    found_rpcs = set(re.findall(r"\brpc\s+(\w+)\s*\(", without_comments))
    check(required_rpcs == found_rpcs, f"bridge RPC set differs: expected {sorted(required_rpcs)}, got {sorted(found_rpcs)}")
    for kind, name, brace in re.findall(r"\b(message|enum)\s+(\w+)\s*(\{)", without_comments):
        start = without_comments.find(brace, without_comments.find(f"{kind} {name}"))
        try:
            block = extract_balanced_block(without_comments, start)
        except ValueError as error:
            check(False, f"{kind} {name}: {error}")
            continue
        values = [int(value) for value in re.findall(r"=\s*(\d+)\s*;", block)]
        check(len(values) == len(set(values)), f"{kind} {name} has duplicate field/value numbers")
        if kind == "enum":
            check(bool(values) and values[0] == 0, f"enum {name} must begin with zero")


IGNORED_TREE_PARTS = {".git", "target", "__pycache__", ".pytest_cache", ".mypy_cache"}


def is_repository_source(path: Path) -> bool:
    return not any(part in IGNORED_TREE_PARTS for part in path.relative_to(ROOT).parts)


def validate_markdown_links() -> None:
    pattern = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")
    for document in ROOT.rglob("*.md"):
        if not is_repository_source(document):
            continue
        text = document.read_text(encoding="utf-8")
        for raw_target in pattern.findall(text):
            target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
            if not target or target.startswith(("#", "http://", "https://", "mailto:", "data:")):
                continue
            target = unquote(target.split("#", 1)[0])
            if not target:
                continue
            resolved = (document.parent / target).resolve()
            check(resolved.exists(), f"broken local link in {document.relative_to(ROOT)}: {raw_target}")


def validate_repository() -> None:
    required = [
        "README.md", "COMPREHENSIVE_PLAN_FOR_DWARF_FORTRESS_MCP.md", "IMPLEMENTATION_STATUS.md",
        "ARCHITECTURE.md", "MCP_SURFACE.md", "ROADMAP.md", "SECURITY.md", "AGENTS.md",
        "Cargo.toml", "Cargo.lock", "LICENSE", "proto/dfmcp.proto", "schemas/dfmcp.schema.json",
        "design/registries/INVARIANTS.md", "scripts/verify.sh", "scripts/qualify_local.sh",
        "scripts/check_dependency_policy.py", ".github/workflows/ci.yml", ".github/workflows/release.yml",
        "FRANKENSTACK_DEEP_DIVE.md", "docs/WORLD_STATE_MVCC.md",
        "docs/FORTRESS_GRAPH_ALGORITHMS.md", "docs/ATP_STATE_AND_EVIDENCE_PLANE.md",
        "docs/DEPENDENCY_POLICY.md", "docs/LOCAL_QUALIFICATION_AND_RELEASE.md",
        "docs/PERFORMANCE_ENGINEERING.md", "architecture/franken_imports.json",
        "architecture/publication_primitives.json", "architecture/graph_algorithms.json",
        "architecture/dependency_allowlist.toml", "release/dsr/dwarf_fortress_mcp.yaml.example",
    ]
    for item in required:
        check((ROOT / item).is_file(), f"required file missing: {item}")

    toml_files = [
        ROOT / "Cargo.toml", ROOT / "rust-toolchain.toml",
        ROOT / "architecture/dependency_allowlist.toml",
    ] + sorted(ROOT.glob("crates/*/Cargo.toml"))
    parsed_toml: dict[Path, dict[str, Any]] = {}
    for path in toml_files:
        try:
            parsed_toml[path] = tomllib.loads(path.read_text(encoding="utf-8"))
        except Exception as error:
            check(False, f"invalid TOML {path.relative_to(ROOT)}: {error}")
    lock = tomllib.loads(read("Cargo.lock"))
    check(lock.get("version") == 4, "Cargo.lock must use format version 4")
    locked_packages = {entry.get("name") for entry in lock.get("package", []) if isinstance(entry, dict)}
    expected_locked_packages = {
        "dfmcp-core", "dfmcp-world", "dfmcp-intent", "dfmcp-adapter", "dfmcp-lab", "dwarf-fortress-mcp"
    }
    check(locked_packages == expected_locked_packages, "Cargo.lock package closure differs from the workspace")

    root_manifest = parsed_toml.get(ROOT / "Cargo.toml", {})
    workspace = root_manifest.get("workspace", {})
    package_defaults = workspace.get("package", {})
    check(package_defaults.get("edition") == "2024", "workspace must use Rust edition 2024")
    check("rust-version" not in package_defaults, "nightly workspace must not advertise a misleading stable MSRV")
    toolchain = parsed_toml.get(ROOT / "rust-toolchain.toml", {}).get("toolchain", {})
    check(toolchain.get("channel") == "nightly", "toolchain must track latest nightly")
    components = set(toolchain.get("components", []))
    check({"clippy", "rustfmt", "rust-src", "llvm-tools-preview"} <= components, "nightly toolchain components are incomplete")
    members = workspace.get("members", [])
    check(len(members) == 6, f"expected six workspace members, got {len(members)}")
    for member in members:
        check((ROOT / member / "Cargo.toml").is_file(), f"workspace member missing Cargo.toml: {member}")
    for path, manifest in parsed_toml.items():
        for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, specification in manifest.get(table_name, {}).items():
                is_local = isinstance(specification, dict) and "path" in specification
                check(is_local, f"phase-zero manifest {path.relative_to(ROOT)} has external dependency {name}")

    rust_files = sorted(ROOT.glob("crates/**/*.rs"))
    check(bool(rust_files), "no Rust source files found")
    forbidden = {
        r"\bunsafe\s*(?:fn|impl|trait|extern|\{)": "unsafe block or declaration",
        r"\.unwrap\s*\(": "unwrap call",
        r"\.expect\s*\(": "expect call",
        r"\bpanic!\s*\(": "panic macro",
        r"\bunreachable!\s*\(": "unreachable macro",
        r"\btodo!\s*\(": "todo macro",
        r"\bunimplemented!\s*\(": "unimplemented macro",
    }
    for path in rust_files:
        source = path.read_text(encoding="utf-8")
        if path.name in ("lib.rs", "main.rs"):
            check("#![forbid(unsafe_code)]" in source, f"{path.relative_to(ROOT)} does not forbid unsafe code")
        for pattern, description in forbidden.items():
            check(re.search(pattern, source) is None, f"{path.relative_to(ROOT)} contains forbidden {description}")

    json_files = (
        sorted((ROOT / "schemas").glob("*.json"))
        + sorted((ROOT / "examples").glob("*.json"))
        + sorted((ROOT / "architecture").glob("*.json"))
    )
    documents: dict[Path, Any] = {}
    for path in json_files:
        try:
            documents[path] = json.loads(path.read_text(encoding="utf-8"))
        except Exception as error:
            check(False, f"invalid JSON {path.relative_to(ROOT)}: {error}")
    for name, expected_schema, collection, minimum in (
        ("architecture/franken_imports.json", "dfmcp.franken-imports.v1", "projects", 8),
        ("architecture/publication_primitives.json", "dfmcp.publication-primitives.v1", "primitives", 8),
        ("architecture/graph_algorithms.json", "dfmcp.graph-algorithms.v1", "algorithms", 10),
    ):
        document = documents.get(ROOT / name)
        check(isinstance(document, dict), f"architecture registry is not an object: {name}")
        if isinstance(document, dict):
            check(document.get("schema") == expected_schema, f"wrong architecture schema in {name}")
            entries = document.get(collection, [])
            check(isinstance(entries, list) and len(entries) >= minimum, f"architecture registry is underspecified: {name}")
            ids = [entry.get("id") for entry in entries if isinstance(entry, dict) and "id" in entry]
            if ids:
                check(len(ids) == len(set(ids)), f"duplicate IDs in {name}")

    main_schema_path = ROOT / "schemas/dfmcp.schema.json"
    main_schema = documents.get(main_schema_path)
    if isinstance(main_schema, dict):
        for path in sorted((ROOT / "examples").glob("*.json")):
            try:
                validate_instance(documents[path], main_schema, main_schema)
            except ValidationError as error:
                check(False, f"schema validation failed for {path.relative_to(ROOT)}: {error}")
            else:
                check(True, f"validated {path.relative_to(ROOT)}")
        try:
            import jsonschema  # type: ignore
        except ImportError:
            pass
        else:
            try:
                jsonschema.Draft202012Validator.check_schema(main_schema)
            except Exception as error:
                check(False, f"JSON Schema meta-validation failed: {error}")
            else:
                check(True, "JSON Schema meta-validation")

    validate_proto()
    validate_markdown_links()

    plan = read("COMPREHENSIVE_PLAN_FOR_DWARF_FORTRESS_MCP.md")
    check(len(plan.split()) >= 22000, "comprehensive plan is unexpectedly short for revision 2")
    check("# Part XXI — Deep Franken-substrate revision" in plan, "deep Franken-substrate revision is missing")
    invariant_ids = set(re.findall(r"\bINV-\d{3}\b", plan))
    expected_invariants = {f"INV-{index:03d}" for index in range(1, 51)}
    check(invariant_ids == expected_invariants, "plan must contain exactly INV-001 through INV-050")
    registry_invariants = set(re.findall(r"\bINV-\d{3}\b", read("design/registries/INVARIANTS.md")))
    check(registry_invariants == expected_invariants, "invariant registry must contain exactly INV-001 through INV-050")

    tool_names = {
        "fortress.open_session", "fortress.observe", "fortress.query", "fortress.plan",
        "fortress.commit", "fortress.wait", "fortress.cancel", "fortress.checkpoint",
        "fortress.restore", "fortress.explain", "fortress.doctor",
    }
    schema_tools = set()
    if isinstance(main_schema, dict):
        for branch in main_schema["$defs"]["ToolCall"]["oneOf"]:
            schema_tools.add(branch["properties"]["tool"]["const"])
    check(schema_tools == tool_names, "JSON Schema tool set differs from frozen 11-tool surface")
    for document in (read("README.md"), read("MCP_SURFACE.md"), plan):
        check(tool_names <= set(re.findall(r"fortress\.[a-z_]+", document)), "a core document omits one or more frozen tools")

    status = read("IMPLEMENTATION_STATUS.md").lower()
    check("not" in status and "mcp server" in status, "implementation status must clearly deny a finished MCP server")
    check("dfhack" in status and ("not implemented" in status or "not yet" in status), "implementation status must clearly deny live DFHack integration")

    sources = read("docs/SOURCES.md")
    for name in (
        "asupersync", "frankensqlite", "frankenfs", "frankensearch", "franken_markdown",
        "frankengraphdb", "franken_networkx", "doodlestein_self_releaser", "DFHack",
        "Model Context Protocol",
    ):
        check(name in sources, f"source ledger omits {name}")

    for workflow_name in ("ci.yml", "portability.yml", "release.yml"):
        workflow = read(f".github/workflows/{workflow_name}")
        check("self-hosted" in workflow, f"{workflow_name} must be local/self-hosted")
        check("ubuntu-latest" not in workflow and "macos-latest" not in workflow and "windows-latest" not in workflow,
              f"{workflow_name} must not depend on GitHub-hosted runners")
    check("scripts/qualify_local.sh" in read(".github/workflows/ci.yml"), "CI specification must call local qualification")


def main() -> int:
    validate_repository()
    if FAILURES:
        print(f"repository validation failed: {len(FAILURES)} failure(s) across {CHECKS} checks", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    file_count = sum(
        1
        for path in ROOT.rglob("*")
        if path.is_file() and is_repository_source(path)
    )
    print(f"repository validation passed: {CHECKS} checks across {file_count} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
