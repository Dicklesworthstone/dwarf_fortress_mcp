#!/usr/bin/env python3
"""Prove the native bridge authenticates before caller echo or world reads.

This is a source-order contract, not a native-build or runtime authentication
proof. It exists because merely counting calls to `authenticate` does not show
that initialization code has not already leaked world posture or copied an
unbounded caller nonce into the reply.
"""

from __future__ import annotations

import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE_PATH = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"


@dataclass(frozen=True)
class Failure:
    function: str
    message: str


def function_body(source: str, name: str) -> str | None:
    pattern = re.compile(rf"\b{name}\s*\([^;{{]*\)\s*\{{", re.DOTALL)
    match = pattern.search(source)
    if match is None:
        return None
    opening = source.find("{", match.start())
    depth = 0
    offset = opening
    in_string = False
    escaped = False
    in_line_comment = False
    in_block_comment = False
    while offset < len(source):
        char = source[offset]
        next_char = source[offset + 1] if offset + 1 < len(source) else ""
        if in_line_comment:
            if char == "\n":
                in_line_comment = False
            offset += 1
            continue
        if in_block_comment:
            if char == "*" and next_char == "/":
                in_block_comment = False
                offset += 2
            else:
                offset += 1
            continue
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            offset += 1
            continue
        if char == "/" and next_char == "/":
            in_line_comment = True
            offset += 2
        elif char == "/" and next_char == "*":
            in_block_comment = True
            offset += 2
        elif char == '"':
            in_string = True
            offset += 1
        elif char == "{":
            depth += 1
            offset += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[opening + 1 : offset]
            offset += 1
        else:
            offset += 1
    return None


def require_before(
    body: str,
    earlier: str,
    later: str,
    function: str,
    failures: list[Failure],
) -> None:
    earlier_offset = body.find(earlier)
    later_offset = body.find(later)
    if earlier_offset < 0:
        failures.append(Failure(function, f"missing required marker {earlier!r}"))
    elif later_offset < 0:
        failures.append(Failure(function, f"missing required marker {later!r}"))
    elif earlier_offset >= later_offset:
        failures.append(
            Failure(
                function,
                f"{earlier!r} must occur before {later!r}",
            )
        )


def main() -> int:
    try:
        source = SOURCE_PATH.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"bridge auth-order contract: FAIL: {exc}", file=sys.stderr)
        return 1

    failures: list[Failure] = []
    bodies: dict[str, str] = {}
    for name in [
        "initialize_handshake_reply",
        "populate_authenticated_handshake_reply",
        "Handshake",
        "initialize_observation_reply",
        "ReadObservation",
    ]:
        body = function_body(source, name)
        if body is None:
            failures.append(Failure(name, "function body was not found or was unbalanced"))
        else:
            bodies[name] = body

    handshake_init = bodies.get("initialize_handshake_reply", "")
    for forbidden in [
        "Core::",
        "World::",
        "Units::",
        "Version::",
        "df_version(",
        "in->client_nonce",
    ]:
        if forbidden in handshake_init:
            failures.append(
                Failure(
                    "initialize_handshake_reply",
                    f"neutral pre-auth initializer contains {forbidden!r}",
                )
            )
    if 'set_client_nonce("")' not in handshake_init:
        failures.append(
            Failure(
                "initialize_handshake_reply",
                "pre-auth required nonce field is not initialized to an empty value",
            )
        )

    authenticated = bodies.get("populate_authenticated_handshake_reply", "")
    for required in [
        "Core::getInstance().isWorldLoaded()",
        "World::isFortressMode()",
        "set_client_nonce(client_nonce)",
        "set_accepted(true)",
    ]:
        if required not in authenticated:
            failures.append(
                Failure(
                    "populate_authenticated_handshake_reply",
                    f"authenticated population is missing {required!r}",
                )
            )

    handshake = bodies.get("Handshake", "")
    require_before(
        handshake,
        "authenticate(in->bearer_token()",
        "populate_authenticated_handshake_reply",
        "Handshake",
        failures,
    )
    for forbidden in ["Core::", "World::", "Units::"]:
        if forbidden in handshake:
            failures.append(
                Failure(
                    "Handshake",
                    f"world inspection {forbidden!r} must remain behind the authenticated helper",
                )
            )

    observation_init = bodies.get("initialize_observation_reply", "")
    for forbidden in ["Core::", "World::", "Units::", "in->client_nonce"]:
        if forbidden in observation_init:
            failures.append(
                Failure(
                    "initialize_observation_reply",
                    f"neutral pre-auth initializer contains {forbidden!r}",
                )
            )
    if 'set_client_nonce("")' not in observation_init:
        failures.append(
            Failure(
                "initialize_observation_reply",
                "pre-auth required nonce field is not initialized to an empty value",
            )
        )

    observation = bodies.get("ReadObservation", "")
    for later in [
        "out->set_client_nonce(in->client_nonce())",
        "Core::getInstance().isWorldLoaded()",
        "World::isFortressMode()",
        "Units::getCitizens",
    ]:
        require_before(
            observation,
            "authenticate(in->bearer_token()",
            later,
            "ReadObservation",
            failures,
        )

    if "return mixed == 0 ? 1 : mixed;" not in source:
        failures.append(
            Failure(
                "process_generation",
                "bridge generation is not explicitly kept out of the reserved zero value",
            )
        )

    if failures:
        print(
            f"bridge auth-order contract: FAIL ({len(failures)} violation(s))",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  {failure.function}: {failure.message}", file=sys.stderr)
        return 1

    print("bridge auth-order contract: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
