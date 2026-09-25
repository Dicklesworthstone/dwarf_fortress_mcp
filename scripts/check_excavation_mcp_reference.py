#!/usr/bin/env python3
"""Executed schema/fixture/sizing references; NOT execution of the Rust MCP server."""
import hashlib
import json
from pathlib import Path
import re
import struct

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'crates/dfmcp-adapter/tests/fixtures'


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()


def git_blob(raw):
    return hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()


def field(raw):
    return struct.pack('>I', len(raw)) + raw


def check_schema():
    schema = json.loads((ROOT / 'schemas/mcp_excavation_run_v1.json').read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    digest = 'a' * 64
    identity = {'key': 'goal', 'plan_digest': digest}
    plan = {'key': 'goal', 'observation_witness': digest, 'game_ticks': 100, 'wall_millis': 1000}
    accepted = [identity, dict(identity, confirm=True), dict(identity, confirm=False), plan,
                {'kind': 'schema'}, {'kind': 'records'}, {'scope': 'session'}]
    accepted += [dict(identity, scope=scope) for scope in ('plan', 'effect')]
    accepted += [{'scope': 'session', 'release_for_recovery': value} for value in (True, False, None)]
    accepted += [dict(plan, **{name: value}) for name, values in (
        ('samples', (None, 1, 128)), ('stable_ticks', (None, 0, 1200)),
        ('interval_ticks', (None, 1, 1200)), ('max_gap_ticks', (None, 1, 1200))) for value in values]
    accepted += [{'kind': 'records', 'state': state, 'limit': limit, 'continuation': token}
                 for state in ('all', 'unresolved', 'terminal', None)
                 for limit in (1, 4, None) for token in (None, digest)]
    rejected = []
    for base in (identity, plan, dict(identity, confirm=True), {'kind': 'schema'},
                 {'kind': 'records'}, {'scope': 'session'}, dict(identity, scope='effect')):
        for extra in ('path', 'endpoint', 'token', 'native_method', 'raw_lua', 'force', 'protocol'):
            rejected.append(dict(base, **{extra: 'untrusted'}))
    for key in ('', 'a' * 129, 'a\n', 'a/b', 'a\0b', 'é', True, 1, None):
        rejected.append(dict(identity, key=key))
    for digest in ('A' * 64, 'a' * 65, 'a' * 63, 'a' * 63 + '\n', 'g' * 64, None, 1):
        rejected.append(dict(identity, plan_digest=digest))
    for name, values in (('game_ticks', (0, 1201, True, None, '100')),
                         ('wall_millis', (0, 60001, False, '1000')),
                         ('samples', (0, 129, True)), ('stable_ticks', (-1, 1201, True)),
                         ('interval_ticks', (0, 1201)), ('max_gap_ticks', (0, 1201))):
        rejected += [dict(plan, **{name: value}) for value in values]
    rejected += [{'kind': 'records', 'limit': value} for value in (0, 5, True, '4')]
    rejected += [{'kind': 'records', 'state': value} for value in ('active', 'control', [], 1)]
    rejected += [dict(identity, confirm=value) for value in (1, 'true', None)]
    rejected += [{'scope': 'session', 'key': 'goal'}, {'scope': 'effect'}, {'kind': 'native'}, []]
    for value in accepted:
        assert validator.is_valid(value), value
    for value in rejected:
        assert not validator.is_valid(value), value
    opening = Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/open_session'})
    for value in ({}, {'mode': 'offline'}, {'mode': 'recover'}, {'mode': 'control'},
                  {'max_bytes': 64 * 1024 * 1024, 'max_output_tokens': 8192}):
        assert opening.is_valid(value), value
    for value in ({'mode': 'production'}, {'max_output_tokens': 8191}, {'max_wall_millis': 60001},
                  {'max_bytes': 16 * 1024 * 1024 + 32768}, {'journal_path': '/tmp/journal'}):
        assert not opening.is_valid(value), value
    return {'accepted': len(accepted) + 5, 'rejected': len(rejected) + 5,
            'scope': 'JSON value shape only; byte extent, duplicate fields and integer lexical forms require Rust parsing'}


def check_native_and_inventory():
    expected = {
        'intent': '5a997f043d15461e8b1da330f90dd526b2cc25d5',
        'prepared': 'd335287ca02259886aabf94cd52beed8ed5b63c0',
        'stopped': 'cad4fd3a2874e93d68b18261fb6e73430ca0793e',
    }
    values = {}
    for name, digest in expected.items():
        raw = (FIXTURES / f'excavation_run_{name}_v1_18.hex').read_bytes()
        assert git_blob(raw) == digest, name
        values[name] = bytes.fromhex(raw.decode().strip())
    intent = values['intent']
    assert intent[:8] == b'DFMEP018'
    key_len = struct.unpack_from('>H', intent, 8)[0]
    key = intent[10:10 + key_len]
    spec_start = 10 + key_len
    spec = intent[spec_start:spec_start + 24]
    capture_size = struct.unpack_from('>H', intent, spec_start + 24)[0]
    capture = intent[spec_start + 26:]
    assert len(capture) == capture_size == 106
    assert struct.unpack('>6I', spec) == (100, 1000, 2, 2, 1, 10)
    plan = hashlib.sha256(b'dfmcp-excavation-run-plan/1\0' + spec + capture).digest()
    token = hashlib.sha256(b'dfmcp-excavation-run-token/1\0' + struct.pack('>H', key_len) + key + plan).digest()[:16]
    for name in ('prepared', 'stopped'):
        raw = values[name]
        assert raw[:8] == b'DFMER018' and raw[8:len(intent)] == intent[8:]
        assert raw[len(intent):len(intent) + 48] == plan + token
        assert hashlib.sha256(b'dfmcp-excavation-run-receipt/1\0' + raw[:-32]).digest() == raw[-32:]
    # Native fixture scope: region1/site 2, generation 41, dimensions 64x64x8.
    assert capture[:16] == b'DFMEC018DFMRO013'
    assert struct.unpack_from('>QQQ', capture, 16) == (41, 3, 806500)
    assert struct.unpack_from('>4I', capture, 43) == (2, 64, 64, 8)
    prefix = (b'dfmcp-excavation-inventory/1\0' + field(b'127.0.0.1:5000')
              + field(b'region1') + struct.pack('>IQ3I', 2, 41, 64, 64, 8)
              + field(b'df') + field(b'dfhack'))
    vectors = {'empty': hashlib.sha256(prefix + struct.pack('>I', 0)).hexdigest()}
    for name in ('prepared', 'stopped'):
        # Both represent dispatched coordinator records; prepared remains pending.
        entry = field(intent) + bytes((1, 0, 1)) + field(values[name])
        vectors[name] = hashlib.sha256(prefix + struct.pack('>I', 1) + entry).hexdigest()
    return {'fixture_blobs': expected, 'native_plan': plan.hex(), 'native_token': token.hex(),
            'inventory_vectors': vectors, 'rust_codec_executed': False}


def sampling_reference():
    cases = 0
    for ticks in (1, 2, 5, 10, 100, 1200):
        for samples in (1, 2, 3, 128):
            for stable in (0, 1, ticks - 1, ticks):
                for interval in (1, 2, ticks):
                    # Earliest feasible count sequence is interval,2*interval,...;
                    # the last sample may be delayed to meet the stable span.
                    earliest = max(samples * interval, interval + stable)
                    algebra = interval + max((samples - 1) * interval, stable)
                    assert earliest == algebra
                    valid = interval <= ticks and earliest < ticks
                    assert valid == (1 <= interval <= ticks and algebra < ticks)
                    cases += 1
    return {'cases': cases, 'scope': 'independent arithmetic feasibility reference, not native callbacks or Rust evaluation'}


def output_reference():
    # Overestimate the fixed Agent Turn, summaries, review, pending identity and
    # error/recovery prose by 8192 bytes beyond the variable fields below.
    # This is a sizing model, not execution of serde_json or the Rust renderer.
    source = {'world_folder': '\1' * 512, 'df_version': '\1' * 128,
              'dfhack_version': '\1' * 128, 'site_id': 2**31 - 1,
              'generation': 2**64 - 1, 'dimensions': [32768] * 3}
    cells = [{'coordinate': [32767] * 3, 'presence': 'visible', 'shape': 8,
              'liquid_depth': 7, 'dig': 7} for _ in range(64)]
    record = {'key': 'g' * 128, 'plan_digest': 'f' * 64, 'dispatch_started': True,
              'cancel_requested': True, 'terminal': True, 'unresolved': True,
              'native': {'phase': 'source_lost', 'reason': 'clock_regression',
                         'trigger': 'capture_failure', 'receipt_digest': 'f' * 64,
                         'observed_tick': 2**64 - 1, 'stable_samples': 128,
                         'first_stable_tick': 2**64 - 1, 'last_sample_tick': 2**64 - 1,
                         'historical_pause_verified': False, 'sampled_floor_reported': False},
              'current_pause_proven': False, 'mining_causality_proven': False,
              'retry_commit_permitted': False}
    schema = json.loads((ROOT / 'schemas/mcp_excavation_run_v1.json').read_text())
    families = {'explain': {'before': cells, 'sample': cells, 'record': record},
                'records': {'rows': [record] * 4, 'pending': [record] * 4},
                'schema': {'schema': schema, 'pending': [record] * 4}}
    sizes = {name: len(canonical(dict(body, source=source))) + 8192 for name, body in families.items()}
    assert max(sizes.values()) <= 32768, sizes
    return {'conservative_reference_bytes': sizes, 'limit': 32768, 'rust_renderer_executed': False}


def source_checks():
    paths = [
        'crates/dfmcp-adapter/src/excavation_run/session.rs',
        'crates/dfmcp-adapter/src/excavation_run/session/native.rs',
        'crates/dfmcp-adapter/src/excavation_run/session/tests.rs',
        'crates/dfmcp-mcp/src/live_excavation_run_server.rs',
        'crates/dfmcp-mcp/src/live_excavation_run_server/requests.rs',
        'crates/dfmcp-mcp/src/live_excavation_run_server/presentation.rs',
        'crates/dfmcp-mcp/src/live_excavation_run_server/runtime.rs',
        'crates/dfmcp-mcp/src/live_excavation_run_server/tests.rs',
        'crates/dfmcp-mcp/src/bin/dfmcp-excavation-run-dev-server.rs',
        'crates/dfmcp-mcp/src/lib.rs',
        'schemas/mcp_excavation_run_v1.json',
        'scripts/check_excavation_mcp_reference.py',
    ]
    digests = {}
    for name in paths:
        raw = (ROOT / name).read_bytes()
        assert b'\0' not in raw and len(raw) <= 100000
        raw.decode('utf-8')
        digests[name] = hashlib.sha256(raw).hexdigest()
    server = (ROOT / paths[3]).read_text()
    names = re.findall(r'#\[tool\(name="([^"]+)"', server)
    assert sorted(names) == sorted('fortress.' + name for name in (
        'open_session', 'observe', 'query', 'plan', 'commit', 'wait', 'cancel',
        'checkpoint', 'restore', 'explain', 'doctor'))
    assert 'pub mod live_excavation_run_server;' in (ROOT / paths[9]).read_text()
    assert 'spawn_blocking' in (ROOT / paths[6]).read_text()
    assert 'mod tests;' in server and (ROOT / paths[7]).exists()
    return {'source_sha256': digests, 'registered_tools_static': len(names),
            'rust_mcp_groups_unexecuted': (ROOT / paths[7]).read_text().count('#[test]'),
            'runtime_groups_unexecuted': (ROOT / paths[6]).read_text().count('#[test]'),
            'binary_groups_unexecuted': (ROOT / paths[8]).read_text().count('#[test]')}


def main():
    print(json.dumps({'schema': 'dfmcp.excavation-mcp-reference-evidence/1',
        'evidence_class': 'executed_python_references_and_static_rust_inventory_only',
        'schema_checks': check_schema(), 'native_fixture_reference': check_native_and_inventory(),
        'sampling_reference': sampling_reference(), 'output_reference': output_reference(),
        'static_source': source_checks(), 'rust_compiled': False, 'rust_tests_executed': False,
        'mcp_execution': False, 'real_dfhack_sdk': False, 'live_fortress': False,
        'full_repository_qualification': False}, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
