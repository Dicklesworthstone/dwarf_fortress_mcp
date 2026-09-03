#!/usr/bin/env python3
"""Resolve one deployment against an exact append-only compatibility policy generation."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
MANIFEST_SCHEMA = "dfmcp.live-deployment-manifest/1"
DECISION_SCHEMA = "dfmcp.live-compatibility-decision/2"

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility registry contract")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class ResolutionError(ValueError):
    pass


def fail(message: str) -> NoReturn:
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
            version.get("dwarf_fortress"),
            "deployment.version_tuple.dwarf_fortress",
            128,
        ),
        "dfhack": promotion.require_string(
            version.get("dfhack"),
            "deployment.version_tuple.dfhack",
            128,
        ),
        "bridge": promotion.require_string(
            version.get("bridge"),
            "deployment.version_tuple.bridge",
            128,
        ),
        "protocol": promotion.require_string(
            version.get("protocol"),
            "deployment.version_tuple.protocol",
            16,
        ),
    }
    platform_value = promotion.require_object(value.get("platform"), "deployment.platform")
    if set(platform_value) != {"system", "machine"}:
        fail("deployment platform fields drifted")
    normalized_platform = {
        "system": promotion.require_string(
            platform_value.get("system"),
            "deployment.platform.system",
            128,
        ),
        "machine": promotion.require_string(
            platform_value.get("machine"),
            "deployment.platform.machine",
            128,
        ),
    }
    source = promotion.require_object(value.get("source"), "deployment.source")
    if set(source) != {"dfmcp_commit", "dfhack_commit", "plugin_sha256"}:
        fail("deployment source fields drifted")
    normalized_source = {
        "dfmcp_commit": promotion.require_commit(
            source.get("dfmcp_commit"),
            "deployment.source.dfmcp_commit",
        ),
        "dfhack_commit": promotion.require_commit(
            source.get("dfhack_commit"),
            "deployment.source.dfhack_commit",
        ),
        "plugin_sha256": promotion.require_hash(
            source.get("plugin_sha256"),
            "deployment.source.plugin_sha256",
        ),
    }
    if normalized_version["protocol"] != "1.0":
        fail("the compatibility resolver currently accepts only bridge protocol 1.0")
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
    reasons = ["no exact active source/binary/version/platform tuple exists"]
    same_versions = [entry for entry in entries if entry["version_tuple"] == manifest["version_tuple"]]
    if same_versions:
        reasons.append("the version strings exist in registry history but another exact tuple was qualified")
        if not any(entry["platform"] == manifest["platform"] for entry in same_versions):
            reasons.append("the operating-system or machine architecture is not admitted")
        if not any(
            entry["source"]["dfmcp_commit"] == manifest["source"]["dfmcp_commit"]
            for entry in same_versions
        ):
            reasons.append("the dwarf_fortress_mcp source revision is not admitted")
        if not any(
            entry["source"]["dfhack_commit"] == manifest["source"]["dfhack_commit"]
            for entry in same_versions
        ):
            reasons.append("the DFHack source revision is not admitted")
        if not any(
            entry["source"]["plugin_sha256"] == manifest["source"]["plugin_sha256"]
            for entry in same_versions
        ):
            reasons.append("the native plugin binary digest is not admitted")
    else:
        reasons.append("the exact DF, DFHack, bridge, and protocol version tuple has no evidence entry")
    return reasons


def revocation_reason(revocation: dict[str, Any]) -> str:
    return (
        f"entry {revocation['entry_id']} is revoked for {revocation['reason_code']}: "
        f"{revocation['reason']}"
    )


def resolve(
    registry_value: dict[str, Any],
    deployment_value: dict[str, Any],
    required_entry_id: str | None = None,
) -> dict[str, Any]:
    entries, revocations = promotion.validate_registry_components(registry_value)
    manifest = validate_manifest(deployment_value)
    registry_digest = promotion.sha256_bytes(promotion.canonical_json(registry_value))
    revocations_digest = promotion.sha256_bytes(promotion.canonical_json(revocations))
    normalized_required_entry_id = None
    if required_entry_id is not None:
        normalized_required_entry_id = promotion.require_hash(
            required_entry_id,
            "required_entry_id",
        )

    revoked_by_entry = {item["entry_id"]: item for item in revocations}
    active = [entry for entry in entries if entry["entry_id"] not in revoked_by_entry]
    key = deployment_key(manifest)
    historical_matches = [entry for entry in entries if promotion.compatibility_key(entry) == key]
    active_matches = [entry for entry in historical_matches if entry["entry_id"] not in revoked_by_entry]
    if len(active_matches) > 1:
        fail("registry contains more than one active entry for the exact deployment tuple")
    matching_entry_ids = [entry["entry_id"] for entry in historical_matches]
    matching_revocations = [
        copy.deepcopy(revoked_by_entry[entry_id])
        for entry_id in matching_entry_ids
        if entry_id in revoked_by_entry
    ]
    matching_revocations.sort(key=lambda item: item["revocation_id"])

    admitted = False
    chosen: dict[str, Any] | None = None
    reasons: list[str] = []
    if normalized_required_entry_id is not None:
        required_match = next(
            (
                entry
                for entry in historical_matches
                if entry["entry_id"] == normalized_required_entry_id
            ),
            None,
        )
        if required_match is None:
            if active_matches:
                reasons = ["the exact tuple is active under a different entry identifier"]
            elif historical_matches:
                reasons = ["the required entry identifier does not name this exact historical tuple"]
            else:
                reasons = classify_miss(entries, manifest)
        elif normalized_required_entry_id in revoked_by_entry:
            reasons = [
                "the explicitly required exact compatibility entry is revoked",
                revocation_reason(revoked_by_entry[normalized_required_entry_id]),
            ]
        else:
            admitted = True
            chosen = required_match
    elif active_matches:
        admitted = True
        chosen = active_matches[0]
    elif historical_matches:
        reasons = [
            "every historical compatibility entry for the exact tuple is revoked",
            *[revocation_reason(item) for item in matching_revocations],
        ]
    else:
        reasons = classify_miss(entries, manifest)

    if chosen is None:
        entry_id: str | None = None
        support_level: str | None = None
        capabilities: list[str] = []
        omitted_domains: list[str] = []
    else:
        entry_id = chosen["entry_id"]
        support_level = chosen["support_level"]
        capabilities = list(chosen["capabilities"])
        omitted_domains = list(chosen["omitted_domains"])

    unsigned: dict[str, Any] = {
        "schema": DECISION_SCHEMA,
        "admitted": admitted,
        "entry_id": entry_id,
        "required_entry_id": normalized_required_entry_id,
        "support_level": support_level,
        "manifest": manifest,
        "capabilities": capabilities,
        "mutation_capabilities": [],
        "omitted_domains": omitted_domains,
        "reasons": reasons,
        "matching_entry_ids": matching_entry_ids,
        "matching_revocations": matching_revocations,
        "registry_status": registry_value["status"],
        "registry_historical_entry_count": len(entries),
        "registry_active_entry_count": len(active),
        "registry_revocation_count": len(revocations),
        "registry_revocations_digest": revocations_digest,
        "registry_digest": registry_digest,
    }
    return {
        **unsigned,
        "decision_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    promotion.write_atomic(path, value)


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
        registry = promotion.read_object(
            args.registry,
            promotion.MAX_JSON_BYTES,
            "compatibility registry",
        )
        manifest = promotion.read_object(
            args.manifest,
            1024 * 1024,
            "deployment manifest",
        )
        decision = resolve(registry, manifest, args.require_entry_id)
        if args.output is None:
            print(
                json.dumps(
                    decision,
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=False,
                )
            )
        else:
            write_atomic(args.output, decision)
    except (OSError, promotion.PromotionError, ResolutionError) as exc:
        print(f"live compatibility resolution: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0 if decision["admitted"] else 3


if __name__ == "__main__":
    raise SystemExit(main())
