#!/usr/bin/env python3
"""Check the closed Manager/1.23 development source inventory; no admission."""
from pathlib import Path
import ast
import json
import re
from read_stable_repository_file import read_stable_regular_file
from check_manager_appointment import SOURCES as LEGACY_SOURCES, METHODS, TOOLS

ROOT = Path(__file__).resolve().parents[1]
ADDITIONS = [
    'architecture/manager_appointment_v1_23.json',
    'docs/MANAGER_APPOINTMENT_V1_23.md',
    'bridge/dfhack-manager-v1_23/CMakeLists.txt',
    'bridge/dfhack-manager-v1_23/DfmcpManagerV1_23.proto',
    'bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp',
    'crates/dfmcp-adapter/src/manager_appointment/tests_v1_23.rs',
    'crates/dfmcp-adapter/src/manager_appointment/production_origin.rs',
    'crates/dfmcp-adapter/src/manager_appointment/production_origin_tests.rs',
    'crates/dfmcp-mcp/src/manager_appointment_v1_23_server.rs',
    'crates/dfmcp-mcp/src/bin/dfmcp-manager-v1-23-dev-server.rs',
    'crates/dfmcp-mcp/tests/manager_appointment_v1_23_stdio.rs',
    'crates/dwarf-fortress-mcp/tests/manager_gameplay_v1_23_cli.rs',
    'tests/native/manager_v1_23/handler.cpp', 'tests/native/manager_v1_23/stubs.h',
    'tests/fixtures/manager_v1_23_component.json', 'scripts/test_manager_v1_23_native.py',
    'scripts/check_manager_v1_23.py', 'scripts/test_manager_v1_23_source_inventory.py',
]
SOURCES = LEGACY_SOURCES + ADDITIONS
NATIVE = [
    'bridge/common/manager_appointment.h', 'bridge/common/one_attempt_native.h',
    'bridge/common/workshop_construction.h', 'bridge/common/retained_snapshot.h',
    'bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp',
    'bridge/dfhack-manager-v1_23/DfmcpManagerV1_23.proto',
    'bridge/dfhack-manager-v1_23/CMakeLists.txt', 'tests/native/manager_v1_23/handler.cpp',
    'tests/native/manager_v1_23/stubs.h', 'tests/fixtures/manager_v1_23_component.json',
    'scripts/test_manager_v1_23_native.py', 'scripts/read_stable_repository_file.py',
]
MUTANTS = ['terminal_link_bound_off_by_one', 'missing_link_capacity_before_prepare', 'terminal_history_bound_off_by_one', 'missing_cache_capacity_before_prepare', 'missing_production_cache', 'empty_event_flag_storage', 'missing_history_link', 'terminal_replay_not_immutable']


