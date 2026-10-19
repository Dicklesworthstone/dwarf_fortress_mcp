#!/usr/bin/env python3
"""Acquire one map/1.5 capture, then place all rooms against its terrain constraints.

The existing native read client closes before layout search. No mutation, retry,
background work, or automatic allocation/excavation stage. Original artifacts
can be revalidated offline. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import os
import time

import excavation_observer as native
import layout_rooms as offline
import room_layout as layout
import room_layout_observed as observed
from furniture_plan import canonical, require

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROOM_LAYOUT'
ENDPOINT = 'DFMCP_ROOM_LAYOUT_ENDPOINT'
TOKEN = 'DFMCP_MAP_TOKEN'
MAX_OUTPUT = 256 * 1024
EXPORTS = ('report', 'survey', 'layout', 'room-plan', 'room-request', 'excavation', 'furniture-request')


@dataclass(frozen=True)
class Authority:
    address: str
    token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in (OPT_IN, ENDPOINT, TOKEN) for key in os.environ)
                and os.environ.get(OPT_IN) == '1', 'isolated read-only room-layout opt-in required')
        address = os.environ.get(ENDPOINT, '127.0.0.1:5000')
        native.endpoint(address)
        token = os.environ.get(TOKEN, '').encode('utf-8')
        require(32 <= len(token) <= 256 and b'\0' not in token, 'invalid map query credential')
        return cls(address, token)

    def guard(self) -> None:
        require(Authority.load() == self, 'room-layout read authority changed')


class Budget(offline.Deadline):
    def __init__(self, milliseconds: int):
        super().__init__(milliseconds)
        self.authority = None
        self.left = layout.MAX_WORK

    def check(self) -> None:
        super().check()
        if self.authority is not None:
            self.authority.guard()

    def work(self) -> None:
        self.check()
        require(self.left > 0, 'room-layout command work allowance exhausted')
        self.left -= 1

    def remaining(self) -> float:
        self.check()
        return self.end - time.monotonic()


class _MapClient(native.MapClient):
    """Retain the parent's fixed protocol, adding the enclosing authority/deadline."""
    def __init__(self, authority: Authority, region: native.Region, budget: Budget):
        self.budget, self.authority = budget, authority
        authority.guard()
        milliseconds = min(60000, int(budget.remaining() * 1000))
        require(milliseconds > 0, 'room-layout native acquisition deadline exhausted')
        super().__init__(authority.address, authority.token, region, milliseconds)

    def remaining(self) -> float:
        self.authority.guard()
        return min(self.budget.remaining(), super().remaining())


def packet(result: observed.ObservedLayout | None, contacted: bool = False,
           code: str | None = None) -> dict:
    value = None if result is None else result.json()
    out = offline.report(None if value is None else value['layout'],
                         (code or 'ROOM_TERRAIN_LAYOUT_REFUSED') if value is None else None)
    out['schema'] = 'dfmcp.observed-room-layout-command/1'
    out['result'] = value
    if value is None:
        out['error']['detail'] = 'Original layout request, source capture, authority, shared budget or output refused. No partial layout or effect was produced.'
    turn = out['agent_turn']
    turn['operation'] = 'rooms.survey_layout'
    turn['budget']['output_bytes_limit'] = MAX_OUTPUT
    turn['coverage'].update(domain='complete_requested_map_constrained_layout',
        native_observations=1 if contacted and value is not None else 0,
        native_connection_attempted=contacted, native_acquisition_verified_this_call=contacted and value is not None)
    turn['uncertainty'].append(
        'The complete map payload constrains the selected geometry at its recorded tick. Hidden, wet, designated or occupied cells are unavailable; wall preservation requires wall-shaped evidence. Aquifers, adjacent hazards, structural support, native pathfinding and present safety are not established.')
    if value is not None:
        turn['references'].append({'kind': 'room_layout_map_capture', 'profile': 'map/1.5',
            'witness': value['source']['map_witness'], 'game_tick': value['source']['game_tick'],
            'survey_digest': value['survey_digest'], 'canonical_anchor': False,
            'acquired_this_call': contacted})
    if value is not None and 'excavation_evidence' in value:
        turn['briefing']['excavation_comparison'] = value['excavation_evidence']
        turn['uncertainty'].append(
            'Excavation counts distinguish wall-shaped targets from existing floors in the original capture; '
            'they do not establish mineability or game-time cost. The connection compiler is not cost-rerouted.')
        if value['excavation_evidence'].get('corridor_cost_routing_enabled'):
            turn['uncertainty'][-1] = (
                'Corridor candidates are compared using costs derived from the original capture. '
                'The tile and rectangle limits remain binding; global network cost optimality, '
                'mineability and game-time cost are not established.')
    return out


class OptimumUnproven(ValueError):
    pass


def export(result: observed.ObservedLayout, kind: str, contacted: bool, *,
           require_optimal: bool = False) -> bytes:
    require(type(require_optimal) is bool, 'optimal-layout selection must be Boolean')
    if require_optimal:
        selected = result.json()['layout']
        if not (selected['status'] == 'layout_found'
                and selected.get('optimization', {}).get('minimum_in_compiler_domain_proven') is True):
            raise OptimumUnproven('complete observed layout optimum is unproven')
    if kind == 'report':
        raw = canonical(packet(result, contacted)) + b'\n'
    elif kind == 'survey':
        raw = result.encode()
    else:
        # Only a solve/decode result reaches here; use the existing exact exports.
        value = result.json()['layout'].copy()
        value.pop('layout_digest')
        raw = offline.export(layout.Layout(canonical(value)), kind)
    require(len(raw) <= MAX_OUTPUT, 'complete surveyed layout output exceeds its allowance')
    return raw


def main(argv: list[str] | None = None) -> int:
    contacted = False
    try:
        parser = offline.Parser(description=__doc__, allow_abbrev=False)
        source = parser.add_mutually_exclusive_group(required=True)
        source.add_argument('--request-file', help='acquire one native map for the complete original request')
        source.add_argument('--survey-file', help='revalidate saved evidence offline, without native contact')
        parser.add_argument('--emit', choices=EXPORTS, default='report')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        parser.add_argument('--require-optimal-layout', action='store_true',
                            help='export only a proven optimum in the original surveyed/compiler domain')
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        raw = offline.read_input(args.survey_file or args.request_file,
                                 observed.MAX_OUTPUT if args.survey_file else layout.MAX_INPUT, budget)
        if args.survey_file:
            result = observed.ObservedLayout.decode(raw, budget.work)
        else:
            value = layout._json(raw, layout.MAX_INPUT, budget.check)
            require(type(value) is dict, 'original layout request required')
            search = value.get('search', {})
            layout._object(search, set(), {'max_nodes', 'max_work'})
            budget.left = native.integer(search.get('max_work', layout.DEFAULT_WORK), 1, layout.MAX_WORK)
            request, _ = observed.prepare(raw, budget.work)
            budget.authority = Authority.load()
            budget.check()
            contacted = True
            with _MapClient(budget.authority, native.Region.from_json(request['region']), budget) as client:
                capture = client.observe()
            # Keep the original raw request, not a substituted narrowed intention.
            result = observed.solve(raw, capture.raw, capture.manifest, budget.work)
        raw = export(result, args.emit, contacted, require_optimal=args.require_optimal_layout)
        budget.check()
    except OptimumUnproven:
        offline.publish(canonical(packet(None, contacted, 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')) + b'\n')
        return 2
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        offline.publish(canonical(packet(None, contacted)) + b'\n')
        return 2
    return offline.publish(raw)


if __name__ == '__main__':
    raise SystemExit(main())
