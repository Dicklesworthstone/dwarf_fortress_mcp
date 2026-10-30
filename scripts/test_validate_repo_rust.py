#!/usr/bin/env python3
"""Regression tests for production-only Rust validation."""
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import validate_repo as validator


class RustValidationTests(unittest.TestCase):
    def test_inline_module_only_and_literals_are_masked(self):
        source = r'''
// panic!() and #[cfg(test)] mod fake { }
/* outer /* .unwrap() */ todo!() */
const A: &str = "panic!() \" .unwrap()";
const B: &str = r##"#[cfg(test)] mod fake { todo!() }"##;
const C: &[u8] = br#"unreachable!()"#;
#[cfg(test)] mod tests { fn bad() { panic!("} { "); } }
fn live<'a>(x: &'a str) { panic! { "after tests" } }
'''
        mask, _ = validator.rust_module_scan(Path("sample.rs"), source)
        pattern = next(k for k, v in validator.RUST_FORBIDDEN.items() if v == "panic macro")
        self.assertEqual(len(re.findall(pattern, mask)), 1)
        self.assertIn("after tests", source)
        self.assertIn("fn live<'a>", mask)
        self.assertNotIn(".unwrap()", mask)
        self.assertNotIn("todo!", mask)

    def test_external_test_module_and_shared_production_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "crates" / "example" / "src"
            src.mkdir(parents=True)
            main = src / "lib.rs"
            test = src / "private_test.rs"
            shared = src / "shared.rs"
            included = src / "included.rs"
            main.write_text('''#[cfg(test)] #[path = "private_test.rs"] mod tests;
#[cfg(all(test, unix))] #[path = "shared.rs"] mod shared_test;
#[path = "shared.rs"] mod shared_prod;
#[cfg(test)] mod inline { include!("included.rs"); }
fn production() { todo!(); }
''')
            test.write_text('fn test() { panic!(); }')
            shared.write_text('fn shared() { unreachable![]; }')
            included.write_text('fn included() { panic!(); }')
            with patch.object(validator, "ROOT", root):
                masks = validator.rust_production_masks([main, test, shared, included])
            self.assertIn(main, masks)
            self.assertNotIn(test, masks)
            self.assertNotIn(included, masks)
            self.assertIn(shared, masks)
            self.assertIn("todo!()", masks[main])
            self.assertIn("unreachable![]", masks[shared])
            for description, path in (("todo macro", main), ("unreachable macro", shared)):
                pattern = next(k for k, v in validator.RUST_FORBIDDEN.items() if v == description)
                self.assertRegex(masks[path], pattern)

    def test_unclosed_test_module_does_not_hide_production(self):
        mask, _ = validator.rust_module_scan(Path("sample.rs"),
            '#[cfg(test)] mod tests { panic!(); fn live() { todo!(); }')
        self.assertIn("todo!()", mask)

    def test_each_forbidden_form_after_test_module_is_still_detected(self):
        mutations = {
            "unsafe block or declaration": "unsafe { f() }",
            "unwrap call": "value.unwrap()",
            "expect call": "value.expect(\"error\")",
            "panic macro": "panic! { \"error\" }",
            "unreachable macro": "unreachable![]",
            "todo macro": "todo!()",
            "unimplemented macro": "unimplemented!()",
        }
        for pattern, description in validator.RUST_FORBIDDEN.items():
            with self.subTest(description=description):
                source = '#[cfg(test)] mod tests { fn f() { panic!(); } }\n'
                source += 'fn production() { ' + mutations[description] + '; }'
                mask, _ = validator.rust_module_scan(Path("sample.rs"), source)
                self.assertRegex(mask, pattern)


class ExampleValidationTests(unittest.TestCase):
    def test_all_room_examples_use_their_real_closed_request_decoder(self):
        import json
        schema = json.loads((validator.ROOT / "schemas/dfmcp.schema.json").read_text())
        examples = list((validator.ROOT / "examples").glob("room_*.json"))
        self.assertGreaterEqual(len(examples), 9)
        for path in examples:
            with self.subTest(path=path.name):
                validator.validate_example(json.loads(path.read_text()), schema)

    def test_room_discriminator_cannot_bypass_input_or_policy_validation(self):
        import json
        schema = json.loads((validator.ROOT / "schemas/dfmcp.schema.json").read_text())
        request = json.loads((validator.ROOT / "examples/room_layout_minimum_targets.json").read_text())
        mutations = [{**request, "unexpected": True}, {**request, "objective": "invented"},
                     {**request, "site": True}, {**request, "region": {}},
                     {**request, "schema": "invented"},
                     {**request, "search": {"max_nodes": 0}},
                     {**request, "world_folder": "x" * 20000}]
        for value in mutations:
            with self.subTest(value=str(value)[:100]):
                with self.assertRaises(validator.ValidationError):
                    validator.validate_example(value, schema)


if __name__ == "__main__":
    unittest.main()
