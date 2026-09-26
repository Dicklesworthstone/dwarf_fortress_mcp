#!/usr/bin/env python3
"""Execute JSON contract/independent predicate-fixture checks, NOT Rust or MCP.

Rust regression source separately checks that actual generated predicates equal
this fixture and runs them through the real shared condition/watch engine. Those
Rust tests must be compiled and executed separately; this script cannot do that.
"""
from __future__ import annotations

import copy
import hashlib
import itertools
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
B, I = str((1 << 40) + 10), str((2 << 40) + 20)
SCHEMA = ROOT / 'schemas/mcp_construction_progress_v1.json'
FIXTURE = ROOT / 'crates/dfmcp-mcp/tests/fixtures/construction_condition_v1.json'


def combine(values, all_values=True):
    decisive = False if all_values else True
    if decisive in (v for v in values if v is not None):
        return decisive
    return None if any(v is None for v in values) else all_values


def roots(node):
    if node['op'] == 'related':
        return [(node['entity_id'], node['generation'])]
    return [pair for arg in node.get('args', []) for pair in roots(arg)]


def evaluate(node, entities, edges, row=None):
    op = node['op']
    if op in ('all', 'any'):
        return combine([evaluate(arg, entities, edges, row) for arg in node['args']], op == 'all')
    if op == 'field':
        entity = entities.get(node['entity_id']) if 'entity_id' in node else row
        if entity is None or ('generation' in node and entity['generation'] != node['generation']):
            return None
        if node['field'] not in entity['fields']:
            return None
        actual = entity['fields'][node['field']]
        expected = node['value']['value']
        return type(actual) is type(expected) and actual == expected
    if op == 'related':
        root = entities.get(node['entity_id'])
        if root is None or root['generation'] != node['generation']:
            return None
        return any(kind == node['relation'] and
                   ((src == row['id'] and dst == root['id']) if node['direction'] == 'incoming'
                    else (src == root['id'] and dst == row['id'])) for src, dst, kind in edges)
    if op != 'entity_count':
        raise ValueError('unsupported independent fixture operation')
    # Prebind even over empty populations, as required by the real contract.
    if any(root not in entities or entities[root]['generation'] != generation
           for root, generation in roots(node['predicate'])):
        return None
    values = [evaluate(node['predicate'], entities, edges, entity)
              for entity in entities.values() if entity['kind'] == node['kind']]
    low = sum(v is True for v in values)
    high = low + sum(v is None for v in values)
    expected = node['value']
    if node['comparison'] == 'gt':
        return True if low > expected else False if high <= expected else None
    return True if low == high == expected else False if not low <= expected <= high else None


def sample(stage=3, maximum=3, flags=256, holder=True, container=False,
           attached=False, construction=False, removal=False):
    def entity(id_, kind, fields):
        return {'id': id_, 'kind': kind, 'generation': 1, 'fields': fields}
    entities = {
        B: entity(B, 'building', {'type_key': 'Bed', 'build_stage': stage, 'max_build_stage': maximum}),
        I: entity(I, 'item', {'type_key': 'BED', 'native_item_id': 20,
            **{name: bool(flags & (1 << bit)) for bit, name in
               [(8, 'in_building'), (1, 'in_job'), (3, 'removed'), (6, 'on_ground'), (7, 'in_inventory')]}}),
    }
    edges = [(I, B, 'contained_in')] if holder else []
    for enabled, id_, kind in [(construction, '7', 'ConstructBuilding'), (removal, '8', 'DestroyBuilding'),
                              (attached, '9', 'StoreItemInStockpile')]:
        if enabled:
            entities[id_] = entity(id_, 'job', {'type_key': kind})
            if id_ == '9':
                edges.append((id_, I, 'uses'))
            else:
                edges.append((id_, B, 'contained_in'))
    if container:
        entities['21'] = entity('21', 'item', {'native_item_id': 21})
        edges.append((I, '21', 'contained_in'))
    return entities, edges


