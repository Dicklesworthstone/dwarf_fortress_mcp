#!/usr/bin/env python3
"""Resolve one deployment manifest against the exact live compatibility registry."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
MANIFEST_SCHEMA = "dfmcp.live-deployment-manifest/1"
DECISION_SCHEMA = "dfmcp.live-compatibility-decision/1"

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility registry contract")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class ResolutionError(ValueError):
    pass


def fail(message: str) -> None:
    raise ResolutionError(message)


def validate_manifest(value: dict[str, Any]) -> dict[str, Any]:
    if set(value) != {"schema", "version_tuple", "platform", "source"}:
        fail("deployment manifest fields differ from the exact V1 schema")
    if value.get("schema") != MANIFEST_SCHEMA:
        fail("deployment manifest schema is unsupported")
    version = promotion.require_object(value.get("version_tuple"), "deployment.version_tuple")
    if set(version) != {"dwarf_fortress", "dfhack", "bridge", "protocol"}:
        fail("deployment version tuple fields drifted")
    normalized_version = {
        "dwarf_fortress": promotion.require_string(
            version.get("dwarf_fortress"), "deployment.version_tuple.dwarf_fortress", 128
        ),
        "dfhack": promotion.require_string(version.get("dfhack"), "deployment.version_tuple.dfhack", 128),
        "bridge": promotion.require_string(version.get("bridge"), "deployment.version_tuple.bridge", 128),
        "protocol": promotion.require_string(version.get("protocol"), "deployment.version_tuple.protocol", 16),
    }
    platform = promotion.require_object(value.get("platform"), "deployment.platform")
    if set(platform) != {"system", "machine"}:
        fail("deployment platform fields drifted")
    normalized_platform = {
        "system": promotion.require_string(platform.get("system"), "deployment.platform.system", 128),
        "machine": promotion.require_string(platform.get("machine"), "deployment.platform.machine", 128),
    }
    source = promotion.require_object(value.get("source"), "deployment.source")
    if set(source) != {"dfmcp_commit", "dfhack_commit", "plugin_sha256"}:
        fail("deployment source fields drifted")
    normalized_source = {
        "dfmcp_commit": promotion.require_commit(source.get("dfmcp_commit"), "deployment.source.dfmcp_commit"),
        "dfhack_commit": promotion.require_commit(source.get("dfhack_commit"), "deployment.source.dfhack_commit"),
        "plugin_sha256": promotion.require_hash(source.get("plugin_sha256"), "deployment.source.plugin_sha256"),
    }
    if normalized_version["protocol"] != "1.0":
        fail("the V1 resolver accepts only bridge protocol 1.0")
    return {
        "version_tuple": normalized_version,
        "platform": normalized_platform,
        "source": normalized_source,
    }


def deployment_key(manifest: dict[str, Any]) -> bytes:
    return promotion.canonical_json(
        {
            "version_tuple": manifest["version_tuple"],
            "platform": manifest["platform"],
            "dfmcp_commit": manifest["source"]["dfmcp_commit"],
            "dfhack_commit": manifest["source"]["dfhack_commit"],
            "plugin_sha256": manifest["source"]["plugin_sha256"],
        }
    )


def classify_miss(entries: list[dict[str, Any]], manifest: dict[str, Any]) -> list[str]:
    reasons = ["no exact admitted source/binary/version/platform tuple exists"]
    same_versions = [entry for entry in entries if entry["version_tuple"] == manifest["version_tuple"]]
    if same_versions:
        reasons.append("the version strings exist in the registry but another exact tuple was qualified")
        if not any(entry["platform"] == manifest["platform"] for entry in same_versions):
            reasons.append("the operating-system or machine architecture is not admitted")
        if not any(entry["source"]["dfmcp_commit"] == manifest["source"]["dfmcp_commit"] for entry in same_versions):
            reasons.append("the dwarf_fortress_mcp source revision is not admitted")
        if not any(entry["source"]["dfhack_commit"] == manifest["source"]["dfhack_commit"] for entry in same_versions):
            reasons.append("the DFHack source revision is not admitted")
        if not any(entry["source"]["plugin_sha256"] == manifest["source"]["plugin_sha256"] for entry in same_versions):
            reasons.append("the native plugin binary digest is not admitted")
    else:
        reasons.append("the exact DF, DFHack, bridge, and protocol version tuple has no evidence entry")
    return reasons


def resolve(
    registry_value: dict[str, Any],
    deployment_value: dict[str, Any],
    required_entry_id: str | None = None,
) -> dict[str, Any]:
    entries = promotion.validate_registry(registry_value)
    manifest = validate_manifest(deployment_value)
    key = deployment_key(manifest)
    matches = [entry for entry in entries if promotion.compatibility_key(entry) == key]
    if len(matches) > 1:
        fail("registry contains more than one canonical entry for the exact deployment tuple")
    if required_entry_id is not None:
        promotion.require_hash(required_entry_id, "required_entry_id")
    if matches:
        entry = matches[0]
        if required_entry_id is not None and entry["entry_id"] != required_entry_id:
            admitted = False
            reasons = ["the exact tuple is admitted under a different entry identifier"]
            entry_id: str | None = None
            support_level: str | None = None
            capabilities: list[str] = []
            omitted_domains: list[str] = []
        else:
            admitted = True
            reasons = []
            entry_id = entry["entry_id"]
            support_level = entry["support_level"]
            capabilities = entry["capabilities"]
            omitted_domains = entry["omitted_domains"]
    else:
        admitted = False
        reasons = classify_miss(entries, manifest)
        entry_id = None
        support_level = None
        capabilities = []
        omitted_domains = []
    unsigned: dict[str, Any] = {
        "schema": DECISION_SCHEMA,
        "admitted": admitted,
        "entry_id": entry_id,
        "support_level": support_level,
        "manifest": manifest,
        "capabilities": capabilities,
        "mutation_capabilities": [],
        "omitted_domains": omitted_domains,
        "reasons": reasons,
        "registry_entry_count": len(entries),
    }
    return {
        **unsigned,
        "decision_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--require-entry-id")
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        registry = promotion.read_object(args.registry, 8 * 1024 * 1024, "compatibility registry")
        manifest = promotion.read_object(args.manifest, 1024 * 1024, "deployment manifest")
        decision = resolve(registry, manifest, args.require_entry_id)
        if args.output is None:
            print(json.dumps(decision, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
        else:
            write_atomic(args.output, decision)
    except (OSError, promotion.PromotionError, ResolutionError) as exc:
        print(f"live compatibility resolution: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0 if decision["admitted"] else 3


if __name__ == "__main__":
    raise SystemExit(main())
