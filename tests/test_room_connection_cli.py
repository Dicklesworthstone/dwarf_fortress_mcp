"""Real subprocess solve/replay/export of the bounded corridor fallback."""
from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import layout_rooms as cli
from furniture_plan import canonical
from test_room_connection_layout import request, solve


class FallbackCLITests(unittest.TestCase):
    def invoke(self, path, kind='layout', *, replay=False):
        return subprocess.run([sys.executable, '-W', 'error', str(ROOT / 'scripts/layout_rooms.py'),
            '--layout-file' if replay else '--request-file', str(path), '--emit', kind],
            capture_output=True, timeout=15,
            env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def test_all_six_exports_from_request_and_saved_layout(self):
        value = request(fallback=True)
        expected = solve(value)
        with tempfile.TemporaryDirectory() as directory:
            source, saved = Path(directory) / 'request.json', Path(directory) / 'layout.json'
            source.write_bytes(canonical(value))
            saved.write_bytes(expected.encode())
            for path, replay in ((source, False), (saved, True)):
                for kind in cli.EXPORTS:
                    with self.subTest(replay=replay, kind=kind):
                        result = self.invoke(path, kind, replay=replay)
                        self.assertEqual(result.returncode, 0, result.stdout)
                        self.assertEqual(result.stderr, b'')
                        self.assertEqual(result.stdout, cli.export(expected, kind))

    def test_legacy_success_stays_at_original_selected_location_on_cli_replay(self):
        old = solve(request(alternatives=True))
        self.assertEqual(old.digest, 'a13e618c17721b8910aef1f75632bb5a3235ed9d332519800ac44c18ba079f4f')
        with tempfile.TemporaryDirectory() as directory:
            saved = Path(directory) / 'legacy.json'
            saved.write_bytes(old.encode())
            result = self.invoke(saved, 'room-plan', replay=True)
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(result.stdout, old.room_plan().encode())
            self.assertEqual(json.loads(result.stdout)['intent']['areas'][1]['origin'], [55, 16, 2])

    def test_original_no_layout_report_stays_replayable(self):
        old = solve(request())
        with tempfile.TemporaryDirectory() as directory:
            saved = Path(directory) / 'legacy.json'
            saved.write_bytes(old.encode())
            result = self.invoke(saved, replay=True)
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(result.stdout, old.encode())
            refused = self.invoke(saved, 'room-plan', replay=True)
            self.assertEqual(refused.returncode, 2)
            packet = json.loads(refused.stdout)
            self.assertEqual(packet['error']['code'], 'ROOM_LAYOUT_NOT_FOUND')
            self.assertIsNone(packet['result'])

    def test_node_and_work_limits_refuse_partial_exports(self):
        for search, code in (({'max_nodes': 3}, 'ROOM_LAYOUT_SEARCH_LIMIT'),
                             ({'max_work': 30000}, 'ROOM_LAYOUT_WORK_LIMIT')):
            value = request(fallback=True)
            value['search'] = search
            with tempfile.TemporaryDirectory() as directory:
                source = Path(directory) / 'request.json'
                source.write_bytes(canonical(value))
                result = self.invoke(source, 'room-plan')
                self.assertEqual(result.returncode, 2)
                packet = json.loads(result.stdout)
                self.assertEqual(packet['error']['code'], code)
                self.assertIsNone(packet['result'])
                self.assertFalse(packet['agent_turn']['briefing']['mutation_admissible'])

    def test_tampered_strategy_is_not_reinterpreted_on_cli_replay(self):
        value = solve(request(fallback=True)).json()
        value['request'].pop('connection_strategy')
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'tampered.json'
            source.write_bytes(canonical(value))
            result = self.invoke(source, 'room-plan', replay=True)
            self.assertEqual(result.returncode, 2)
            packet = json.loads(result.stdout)
            self.assertEqual(packet['error']['code'], 'ROOM_LAYOUT_REFUSED')
            self.assertIsNone(packet['result'])


if __name__ == '__main__':
    unittest.main()
