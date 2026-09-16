#!/usr/bin/env python3
"""Check population-condition schema and an independent interval model, not Rust.

Only the base schema's name/watch_literal definitions are used. No MCP envelope,
replay, authority, scan budget, publication or native-game behavior is executed.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import itertools
import json
import operator
from pathlib import Path
from jsonschema import Draft202012Validator

OPS = {'eq': operator.eq, 'ne': operator.ne, 'lt': operator.lt,
       'le': operator.le, 'gt': operator.gt, 'ge': operator.ge}


def interval(lo: int, hi: int, op: str, target: int) -> bool | None:
    yes, no = {
        'eq': (lo == hi == target, target < lo or target > hi),
        'ne': (target < lo or target > hi, lo == hi == target),
        'lt': (hi < target, lo >= target),
        'le': (hi <= target, lo > target),
        'gt': (lo > target, hi <= target),
        'ge': (lo >= target, hi < target),
    }[op]
    return True if yes else False if no else None


def run(root: Path) -> dict:
    extension_path = root / 'schemas/mcp_watch_count_v1.json'
    extension_bytes = extension_path.read_bytes()
    extension = json.loads(extension_bytes)
    base = json.loads((root / 'schemas/mcp_query_v1.json').read_bytes())
    definitions = {key: base['$defs'][key] for key in ('name', 'watch_literal')}
    schema = dict(extension['condition'], **{'$defs': dict(definitions, **extension['$defs'])})
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    leaf = {'op': 'field', 'field': 'suspended', 'comparison': 'eq',
            'value': {'type': 'bool', 'value': True}}
    template = {'op': 'entity_count', 'scope': 'observed_projection', 'kind': 'job',
                'predicate': leaf, 'comparison': 'ge', 'value': 3}
    valid = []
    for kind, op, count in itertools.product(
        ('unit', 'job', 'building', 'item', 'tile_feature', 'announcement'), OPS,
        (0, 3, 2**64 - 1)):
        valid.append(dict(template, kind=kind, comparison=op, value=count))
    for predicate in ({'op': 'always'}, {'op': 'not', 'arg': leaf},
                      {'op': 'all', 'args': [leaf, {'op': 'always'}]},
                      {'op': 'any', 'args': [leaf, {'op': 'always'}]}):
        valid.append(dict(template, predicate=predicate))
    invalid = []
    for key in template:
        item = copy.deepcopy(template)
        del item[key]
        invalid.append(item)
    for key, values in {
        'scope': ['complete_world', '', None, True],
        'kind': ['other:unit', 'citizen', '', None, ['job']],
        'value': [-1, 2**64, 1.5, '3', None, True],
        'comparison': ['=', 'gte', '', None],
        'predicate': [None, {}, {'op': 'always', 'unexpected': True},
                      {'op': 'all', 'args': []}, {'op': 'any', 'args': []},
                      {'op': 'entity_count'}, {'op': 'not'},
                      {'op': 'all', 'args': [leaf] * 65}],
    }.items():
        invalid.extend(dict(template, **{key: value}) for value in values)
    invalid.append(dict(template, extra=True))
    for field in ('', 'x' * 129, 'x\0y'):
        invalid.append(dict(template, predicate=dict(leaf, field=field)))
    for value in ({'type': 'bool', 'value': 'true'}, {'type': 'text', 'value': 'x' * 1025},
                  {'type': 'text', 'value': 'a\0b'}):
        invalid.append(dict(template, predicate=dict(leaf, value=value)))
    for item in valid:
        assert validator.is_valid(item), ('rejected valid condition', item)
    for item in invalid:
        assert not validator.is_valid(item), ('accepted invalid condition', item)
    interval_cases = 0
    for lo in range(9):
        for hi in range(lo, 9):
            for op, operation in OPS.items():
                for target in range(11):
                    answers = {operation(n, target) for n in range(lo, hi + 1)}
                    expected = next(iter(answers)) if len(answers) == 1 else None
                    assert interval(lo, hi, op, target) is expected
                    interval_cases += 1
    population_cases = 0
    for size in range(7):
        for rows in itertools.product((False, True, None), repeat=size):
            lo = rows.count(True)
            hi = lo + rows.count(None)
            possible_counts = {lo + sum(bits) for bits in itertools.product(
                (False, True), repeat=rows.count(None))}
            for op, operation in OPS.items():
                for target in range(9):
                    answers = {operation(n, target) for n in possible_counts}
                    expected = next(iter(answers)) if len(answers) == 1 else None
                    assert interval(lo, hi, op, target) is expected
                    population_cases += 1
    reference = json.dumps(definitions, sort_keys=True, separators=(',', ':')).encode()
    return {'status': 'passed', 'scope': 'condition-schema-and-independent-count-model-only',
            'schema_accepted': len(valid), 'schema_rejected': len(invalid),
            'interval_cases': interval_cases, 'population_completion_cases': population_cases,
            'extension_sha256': hashlib.sha256(extension_bytes).hexdigest(),
            'reference_definitions_sha256': hashlib.sha256(reference).hexdigest(),
            'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'rust_executed': False, 'mcp_executed': False, 'qualification_established': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    print(json.dumps(run(args.root), indent=2, sort_keys=True))