def main():
    schema = json.loads(SCHEMA.read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    base = {'kind': 'construction_progress', 'targets': [{'building_native_id': 10}]}
    valid = [base]
    for count in (1, 8, 32):
        for limit in (1, 4, 32):
            for kind in ('Bed', 'Chair', 'Table'):
                valid.append({**base, 'targets': [{'building_native_id': i, 'expected_type': kind,
                    'expected_generation': 1, 'item_native_id': 100 + i} for i in range(count)], 'limit': limit,
                    'monitor': {'key_prefix': 'rooms', 'deadline_tick': 200}})
    valid.append({**base, 'monitor': None, 'limit': None, 'continuation': None, 'max_work': None})
    invalid = []
    for field, values in {
        'kind': ['build', 'watch'], 'targets': [[], {}, [base['targets'][0]] * 33],
        'limit': [0, 33, True, '4'], 'max_work': [0, 1000001, True],
        'continuation': ['', 'sp1:0:' + 'a' * 64, 'sp1:1:' + 'a' * 64 + '\n'],
    }.items():
        invalid.extend({**base, field: value} for value in values)
    for field in ('commit', 'path', 'protocol', 'token', 'lua'):
        invalid.append({**base, field: 'forbidden'})
        invalid.append({**base, 'targets': [{'building_native_id': 10, field: 'forbidden'}]})
    for field, values in {'building_native_id': [-1, 2147483647, True, '10'],
                          'item_native_id': [-1, 2147483647, True],
                          'expected_generation': [0, 4294967296],
                          'expected_type': ['', 'Workshop', 'bed']}.items():
        invalid.extend({**base, 'targets': [{**base['targets'][0], field: value}]} for value in values)
    for field, values in {'key_prefix': ['', 'x' * 33, 'é', 'a\n', 'a\0', 'a/b'],
                          'deadline_tick': [0, 18446744073709551616, True],
                          'poll_interval_ticks': [0, 1000001],
                          'stable_observations': [0, 65]}.items():
        invalid.extend({**base, 'monitor': {'key_prefix': 'rooms', 'deadline_tick': 200, field: value}}
                       for value in values)
    for value in valid:
        validator.validate(value)
    for value in invalid:
        assert list(validator.iter_errors(value)), value
    fixture = json.loads(FIXTURE.read_text())
    predicate_cases = 0
    for flags, holder, container, attached in itertools.product(range(512), (False, True), (False, True), (False, True)):
        entities, edges = sample(flags=flags, holder=holder, container=container, attached=attached)
        # Independent arithmetic policy, not bitmask reuse or a Rust execution.
        expected = flags // 256 % 2 == 1 and all(flags // (2 ** bit) % 2 == 0 for bit in (1, 3, 6, 7))
        expected = expected and holder and not container and not attached
        assert evaluate(fixture['condition'], entities, edges) is expected
        assert evaluate(fixture['failure_condition'], entities, edges) is False
        predicate_cases += 1
    for stage, construction, removal in itertools.product(range(4), (False, True), (False, True)):
        entities, edges = sample(stage=stage, construction=construction, removal=removal)
        assert evaluate(fixture['condition'], entities, edges) is (stage == 3 and not construction and not removal)
        assert evaluate(fixture['failure_condition'], entities, edges) is removal
        predicate_cases += 1
    for missing in (B, I):
        entities, edges = sample()
        del entities[missing]
        edges = [e for e in edges if missing not in e[:2]]
        assert evaluate(fixture['condition'], entities, edges) is not True
        predicate_cases += 1
    nodes = 0
    depth = 0
    pending = [(fixture['condition'], 0), (fixture['failure_condition'], 0)]
    while pending:
        node, current = pending.pop()
        nodes += 1; depth = max(depth, current)
        pending.extend((arg, current + 1) for arg in node.get('args', []))
        if 'predicate' in node:
            pending.append((node['predicate'], current + 1))
    assert nodes <= 64 and depth <= 8
    # Fixture is only the predicates; this is not a complete MCP response size.
    raw = json.dumps(fixture, separators=(',', ':'), sort_keys=True).encode()
    print(json.dumps({'schema': 'dfmcp.construction-reference-check/1',
        'evidence_scope': 'executed JSON value-shape and independent predicate-fixture reference only',
        'schema_valid': len(valid), 'schema_invalid': len(invalid),
        'predicate_reference_cases': predicate_cases, 'fixture_condition_nodes': nodes,
        'fixture_condition_depth': depth, 'fixture_predicate_bytes': len(raw),
        'schema_sha256': hashlib.sha256(SCHEMA.read_bytes()).hexdigest(),
        'fixture_sha256': hashlib.sha256(FIXTURE.read_bytes()).hexdigest(),
        'rust_compiled': False, 'rust_tests_executed': False, 'mcp_executed': False,
        'live_fortress': False, 'full_repository_qualification': False}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
