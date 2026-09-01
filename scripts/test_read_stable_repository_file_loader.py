#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


class StableReaderModuleLoaderTests(unittest.TestCase):
    def test_module_registers_before_dataclass_execution(self) -> None:
        module_path = Path(__file__).with_name("read_stable_repository_file.py")
        specification = importlib.util.spec_from_file_location(
            "read_stable_repository_file_loader_test", module_path
        )
        self.assertIsNotNone(specification)
        self.assertIsNotNone(specification.loader if specification is not None else None)
        if specification is None or specification.loader is None:
            self.fail("cannot load stable repository file reader")
        module = importlib.util.module_from_spec(specification)
        sys.modules[specification.name] = module
        try:
            specification.loader.exec_module(module)
            self.assertTrue(hasattr(module, "StableFile"))
            self.assertTrue(hasattr(module, "read_stable_regular_file"))
        finally:
            sys.modules.pop(specification.name, None)


if __name__ == "__main__":
    unittest.main()
