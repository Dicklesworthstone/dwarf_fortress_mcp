#!/usr/bin/env python3
"""Independent stdlib wire/journal reference checks. Does NOT execute Rust.

Uses the native-emitted fixtures already in the repository. This checks format
arithmetic and fail-closed reference invariants, not the Rust implementation,
DFHack integration, filesystem fault behavior or production admission.
"""
from __future__ import annotations

import hashlib
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "crates/dfmcp-adapter/tests/fixtures"
sha = lambda value: hashlib.sha256(value).digest()
text = lambda value: struct.pack(">H", len(value)) + value


def require(condition: bool) -> None:
    if not condition:
        raise ValueError("reference invariant failed")


def fortress_id(observation: bytes) -> bytes:
    size = int.from_bytes(observation[85:87], "big")
    folder = observation[87:87 + size]
    identity = sha(b"dfmcp-live-fortress-id-v1\0" + folder + b"\0" + observation[40:44])
    return (int.from_bytes(identity[:8], "big") | 1).to_bytes(8, "big")


def effect_check(observation: bytes, effect: bytes, key: bytes = b"job-001", desired: bool = True) -> int:
    require(len(observation) >= 91 and observation[:8] == b"DFMJS019")
    require(len(effect) == 194 + len(key) and len(effect) <= 322)
    require(effect[:8] == b"DFMJSE19" and effect[8:36] == observation[8:36])
    witness = sha(observation)
    plan = sha(b"dfmcp-job-suspension-plan/1\0" + observation[32:36] + bytes([desired]) + witness)
    token = sha(b"dfmcp-job-suspension-token/1\0" + observation[8:16] + text(key) + plan)[:16]
    require(effect[36] == desired and effect[37:69] == witness and effect[69:101] == plan)
    require(effect[101:117] == token and effect[192:] == text(key))
    state, known, after = effect[117:120]
    require(state in range(5) and known in (0, 1) and after in (0, 1))
    require(bool(known) == (state in (2, 3)))
    if known:
        changed = bytearray(observation)
        sequence = int.from_bytes(observation[16:24], "big") + 1
        require(sequence < (1 << 64) - 1)
        changed[16:24] = sequence.to_bytes(8, "big")
        changed[84] = (changed[84] & ~1) | after
        require(effect[120:128] == observation[24:32] and effect[128:160] == sha(changed))
        require((after == desired) == (state == 2))
    else:
        require(effect[118:160] == bytes(42))
    receipt = sha(b"dfmcp-job-suspension-receipt/1\0" + observation[8:16] + text(key) + effect[69:160]) if state in (2, 3, 4) else bytes(32)
    require(effect[160:192] == receipt)
    return state


def native_state(observation: bytes, original: bytes, state: int) -> bytes:
    out = bytearray(original)
    out[117] = state
    if state in (0, 1, 4):
        out[118:192] = bytes(74)
    elif state == 3:
        out[119] = 0
        changed = bytearray(observation)
        changed[16:24] = (int.from_bytes(changed[16:24], "big") + 1).to_bytes(8, "big")
        out[128:160] = sha(changed)
    if state in (2, 3, 4):
        out[160:192] = sha(b"dfmcp-job-suspension-receipt/1\0" + out[8:16] + out[192:] + out[69:160])
    return bytes(out)


def journal_body(observation: bytes, effect: bytes, state: int) -> bytes:
    return bytes([state]) + text(b"job-001") + text(observation) + b"\1" + text(b"df") + text(b"dfhack") + text(effect)


