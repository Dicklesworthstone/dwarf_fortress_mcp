"""Publication authorization after real room evidence and final custody checks."""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import room_readiness_peer as native
import room_readiness_rpc as rpc
import track_room_readiness as cli
from test_room_readiness_store import create_room_batch, goal_from


class RoomReadinessPublicationAuthorityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-room-publication-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.peer = self.enterContext(native.Peer())
        self.origin = create_room_batch(self.root / 'batch', address=self.peer.address)
        self.peer.goal = goal_from(self.origin).readiness_goal
        self.journal = self.root / 'monitor'

    def call(self, operation, *extra):
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            status = cli.main([operation, '--journal', str(self.journal),
                               '--timeout-ms', '60000', *extra])
        raw = stdout.getvalue()
        return status, json.loads(raw), raw

    def check_late_revocation(self, boundary):
        with native.environment(self.peer):
            status, started, raw = self.call('start', '--batch', str(self.root / 'batch'),
                '--batch-id', self.origin.batch_id, '--deadline-tick', '10000',
                '--stable-span-ticks', '10')
            self.assertEqual(status, 0, raw)
            self.assertEqual(started['result']['progress']['phase'], 'candidate')
            self.peer.tick += 10
            stage = {'rendered': False, 'revoked': False}
            original_output = cli.output

            def output(value):
                rendered = original_output(value)
                frames = value['result'].get('journal', {}).get('frames')
                if value['ok'] and frames not in (None, cli.MAX_FRAMES):
                    stage['rendered'] = True
                return rendered

            def revoke():
                if stage['rendered']:
                    os.environ.pop(rpc.OPT_IN, None)
                    stage['revoked'] = True

            if boundary == 'source_custody':
                original_verify = cli.Origin.verify_batch

                def verify(origin, batch):
                    original_verify(origin, batch)
                    revoke()

                injected = patch.object(cli.Origin, 'verify_batch', verify)
            else:
                original_close = cli.Batch.close

                def close(batch):
                    original_close(batch)
                    revoke()

                injected = patch.object(cli.Batch, 'close', close)

            with patch.object(cli, 'output', output), injected:
                status, refused, raw = self.call('sample')
            self.assertTrue(stage['rendered'])
            self.assertTrue(stage['revoked'])
            self.assertNotIn(rpc.OPT_IN, os.environ)
            self.assertEqual(status, 2, raw)
            self.assertFalse(refused['ok'])
            self.assertEqual(refused['error']['code'], 'ROOM_READINESS_MONITOR_REFUSED')
            self.assertIsNone(refused['result']['progress'])
            self.assertFalse(refused['result']['room_readiness_sampled_condition'])
            self.assertFalse(refused['result']['storage_acknowledged_this_call'])
            self.assertEqual(self.peer.connections, 2)
            for token in (native.BUILD_TOKEN, native.OPS_TOKEN, native.MAP_TOKEN):
                self.assertNotIn(token.decode(), raw)

        # Revocation withholds publication; it does not erase a valid historical
        # sample already synchronized before the final output checks.
        offline = {key: value for key, value in os.environ.items()
                   if not key.startswith('DFMCP_')}
        with patch.dict(os.environ, offline, clear=True):
            status, retained, raw = self.call('inspect')
        self.assertEqual(status, 0, raw)
        self.assertEqual(retained['result']['progress']['phase'], 'satisfied')
        self.assertEqual(retained['result']['progress']['observations'], 2)
        self.assertTrue(retained['result']['room_readiness_sampled_condition'])
        self.assertFalse(retained['result']['native_connection_attempted'])
        self.assertEqual(self.peer.connections, 2)

    def test_revocation_during_postserialization_source_check_withholds_terminal_result(self):
        self.check_late_revocation('source_custody')

    def test_revocation_during_final_owner_close_withholds_terminal_result(self):
        self.check_late_revocation('owner_close')


if __name__ == '__main__':
    unittest.main()
