#!/usr/bin/env python3
"""Monitor complete original room terrain and receipt-linked furnishings together.

Query-only game workflow. Keep the original room-backed batch and separate
append-only monitor open through final output. Never advance time or retry effects.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-10.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

from furniture_plan import canonical, require
from route_furniture_review import digest_text
from route_furniture_completion import origin_path
import construction_wait as foreground
import room_route_completion_rpc as native
from room_route_completion import POLICY, Progress, from_batch, verify_origin
from room_route_completion_store import Journal, State, MAX_FILE, MAX_FRAMES

MAX_OUTPUT = 256 * 1024
TIMING = ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations')


def open_batch(path, expected, budget):
    from route_furniture_batch_store import Batch
    return Batch(path, expected, budget)


def guard(owner, batch, authority=None):
    if authority is not None:
        authority.guard()
        require(authority.address == owner.state.address, 'original room query endpoint differs')
    require(owner.state.address == batch.address, 'room monitor endpoint differs from original batch')
    verify_origin(owner.state.goal, batch)
    owner.check()
    owner.budget.remaining()


def packet(operation, state=None, *, stored=False, contacted=False, wait=None, failed=False, checkpoint=lambda:None):
    goal = None if state is None else state.goal
    progress = None if state is None else state.progress
    result = None
    if state is not None:
        room, _ = goal.handoff.unpack(checkpoint)
        result = {'goal_digest': goal.digest, 'batch_id': goal.batch_id,
            'room_handoff_digest': goal.handoff.digest, 'room_plan_digest': room.digest,
            'review_digest': goal.review_digest, 'original_room_plan': room.json(),
            'original_furniture_plan': goal.plan.json(), 'target_count': len(goal.plan.steps),
            'progress': progress.view(), 'goal': {
                **{name:getattr(goal.condition,name) for name in ('deadline',*TIMING)},
                'generation': goal.generation,
                'require_circulation': goal.require_circulation,
                'require_project_access': goal.require_project_access},
            'journal': {'frames':state.frames,'head':state.tail.hex()},
            'native_contacted_this_call':contacted, 'storage_acknowledged_this_call':stored,
            'original_custody_verified':True, 'placement_effects_discharged':False,
            'current_usability_proven':False, 'current_routes_proven':False, 'wait':wait}
    return {'schema':'dfmcp.room-route-completion-result/1','ok':not failed,'policy':POLICY,
        'result':result,'runtime_admitted':False,
        **({'error':{'code':'ROOM_ROUTE_COMPLETION_REFUSED', 'detail':
            'Original room, batch, monitor, source, authority or budget refused. Preserve original journals; do not repeat placement.'}} if failed else {}),
        'agent_turn': {'schema':'dfmcp.agent_turn/1','operation':'room_route_completion.'+operation,
            'phase':'reconcile' if failed else 'inspect','profile':'forensic',
            'session_id':None,'turn_id':None,'request_id':None,'anchor':None,
            'continuity':{'status':'indeterminate' if failed else 'stale','basis':None,'gap':None},
            'briefing':{'runtime_admitted':False,'mutation_admissible':False,
                'sampled_whole_room_condition':bool(progress and progress.phase=='satisfied')},
            'active_work':[] if progress is None or progress.terminal else [{
                'kind':'original_room_completion','goal_digest':goal.digest,'state':progress.phase,
                'read_outcome_unknown':progress.reading}],
            'changes':[],'attention':[],'affordances':[],'recommendations':[],
            'coverage':{'complete_original_room':goal is not None,'all_original_furnishings':goal is not None,
                'circulation_required':bool(goal and goal.require_circulation),
                'project_access_required':bool(goal and goal.require_project_access),
                'continuous_stability_proven':False,'native_room_assignment_proven':False},
            'uncertainty':['Completion is historical sampled room/furnishing evidence, not current usability.',
                'Equal paused map endpoints do not prove an atomic cross-profile snapshot.',
                'This monitor does not authorize placement retries, room assignment or time control.'],
            'budget':{'maximum_output_bytes':MAX_OUTPUT,'token_count_measured':False},
            'references':[] if goal is None else [goal.batch_id,goal.handoff.digest,goal.digest]}}


def output(value):
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete original room response exceeds bound')
    return raw


def reserve(operation, state, budget, limits=None):
    value = packet(operation,state,stored=True,contacted=True,
        wait=None if limits is None else foreground.reserve_view(limits),checkpoint=budget.work)
    value['result']['journal'] = {'frames':MAX_FRAMES,'head':'f'*64,'bytes':MAX_FILE}
    require(len(output(value)) + 65536 <= MAX_OUTPUT, 'complete future room evidence cannot fit')


def sample_once(owner, batch, authority, render):
    guard(owner,batch,authority)
    require(owner.writable and not owner.state.progress.terminal, 'room monitor terminal or read-only')
    require(owner.budget.calls >= 13 + 2*len(owner.state.goal.receipts),
            'complete room read does not fit remaining calls')
    render(owner.state)
    guard(owner,batch,authority)
    owner.start_read()
    guard(owner,batch,authority)
    sample = native.acquire(authority,owner.state.goal,owner.budget)
    guard(owner,batch,authority)
    def before_publish(candidate):
        render(candidate)
        guard(owner,batch,authority)
    owner.accept(sample,before_publish)
    guard(owner,batch,authority)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid original room completion arguments')


def write_output(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw): return False
        sys.stdout.buffer.flush()
        return True
    except (OSError,ValueError,KeyboardInterrupt):
        return False


def main(argv=None):
    operation, owner, batch, authority = 'unknown',None,None,None
    attempted, code = False,2
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation',choices=('start','sample','wait','inspect','cancel'))
        parser.add_argument('--journal',required=True)
        parser.add_argument('--batch-directory',required=True)
        parser.add_argument('--batch-id',required=True)
        parser.add_argument('--expected-goal')
        parser.add_argument('--deadline-tick',type=int)
        for flag,name in (('--interval-ticks','interval'),('--stable-samples','stable_samples'),
                          ('--stable-span-ticks','stable_span'),('--max-gap-ticks','max_gap'),
                          ('--max-observations','max_observations')):
            parser.add_argument(flag,dest=name,type=int)
        parser.add_argument('--timeout-ms',type=int,default=10000)
        foreground.add_arguments(parser)
        args = parser.parse_args(argv)
        operation = args.operation
        limits = foreground.parse_limits(args)
        origin_path(args.batch_directory)
        digest_text(args.batch_id)
        require((args.deadline_tick is not None)==(operation=='start')
                and (args.expected_goal is None)==(operation=='start'),
                'start fixes a deadline; reopening requires the original goal digest')
        require(operation=='start' or all(getattr(args,n) is None for n in TIMING),
                'reopening cannot renew original room timing')
        require(not Path(args.journal).is_relative_to(Path(args.batch_directory)),
                'monitor must remain outside the closed original batch directory')
        if args.expected_goal is not None: digest_text(args.expected_goal)
        budget = native.Budget(args.timeout_ms)
        batch = open_batch(args.batch_directory,args.batch_id,budget)
        if operation=='start':
            goal = from_batch(batch,args.deadline_tick,**{n:getattr(args,n) for n in TIMING if getattr(args,n) is not None})
            authority = native.Authority.load()
            require(authority.address==batch.address,'room query endpoint changed')
            reserve(operation,State(goal,batch.address,Progress(goal.digest)),budget)
            authority.guard()
            verify_origin(goal,batch)
            owner = Journal(args.journal,budget,writable=True,create=(goal,batch.address))
        else:
            owner = Journal(args.journal,budget,writable=operation in ('sample','wait','cancel'))
            require(owner.state.goal.digest==args.expected_goal,'selected original room goal differs')
        guard(owner,batch)
        stored,contacted,wait = operation=='start',False,None
        if operation in ('start','sample') and not owner.state.progress.terminal:
            authority = authority or native.Authority.load()
            sample_once(owner,batch,authority,lambda state:reserve(operation,state,budget))
            stored=contacted=True
        elif operation=='wait':
            authority = None if owner.state.progress.terminal else native.Authority.load()
            result = foreground.run(owner,authority,native.acquire,
                lambda state:reserve(operation,state,budget,limits),limits,
                source_guard=lambda:guard(owner,batch),additional_rpc_calls=5)
            wait=result.view()
            stored=contacted=result.samples>0
        elif operation=='cancel':
            stored=owner.cancel(lambda state:reserve(operation,state,budget))
        value=packet(operation,owner.state,stored=stored,contacted=contacted,wait=wait,checkpoint=budget.work)
        value['result']['journal']['bytes']=owner.length
        raw=output(value)
        guard(owner,batch,authority)
        attempted=True
        code=0 if write_output(raw) else 2
    except (OSError,ValueError,TypeError,KeyError,RecursionError,KeyboardInterrupt):
        if not attempted:
            attempted=True
            write_output(output(packet(operation,failed=True)))
        code=2
    finally:
        for resource in (owner,batch):
            if resource is not None:
                try: resource.close()
                except (OSError,ValueError): code=2
    return code


if __name__=='__main__':
    raise SystemExit(main())
