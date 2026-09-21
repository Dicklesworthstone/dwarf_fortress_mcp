#!/usr/bin/env python3
"""Independent contract/output models and source wiring checks, NOT Rust execution.

Runs without DFHack, credentials, network or writable journals. Source checks do
not expand macros, type-check Rust, invoke the MCP process or qualify durability.
"""
from __future__ import annotations
import hashlib
import itertools
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / 'crates/dfmcp-mcp/src/live_workforce_server.rs'
PRESENTATION = SERVER.with_suffix('') / 'presentation.rs'
NAMES = ['open_session', 'observe', 'query', 'plan', 'commit', 'wait', 'cancel', 'checkpoint', 'restore', 'explain', 'doctor']
MAX_ID = 2**31-1
COMPACT, DETAIL = 32768, 196608
JOURNAL, PLAN, EFFECT, KEYS = 64*1024*1024, 65675, 8192, 64
VIEW = JOURNAL + KEYS*(PLAN+EFFECT) + 4096
STATISTICS: dict[str, object] = {}


def strict_json(raw: str):
    if len(raw.encode()) > 2048:
        raise ValueError('request bound')
    def pairs(values):
        out = {}
        for key, value in values:
            if key in out:
                raise ValueError('duplicate field')
            out[key] = value
        return out
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite number')))


def uint(value, maximum=MAX_ID):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError('not a bounded unsigned integer')
    return value


def selection(raw):
    value = strict_json(raw)
    if type(value) is not dict or set(value) != {'unit_ids'}:
        raise ValueError('selection shape')
    ids = value['unit_ids']
    if type(ids) is not list or not 1 <= len(ids) <= 32:
        raise ValueError('selection count')
    for n in ids:
        uint(n)
    if any(a >= b for a, b in zip(ids, ids[1:])):
        raise ValueError('ordering')
    return ids


def digest(value):
    if type(value) is not str or re.fullmatch('[0-9a-f]{64}', value) is None:
        raise ValueError('digest')


def query(raw):
    value = strict_json(raw)
    if type(value) is not dict:
        raise ValueError('object required')
    mode = value.get('mode')
    limit = value.get('limit')
    limit = 4 if limit is None else uint(limit, 8)
    if not limit:
        raise ValueError('empty page')
    if mode == 'records':
        if set(value) - {'mode', 'state', 'limit', 'continuation'}:
            raise ValueError('unknown field')
        if value.get('state') not in (None, 'all', 'pending', 'unresolved', 'terminal'):
            raise ValueError('state')
        token = value.get('continuation')
        if token is not None:
            digest(token)
    elif mode == 'details':
        if set(value) - {'mode', 'witness', 'offset', 'limit'} or not {'mode', 'witness', 'offset'} <= set(value):
            raise ValueError('detail shape')
        digest(value['witness']); uint(value['offset'], 63)
    else:
        raise ValueError('mode')
    return value


def encoded(value):
    # ASCII escaping is conservative relative to serde_json's UTF-8 output.
    return json.dumps(value, ensure_ascii=True, separators=(',', ':')).encode()


def source():
    return {'fortress_id': 'f'*32, 'world_folder': '\x01'*512, 'site_id': MAX_ID,
            'generation': 2**64-2, 'sequence': 2**64-1, 'tick': (2**32-1)*403200+403199,
            'paused': True, 'automatic_professions': True, 'witness': 'f'*64,
            'current_freshness_proven': False}


def citizen(n, masks=True):
    value = {'native_id': MAX_ID-n, 'historical_figure_id': MAX_ID-n, 'eligible': True}
    if masks:
        value['labor_mask'] = [1]*128
    return value


def detail(n, memberships):
    return {'detail_index': n, 'name': '\x01'*256, 'selected_only': True,
            'allowed_labor_mask': [1]*128, 'member_count': memberships,
            'native_member_ids': list(range(MAX_ID-memberships+1, MAX_ID+1))}


def record(n=0):
    return {'idempotency_key': 'x'*125+f'{n:03}', 'plan_digest': 'f'*64, 'state': 'cancel_requested',
            'settled_in_this_coordinator': False, 'reconciliation_required': True,
            'native_query_can_help': False, 'native_phase': 'unknown', 'receipt_digest': 'f'*64,
            'detail_index': 63, 'assigned': True, 'selected_citizens': 32,
            'evidence_scope': 'historical_coordination_not_current_workforce'}


def packet(result):
    # Deliberately reserve more metadata than the emitted builder fields. Each
    # fixed section gets an additional 512-byte payload; four refs are repeated
    # across every active-work list even though one unsettled record is enforced.
    turn = {key: 'x'*512 for key in ('schema','operation','phase','session_id','request_id','anchor','continuity','profile',
                                   'briefing','changes','attention','affordances','recommendations','uncertainty','coverage','budget','references')}
    turn['active_work'] = {key: [record(i) for i in range(4)] for key in ('pending_plans','actions','indeterminate_effects','cancellation_drains')}
    return {'result': result, 'agent_turn': turn}


