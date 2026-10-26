"""Direct-script identity regression using the explicit routing test ports."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
from test_progress_family_routing import environment

ROOT = Path(__file__).resolve().parent


class ScriptEntrypointTests(unittest.TestCase):
    def setUp(self):
        self.p, self.f, self.back = self.enterContext(environment())
        self.batch = self.back.batch

    def test_direct_script_final_review_reuses_its_exact_authority_type(self):
        import io
        import runpy
        output = io.BytesIO()
        stream = io.TextIOWrapper(output, encoding='utf-8')
        args = ['route_furniture_progress.py', 'review', '--directory', '/private/batch',
                '--batch-id', self.batch.id, '--ticks', '20', '--wall-ms', '5000']
        # Make the cross-owner import occur inside the real __main__ entrypoint.
        sys.modules.pop('route_furniture_final_progress', None)
        with patch.object(sys, 'argv', args), patch.object(sys, 'stdout', stream):
            with self.assertRaises(SystemExit) as result:
                runpy.run_path(str(ROOT/'route_furniture_progress.py'), run_name='__main__')
        stream.flush()
        self.assertEqual(result.exception.code, 0)
        value = json.loads(output.getvalue())
        self.assertIsNotNone(value['result']['progress_review']['confirm_progress'])
        self.assertEqual(self.back.reads, ['final', 'final'])
        self.assertTrue(self.batch.closed)


if __name__ == '__main__':
    unittest.main()
