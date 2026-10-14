"""Compose fresh route readiness with current original-prefix construction.

Input is the execution owner's validated route-frontier result, not a client
assertion or authorization token. Preserve every existing route and receipt
check; add current construction evidence under the SAME total work allowance.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import hashlib

from build_placement_wire import canonical, integer, require
from construction_prefix import MAX_WORK, Work, assess_prefix

ROUTE_POLICY = 'original-route-constraints-and-fixed-delivery-suffix/2'
POLICY = 'original-route-delivery-and-quiescent-construction-prefix/3'
MAX_INPUT = 96 * 1024
MAX_OUTPUT = 160 * 1024
DOMAIN = b'dfmcp-route-furniture-frontier/1\0'


def _bound(value: object, work: Work) -> None:
    """Bound nesting, visits and text before serialization; cycles exhaust depth."""
    pending, count, text_bytes = [(value, 0)], 0, 0
    while pending:
        node, depth = pending.pop()
        work()
        count += 1
        require(count <= 32768 and depth <= 16, 'route frontier shape exceeds bound')
        if type(node) is dict:
            require(count + len(pending) + len(node) <= 32768, 'route frontier nodes exceed bound')
            for key, child in node.items():
                require(type(key) is str and len(key) <= 128, 'invalid route frontier field')
                text_bytes += len(key.encode('utf-8'))
                pending.append((child, depth + 1))
        elif type(node) is list:
            require(count + len(pending) + len(node) <= 32768, 'route frontier nodes exceed bound')
            pending.extend((child, depth + 1) for child in node)
        elif type(node) is str:
            require(len(node) <= MAX_INPUT and '\0' not in node, 'invalid route frontier text')
            text_bytes += len(node.encode('utf-8'))
        elif type(node) is int:
            require(-(2**63) <= node < 2**64, 'route frontier integer exceeds bound')
        else:
            require(node is None or type(node) is bool, 'invalid route frontier value')
        require(text_bytes <= MAX_INPUT, 'route frontier text exceeds bound')
    require(len(canonical(value)) <= MAX_INPUT, 'route frontier bytes exceed bound')


def gate_frontier(route: dict, placed: tuple[bytes, ...], operations_raw: bytes, guard,
                  *, maximum_work: int = MAX_WORK) -> dict:
    """Keep the existing result shape; ready now additionally gates construction.

    The route owner has already charged its analysis. Only the unconsumed
    portion of maximum_work is available to this composition. Current receipt
    conditions cannot make an unready route ready or discharge any goal/effect.
    """
    work = Work(maximum_work, guard)
    require(type(route) is dict, 'validated original route frontier required')
    work.used = integer(route.get('work_units'), 1, maximum_work)
    integer(route.get('maximum_work'), work.used, MAX_WORK)
    _bound(route, work)
    require(route.get('schema') == 'dfmcp.route-furniture-frontier/1'
            and route.get('policy') == ROUTE_POLICY
            and route.get('status') in ('ready', 'route_blocked', 'route_unestablished')
            and route.get('all_remaining_selections_checked') is True,
            'unsupported or incomplete original route frontier')
    evidence = {key: value for key, value in route.items()
                if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    require(route.get('evidence_digest') == hashlib.sha256(DOMAIN + canonical(evidence)).hexdigest(),
            'route frontier evidence changed before construction checks')
    require(type(placed) is tuple and type(route.get('placed_prefix')) is list
            and len(placed) == len(route['placed_prefix']) <= 32,
            'construction selection differs from the complete route prefix')
    allowance = maximum_work - work.used - len(placed) - 1
    require(allowance > 0, 'no shared allowance remains for predecessor construction')
    construction = assess_prefix(placed, operations_raw, work.check, maximum_work=allowance)
    work.used += construction['work_units']
    source = route.get('source')
    require(type(source) is dict
            and (construction['capture_sha256'], construction['game_tick'], construction['horizons']) ==
                (source.get('inventory_sha256'), source.get('tick'), source.get('horizons')),
            'construction and routes do not share the exact operations capture')
    for saved, observed in zip(route['placed_prefix'], construction['rows']):
        work()
        require(type(saved) is dict and
                (saved.get('key'), saved.get('receipt_digest'), saved.get('native_plan_digest')) ==
                (observed['key'], observed['receipt_digest'], observed['native_plan_digest']),
                'construction changed an original route predecessor')
    out = dict(route)
    out.update(policy=POLICY, route_status=route['status'],
               route_evidence_digest=route['evidence_digest'],
               construction_prefix={k: v for k, v in construction.items()
                                    if k not in ('work_units', 'maximum_work')},
               prefix_matches_route_frontier=True)
    if route['status'] == 'ready' and not construction['predecessor_gate_met']:
        out['status'] = 'construction_' + construction['status']
    work()
    evidence = {key: value for key, value in out.items()
                if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    out['evidence_digest'] = hashlib.sha256(DOMAIN + canonical(evidence)).hexdigest()
    out.update(work_units=work.used, maximum_work=maximum_work)
    require(len(canonical(out)) <= MAX_OUTPUT, 'complete execution frontier exceeds output bound')
    work.check()
    return out
