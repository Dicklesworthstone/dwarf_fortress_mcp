"""Compare current and exact pre-room batch code on the same real private stores.

Usage: PYTHONPATH=scripts:tests python3 tests/check_room_dig_legacy.py reference.py
Obtain reference.py with git show 42c49bdd9319753d37dd75030892c657a6249fdc:scripts/dig_blueprint_client.py.
Its Git blob is checked before loading it. Native I/O uses a synthetic TCP peer.
"""
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import types
from unittest.mock import patch

import dig_blueprint as b
import dig_blueprint_client as current
import dig_designation_client as d
from room_dig_peer import DigPeer
import room_terrain_fixtures as f
from test_room_dig_batch import environment
from test_room_excavation_handoff import fixture
import room_excavation_handoff as h

REFERENCE = '30d4d71759f237a6a1d75f223e56b58fa80a17f1'


def run(path):
    with path.open('rb') as source:
        raw = source.read(65537)
    assert hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest() == REFERENCE
    legacy = types.ModuleType('pre_room_dig_blueprint')
    legacy.__file__ = str(path)
    sys.modules[legacy.__name__] = legacy
    exec(compile(raw, str(path), 'exec'), legacy.__dict__)
    plan, selection, raw, dug = fixture()
    comparisons = 0
    with tempfile.TemporaryDirectory() as directory, DigPeer(raw, selection.region, already_dug=dug) as peer:
        root = Path(directory).resolve()
        root.chmod(0o700)
        handoff = h.RoomExcavationHandoff.create(plan, raw, f.MANIFEST, peer.address)
        layout = b.Layout.from_json(handoff.json()['remaining_blueprint'])
        with patch.dict(os.environ, environment(peer.address), clear=True):
            first = legacy.initialize(root, layout, 'region1', 2, False, legacy.POLICY, legacy.Budget(10000))
            for step in range(first['total_steps']):
                old = legacy.inspect(root, legacy.Budget(10000))
                new = current.inspect(root, current.Budget(10000))
                assert d.canonical(old) == d.canonical(new)
                old = legacy.observe(root, legacy.Budget(10000))
                new = current.observe(root, current.Budget(10000))
                assert d.canonical(old) == d.canonical(new)
                comparisons += 2
                current.advance(root, old['batch_id'], step, old['observation']['witness'],
                    old['plan_digest_for_confirmation'], old['review_seal'], current.Budget(10000))
        with patch.dict(os.environ, {}, clear=True):
            for step in range(first['total_steps']):
                old = legacy.recover(root, first['batch_id'], step, False, legacy.Budget(10000))
                new = current.recover(root, first['batch_id'], step, False, current.Budget(10000))
                assert d.canonical(old) == d.canonical(new)
                comparisons += 1
            assert d.canonical(legacy.inspect(root, legacy.Budget(10000))) == d.canonical(current.inspect(root, current.Budget(10000)))
            comparisons += 1
        assert len(peer.commits) == first['total_steps'] and peer.designated == selection.floors - dug
    print(f'PASS: {comparisons} byte-identical legacy/current results across {first["total_steps"]} native fixture steps; reference {REFERENCE}')


if __name__ == '__main__':
    run(Path(sys.argv[1]))
