#!/usr/bin/env python3
"""Request-schema checks only; does not execute Rust or the allocation solver."""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    schema_path = root / 'schemas/mcp_production_portfolio_v1.json'
    data = schema_path.read_bytes()
    schema = json.loads(data)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    seed = {'kind': 'production_portfolio', 'origin': [0, 0, 5], 'quantity_unit': 'stack_units',
            'tasks': [{'key': 'wood', 'workers': 1, 'skill_key': 'CARPENTRY',
                       'materials': [{'key': 'input', 'units': 1, 'item_types': ['WOOD']}]}]}
    cases: list[tuple[str, object, bool]] = [('minimal', seed, True)]

    def changed(path: tuple[str | int, ...], value: object, accepted: bool) -> None:
        query = copy.deepcopy(seed)
        target = query
        for key in path[:-1]:
            target = target[key]
        target[path[-1]] = value
        cases.append((f'{path!r}={value!r}', query, accepted))

    for count in [1, 2, 8]:
        entries = [dict(copy.deepcopy(seed['tasks'][0]), key=f't{i}') for i in range(count)]
        changed(('tasks',), entries, True)
    for name, values in {
        'priority': [None, 1, 1000000], 'min_effective_skill': [None, 0, 2147483647],
        'preserve_social': [None, False, True], 'adults_only': [None, False, True]
    }.items():
        for value in values:
            changed(('tasks', 0, name), value, True)
    for name, values in {'limit': [None, 1, 128], 'max_work': [None, 1, 10000000],
                         'continuation': [None, 'pp1:1:' + 'a' * 64]}.items():
        for value in values:
            changed((name,), value, True)
    for value in [1, 18446744073709551615]:
        changed(('tasks', 0, 'materials', 0, 'units'), value, True)
    for value in [[], [0, 0], [0, 0, 0, 0], [-1, 0, 5], [32768, 0, 5], ['0', 0, 5]]:
        changed(('origin',), value, False)
    for value in [None, [], [seed['tasks'][0]] * 9]:
        changed(('tasks',), value, False)
    for name, values in {
        'key': ['', 'space key', 'a' * 33], 'priority': [0, -1, 1000001, True, '1'],
        'workers': [0, -1, 129, None, True, '1'], 'skill_key': ['', 'a' * 97, '\x00', 'a\nb'],
        'min_effective_skill': [-1, 2147483648, True], 'adults_only': [0, 'true'],
        'preserve_social': [1, 'false'], 'unknown': [True],
        'materials': [[], None, seed['tasks'][0]['materials'] * 5]
    }.items():
        for value in values:
            changed(('tasks', 0, name), value, False)
    for name, values in {
        'key': ['', 'a' * 49, 'bad/key'], 'units': [0, -1, 18446744073709551616, True, '1'],
        'item_types': [[], ['WOOD'] * 9, [''], ['a' * 129], ['a\x00b']],
        'subtype': [-2, 2147483648], 'material_type': [-2, 2147483648],
        'material_index': [1, -2], 'unknown': [True]
    }.items():
        for value in values:
            changed(('tasks', 0, 'materials', 0, name), value, False)
    for name, values in {
        'kind': ['workforce_plan', None], 'quantity_unit': ['items', None],
        'limit': [0, 129, True], 'max_work': [0, 10000001, True],
        'continuation': ['pp1:0:' + 'a' * 64, 'pp1:01:' + 'a' * 64, 'wp1:1:' + 'a' * 64, 'bad'],
        'unknown': [True]
    }.items():
        for value in values:
            changed((name,), value, False)
    for key in ['kind', 'origin', 'quantity_unit', 'tasks']:
        query = copy.deepcopy(seed)
        del query[key]
        cases.append((f'missing {key}', query, False))
    paired = copy.deepcopy(seed)
    paired['tasks'][0]['materials'][0].update(material_type=0, material_index=1)
    cases.append(('paired material identity', paired, True))
    for label, query, expected in cases:
        actual = validator.is_valid(query)
        if actual != expected:
            raise AssertionError(f'{label}: expected {expected}, got {actual}')
    blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
    result = {'schema_cases': len(cases), 'accepted': sum(expected for _, _, expected in cases),
              'rejected': sum(not expected for _, _, expected in cases),
              'schema_sha256': hashlib.sha256(data).hexdigest(), 'schema_git_blob': blob,
              'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'evidence': 'request_schema_only_not_rust_allocation_replay_or_mcp_execution'}
    text = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(text)
    print(text, end='')


if __name__ == '__main__':
    main()
