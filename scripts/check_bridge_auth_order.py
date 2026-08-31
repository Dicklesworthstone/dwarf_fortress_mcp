#!/usr/bin/env python3
"""Fail closed if the native bridge discloses posture before authentication."""

from __future__ import annotations

import pathlib
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
CPP_PATH = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"


@dataclass(frozen=True)
class Failure:
    message: str


def code_mask(source: str) -> str:
    output = list(source)
    state = "code"
    index = 0
    while index < len(source):
        current = source[index]
        following = source[index + 1] if index + 1 < len(source) else ""
        if state == "code":
            if current == "/" and following == "/":
                output[index] = output[index + 1] = " "
                state = "line_comment"
                index += 2
                continue
            if current == "/" and following == "*":
                output[index] = output[index + 1] = " "
                state = "block_comment"
                index += 2
                continue
            if current == '"':
                output[index] = " "
                state = "string"
            elif current == "'":
                output[index] = " "
                state = "character"
            index += 1
            continue
        output[index] = "\n" if current == "\n" else " "
        if state == "line_comment":
            if current == "\n":
                state = "code"
        elif state == "block_comment":
            if current == "*" and following == "/":
                output[index + 1] = " "
                state = "code"
                index += 1
        elif state in {"string", "character"}:
            delimiter = '"' if state == "string" else "'"
            if current == "\\" and following:
                output[index + 1] = " "
                index += 1
            elif current == delimiter:
                state = "code"
        index += 1
    return "".join(output)


def function_body(source: str, signature: str) -> str:
    start = source.find(signature)
    if start < 0:
        raise ValueError(f"missing function signature {signature!r}")
    masked = code_mask(source)
    opening = masked.find("{", start)
    if opening < 0:
        raise ValueError(f"missing function body for {signature!r}")
    depth = 0
    for index in range(opening, len(masked)):
        if masked[index] == "{":
            depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0:
                return source[opening + 1 : index]
    raise ValueError(f"unterminated function body for {signature!r}")


def before(body: str, first: str, second: str) -> bool:
    left = body.find(first)
    right = body.find(second)
    return left >= 0 and right >= 0 and left < right


def check(source: str) -> list[Failure]:
    failures: list[Failure] = []
    try:
        comparison = function_body(source, "bool constant_time_equal")
        handshake_init = function_body(source, "void initialize_handshake_reply")
        handshake_publish = function_body(source, "void publish_handshake_manifest")
        handshake = function_body(source, "command_result Handshake")
        observation_init = function_body(source, "void initialize_observation_reply")
        observation = function_body(source, "command_result ReadObservation")
    except ValueError as exc:
        return [Failure(str(exc))]

    if "index < MAX_TOKEN_BYTES" not in comparison:
        failures.append(Failure("token comparison does not perform the full admitted fixed workload"))
    if "std::max" in comparison or "left.size(), right.size()" in comparison:
        failures.append(Failure("token comparison workload still depends on presented lengths"))
    if "MAX_TOKEN_BYTES = 256" not in source:
        failures.append(Failure("token work bound is not frozen at 256 bytes"))

    for needle in [
        'out->set_bridge_version("")',
        'out->set_dfhack_version("")',
        'out->set_df_version("")',
        "out->set_world_loaded(false)",
        "out->set_fortress_mode(false)",
        "out->set_bridge_generation(0)",
    ]:
        if needle not in handshake_init:
            failures.append(Failure(f"handshake neutral initializer is missing {needle}"))
    for forbidden in ["Core::", "Version::", "World::", "BRIDGE_GENERATION", "add_supported_methods"]:
        if forbidden in handshake_init:
            failures.append(Failure(f"handshake neutral initializer inspects or discloses {forbidden}"))

    for needle in [
        "Version::dfhack_version()",
        "df_version()",
        "Core::getInstance().isWorldLoaded()",
        "World::isFortressMode()",
        "BRIDGE_GENERATION",
        'add_supported_methods("Handshake")',
        'add_supported_methods("ReadObservation")',
    ]:
        if needle not in handshake_publish:
            failures.append(Failure(f"authenticated handshake manifest is missing {needle}"))
    if not before(handshake, "!authenticate(in->bearer_token()", "publish_handshake_manifest(out)"):
        failures.append(Failure("handshake publishes the manifest before authentication succeeds"))
    for forbidden in ["Core::", "Version::", "World::", "add_supported_methods"]:
        if forbidden in handshake:
            failures.append(Failure(f"handshake handler directly reaches sensitive source {forbidden}"))

    for needle in [
        'out->set_client_nonce("")',
        "out->set_bridge_generation(0)",
        "out->set_world_loaded(false)",
        "out->set_fortress_mode(false)",
    ]:
        if needle not in observation_init:
            failures.append(Failure(f"observation neutral initializer is missing {needle}"))
    if not before(observation, "!authenticate(in->bearer_token()", "out->set_bridge_generation(BRIDGE_GENERATION)"):
        failures.append(Failure("observation discloses generation before authentication succeeds"))
    for sensitive in [
        "Core::getInstance().isWorldLoaded()",
        "World::isFortressMode()",
        "World::ReadCurrentTick()",
        "Units::getCitizens",
    ]:
        if not before(observation, "!authenticate(in->bearer_token()", sensitive):
            failures.append(Failure(f"observation reaches {sensitive} before authentication"))
    return failures


def main() -> int:
    try:
        source = CPP_PATH.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"bridge auth order: FAIL: {exc}", file=sys.stderr)
        return 1
    failures = check(source)
    if failures:
        print(f"bridge auth order: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.message}", file=sys.stderr)
        return 1
    print("bridge auth order: PASS (fixed-work comparison and post-auth disclosure only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
