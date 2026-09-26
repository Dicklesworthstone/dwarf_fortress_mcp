#!/usr/bin/env python3
"""Run four executable negative regressions in isolated source copies."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MUTANTS = (
    ('review_not_bound', 'build_placement_client.py',
     "guard(plan, known, 'before_intent')", 'pass  # omit batch confirmation',
     'test_copied_confirmation_does_not_authorize_another_batch'),
    ('final_guard_missing', 'build_placement_client.py',
     "guard(plan, known, 'before_commit')", 'pass  # omit final batch guard',
     'test_stop_appearing_after_prepare_prevents_dispatch'),
    ('index_not_synced', 'furniture_batch.py',
     'storage.write_all(file.fd, line)\n            os.fsync(file.fd)',
     'storage.write_all(file.fd, line)\n            # missing index fsync',
     'test_all_ten_advance_sync_failures_preserve_one_shot_semantics'),
    ('missing_intent_forgotten', 'furniture_batch.py',
     "require(journal is not None or step.name not in indexed, 'registered child journal missing; cannot forget attempted work')",
     "require(True, 'missing intent silently forgotten')",
     'test_deleted_registered_child_or_replaced_inode_never_becomes_new_work'),
)


def main() -> None:
    results = []
    for name, filename, old, new, test in MUTANTS:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'scripts').mkdir()
            for source in ('furniture_plan.py', 'furniture_batch.py', 'test_furniture_batch.py',
                           'build_placement_client.py', 'build_placement_rpc.py',
                           'build_placement_store.py', 'build_placement_wire.py'):
                shutil.copyfile(ROOT / 'scripts' / source, root / 'scripts' / source)
            fixture = Path('bridge/common/tests/fixtures/build_placement_v1_19.json')
            (root / fixture).parent.mkdir(parents=True)
            shutil.copyfile(ROOT / fixture, root / fixture)
            path = root / 'scripts' / filename
            source = path.read_text()
            if source.count(old) != 1:
                raise RuntimeError('mutation site is not unique: ' + name)
            path.write_text(source.replace(old, new))
            case = 'test_furniture_batch.BatchTests.' + test
            result = subprocess.run([sys.executable, '-m', 'unittest', case, '-v'], cwd=root,
                env=dict(os.environ, PYTHONPATH=str(root / 'scripts')),
                capture_output=True, timeout=30)
            if result.returncode == 0 or b'FAILED (failures=' not in result.stderr:
                raise RuntimeError('mutation did not fail a regression assertion: ' + name)
            results.append({'name': name, 'test': case, 'rejected': True, 'returncode': result.returncode})
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
