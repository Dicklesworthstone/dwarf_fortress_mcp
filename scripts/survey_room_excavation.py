#!/usr/bin/env python3
"""Acquire one complete map, lay out rooms, and export their original dig handoff.

Read-only foreground command using the existing isolated room-layout authority.
No automatic digging, retry, background work or new native method. Saved
compilations replay offline without credentials or additional source contact.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

import excavation_observer as native
import layout_rooms as offline
import room_layout as layout
import room_layout_excavation as compiler
import room_terrain as terrain
import survey_room_layout as survey
from furniture_plan import canonical, require
from room_excavation_handoff import MAX_BYTES as HANDOFF_BYTES, RoomExcavationHandoff

MAX_REPORT = 64 * 1024
EXPORTS = ('report', 'compilation', 'layout', 'room-plan', 'room-request',
           'furniture-request', 'terrain-survey', 'remaining-excavation', 'excavation-handoff')


class Budget(survey.Budget):
    def work(self) -> None:
        self.check()
        if self.left <= 0:
            raise layout.WorkLimit('room excavation command work allowance exhausted')
        self.left -= 1


class ExportUnavailable(ValueError):
    """A well-formed result contains no requested downstream artifact."""

    def __init__(self, status: str):
        self.code = {
            'optimality_unproven': 'ROOM_EXCAVATION_OPTIMUM_UNPROVEN',
            'search_limit_reached': 'ROOM_EXCAVATION_SEARCH_LIMIT',
            'no_layout_in_domain': 'ROOM_EXCAVATION_LAYOUT_NOT_FOUND',
            'blocked': 'ROOM_EXCAVATION_TERRAIN_BLOCKED',
            'terrain_shapes_satisfied_at_sample': 'ROOM_EXCAVATION_NO_REMAINING_WORK',
        }.get(status, 'ROOM_EXCAVATION_EXPORT_UNAVAILABLE')
        super().__init__(self.code)


def packet(result: compiler.ExcavationLayout | None, contacted: bool,
           code: str | None = None) -> dict:
    """Compact decision packet, not an unbounded duplicate of retained evidence."""
    value = None if result is None else result.json()
    selected = None if value is None else value['layout']['room_plan']
    evidence = None if value is None else value['terrain_survey']
    ready = value is not None and value['handoff_digest'] is not None
    out = offline.report(None, code)
    out['schema'] = 'dfmcp.room-layout-excavation-command/1'
    if code is not None:
        out['error']['detail'] = (
            'Original request, source, authority, budget, artifact or requested export refused. '
            'No game effect or partial downstream artifact was produced.')
    out['result'] = None if value is None else {
        'status': value['status'], 'compilation_digest': value['compilation_digest'],
        'source': value['source'], 'original_request': value['request'],
        'layout_digest': value['layout']['layout_digest'],
        'room_plan_digest': None if selected is None else selected['plan_digest'],
        'handoff_digest': value['handoff_digest'],
        'planned_areas': 0 if selected is None else len(selected['areas']),
        'planned_furnishings': 0 if selected is None else selected['furniture_count'],
        'target_tiles': None if selected is None else selected['target_tiles'],
        'remaining_wall_tiles': None if evidence is None else evidence['target_counts']['remaining_wall'],
        'terrain_survey_digest': None if evidence is None else evidence['survey_digest'],
        'blocker_counts': {} if evidence is None else evidence['blockers']['counts'],
        'search': value['layout']['search'],
        'handoff_available': ready, 'game_effect_performed': False,
        'native_excavation_eligibility_proven': False, 'room_completion_proven': False,
        'mutation_authority_granted': False, 'production_admitted': False}
    turn = out['agent_turn']
    turn['operation'] = 'rooms.survey_excavation'
    turn['phase'] = 'propose' if ready else 'inspect'
    turn['briefing'].update(
        layout_status=None if value is None else value['status'],
        planned_areas=0 if selected is None else len(selected['areas']),
        planned_furnishings=0 if selected is None else selected['furniture_count'],
        excavation_handoff_available=ready)
    turn['coverage'].update(domain='single_padded_capture_room_layout_and_remaining_excavation',
        original_request_retained=value is not None,
        native_observations=int(contacted and value is not None),
        native_connection_attempted=contacted,
        native_acquisition_verified_this_call=contacted and value is not None,
        derived_subregions=0 if value is None else 1 + int(evidence is not None),
        additional_native_reads=0)
    turn['budget']['output_bytes_limit'] = MAX_REPORT
    turn['uncertainty'] = [
        'The parent map is historical at its recorded tick; exact subregions are derived from that one capture, not separate native observations.',
        'Layout, residual terrain shapes and original-room handoff do not prove current mining eligibility, structural or aquifer safety, native pathfinding, furniture availability or room completion.',
        'Fresh dig review, explicit confirmation and original-key custody remain mandatory. No uncertain effect is reconciled and no replacement effect key is authorized.',
        'A saved compilation replays supplied evidence without attesting its original acquisition or refreshing current fortress truth.']
    turn['references'] = [] if value is None else [{
        'kind': 'room_layout_excavation', 'digest': value['compilation_digest'],
        'parent_map_witness': value['source']['map_witness'],
        'game_tick': value['source']['game_tick'], 'canonical_anchor': False,
        'acquired_this_call': contacted}]
    turn['recommendations'] = [] if not ready else [{
        'kind': 'export_and_review_original_room_excavation',
        'handoff_digest': value['handoff_digest'],
        'export': 'excavation-handoff', 'from': 'retained_compilation',
        'consumer': 'dig_blueprint_client.py init --room-handoff',
        'fresh_native_review_required': True, 'confirmation_required': True,
        'automatic_execution': False, 'mutation_authority_granted': False}]
    if value is not None and 'residual_partition' in value['request']:
        out['result']['residual_partition'] = value['layout']['residual_partition']
        out['result']['optimization'] = value['layout']['optimization']
        turn['briefing']['residual_capacity'] = value['layout_evidence']['residual_capacity_evidence']
        turn['uncertainty'].append(
            'Residual capacity is checked for the complete selected wall mask, not a minimum rectangle cover. '
            'Optimality is restricted to capacity-valid candidates in the declared compiler domain.')
    return out


def export(result: compiler.ExcavationLayout, handoff: RoomExcavationHandoff | None,
           kind: str, contacted: bool, *, require_optimal: bool = False) -> bytes:
    """Only the current solve/replay pair enters; no unchecked imported objects."""
    require(kind in EXPORTS, 'unknown room excavation export')
    value = result.json()
    require(type(require_optimal) is bool, 'optimal-layout selection must be an exact Boolean')
    if require_optimal:
        selected = value['layout']
        if not (selected['status'] == 'layout_found'
                and selected.get('optimization', {}).get('minimum_in_compiler_domain_proven') is True):
            raise ExportUnavailable('optimality_unproven')
    maximum = compiler.MAX_OUTPUT
    if kind == 'report':
        raw = canonical(packet(result, contacted)) + b'\n'
        maximum = MAX_REPORT
    elif kind == 'compilation':
        raw = result.encode()
    elif kind in ('excavation-handoff', 'remaining-excavation'):
        if handoff is None:
            raise ExportUnavailable(value['status'])
        require(handoff.digest == value['handoff_digest'], 'handoff and compilation differ')
        raw = handoff.encode() if kind == 'excavation-handoff' else canonical(handoff.json()['remaining_blueprint'])
        maximum = HANDOFF_BYTES if kind == 'excavation-handoff' else layout.MAX_INPUT
    elif kind == 'terrain-survey':
        if value['terrain_survey'] is None:
            raise ExportUnavailable(value['status'])
        raw = canonical({**value['terrain_survey'], 'room_plan': value['layout']['room_plan']})
        maximum = terrain.MAX_OUTPUT
    else:
        if kind != 'layout' and value['layout']['room_plan'] is None:
            raise ExportUnavailable(value['status'])
        selected = value['layout'].copy()
        selected.pop('layout_digest')
        raw = offline.export(layout.Layout(canonical(selected)), kind)
        maximum = offline.MAX_COMMAND_OUTPUT
    require(len(raw) <= maximum, 'complete room excavation export exceeds bound')
    return raw


def main(argv: list[str] | None = None) -> int:
    contacted = False
    try:
        parser = offline.Parser(description=__doc__, allow_abbrev=False)
        source = parser.add_mutually_exclusive_group(required=True)
        source.add_argument('--request-file', help='acquire one padded map for the complete room request')
        source.add_argument('--compilation-file', help='replay retained evidence offline, without native contact')
        parser.add_argument('--emit', choices=EXPORTS, default='report')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        parser.add_argument('--require-optimal-layout', action='store_true',
                            help='export only a proven optimum in the declared layout and residual-partition domain')
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        saved = args.compilation_file is not None
        raw = offline.read_input(args.compilation_file if saved else args.request_file,
                                 compiler.MAX_OUTPUT if saved else layout.MAX_INPUT, budget)
        if saved:
            result, handoff = compiler.replay(raw, budget.work)
        else:
            value = layout._json(raw, layout.MAX_INPUT, budget.check)
            require(type(value) is dict, 'original room layout request required')
            limits = value.get('search', {})
            layout._object(limits, set(), {'max_nodes', 'max_work'})
            budget.left = native.integer(limits.get('max_work', layout.DEFAULT_WORK), 1, layout.MAX_WORK)
            _, region, _ = compiler.prepare(raw, budget.work)
            budget.authority = survey.Authority.load()
            budget.check()
            contacted = True
            with survey._MapClient(budget.authority, region, budget) as client:
                capture = client.observe()
            # Release the native stream before search. Both later views come
            # from these same bytes; never reconnect or automatically dig.
            result, handoff = compiler.solve(raw, capture.raw, capture.manifest,
                                             budget.authority.address, budget.work)
        output = (export(result, handoff, args.emit, contacted, require_optimal=True)
                  if args.require_optimal_layout else export(result, handoff, args.emit, contacted))
        budget.check()  # Includes revocation AFTER complete output serialization.
    except ExportUnavailable as cause:
        code = cause.code
    except offline.DeadlineExceeded:
        code = 'ROOM_EXCAVATION_TIMEOUT'
    except layout.WorkLimit:
        code = 'ROOM_EXCAVATION_WORK_LIMIT'
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        code = 'ROOM_EXCAVATION_REFUSED'
    else:
        return offline.publish(output)  # Short write never retries or emits another object.
    offline.publish(canonical(packet(None, contacted, code)) + b'\n')
    return 2


if __name__ == '__main__':
    raise SystemExit(main())
