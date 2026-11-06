#!/usr/bin/env python3
"""Actual combined MCP portfolio consumer against bounded native TCP doubles."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location('combined', Path(__file__).with_name('combined_production.py'))
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


class PortfolioPeer(b.Peer):
    def __init__(self, directory):
        self.plans = {}
        self.orders = {}
        self.created_keys = []
        self.drop_order_reply = False
        self.stocks = {'BED': 0, 'DOOR': 0, 'TABLE': 0, 'CHAIR': 0}
        super().__init__(directory)

    def decode_request(self, plugin, raw):
        if plugin != 'dfmcp_work_order_progress_v1_12':
            return super().decode_request(plugin, raw)
        at, result = 0, {}
        def integer():
            nonlocal at
            value = shift = 0
            while True:
                byte = raw[at]; at += 1
                value |= (byte & 127) << shift
                if byte < 128: return value
                shift += 7
                assert shift < 70
        while at < len(raw):
            field = integer(); tag, kind = field >> 3, field & 7
            if kind == 0: value = integer()
            else:
                assert kind == 2
                size = integer(); value = raw[at:at + size]; at += size
            if tag == 5: result.setdefault(tag, []).append(value)
            else:
                assert tag not in result
                result[tag] = value
        return result

    def queue(self):
        ids = sorted(i for i, order in self.orders.items() if not order['retired'])
        return (b'DFMWO010' + struct.pack('>QQQII', b.OGEN, self.order_sequence, self.order_tick,
                100 + len(self.orders), b.SITE) + b'\x01' + b.text(b'region1') + struct.pack('>I', len(ids))
                + b''.join(struct.pack('>I', i) for i in ids))

    def effect(self, plan, created):
        key, before, recipe, amount = plan['key'], plan['before'], plan['recipe'], plan['amount']
        spec = bytes([recipe]) + struct.pack('>I', amount)
        digest = hashlib.sha256(b'dfmcp-work-order-plan/1\0' + spec + hashlib.sha256(before).digest()).digest()
        token = hashlib.sha256(b'dfmcp-work-order-token/1\0' + struct.pack('>Q', b.OGEN) + b.text(key) + digest).digest()[:16]
        assert digest == plan['digest']
        raw = b'DFMWOE10' + struct.pack('>QQQI', b.OGEN, plan['sequence'], plan['tick'], plan['id']) + spec
        raw += hashlib.sha256(before).digest() + digest + token
        raw += bytes([2 if created else 0, int(created)]) + struct.pack('>Q', plan['tick'] if created else 0)
        raw += hashlib.sha256(plan['after']).digest() if created else bytes(32)
        raw += hashlib.sha256(b'DFMWOC10' + struct.pack('>I', plan['id']) + spec).digest() if created else bytes(32)
        proof = hashlib.sha256(b'dfmcp-work-order-receipt/1\0' + struct.pack('>Q', b.OGEN) + b.text(key) + raw[73:195]).digest() if created else bytes(32)
        return raw + proof + b.text(key)

    def progress(self, ids):
        self.progress_sequence += 1
        raw = (b'DFMWP012' + struct.pack('>QQQIIIB', b.PGEN, self.progress_sequence, self.order_tick, b.SITE,
                100 + len(self.orders), sum(not o['retired'] for o in self.orders.values()), 1)
                + b.text(b'region1') + struct.pack('>I', len(ids)))
        for ident in ids:
            order = self.orders[ident]
            raw += struct.pack('>IB', ident, int(not order['retired']))
            if not order['retired']:
                token = [b'', b'ConstructBed', b'ConstructDoor', b'ConstructTable', b'ConstructThrone'][order['recipe']]
                raw += (struct.pack('>iBiiIiiiiiII', 69, order['recipe'], order['remaining'], order['amount'],
                        order['status'], 0, -1, 1, -1, -1, 0, 0) + b.text(token) + b.text(b''))
        return raw

    def operations(self):
        year, tick = divmod(self.order_tick, 403200)
        jobs = b'DFMJ1200' + struct.pack('>IIBiI', year, tick, 1, b.SITE, 100) + b.text(b'region1') + struct.pack('>I', 0)
        raw = b'DFMO1300' + struct.pack('>I', len(jobs)) + jobs + struct.pack('>III', 100, 100, 0)
        raw += struct.pack('>I', sum(value > 0 for value in self.stocks.values()))
        for ident, (kind, stack) in enumerate(sorted(self.stocks.items()), start=2):
            if stack:
                raw += (struct.pack('>Ii', ident, 5 + ident) + b.text(kind.encode())
                    + struct.pack('>iiiIiiiI', -1, 420, 0, stack, 1, 1, 1, 64) + b'\x00\x00')
        return raw + struct.pack('>I', 0)

    def capture(self, ident):
        order = self.orders[ident]; present = not order['retired']
        return (b'DFMOR014' + struct.pack('>QQQI', b.CLOCK_GENERATION, self.clock_sequence, self.order_tick, b.SITE)
            + b.text(b'region1') + struct.pack('>BIIBBiiI', 1, ident, 100 + len(self.orders), int(present),
                order['recipe'] if present else 0, order['amount'] if present else 0,
                order['remaining'] if present else 0, order['status'] if present else 0))

    def reply(self, plugin, op, req):
        if plugin not in ('dfmcp_work_orders_v1_10', 'dfmcp_work_order_progress_v1_12', 'dfmcp_operations_v1_3', 'dfmcp_order_run_v1_14'):
            return super().reply(plugin, op, req)
        assert req[1] == b.TOKEN
        minor, generation = {'dfmcp_work_orders_v1_10': (10, b.OGEN), 'dfmcp_work_order_progress_v1_12': (12, b.PGEN),
            'dfmcp_operations_v1_3': (3, b.IGEN), 'dfmcp_order_run_v1_14': (14, b.CLOCK_GENERATION)}[plugin]
        response = (b.number(1, 1) + b.number(2, 0) + b.blob(3, req[2]) + b.number(4, 1) + b.number(5, minor)
            + b.number(6, generation) + b.blob(7, b.DF) + b.blob(8, b.DFH))
        if minor == 12:
            if op == 'ReadObservation': response += b.blob(9, self.progress(req[5]))
            return response
        if minor == 3:
            if op == 'ReadObservation':
                response += b.blob(9, self.operations())
                if self.after_operations_read is not None:
                    hook, self.after_operations_read = self.after_operations_read, None
                    hook()
            return response
        if minor == 10:
            if op == 'ReadOrders': response += b.blob(9, self.queue())
            elif op == 'PrepareOrder':
                assert req[5] not in self.plans
                plan = {'key': req[5], 'recipe': req[6], 'amount': req[7], 'digest': req[9],
                    'before': self.queue(), 'sequence': self.order_sequence, 'tick': self.order_tick, 'id': 100 + len(self.orders)}
                assert req[8] == hashlib.sha256(plan['before']).digest()
                self.plans[req[5]] = plan
                response += b.blob(10, self.effect(plan, False)) + b.number(11, 0)
            elif op == 'CommitOrder':
                plan = self.plans[req[5]]
                assert plan['key'] not in self.created_keys, 'original order key must execute once'
                assert req[9] == plan['digest']
                self.created_keys.append(plan['key'])
                self.orders[plan['id']] = {'recipe': plan['recipe'], 'amount': plan['amount'], 'remaining': plan['amount'], 'status': 3, 'retired': False}
                self.order_created = True; self.order_sequence += 1
                plan['after'] = self.queue()
                if self.drop_order_reply:
                    self.drop_order_reply = False
                    raise ConnectionResetError('lost original portfolio insertion reply')
                response += b.blob(10, self.effect(plan, True))
            elif op == 'QueryOrder' and req[5] in self.plans:
                response += b.blob(10, self.effect(self.plans[req[5]], req[5] in self.created_keys))
            return response
        if op == 'ObserveRun': response += b.blob(9, self.capture(req[11]))
        elif op == 'PrepareRun':
            assert req[8] == self.capture(req[11])
            plan = b'DFMOP014' + struct.pack('>IIBIII', req[6], req[7], req[12], req[13], req[14], req[15]) + b.text(req[8])
            assert req[9] == b.run_wire.plan_digest(plan) and req[5] not in self.clock_plans
            self.clock_plans[req[5]] = plan; self.clock_records[req[5]] = b.clock_record(plan, req[5])
            response += b.blob(10, self.clock_records[req[5]])
        elif op != 'Handshake' and req[5] in self.clock_plans:
            key = req[5]; plan = self.clock_plans[key]
            assert req[9] == b.run_wire.plan_digest(plan)
            if op == 'CommitRun':
                assert key not in self.clock_commits, 'original portfolio unpause must execute once'
                self.clock_commits.append(key)
                if self.hold_clock: self.clock_records[key] = b.clock_record(plan, key, phase=1, observed=self.order_tick)
                else: self.finish_clock(key)
                if self.drop_clock_reply:
                    self.drop_clock_reply = False
                    raise ConnectionResetError('lost original portfolio unpause reply')
            elif op == 'CancelRun':
                self.clock_records[key] = b.clock_record(plan, key, phase=3 if key in self.clock_commits else 4,
                    reason=3, observed=self.order_tick if key in self.clock_commits else None)
            response += b.blob(10, self.clock_records[key])
        active = any(b.run_wire.record(raw)['phase'] in ('running', 'stopping') for raw in self.clock_records.values())
        return response + b.number(11, int(active)) + b.number(12, len(self.clock_records))

    def finish_clock(self, key):
        plan = self.clock_plans[key]; parsed = b.run_wire.plan(plan)
        self.goal_tick(parsed['before']['tick'] + parsed['game_ticks']); self.clock_sequence += 1
        if self.complete_order_on_clock:
            order = self.orders[parsed['before']['order_id']]
            order['remaining'] = 0; order['status'] = 1
            self.stocks[{1: 'BED', 2: 'DOOR', 3: 'TABLE', 4: 'CHAIR'}[order['recipe']]] = order['amount']
        self.clock_records[key] = b.clock_record(plan, key, phase=3, reason=1, observed=self.order_tick)


class PortfolioProduction(unittest.TestCase):
    enable_clock = b.CombinedProduction.enable_clock
    enable_inventory_clock = b.CombinedProduction.enable_inventory_clock
    tearDown = b.CombinedProduction.tearDown
    def setUp(self):
        self.directory = LOGS / self._testMethodName; self.directory.mkdir(mode=0o700)
        self.peer = PortfolioPeer(self.directory)
        self.env = b.environment(self.directory, self.peer.endpoint)
        self.client = b.stdio.Client(self.directory, {}, env=self.env)

    def plan(self, count=3, inventory=False, stable=1, ticks=0, deadline=300, lose_order_step=None):
        opened = self.client.open(max_wall_millis=60000, max_game_ticks=ticks)
        self.assertTrue(opened['ok'], opened); sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid); self.assertTrue(observed['ok'], observed)
        types = [('wooden_bed','BED'),('wooden_chair','CHAIR'),('wooden_door','DOOR'),('wooden_table','TABLE')][:count]
        orders = [{'recipe': recipe, 'amount': 1, **({'objective': {'kind':'observed_inventory_minimum','item_type':kind,'minimum_stack_units':1}} if inventory else {})} for recipe, kind in types]
        request = {'kind':'portfolio','expected_observation':observed['observation']['seal'],'units':['43'],
            'labor':'MINE','enabled':True,'orders':orders,'deadline_tick':deadline,'poll_interval_ticks':5,'stable_samples':stable}
        current = self.client.tool('plan', session_id=sid, request=json.dumps(request)); self.assertTrue(current['ok'], current)
        original = current['original_plan']; self.assertEqual(len(original['steps']), count + 1)
        for step in original['steps'][1:]: self.assertEqual(step['depends_on'],[0])
        for index in range(count + 1):
            prepared = self.client.execute_next(current); self.assertTrue(prepared['ok'], prepared)
            self.assertEqual(prepared['review']['step_id'],index)
            if index == lose_order_step: self.peer.drop_order_reply = True
            current = self.client.execute_next(prepared)
            if index == lose_order_step:
                self.assertFalse(current['ok'], current)
                return sid, original, current
            self.assertTrue(current['ok'], current)
        self.assertFalse(current['original_goal_proven']); self.assertEqual(len(self.peer.created_keys),count)
        return sid, original, current

    def cohort(self, sid, attempts=12):
        for _ in range(attempts):
            current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
            self.assertTrue(current['ok'], current)
            if current['current_parent_goal'] is not None: return current
        self.fail('bounded original portfolio cohort did not finish')

    def prepare_run(self, sid, step, ticks=5):
        selected = self.client.tool('query', session_id=sid, order_step_id=step); self.assertTrue(selected['ok'], selected)
        request = {'kind':'run','order_step_id':step,'original_paired_seal':selected['clock']['original_paired_seal'],
            'game_ticks':ticks,'wall_millis':1000}
        prepared = self.client.tool('plan', session_id=sid, request=json.dumps(request)); self.assertTrue(prepared['ok'], prepared)
        return prepared

    def test_four_original_goals_require_the_complete_coherent_portfolio(self):
        sid, original, _ = self.plan(count=4, inventory=True)
        self.peer.stocks.update(BED=1, CHAIR=1, DOOR=1)
        self.peer.goal_tick(b.TICK + 5)
        partial = self.cohort(sid)
        self.assertFalse(partial['original_goal_proven']); self.assertEqual(partial['current_parent_goal']['current_truth'],'false')
        self.peer.stocks['TABLE'] = 1
        self.peer.goal_tick(b.TICK + 10)
        complete = self.cohort(sid)
        self.assertTrue(complete['original_goal_proven'], complete)
        self.assertTrue(complete['agent_turn']['briefing']['original_goal_proven'])
        self.assertEqual(complete['original_plan'], original)
        self.assertEqual(len(complete['current_parent_goal']['selected_anchors']),4)
        self.assertEqual(len(set(self.peer.created_keys)),4)
        self.assertEqual(sum(op == 'CommitAssignment' for _,op in self.peer.calls),1)

    def test_each_original_order_has_its_own_reviewed_clock(self):
        self.enable_clock(); self.peer.complete_order_on_clock = True
        sid, original, created = self.plan(count=3, ticks=5)
        journal_ids = [row['clock_journal_id'] for row in created['recovery']['portfolio']]
        self.assertEqual(len(set(journal_ids)),3)
        for step in (1,2,3):
            prepared = self.prepare_run(sid,step)
            committed = self.client.tool('commit', session_id=sid, scope='run', order_step_id=step,
                paired_seal=prepared['clock']['active_run']['review_seal'], confirmed=True)
            self.assertTrue(committed['ok'],committed); self.assertFalse(committed['original_goal_proven'])
        complete = self.cohort(sid)
        self.assertTrue(complete['original_goal_proven'],complete); self.assertEqual(complete['original_plan'],original)
        self.assertEqual(len(self.peer.clock_commits),3)
        self.assertEqual([b.run_wire.plan(self.peer.clock_plans[key])['before']['order_id'] for key in self.peer.clock_commits],[100,101,102])

    def test_retired_default_leaf_is_unknown_and_missing_quota_never_proves_parent(self):
        sid, _, _ = self.plan(count=2)
        for order in self.peer.orders.values(): order.update(remaining=0,status=1)
        self.peer.orders[100]['retired'] = True
        self.peer.stocks.update(BED=10,CHAIR=10)
        current = self.cohort(sid)
        self.assertFalse(current['original_goal_proven']); self.assertEqual(current['current_parent_goal']['current_truth'],'unknown')
        self.assertEqual(current['portfolio']['orders'][0]['goal']['current_truth'],'unknown')

    def test_current_labor_and_changed_inventory_break_the_portfolio_join(self):
        sid, _, _ = self.plan(count=2, inventory=True)
        self.peer.stocks.update(BED=1,CHAIR=1)
        self.peer.goal_tick(b.TICK + 5)
        complete = self.cohort(sid); self.assertTrue(complete['original_goal_proven'])
        self.peer.workforce_labor = False; self.peer.workforce_sequence = 5
        self.peer.goal_tick(b.TICK + 10)
        false = self.cohort(sid); self.assertFalse(false['original_goal_proven'])
        self.assertEqual(false['current_parent_goal']['current_truth'],'false')
        first = self.client.tool('wait', session_id=sid); self.assertTrue(first['ok'],first)
        self.peer.stocks['BED'] = 2
        refused = self.client.tool('wait', session_id=sid)
        self.assertFalse(refused['ok'],refused)
        self.assertFalse(refused['retained_reference']['original_goal_proven'])

    def test_all_three_original_streaks_finish_at_the_unchanged_tight_deadline(self):
        self.enable_clock()
        sid, original, _ = self.plan(count=3, inventory=True, stable=2, ticks=5, deadline=b.TICK + 10)
        self.peer.stocks.update(BED=1, CHAIR=1, DOOR=1)
        self.peer.goal_tick(b.TICK + 5)
        first = self.cohort(sid)
        self.assertFalse(first['original_goal_proven'], first)
        self.assertEqual([row['goal']['current_truth'] for row in first['portfolio']['orders']], ['true'] * 3)
        prepared = self.prepare_run(sid, 2)
        committed = self.client.tool('commit', session_id=sid, scope='run', order_step_id=2,
            paired_seal=prepared['clock']['active_run']['review_seal'], confirmed=True)
        self.assertTrue(committed['ok'], committed)
        self.assertEqual(self.peer.order_tick, b.TICK + 10)
        self.assertFalse(committed['original_goal_proven'])
        complete = self.cohort(sid)
        self.assertTrue(complete['original_goal_proven'], complete)
        self.assertTrue(complete['agent_turn']['briefing']['original_goal_proven'])
        self.assertEqual(complete['original_plan'], original)
        self.assertEqual([row['goal']['deadline_tick'] for row in complete['portfolio']['orders']], [b.TICK + 10] * 3)
        self.assertEqual(len(self.peer.clock_commits), 1)
        selected = self.client.tool('query', session_id=sid, order_step_id=1)
        self.assertTrue(selected['clock_advanced'], selected)

    def test_four_leaf_dual_clock_offline_recovery_fits_the_public_default_budget(self):
        self.enable_clock(); self.enable_inventory_clock()
        sid, original, created = self.plan(count=4, inventory=True, ticks=5)
        rows = created['recovery']['portfolio']
        self.assertEqual(len({row['clock_journal_id'] for row in rows}), 4)
        self.assertEqual(len({row['inventory_clock_journal_id'] for row in rows}), 4)
        self.client.close()
        env = b.reopen_environment(self.env, created['recovery'], created['native_custody']['workforce_journal_id'])
        files, calls = b.custody_files(self.directory), list(self.peer.calls)
        self.client = b.stdio.Client(self.directory, {}, suffix='all-offline', env=env)
        recovered = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(recovered['ok'], recovered)
        self.assertEqual(recovered['original_plan'], original)
        self.assertEqual(recovered['recovery']['portfolio'], rows)
        self.assertFalse(recovered['original_goal_proven'])
        self.assertEqual(files, b.custody_files(self.directory)); self.assertEqual(calls, self.peer.calls)

    def test_pending_leaf_and_wrong_selectors_never_choose_another_clock_target(self):
        self.enable_clock()
        sid, _, _ = self.plan(count=2, ticks=5)
        prepared = self.prepare_run(sid, 1)
        seal = prepared['clock']['active_run']['review_seal']
        current = self.cohort(sid)
        self.assertEqual(current['portfolio']['selected_order_step_id'], 2)
        advice = current['agent_turn']['recommendations'][0]
        self.assertEqual(advice['tool'], 'fortress_query', current)
        self.assertEqual(advice['arguments']['order_step_id'], 1)
        selected = self.client.execute_next(current); self.assertTrue(selected['ok'], selected)
        native = list(self.peer.calls)
        wrong = self.client.tool('commit', session_id=sid, scope='run', order_step_id=2, paired_seal=seal, confirmed=True)
        self.assertFalse(wrong['ok'], wrong)
        self.assertEqual(native, self.peer.calls)
        self.assertEqual(len(self.peer.clock_commits), 0)
        absent = self.client.tool('commit', session_id=sid, scope='run', paired_seal=seal, confirmed=True)
        self.assertFalse(absent['ok'], absent)
        cancelled = self.client.tool('cancel', session_id=sid, scope='run', order_step_id=1)
        self.assertTrue(cancelled['ok'], cancelled)
        self.assertEqual(len(self.peer.clock_commits), 0)
        self.assertEqual(len(self.peer.created_keys), 2)

    def test_lost_middle_insertion_reconciles_original_key_and_finishes_remaining_leaves(self):
        sid, original, lost = self.plan(count=4, lose_order_step=2)
        self.assertEqual(len(self.peer.created_keys), 2)
        recovery = lost['retained_reference']['recovery']
        # The complete native journal identities are retained by the original
        # configured stores; read their last successful response from Query.
        current = self.client.tool('query', session_id=sid)
        self.assertTrue(current['ok'], current)
        workforce = current['native_custody']['workforce_journal_id']
        self.client.close(crash=True)
        env = b.reopen_environment(self.env, recovery, workforce)
        self.client = b.stdio.Client(self.directory, {}, suffix='middle-recovered', env=env)
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened); sid = opened['agent_turn']['session_id']
        settled = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(settled['ok'], settled)
        self.assertEqual(len(self.peer.created_keys), 2)
        for _ in range(2):
            reviewed = self.client.execute_next(settled); self.assertTrue(reviewed['ok'], reviewed)
            settled = self.client.execute_next(reviewed); self.assertTrue(settled['ok'], settled)
        self.assertEqual(settled['original_plan'], original)
        self.assertEqual(len(self.peer.created_keys), 4)
        self.assertEqual(len(set(self.peer.created_keys)), 4)
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)

    def test_lost_second_clock_reply_reopens_all_custody_without_duplicate_unpause(self):
        self.enable_clock(); self.peer.complete_order_on_clock = True
        sid, _, created = self.plan(count=2, ticks=5)
        prepared = self.prepare_run(sid,2); seal = prepared['clock']['active_run']['review_seal']
        self.peer.hold_clock = True; self.peer.drop_clock_reply = True
        lost = self.client.tool('commit', session_id=sid, scope='run', order_step_id=2, paired_seal=seal, confirmed=True)
        self.assertFalse(lost['ok'],lost); self.assertEqual(len(self.peer.clock_commits),1)
        self.client.close(crash=True)
        env = b.reopen_environment(self.env,lost['retained_reference']['recovery'],created['native_custody']['workforce_journal_id'])
        before,calls = b.custody_files(self.directory),list(self.peer.calls)
        self.client = b.stdio.Client(self.directory,{},suffix='offline',env=env)
        offline = self.client.open(mode='offline',max_wall_millis=60000)
        self.assertTrue(offline['ok'],offline); self.assertFalse(offline['original_goal_proven'])
        self.assertEqual(before,b.custody_files(self.directory)); self.assertEqual(calls,self.peer.calls)
        self.client.close()
        self.client = b.stdio.Client(self.directory,{},suffix='reconcile',env=env)
        recovered = self.client.open(mode='reconcile',max_wall_millis=60000,max_game_ticks=5)
        self.assertTrue(recovered['ok'],recovered); sid = recovered['agent_turn']['session_id']
        count = sum(plugin == 'dfmcp_work_order_progress_v1_12' and op == 'ReadObservation' for plugin,op in self.peer.calls)
        waiting = self.client.tool('wait',session_id=sid,max_wall_millis=60000); self.assertTrue(waiting['ok'],waiting)
        self.assertEqual(count,sum(plugin == 'dfmcp_work_order_progress_v1_12' and op == 'ReadObservation' for plugin,op in self.peer.calls))
        self.peer.finish_clock(self.peer.clock_commits[0])
        current = self.cohort(sid); self.assertFalse(current['original_goal_proven'])
        self.assertEqual(len(self.peer.clock_commits),1); self.assertEqual(len(self.peer.created_keys),2)
        denied = self.client.tool('commit',session_id=sid,scope='run',order_step_id=2,paired_seal=seal,confirmed=True)
        self.assertFalse(denied['ok']); self.assertEqual(len(self.peer.clock_commits),1)


def main():
    global LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True); parser.add_argument('--logs',type=Path,required=True)
    args = parser.parse_args(); LOGS = args.logs; LOGS.mkdir(mode=0o700)
    b.stdio.BINARY = args.binary.resolve(strict=True); b.stdio.SERVER_ARGS = ['serve-gameplay-dev','labor-order']
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(PortfolioProduction))
    print(json.dumps({'schema':'dfmcp.combined-portfolio-stdio/1','tests':result.testsRun,'failed':len(result.failures),
        'errors':len(result.errors),'skipped':len(result.skipped),'actual_modern_stdio':True,'loopback_peers_are_doubles':True,'live_game':False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 9 and not result.skipped else 1)

if __name__ == '__main__': main()