def check(root=ROOT):
    texts, hashes = {}, {}
    for name in SOURCES:
        value = read_stable_regular_file(root / name, 4 * 1024 * 1024, name)
        texts[name] = value.content.decode('utf-8', errors='strict')
        if '\0' in texts[name]:
            raise ValueError('NUL in Manager 1.23 input: ' + name)
        hashes[name] = value.sha256
    c = json.loads(texts['architecture/manager_appointment_v1_23.json'])
    if c['source_paths'] != SOURCES or c['native_component_source_paths'] != NATIVE or c['native_mutants'] != MUTANTS:
        raise ValueError('Manager 1.23 closed inventory differs')
    if c['protocol'] != [1, 23] or c['native_methods'] != METHODS or c['tool_names'] != TOOLS:
        raise ValueError('Manager 1.23 protocol or waist differs')
    if c['status'] != 'implemented_unadmitted_development_only' or c['production_admitted'] is not False or c['runtime_admitted'] is not False or c['production_runner_changed'] is not False:
        raise ValueError('Manager 1.23 cannot grant admission')
    if c['native_component_expected'] != {'scenarios': 15, 'assertions': 343, 'ignored': 0, 'sdk_doubles_only': True, 'live_game': False}:
        raise ValueError('Manager 1.23 component denominator differs')
    if hashes['tests/fixtures/manager_v1_23_component.json'] != c['component_vector_file_sha256']:
        raise ValueError('Manager 1.23 retained vectors differ')
    runner = ast.parse(texts['scripts/test_manager_v1_23_native.py'])
    declarations = {node.targets[0].id: node.value for node in runner.body
                    if isinstance(node, ast.Assign) and len(node.targets) == 1
                    and isinstance(node.targets[0], ast.Name)}
    # Resolve the four fixed names used in the literal input list; never execute the runner.
    names = {name: ast.literal_eval(declarations[name]) for name in ('ENGINE', 'TRANSACTION', 'HANDLER', 'TEST', 'VECTORS')}
    values = []
    for item in declarations['INPUTS'].elts:
        values.append(names[item.id] if isinstance(item, ast.Name) else ast.literal_eval(item))
    mutations = declarations['MUTATIONS']
    if values != NATIVE or [ast.literal_eval(k) for k in mutations.keys] != MUTANTS:
        raise ValueError('Manager native runner inputs or mutants differ')
    component_expected = [ast.literal_eval(node.value) for node in ast.walk(runner)
                          if isinstance(node, ast.Assign) and len(node.targets) == 1
                          and isinstance(node.targets[0], ast.Name)
                          and node.targets[0].id == 'expected']
    if component_expected != [c['native_component_expected']]:
        raise ValueError('Manager native runner expected denominator differs')
    native = texts['bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp']
    if re.findall(r's->addFunction\("([^"]+)"', native) != METHODS:
        raise ValueError('Manager 1.23 native method order differs')
    if 'package dfmcp.manager.v1_23;' not in texts['bridge/dfhack-manager-v1_23/DfmcpManagerV1_23.proto']:
        raise ValueError('Manager 1.23 protobuf package differs')
    runtime = texts['crates/dfmcp-mcp/src/manager_appointment_server/runtime.rs']
    for name in c['host_environment']:
        if name not in runtime:
            raise ValueError('Manager 1.23 distinct host gate missing: ' + name)
    if c['responsibility']['index'] != 4 or c['responsibility']['exact_definition_responsibilities'] != [4]:
        raise ValueError('Manager 1.23 cache responsibility differs')
    expected_cache = {'entities': ['group', 'civ'], 'bucket_order': 'native_enum_order',
        'entry_order': 'observed_vector_order', 'ids': 'same_entity_assignment_ids',
        'buckets_max': 128, 'per_bucket_max': 256, 'total_entries_max': 4096,
        'group_civ_domain_equal': True, 'duplicate_or_foreign_or_null_refused': True,
        'pointers_serialized': False}
    if c['role_cache'] != expected_cache:
        raise ValueError('Manager 1.23 cache semantics differ')
    cpp = texts['bridge/common/manager_appointment.h']
    rust = texts['crates/dfmcp-adapter/src/manager_appointment/mod.rs']
    append_bounds = {'selected_hf_links_before_max': 255, 'selected_hf_links_after_max': 256,
        'role_entries_before_max': 4095, 'role_entries_after_max': 4096,
        'history_events_before_max': 65535, 'history_events_after_max': 65536}
    if any(c['transaction'].get(name) != value for name, value in append_bounds.items()):
        raise ValueError('Manager 1.23 append capacity semantics differ')
    for required in ('minor==22 || links.size()<MAX_LINKS', 'minor==22 || role_entries<MAX_TOTAL_ROLE_ENTRIES'):
        if required not in cpp:
            raise ValueError('Manager 1.23 append capacity guard missing')
    for required in ('hf->entity_links.size()<=ma::MAX_LINKS', 'world->history.events.size()<=ma::MAX_HISTORY'):
        if required not in native:
            raise ValueError('Manager 1.23 terminal collection readback bound differs')
    for name, value in [('MAX_ROLE_BUCKETS', 128), ('MAX_ROLE_ENTRIES', 256),
                        ('MAX_TOTAL_ROLE_ENTRIES', 4096), ('MANAGE_PRODUCTION', 4)]:
        if not re.search(r'\b'+name+r'\s*=\s*'+str(value)+r'\b', cpp):
            raise ValueError('Manager 1.23 native cache limit differs: '+name)
        if not re.search(r'\b'+name+r'\s*:\s*usize\s*=\s*'+str(value)+r'\s*;', rust):
            raise ValueError('Manager 1.23 Rust cache limit differs: '+name)
    for gate in ['DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23', 'DFMCP_MANAGER_V1_23_TOKEN', 'DFMCP_MANAGER_V1_23_ALLOW_APPOINT']:
        if gate not in native:
            raise ValueError('Manager 1.23 native gate missing: '+gate)

    wrapper = texts['crates/dfmcp-mcp/src/manager_appointment_v1_23_server.rs']
    if wrapper.index('DFMCP_ADMITTED_BRIDGE_PROTOCOL') > wrapper.index('run_stdio_for_protocol(') or 'ManagerProtocol::V1_23' not in wrapper:
        raise ValueError('Manager 1.23 production state guard missing')
    if '"manager-v1-23" => dfmcp_mcp::manager_appointment_v1_23_server::run_stdio()' not in texts['crates/dwarf-fortress-mcp/src/main.rs']:
        raise ValueError('Manager 1.23 CLI route missing')
    for name in ['scripts/verify.sh', 'scripts/qualify_local.sh']:
        if 'python3 scripts/check_manager_v1_23.py' not in texts[name] or 'python3 scripts/test_manager_v1_23_source_inventory.py' not in texts[name] or 'python3 scripts/test_manager_v1_23_native.py --mutations' not in texts[name]:
            raise ValueError('Manager 1.23 standard gate missing: ' + name)
    for name in ['crates/dfmcp-mcp/tests/manager_appointment_v1_23_stdio.rs', 'crates/dwarf-fortress-mcp/tests/manager_gameplay_v1_23_cli.rs']:
        if '--profile=1.23' not in texts[name] or 'assert_eq!(report["tests"], 8);' not in texts[name]:
            raise ValueError('Manager 1.23 real subprocess wrapper missing: ' + name)
    return {'status': 'passed_static_only', 'source_sha256': hashes, 'source_files': len(SOURCES), 'rust_executed': False, 'live_game': False}


if __name__ == '__main__':
    print(json.dumps(check(), sort_keys=True))
