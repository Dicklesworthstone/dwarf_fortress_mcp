#!/usr/bin/env python3
"""Execute the published recovery query JSON Schema, not the Rust dispatcher.

JSON Schema integer semantics are mathematical; this test does not certify Rust
lexical number parsing, duplicate-key handling, raw input lengths, or runtime I/O.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / 'schemas/dig_recovery_query.json'


def main() -> None:
    schema = json.loads(SCHEMA.read_text(encoding='utf-8'))
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    tile = {'mode': 'tiles', 'idempotency_key': 'dig-001', 'plan_digest': 'a' * 64, 'offset': 0}
    good = [{'mode': 'schema'}, {'mode': 'records'}, tile,
            {'mode': 'records', 'limit': None, 'continuation': None}]
    good += [{'mode': 'records', 'limit': n, 'continuation': '0' * 64} for n in range(1, 9)]
    good += [{**tile, 'offset': offset, 'limit': n, 'idempotency_key': 'k' * length}
             for offset in (0, 299) for n in (1, 16, None) for length in (1, 128)]
    bad = [None, [], {}, {'mode': 'unknown'}, {'mode': 'schema', 'extra': 1},
           {'mode': 'records', 'idempotency_key': 'x'}, {**tile, 'continuation': '0' * 64}]
    for raw in (0, 9, -1, True, '8', 1.5, [], {}):
        bad.append({'mode': 'records', 'limit': raw})
    for raw in ('', 'f' * 63, 'f' * 65, 'G' * 64, 'a' * 63 + '\n', True, 1, [], {}):
        bad.append({'mode': 'records', 'continuation': raw})
        bad.append({**tile, 'plan_digest': raw})
    for raw in ('', 'k' * 129, 'with space', 'a/b', 'é', 'x\n', 'x\r', '\x00', True, None):
        bad.append({**tile, 'idempotency_key': raw})
    for raw in (-1, 300, True, None, '0', 0.5, [], {}):
        bad.append({**tile, 'offset': raw})
    for raw in (-1, 0, 17, True, '1', 1.5, [], {}):
        bad.append({**tile, 'limit': raw})
    for required in tile:
        bad.append({k: v for k, v in tile.items() if k != required})
    for example in list(good):
        bad.append({**example, 'unrecognized': True})
    for expected, cases in ((True, good), (False, bad)):
        for index, case in enumerate(cases):
            actual = validator.is_valid(case)
            if actual != expected:
                raise AssertionError(f'schema case {index}: expected {expected}, got {actual}: {case!r}')
    print(json.dumps({
        'schema': 'dfmcp.dig-recovery-query-schema-evidence/1',
        'status': 'passed_json_schema_only',
        'accepted_cases': len(good), 'rejected_cases': len(bad),
        'rust_compiled': False, 'rust_tests_executed': False,
        'mcp_dispatch_executed': False, 'native_or_filesystem_executed': False,
        'schema_sha256': hashlib.sha256(SCHEMA.read_bytes()).hexdigest(),
        'checker_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
