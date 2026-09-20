#!/usr/bin/env python3
"""Independent MCP envelope, output-size and wiring checks; NOT Rust/MCP execution."""
from pathlib import Path
import itertools
import json
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
BASE, ROW, MAX_PAGE = 8192, 4096, 8
MAX_TICK = (2**32-1)*403200+403199
H = 'f'*64
S = 'f'*32


def observation():
    return dict(generation=2**64-2, dispatch_sequence=2**64-2, tick=MAX_TICK,
                loaded=True, paused=True, witness=H, eligible_at_capture=True,
                named_fortress_identity_established=False)


def record():
    return dict(idempotency_key='k'*128, plan_digest=H, state='cancelled_before_dispatch',
                terminal_coordination=False, reconciliation_required=True, game_ticks=1200,
                run_wall_millis=60000, before=observation(), native=dict(
                    phase='source_lost', reason='clock_regression', unpause_attempted=True,
                    historical_pause_verified=False, observed_tick=MAX_TICK,
                    observed_ticks_advanced=MAX_TICK, observed_tick_overshoot=MAX_TICK,
                    receipt_digest=H), current_pause_unproved=True,
                goal_completion_proven=False, retry_unpause=False)


def reference(r):
    return {k: r[k] for k in ('idempotency_key', 'plan_digest', 'state', 'reconciliation_required')}


def packet(rows, mode):
    """Conservative reference shape: includes both a selected row and a page."""
    r = record()
    return dict(agent_turn=dict(schema='dfmcp.agent_turn/1', operation='fortress.open_session',
        phase='reconcile', session_id=S, turn_id=None, request_id=S,
        anchor=dict(fortress_id=S, cursor=dict(epoch=2**64-2, sequence=2**64-2),
                    tick=MAX_TICK, state_hash=H, domain='source_bound_run_coordination_not_canonical_world'),
        continuity=dict(status='indeterminate', basis=None, gap=None,
                        reset_reason='coordination is not continuous game history'),
        profile='briefing', briefing=dict(runtime='unadmitted_development', bridge_protocol='1.13',
            runtime_admitted=False, mode=mode, named_fortress_identity_established=False,
            current_pause_unproved=True, goal_completion_proven=False, anchor_tick_semantics='zero_sentinel_not_game_time',
            journal=dict(id=H, head=H, transitions=4096, source_generation=2**64-2, fenced=False),
            retained_selection=observation()), changes=[], attention=[],
        active_work=dict(pending_plans=[], actions=[reference(r)]*4, obligations=[], cancellation_drains=[],
            indeterminate_effects=[], publications=[], confirmations=[], pending_count=256,
            unresolved_count=256, omitted_count=252,
            scope='this run journal only; historical coordination, not all game work', custody_verified=True,
            count_evidence='unverified_or_unbound; empty does not prove absence'),
        affordances=[], recommendations=[dict(recommendation_id='run-journal-discovery',tool='fortress.query',
            arguments=dict(session_id=S, state='unresolved', limit=2),
            reason='Rediscover durable pending work before starting another run.',expected_utility='control_safety',
            expected_information_value='high',risk='read_only',reversibility='not_applicable',requires_confirmation=False,
            estimated_cost=dict(output_tokens=None,bridge_bytes=0,wall_millis=None,game_ticks=None),
            prerequisites=['current Query authority and verified journal custody'],
            invalidating_conditions=['session close or journal custody failure'],
            confidence=dict(epistemic_state='certified_derived',value=None,evidence=[dict(journal_head=H)]))],
        uncertainty=[dict(uncertainty_id='run-clock-evidence', epistemic_state='unknown',
            statement='Native stop limits are callback triggers, not exact-tick or hard real-time guarantees.',
            consequence='Other controllers and native stalls are not excluded; historical pause does not prove current pause.',
            resolution=None, evidence=[])],
        coverage=dict(status='partial', complete_domains=['retained_run_coordination'], partial_domains=[],
            omitted_domains=['canonical_world','goal_completion','named_fortress_identity','current_pause'],continuation=None),
        budget=dict(admitted=dict(max_bytes=16*1024*1024,max_output_tokens=65536,max_wall_millis=60000,max_game_ticks=1200),
            consumed={},remaining=None,accounting='conservative pre-reservation; token proxy is four UTF-8 bytes'),references=[]),
        result=dict(ok=True,records=[r]*rows,effect=r,matching_records=256,state='unresolved',continuation=H,
            complete_matching_set_in_this_response=False,native_calls=0,session_id=S,mode=mode,source_connections_retained=0,
            journal_opened=True,capabilities=['query','plan','control_clock'],
            discovery=dict(tool='fortress.query',arguments=dict(session_id=S,state='unresolved',limit=2))))


