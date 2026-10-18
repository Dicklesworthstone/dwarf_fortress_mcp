"""Real CLI solve/replay/export of complete optimized room layouts.

No native sources, alternate solvers, storage adapters or AST-extracted functions.
Fault hooks only interrupt the actual output/deadline boundary of the offline CLI.
"""
from __future__ import annotations

from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from furniture_plan import canonical, FurniturePlan
from furniture_allocation import Candidate, Request, allocate
from room_provisioning import RoomPlan
import layout_rooms as cli
import room_layout as layout
from test_room_layout_ranking import dining, request, entrance_example, optimized


class Output:
    def __init__(self, *, short=False, fail_flush=False):
        self.buffer = self
        self.raw = b''
        self.writes = 0
        self.short = short
        self.fail_flush = fail_flush

    def write(self, raw):
        self.writes += 1
        self.raw += raw
        return len(raw) - int(self.short)

    def flush(self):
        if self.fail_flush:
            raise OSError('output failed')


class RankedLayoutCLITests(unittest.TestCase):
    def invoke(self, path, source='--request-file', kind='layout', *, strict=False, seed='1', extra=()):
        args = [sys.executable, '-W', 'error', str(ROOT / 'scripts' / 'layout_rooms.py'),
                source, str(path), '--emit', kind, *extra]
        if strict:
            args.append('--require-optimal-layout')
        return subprocess.run(args, capture_output=True, timeout=20,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', 'PYTHONHASHSEED': seed})

    def rejected(self, process, expected):
        self.assertEqual(process.returncode, 2)
        self.assertEqual(process.stderr, b'')
        out = json.loads(process.stdout)
        self.assertFalse(out['ok'])
        self.assertIsNone(out['result'])
        self.assertEqual(out['error']['code'], expected)
        self.assertFalse(out['agent_turn']['briefing']['mutation_admissible'])
        self.assertEqual(len(process.stdout.splitlines()), 1)

    def test_all_six_exports_from_request_and_saved_optimal_layout(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        result = layout.solve(canonical(value))
        body = result.json()
        plan = result.room_plan()
        expected = {'layout': result.encode(), 'report': canonical(cli.report(body)) + b'\n',
                    'room-plan': plan.encode(), 'room-request': canonical(plan.json()['intent']),
                    'excavation': canonical(plan.json()['excavation_blueprint']),
                    'furniture-request': canonical(plan.json()['furniture_request'])}
        with tempfile.TemporaryDirectory() as directory:
            requested, saved = Path(directory) / 'request.json', Path(directory) / 'layout.json'
            requested.write_bytes(canonical(value))
            saved.write_bytes(result.encode())
            for source, path in (('--request-file', requested), ('--layout-file', saved)):
                for kind in cli.EXPORTS:
                    with self.subTest(source=source, kind=kind):
                        process = self.invoke(path, source, kind, strict=True)
                        self.assertEqual((process.returncode, process.stderr), (0, b''))
                        self.assertEqual(process.stdout, expected[kind])
                        if kind == 'room-plan':
                            self.assertEqual(RoomPlan.decode(process.stdout), plan)
                        if kind == 'furniture-request':
                            wanted = Request.decode(process.stdout)
                            items = tuple(Candidate(i, s.kind, s.target, (0, 1), -1)
                                          for i, s in enumerate(wanted.slots))
                            allocated = allocate(wanted, items)
                            self.assertEqual(allocated['status'], 'allocated')
                            executable = FurniturePlan.from_json(allocated['plan'])
                            self.assertEqual(len(executable.steps), 8)
                            self.assertEqual({s.target for s in executable.steps}, {s.target for s in wanted.slots})
            self.assertEqual(self.invoke(requested).stdout, result.encode())

    def test_node_limited_complete_incumbent_exports_without_implying_optimality(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE,
                 'search': {'max_nodes': 3}}
        result = layout.solve(canonical(value))
        self.assertFalse(result.json()['optimization']['minimum_in_compiler_domain_proven'])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'limited.json'
            path.write_bytes(canonical(value))
            plan = self.invoke(path, kind='room-plan')
            self.assertEqual(plan.returncode, 0)
            self.assertEqual(RoomPlan.decode(plan.stdout), result.room_plan())
            report = self.invoke(path, kind='report')
            self.assertEqual(report.returncode, 0)
            briefing = json.loads(report.stdout)['agent_turn']['briefing']['layout_optimization']
            self.assertFalse(briefing['search_complete'])
            self.assertFalse(briefing['minimum_in_compiler_domain_proven'])
            self.assertEqual(briefing['stop_reason'], 'node_limit_reached')
            self.assertEqual(briefing['selected_target_tiles'], result.json()['room_plan']['target_tiles'])
            self.assertIn(b'best-so-far', report.stdout)

    def test_strict_export_rejects_every_node_limited_request_and_saved_export(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE,
                 'search': {'max_nodes': 3}}
        result = layout.solve(canonical(value))
        with tempfile.TemporaryDirectory() as directory:
            requested, saved = Path(directory) / 'request.json', Path(directory) / 'layout.json'
            requested.write_bytes(canonical(value)); saved.write_bytes(result.encode())
            for source, path in (('--request-file', requested), ('--layout-file', saved)):
                for kind in cli.EXPORTS:
                    with self.subTest(source=source, kind=kind):
                        self.rejected(self.invoke(path, source, kind, strict=True), 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
            self.assertEqual(saved.read_bytes(), result.encode())
            self.assertEqual(requested.read_bytes(), canonical(value))

    def test_no_incumbent_and_infeasible_project_never_export_partial_rooms(self):
        cases = [({**entrance_example(), 'search': {'max_nodes': 1}}, 'ROOM_LAYOUT_SEARCH_LIMIT'),
                 (request(dining(), size=(3, 3, 1)), 'ROOM_LAYOUT_NOT_FOUND')]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'
            for value, expected in cases:
                path.write_bytes(canonical({**value, 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}))
                self.rejected(self.invoke(path, kind='room-plan'), expected)
                self.rejected(self.invoke(path, kind='room-plan', strict=True), 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
                report = self.invoke(path, kind='report')
                self.assertEqual(report.returncode, 0)
                body = json.loads(report.stdout)
                self.assertIsNone(body['result']['room_plan'])
                self.assertIsNone(body['agent_turn']['briefing']['layout_optimization']['selected_target_tiles'])

    def test_legacy_layout_does_not_gain_optimality_from_export_flag(self):
        value = entrance_example()
        result = layout.solve(canonical(value))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'legacy.json'
            path.write_bytes(result.encode())
            self.rejected(self.invoke(path, '--layout-file', 'room-plan', strict=True), 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
            normal = self.invoke(path, '--layout-file', 'room-plan')
            self.assertEqual(normal.returncode, 0)
            self.assertEqual(normal.stdout, result.room_plan().encode())
            self.assertEqual(path.read_bytes(), result.encode())

    def test_tampered_optimality_score_or_original_request_cannot_pass_strict_export(self):
        result = optimized({**entrance_example(), 'search': {'max_nodes': 3}})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'tampered.json'
            for kind in ('optimality', 'score', 'objective', 'selection'):
                value = result.json()
                if kind == 'optimality':
                    value['optimization']['minimum_in_compiler_domain_proven'] = True
                    value['optimization']['search_complete'] = True
                elif kind == 'score':
                    value['optimization']['score']['target_tiles'] -= 1
                elif kind == 'objective':
                    value['request'].pop('objective')
                else:
                    value['room_plan']['intent']['areas'][0]['origin'][0] += 1
                # Re-sign the outer digest: regeneration, not a checksum, must fail.
                value.pop('layout_digest')
                value['layout_digest'] = hashlib.sha256(b'dfmcp-room-layout-result/1\0' + canonical(value)).hexdigest()
                path.write_bytes(canonical(value))
                self.rejected(self.invoke(path, '--layout-file', 'room-plan', strict=True), 'ROOM_LAYOUT_REFUSED')

    def test_work_refusal_after_an_incumbent_exports_no_plan(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        limited = {**value, 'search': {'max_nodes': 2}}
        known = layout.solve(canonical(limited))
        self.assertIsNotNone(known.json()['room_plan'])
        value['search'] = {'max_work': known.json()['search']['work_units']}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'exhausted.json'
            path.write_bytes(canonical(value))
            self.rejected(self.invoke(path, kind='room-plan'), 'ROOM_LAYOUT_WORK_LIMIT')

    def test_deadline_failure_after_final_export_does_not_publish_candidate(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        original_export = cli.export
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'
            path.write_bytes(canonical(value))
            reached = []
            def export(result, kind):
                raw = original_export(result, kind)
                reached.append(True)
                return raw
            def check(_owner):
                if reached:
                    raise cli.DeadlineExceeded('expired after rendering')
            out = Output()
            with patch.object(cli, 'export', export), patch.object(cli.Deadline, 'check', check), patch.object(sys, 'stdout', out):
                code = cli.main(['--request-file', str(path), '--emit', 'room-plan', '--require-optimal-layout'])
            self.assertEqual(code, 2)
            self.assertEqual(out.writes, 1)
            body = json.loads(out.raw)
            self.assertEqual(body['error']['code'], 'ROOM_LAYOUT_TIMEOUT')
            self.assertIsNone(body['result'])

    def test_short_output_or_flush_failure_never_repeats_search_or_output(self):
        value = {**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        original_solve = cli.solve
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
            for options in ({'short': True}, {'fail_flush': True}):
                calls = []
                def solve(*args, **kwargs):
                    calls.append(True)
                    return original_solve(*args, **kwargs)
                out = Output(**options)
                with patch.object(cli, 'solve', solve), patch.object(sys, 'stdout', out):
                    code = cli.main(['--request-file', str(path), '--emit', 'room-plan', '--require-optimal-layout'])
                self.assertEqual((code, len(calls), out.writes), (2, 1, 1))

    def test_process_hash_seed_does_not_change_ranked_replay(self):
        value = {**request(dining('b'), dining('a', rotations=[270, 0, 90])),
                 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        result = layout.solve(canonical(value))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
            for seed in ('1', '13', 'random'):
                process = self.invoke(path, seed=seed, strict=True)
                self.assertEqual((process.returncode, process.stderr, process.stdout), (0, b'', result.encode()))

    def test_published_example_retains_all_furnishings_and_original_entrance(self):
        path = ROOT / 'examples' / 'room_layout_minimum_targets.json'
        process = self.invoke(path, kind='report', strict=True)
        self.assertEqual(process.returncode, 0)
        body = json.loads(process.stdout)
        self.assertEqual(body['result']['room_plan']['furniture_count'], 8)
        self.assertEqual(body['result']['room_plan']['project_connections']['access']['point'], [25, 19, 2])
        self.assertEqual(body['agent_turn']['briefing']['layout_optimization']['first_feasible_target_tiles'], 49)
        self.assertEqual(body['agent_turn']['briefing']['layout_optimization']['selected_target_tiles'], 39)
        self.assertFalse(body['agent_turn']['briefing']['mutation_admissible'])
        self.assertFalse(body['result']['global_optimum_proven'])

    def test_invalid_objective_and_request_options_fail_without_plan(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'
            value = {**entrance_example(), 'objective': True}
            path.write_bytes(canonical(value))
            self.rejected(self.invoke(path), 'ROOM_LAYOUT_REFUSED')
            path.write_bytes(canonical(entrance_example()))
            self.rejected(self.invoke(path, extra=('--require-optimal',)), 'ROOM_LAYOUT_REFUSED')

    def test_legacy_report_bytes_remain_unchanged(self):
        # Generated with the hash-verified old CLI, not this additive presenter.
        cases = [entrance_example(), request(dining(), size=(3, 3, 1)),
                 request(search={'max_nodes': 1})]
        digest = hashlib.sha256()
        for value in cases:
            raw = canonical(cli.report(layout.solve(canonical(value)).json())) + b'\n'
            digest.update(len(raw).to_bytes(8, 'big') + raw)
        raw = canonical(cli.report(None, 'ROOM_LAYOUT_REFUSED')) + b'\n'
        digest.update(len(raw).to_bytes(8, 'big') + raw)
        self.assertEqual(digest.hexdigest(), LEGACY_REPORT_DIGEST)


LEGACY_REPORT_DIGEST = 'd8814fd18d90f50db8aba02d77c08c465053db3b8174066a397f7d6601d51571'

if __name__ == '__main__':
    unittest.main()