class ContractTests(unittest.TestCase):
    def test_explicit_eleven_tool_registration_and_module_entry(self):
        text = SERVER.read_text()
        names = re.findall(r'#\[tool\(name="(fortress\.[^"]+)"', text)
        self.assertEqual(sorted(names), sorted('fortress.'+name for name in NAMES))
        structs = re.findall(r'\.tool\((Fortress\w+)\)', text)
        expected = ['Fortress'+''.join(part.title() for part in name.split('_')) for name in NAMES]
        self.assertEqual(structs, expected)
        self.assertIn('pub mod live_workforce_server;', (SERVER.parent/'lib.rs').read_text())
        self.assertIn('live_workforce_server::run_stdio()', (SERVER.parent/'bin/dfmcp-live-workforce-dev-server.rs').read_text())
        self.assertIn('crate::run_modern_stdio(server)', text)
        for operation in ('observe', 'prepare', 'commit', 'reconcile', 'cancel'):
            self.assertIn('state.control.'+operation+'(', text)
        self.assertNotIn('state.journal.', text)
        self.assertNotIn('std::process::Command', text)
        self.assertNotIn('std::thread::spawn', text)
        for forbidden in ('tokio', 'reqwest', 'unsafe {', 'unimplemented!', 'todo!'):
            self.assertNotIn(forbidden, text)

    def test_all_small_sorted_selections_and_boundaries(self):
        count = 0
        for size in range(1, 9):
            for ids in itertools.combinations(range(8), size):
                self.assertEqual(selection(json.dumps({'unit_ids': ids})), list(ids)); count += 1
        for ids in ([0], [MAX_ID], list(range(32)), list(range(MAX_ID-31, MAX_ID+1))):
            self.assertEqual(selection(json.dumps({'unit_ids': ids})), ids); count += 1
        STATISTICS['accepted_selection_cases'] = count

    def test_selection_rejects_types_duplicates_and_unbounded_work(self):
        invalid = [None, [], {}, {'unit_ids': []}, {'unit_ids': [True]}, {'unit_ids': [1.0]}, {'unit_ids': [-1]},
                   {'unit_ids': [MAX_ID+1]}, {'unit_ids': [2, 2]}, {'unit_ids': [2, 1]}, {'unit_ids': list(range(33))},
                   {'unit_ids': [1], 'path': '/tmp/other'}, {'unit_ids': '1,2'}]
        for value in invalid:
            with self.assertRaises(ValueError): selection(json.dumps(value))
        for raw in ('{"unit_ids":[1],"unit_ids":[2]}', ' '*2049, '{"unit_ids":[NaN]}'):
            with self.assertRaises(ValueError): selection(raw)
        STATISTICS['rejected_selection_cases'] = len(invalid)+3

    def test_query_shape_and_pagination_parameter_matrix(self):
        accepted = rejected = 0
        for state, limit in itertools.product((None,'all','pending','unresolved','terminal'), (None,1,2,4,8)):
            query(json.dumps({'mode':'records','state':state,'limit':limit})); accepted += 1
        for offset, limit in itertools.product((0,1,31,63), (None,1,2,4,8)):
            query(json.dumps({'mode':'details','witness':'a'*64,'offset':offset,'limit':limit})); accepted += 1
        for limit in (-1,0,9,100,True,1.0,'4'):
            for mode in ('records','details'):
                value = {'mode':mode,'limit':limit}
                if mode == 'details': value.update(witness='a'*64, offset=0)
                with self.assertRaises(ValueError): query(json.dumps(value))
                rejected += 1
        for raw in ('{"mode":"records","state":"all","state":"pending"}', '{"mode":"records","command":"x"}',
                    '{"mode":"details","witness":"0","offset":0}', '{"mode":"records","continuation":"bad"}',
                    '{"mode":"details","witness":"'+'a'*64+'","offset":64}', '{"mode":"records","state":"absent"}'):
            with self.assertRaises(ValueError): query(raw)
            rejected += 1
        STATISTICS.update(accepted_query_cases=accepted, rejected_query_cases=rejected)

    def test_whole_record_pagination_exhaustive_small_rosters(self):
        cases = 0
        for size in range(65):
            entries = [{'key':f'{i:03}', 'pending':i%2==0, 'unresolved':i%3==0} for i in range(size)]
            for filter_name in ('all','pending','unresolved','terminal'):
                rows = [r for r in entries if filter_name=='all' or (not r['pending'] if filter_name=='terminal' else r[filter_name])]
                for limit in range(1,9):
                    pages = [rows[i:i+limit] for i in range(0,len(rows),limit)]
                    self.assertEqual([r for page in pages for r in page], rows)
                    self.assertTrue(all(1 <= len(page) <= limit for page in pages)); cases += 1
        STATISTICS['pagination_models'] = cases

    def test_cursor_identity_and_eviction_model(self):
        # This checks the declared binding, not Rust's cache implementation.
        fields = ('session', 'journal', 'head', 'filter', 'limit')
        original = ('a', 'b', 'c', 'all', 4)
        for i in range(5):
            changed = list(original); changed[i] = 'other'
            self.assertNotEqual(tuple(changed), original)
        text = PRESENTATION.read_text()
        for field in fields:
            self.assertIn('c.'+field+' != ', text)
        self.assertIn('if self.values.len() == 64', text)
        self.assertIn('self.values.pop_front()', text)
        self.assertIn('session.get().to_be_bytes()', text)
        self.assertIn('view.head.as_bytes()', text)

    def test_worst_case_escaped_output_models(self):
        keys = ['\x01'*61+f'{i:03}' for i in range(128)]
        lengths = []
        for count in (1,16,32):
            for page_size in (1,4,8):
                # Total membership cap is global, so distribute rather than
                # assuming 4096 members in each of eight different details.
                memberships = [4096//page_size + (i < 4096%page_size) for i in range(page_size)]
                result = {'ok':True, 'selection':{'source':source(),'labor_keys':keys,
                    'citizens':[citizen(i) for i in range(count)],
                    'details':[detail(i,m) for i,m in enumerate(memberships)],'offset':0,'total_details':64,
                    'next_query':{'mode':'details','witness':'f'*64,'offset':page_size,'limit':page_size}}}
                size = len(encoded(packet(result))); self.assertLess(size, DETAIL); lengths.append(size)
                full = {'ok':True,'effect':{'record':record(),'review':{'source':source(),'labor_keys':keys,
                    'detail':detail(63,4096),'assigned':True,'citizens':[citizen(i) for i in range(count)],
                    'changed_citizen_ids':list(range(count))},'native':{'phase':'applied','receipt_digest':'f'*64,
                    'after_witness':'f'*64,'post_citizens':[citizen(i) for i in range(count)]}}}
                size = len(encoded(packet(full))); self.assertLess(size, DETAIL); lengths.append(size)
        for size in range(1,9):
            result = {'ok':True,'records':[record(i) for i in range(size)],'matching_records':64,'continuation':'f'*64}
            width = len(encoded(packet(result))); self.assertLess(width, COMPACT); lengths.append(width)
        STATISTICS.update(output_models=len(lengths), min_model_bytes=min(lengths), max_model_bytes=max(lengths), detailed_reservation_bytes=DETAIL)

    def test_journal_verification_and_connection_reservations(self):
        # Largest view clones <=64 maximum plans/effects plus reads the complete
        # 64MiB journal. Open replays, verifies, and (when requested) obtains a view.
        self.assertGreater(VIEW, JOURNAL+KEYS*(PLAN+EFFECT))
        self.assertGreater(3*VIEW, 3*JOURNAL+KEYS*(PLAN+EFFECT))
        handshake = 7*400*1024+24
        budget = 1024*1024*1024
        remaining = budget-DETAIL-2*VIEW-handshake
        self.assertGreater(remaining, 8*JOURNAL+4*KEYS*(PLAN+EFFECT))
        self.assertGreater(budget-COMPACT-2*VIEW-handshake, 3*VIEW)
        text = SERVER.read_text()
        for fragment in ('2 * VIEW_BYTES','self.consume(c, CONNECT_BYTES)','work.consume(&display, OPEN_BYTES)',
                         'state.selected = None','state.control.reconcile', 'close_allowed', 'release_for_recovery'):
            # The adapter session owns routing and replay eligibility.
            self.assertIn(fragment, text)
        STATISTICS['view_reservation_bytes'] = VIEW

    def test_each_native_edge_has_a_runtime_guard(self):
        text = (SERVER.with_suffix('')/'native.rs').read_text()
        for method, write in (('observe','false'),('prepare','true'),('commit','true'),('query','false'),('cancel','true')):
            body = re.search(r'fn '+method+r'\b.*?\{(.*?)\n    \}', text, re.S).group(1)
            self.assertLess(body.index('(self.check)('+write+')?;'), body.index('self.source.'+method+'('))
        self.assertIn('briefing.remove("admission")', PRESENTATION.read_text())
        self.assertIn('current_workforce_proven', PRESENTATION.read_text())

    def test_machine_contract_and_registered_rust_regressions(self):
        contract = json.loads((ROOT/'architecture/workforce_mcp_v1_17.json').read_text())
        self.assertEqual(contract['tools'], ['fortress.'+name for name in NAMES])
        self.assertFalse(contract['production_admitted'])
        self.assertFalse(contract['rust_tests_executed'])
        self.assertEqual(contract['native_protocol'], '1.17')
        tests = (SERVER.with_suffix('')/'tests.rs').read_text()
        count = tests.count('#[test]')
        self.assertEqual(count, contract['registered_rust_test_groups'])
        STATISTICS['registered_unexecuted_rust_test_groups'] = count


if __name__ == '__main__':
    result = unittest.main(verbosity=2, exit=False)
    STATISTICS['scope'] = 'independent models and lexical wiring only; no Rust/MCP/native/durability execution'
    STATISTICS['source_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in (SERVER, PRESENTATION, SERVER.with_suffix('')/'native.rs', SERVER.with_suffix('')/'tests.rs')}
    print(json.dumps(STATISTICS, sort_keys=True, indent=2))
    raise SystemExit(0 if result.result.wasSuccessful() else 1)
