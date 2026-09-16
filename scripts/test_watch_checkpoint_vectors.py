#!/usr/bin/env python3
"""Independent framing reference and fixed vectors; does NOT execute Rust.

Matches watch_checkpoint.rs's explicit binary layout and SHA-256 domains.
Only in-memory test data is modified. No repository journals are opened.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

MAGIC = b"DFWLOG01"
RECORD = b"DFWREC01"
FOOTER = b"DFWEND01"
HEADER = 104
PREFIX = 84
MAX_PAYLOAD = 1024 * 1024
MAX_BYTES = 64 * 1024 * 1024
MAX_RECORDS = 4096


def sha(domain, *parts):
    return hashlib.sha256(domain + b"".join(parts)).digest()


def header(binding, session, request, state_hash):
    identity = sha(b"dfmcp-watch-journal-incarnation/1\0", binding,
                   session.to_bytes(16, "big"), request.to_bytes(16, "big"), state_hash)
    prefix = MAGIC + binding + identity
    digest = sha(b"dfmcp-watch-journal-header/1\0", prefix)
    return prefix + digest, identity, digest


def frame(identity, predecessor, sequence, payload):
    if not 1 <= len(payload) <= MAX_PAYLOAD:
        raise ValueError("payload bound")
    prefix = RECORD + struct.pack(">Q", sequence) + predecessor + struct.pack(">I", len(payload))
    prefix += sha(b"dfmcp-watch-journal-prefix/1\0", identity, prefix)
    digest = sha(b"dfmcp-watch-journal-record/1\0", identity, prefix, payload)
    return prefix + payload + digest + FOOTER, digest


def decode(data, binding):
    if not HEADER <= len(data) <= MAX_BYTES:
        raise ValueError("file bound")
    if data[:8] != MAGIC or data[8:40] != binding:
        raise ValueError("format or archive binding")
    identity, predecessor = data[40:72], data[72:104]
    if identity == bytes(32) or predecessor != sha(b"dfmcp-watch-journal-header/1\0", data[:72]):
        raise ValueError("header identity/checksum")
    offset = HEADER
    records = []
    while offset < len(data):
        if len(records) >= MAX_RECORDS or len(data) - offset < PREFIX:
            raise ValueError("record bound/incomplete prefix")
        prefix = data[offset:offset + PREFIX]
        sequence = int.from_bytes(prefix[8:16], "big")
        if (prefix[:8] != RECORD or sequence != len(records) + 1
                or prefix[16:48] != predecessor
                or prefix[52:] != sha(b"dfmcp-watch-journal-prefix/1\0", identity, prefix[:52])):
            raise ValueError("record prefix/sequence/predecessor")
        size = int.from_bytes(prefix[48:52], "big")
        if not 1 <= size <= MAX_PAYLOAD:
            raise ValueError("payload bound")
        end = offset + PREFIX + size + 40
        if end > len(data):
            raise ValueError("incomplete checkpoint")
        payload = data[offset + PREFIX:offset + PREFIX + size]
        digest = data[end - 40:end - 8]
        if data[end - 8:end] != FOOTER or digest != sha(
                b"dfmcp-watch-journal-record/1\0", identity, prefix, payload):
            raise ValueError("record checksum/footer")
        records.append(payload)
        predecessor = digest
        offset = end
    return identity, predecessor, records


def run():
    checks = 0
    def check(condition):
        nonlocal checks
        checks += 1
        if not condition:
            raise AssertionError(f"check {checks}")
    binding = hashlib.sha256(b"spatial/1.8 archive").digest()
    state_hash = hashlib.sha256(b"world").digest()
    initial, identity, root = header(binding, 7, 3, state_hash)
    first, first_digest = frame(identity, root, 1, b"first")
    second, second_digest = frame(identity, first_digest, 2, b"second")
    data = initial + first + second
    check(decode(data, binding) == (identity, second_digest, [b"first", b"second"]))
    check(decode(initial, binding) == (identity, root, []))
    check(decode(initial + first, binding) == (identity, first_digest, [b"first"]))
    def reject(candidate, selected_binding=binding):
        try:
            decode(candidate, selected_binding)
        except ValueError:
            check(True)
        else:
            check(False)
    valid_boundaries = {HEADER, HEADER + len(first), len(data)}
    truncations = 0
    for end in range(len(data)):
        if end not in valid_boundaries:
            reject(data[:end])
            truncations += 1
    for offset in range(len(data)):
        damaged = bytearray(data)
        damaged[offset] ^= 1
        reject(bytes(damaged))
    reject(data, hashlib.sha256(b"another archive").digest())
    reject(data + b"unexpected")
    reject(initial + first + first)
    reject(initial + second + first)
    # Authentically checksummed length prefixes still cannot allocate oversized/empty payloads.
    for size in [0, MAX_PAYLOAD + 1, 2**32 - 1]:
        prefix = RECORD + (1).to_bytes(8, "big") + root + size.to_bytes(4, "big")
        prefix += sha(b"dfmcp-watch-journal-prefix/1\0", identity, prefix)
        reject(initial + prefix)
    # Correct frame checksums cannot hide a sequence jump or predecessor fork.
    for sequence, parent in [(2, root), (1, hashlib.sha256(b"fork").digest())]:
        forged, _ = frame(identity, parent, sequence, b"record")
        reject(initial + forged)
    for session, request, state in [(8, 3, state_hash), (7, 4, state_hash), (7, 3, bytes(32))]:
        changed, changed_id, _ = header(binding, session, request, state)
        check(changed != initial and changed_id != identity)
    # Independent files with the same header cannot silently append beyond the record-count cap.
    many = bytearray(initial)
    previous = root
    for number in range(1, MAX_RECORDS + 2):
        encoded, previous = frame(identity, previous, number, b"x")
        many.extend(encoded)
    reject(bytes(many))
    return {
        "evidence": "independent_python_binary_framing_reference_only",
        "reference_source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "rust_compilation_or_execution": False,
        "native_dfhack_or_live_game_execution": False,
        "checks_passed": checks,
        "single_byte_corruptions_rejected": len(data),
        "incomplete_prefixes_rejected": truncations,
        "fixture_bytes": len(data),
        "binding_hex": binding.hex(),
        "journal_id_hex": identity.hex(),
        "header_digest_hex": root.hex(),
        "first_checkpoint_digest_hex": first_digest.hex(),
        "second_checkpoint_digest_hex": second_digest.hex(),
        "fixture_sha256_hex": hashlib.sha256(data).hexdigest(),
        "fixture_hex": data.hex(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = run()
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    print(text, end="")


if __name__ == "__main__":
    main()