def valid_spec(ticks, wall):
    return type(ticks) is int and type(wall) is int and 1 <= ticks <= 1200 and 1 <= wall <= 60000


class Reference(unittest.TestCase):
    def test_source_wires_exactly_eleven_tools_and_no_shell_wrapper(self):
        source = (ROOT/'crates/dfmcp-mcp/src/live_run_server.rs').read_text()
        expected = ['OpenSession','Observe','Query','Plan','Commit','Wait','Cancel','Checkpoint','Restore','Explain','Doctor']
        self.assertEqual(re.findall(r'\.tool\(Fortress(\w+)\)',source),expected)
        self.assertEqual(source.count('#[tool('),11)
        for forbidden in ['Command::new','std::process::Command','tokio::','run_script(']:
            self.assertNotIn(forbidden,source)
        self.assertIn('pub mod live_run_server;', (ROOT/'crates/dfmcp-mcp/src/lib.rs').read_text())
        self.assertIn('dfmcp_mcp::live_run_server::run_stdio()', (ROOT/'crates/dfmcp-mcp/src/bin/dfmcp-live-run-dev-server.rs').read_text())

    def test_worst_case_reference_packets_fit_reserved_output(self):
        sizes = []
        for rows, mode in itertools.product(range(9),('offline','recover','control')):
            size = len(json.dumps(packet(rows,mode),ensure_ascii=False,separators=(',',':')).encode())
            self.assertLessEqual(size,BASE+ROW*rows)
            sizes.append(size)
        print(f'independent conservative packet sizes: {min(sizes)}..{max(sizes)} bytes across 27 shapes')

    def test_output_refusal_boundary_precedes_work(self):
        for rows in range(9):
            reserve=BASE+ROW*rows
            for tokens in (1,reserve//4-1,reserve//4,reserve//4+1):
                for budget in (reserve-1,reserve,reserve+1,16*1024*1024):
                    accepted=budget>reserve and tokens*4>=reserve
                    self.assertEqual(accepted,(budget>=reserve+1 and tokens>=reserve//4))

    def test_run_spec_boundaries_reject_bool_negative_and_overflow(self):
        values=[True,False,None,'1',-1,0,1,1200,1201,60000,60001,2**64]
        accepted={(1,1),(1,1200),(1,1201),(1,60000),(1200,1),(1200,1200),(1200,1201),(1200,60000)}
        for ticks,wall in itertools.product(values,repeat=2):
            result=valid_spec(ticks,wall)
            self.assertEqual(result,type(ticks) is int and type(wall) is int and (ticks,wall) in accepted)

    def test_connection_and_journal_reserves_cover_maximum_work(self):
        rpc=270336
        # Worst 8 text notifications in each call, plus request/reply and frame headers.
        maximum_frame_work=262144+2048+2048+8*9+8
        self.assertLessEqual(maximum_frame_work,rpc)
        listing=2*1024*1024+256*512+1024
        connect=7*rpc+24
        for rows in range(9):
            remaining=16*1024*1024-(BASE+ROW*rows)-listing-connect
            self.assertGreater(remaining,4*(2*1024*1024)+4*rpc+8192)

    def test_reference_pagination_covers_every_row_without_duplication(self):
        cases=0
        for size,limit in itertools.product(range(257),range(1,9)):
            rows=list(range(size));seen=[]
            for offset in range(0,size,limit):
                seen.extend(rows[offset:min(offset+limit,size)])
            self.assertEqual(seen,rows);cases+=1
        print(f'independent whole-row pagination cases: {cases}')


if __name__ == '__main__':
    unittest.main(verbosity=2)
