#!/usr/bin/env python3
"""Structural development checks; not execution of the Rust or DFHack sources."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def validate() -> dict:
    schema = json.loads((ROOT / 'schemas/mcp_shelter_status_v1.json').read_text())
    example = json.loads((ROOT / 'schemas/examples/shelter_status_v1.json').read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    base = example['query']
    validator.validate(base)
    refused = []

    def bad(path: tuple, value: object) -> None:
        candidate = copy.deepcopy(base)
        parent = candidate
        for key in path[:-1]:
            parent = parent[key]
        parent[path[-1]] = value
        if validator.is_valid(candidate):
            raise AssertionError(f'malformed structural input was accepted: {path!r}, {value!r}')
        refused.append({'path': list(path), 'case': len(refused) + 1})

    for key in ('kind', 'citizens', 'shelters'):
        candidate = copy.deepcopy(base)
        del candidate[key]
        assert not validator.is_valid(candidate), key
        refused.append({'missing': key})
    for value in ('labor_allocation', 'shelter_status_v2', '', None):
        bad(('kind',), value)
    for key in ('authority', 'source_digest', 'position', 'safe', 'native_endpoint'):
        bad((key,), True)
    for value in ('0', '01', '+1', '-1', '1\n', '1 ', '１', '', '9' * 21, 10, None):
        bad(('citizens', 0, 'entity_id'), value)
    for value in (0, -1, 4294967296, 1.5, '1', True, None):
        bad(('citizens', 0, 'generation'), value)
    for value in ([], base['citizens'] * 65, [base['citizens'][0]] * 2):
        bad(('citizens',), value)
    bad(('citizens', 0, 'position'), [0, 0, 5])
    for value in ([], base['shelters'] * 17, base['shelters'] * 2):
        bad(('shelters',), value)
    for field in ('min', 'max'):
        for value in ([0, 0], [0, 0, 0, 0], [-1, 0, 5], [32768, 0, 5], [0, 0.5, 5], ['0', 0, 5], [False, 0, 5], None):
            bad(('shelters', 0, field), value)
    bad(('shelters', 0, 'native_burrow'), '3001')
    for value in (0, 129, -1, 1.5, '16', False):
        bad(('limit',), value)
    for value in (0, 1000001, -1, 1.5, '1000', True):
        bad(('max_work',), value)
    token = 'ss1:1:' + 'a' * 64
    for value in ('', token + '\n', token + ' ', 'wp1:1:' + 'a' * 64,
                  'ss1:0:' + 'a' * 64, 'ss1:01:' + 'a' * 64,
                  'ss1:1:' + 'A' * 64, 'ss1:1:' + 'a' * 63, 1):
        bad(('continuation',), value)

    accepted = 0
    for field, values in (
        ('limit', (None, 1, 128)),
        ('max_work', (None, 1, 1000000)),
        ('continuation', (None, token)),
    ):
        for value in values:
            candidate = copy.deepcopy(base)
            candidate[field] = value
            validator.validate(candidate)
            accepted += 1
    maximum = copy.deepcopy(base)
    maximum['citizens'] = [{'entity_id': str((3 << 40) + i), 'generation': 1} for i in range(128)]
    maximum['shelters'] = [{'min': [i, 0, 5], 'max': [i, 0, 5]} for i in range(16)]
    validator.validate(maximum)
    accepted += 1
    return {
        'schema_metaschemas_passed': 1,
        'checked_in_examples_passed': 1,
        'valid_optional_and_maximum_cases_passed': accepted,
        'malformed_structural_cases_rejected': len(refused),
        'rust_tests_executed': False,
        'native_game_tests_executed': False,
        'scope': 'JSON Schema structural checks only; no Rust decoder, source validation, runtime, or native-game execution',
    }


if __name__ == '__main__':
    print(json.dumps(validate(), sort_keys=True, indent=2))
