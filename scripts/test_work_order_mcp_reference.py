#!/usr/bin/env python3
"""Source-shape mutations for the standalone reference checker; no Rust execution."""
import json
from pathlib import Path
import subprocess
import sys
import unittest

from check_work_order_mcp_reference import check_offline_source_branch


SOURCE = '''fn open() {
    let source = if mode == JournalMode::Offline {
        None
    } else {
        let token = configured("DFMCP_WORK_ORDERS_TOKEN", 256)?;
        Some(token)
    };
}'''


class OfflineSourceShape(unittest.TestCase):
    def test_multiline_compact_and_comment_layouts_preserve_the_branch(self):
        for source in [SOURCE, SOURCE.replace('\n', ' '),
                       SOURCE.replace('None', '/* bounded offline branch */ None'),
                       SOURCE.replace('else {', 'else { // online credentials\n'),
                       SOURCE.replace('Some(token)',
                           'let note = r##"configured("DFMCP_WORK_ORDERS_TOKEN", 1)"##; Some(token)')]:
            with self.subTest(source=source):
                check_offline_source_branch(source)

    def test_credential_before_offline_is_rejected(self):
        source = SOURCE.replace('    let source',
            '    let early = configured("DFMCP_WORK_ORDERS_TOKEN", 256)?;\n    let source')
        with self.assertRaises(AssertionError):
            check_offline_source_branch(source)

    def test_credential_outside_online_branch_is_rejected(self):
        source = SOURCE.replace('let token = configured("DFMCP_WORK_ORDERS_TOKEN", 256)?;',
                                'let token = unrelated();')
        source = source.replace('    let source',
            '    let early = configured("DFMCP_WORK_ORDERS_TOKEN", 256)?;\n    let source')
        with self.assertRaises(AssertionError):
            check_offline_source_branch(source)

    def test_missing_or_nonempty_offline_branch_is_rejected(self):
        for source in [SOURCE.replace('JournalMode::Offline', 'JournalMode::Control'),
                       SOURCE.replace('None', 'Some(connect())')]:
            with self.subTest(source=source), self.assertRaises(AssertionError):
                check_offline_source_branch(source)

    def test_commented_branch_cannot_supply_wiring(self):
        with self.assertRaises(AssertionError):
            check_offline_source_branch('/* ' + SOURCE + ' */')

    def test_quoted_branch_cannot_supply_wiring(self):
        for source in [json.dumps(SOURCE), 'r#"prefix " ' + SOURCE + ' " suffix"#',
                       'br###"prefix " ' + SOURCE + ' " suffix"###',
                       'cr#"prefix " ' + SOURCE + ' " suffix"#', 'r#"unclosed']:
            with self.subTest(source=source), self.assertRaises(AssertionError):
                check_offline_source_branch(source)

    def test_duplicate_offline_branches_are_ambiguous(self):
        with self.assertRaises(AssertionError):
            check_offline_source_branch(SOURCE + SOURCE)

    def test_actual_standalone_checker_preserves_its_full_reference_scope(self):
        path = Path(__file__).with_name('check_work_order_mcp_reference.py')
        result = subprocess.run([sys.executable, str(path)], capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['status'], 'passed_reference_only')
        self.assertEqual(report['pagination_model_cases'], 297)
        self.assertEqual(report['mcp_tools_wired'], 11)
        self.assertEqual(report['rust_tests_registered'], 16)
        for key in ['rust_compiled', 'rust_tests_executed', 'mcp_executed',
                    'filesystem_or_native_game_executed']:
            self.assertIs(report[key], False)


if __name__ == '__main__':
    unittest.main()