def journal_check(data: bytes) -> list[int]:
    require(len(data) >= 80 and data[:8] == b"DFMJJ019" and data[48:80] == sha(data[:48]))
    identity, previous, offset, number = data[8:40], data[48:80], 80, 1
    old = None
    states = []
    while offset < len(data):
        prefix = data[offset:offset + 52]
        require(len(prefix) == 52 and prefix[:8] == b"DFMJJR19")
        count, ordinal = struct.unpack(">IQ", prefix[8:20])
        require(count <= 2048 and ordinal == number and prefix[20:52] == previous)
        end = offset + 52 + count
        body = data[offset + 52:end]
        digest = sha(b"dfmcp-job-journal-frame/1\0" + identity + prefix + body)
        require(data[end:end + 32] == digest and data[end + 32:end + 40] == b"DFMJJEND")
        position = 1

        def take_text() -> bytes:
            nonlocal position
            require(position + 2 <= len(body))
            size = int.from_bytes(body[position:position + 2], "big")
            start = position + 2
            position = start + size
            require(position <= len(body))
            return body[start:position]

        state = body[0]
        key, observation = take_text(), take_text()
        require(position < len(body) and body[position] == 1)
        position += 1
        df, dfhack, effect = take_text(), take_text(), take_text()
        require(position == len(body) and df == b"df" and dfhack == b"dfhack")
        native = effect_check(observation, effect, key)
        require(data[40:48] == fortress_id(observation))
        identity_fields = (key, observation, df, dfhack)
        if old is not None:
            require(identity_fields == old[4])
        expected = {1: {0}, 2: {0}, 3: {0, 1}, 4: {2}, 5: {3}, 6: {4}, 7: {0}}
        require(state in expected and native in expected[state])
        if old is None:
            require(state not in (2, 7))
        elif old[0] in (4, 5, 6, 7):
            require(body == old[1])
        elif old[0] in (2, 3):
            require(state in (3, 4, 5, 6))
        if old is not None and old[2] == 1:
            require(effect == old[3])
        old = state, body, native, effect, identity_fields
        previous, offset, number = digest, end + 40, number + 1
        states.append(state)
    require(offset == len(data))
    return states


def make_journal(observation: bytes, original: bytes, states: list[int]) -> tuple[bytes, set[int]]:
    header = b"DFMJJ019" + sha(b"reference identity") + fortress_id(observation)
    out = header + sha(header)
    previous = out[48:80]
    boundaries = {80}
    for number, state in enumerate(states, 1):
        native = {1: 0, 2: 0, 3: 1, 4: 2, 5: 3, 6: 4, 7: 0}[state]
        body = journal_body(observation, native_state(observation, original, native), state)
        prefix = b"DFMJJR19" + struct.pack(">IQ", len(body), number) + previous
        previous = sha(b"dfmcp-job-journal-frame/1\0" + out[8:40] + prefix + body)
        out += prefix + body + previous + b"DFMJJEND"
        boundaries.add(len(out))
    return out, boundaries


def rejected(function, *args) -> bool:
    try:
        function(*args)
    except (ValueError, IndexError, struct.error):
        return True
    return False


def main() -> None:
    observation = bytes.fromhex((FIXTURES / "job_suspension_observation_v1_9.hex").read_text())
    effect = bytes.fromhex((FIXTURES / "job_suspension_effect_v1_9.hex").read_text())
    require(effect_check(observation, effect) == 2)
    for state in range(5):
        require(effect_check(observation, native_state(observation, effect, state)) == state)
    bit_rejections = 0
    for offset in range(len(effect)):
        for bit in range(8):
            corrupt = bytearray(effect)
            corrupt[offset] ^= 1 << bit
            require(rejected(effect_check, observation, bytes(corrupt)))
            bit_rejections += 1
    journal, boundaries = make_journal(observation, effect, [1, 2, 4])
    require(journal_check(journal) == [1, 2, 4])
    for offset in range(len(journal)):
        corrupt = bytearray(journal)
        corrupt[offset] ^= 1
        require(rejected(journal_check, bytes(corrupt)))
    truncated = 0
    for end in range(len(journal)):
        if end not in boundaries:
            require(rejected(journal_check, journal[:end]))
            truncated += 1
    for states in [[1, 2, 1], [1, 2, 7], [1, 2, 3, 1], [1, 2, 3, 4], [1, 7, 2], [1, 2, 4, 1]]:
        require(rejected(journal_check, make_journal(observation, effect, states)[0]))
    sources = [ROOT / "crates/dfmcp-adapter/src/job_suspension.rs", *sorted((ROOT / "crates/dfmcp-adapter/src/job_suspension").glob("*.rs"))]
    print(json.dumps({
        "evidence": "Python reference only; no Rust, filesystem crash execution, real DFHack or admission",
        "native_hash_chain": "witness, plan, token, after-witness, receipt verified",
        "native_states_verified": 5,
        "effect_bit_corruptions_rejected": bit_rejections,
        "journal_byte_corruptions_rejected": len(journal),
        "journal_incomplete_prefixes_rejected": truncated,
        "rehashed_invalid_state_histories_rejected": 6,
        "source_sha256": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sources},
    }, indent=2))


if __name__ == "__main__":
    main()
