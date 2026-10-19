"""Whole-room layout constrained by one complete map/1.5 payload.

Decode original source bytes; never accept caller-authored derived tiles. This
filters unsuitable sites, not native mining safety or placement authority.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json

import excavation_observer as native
import room_layout as layout
from furniture_plan import canonical, require
from room_site_mask import SiteMask

SCHEMA = 'dfmcp.observed-room-layout/1'
POLICY = 'dfmcp.visible-dry-unoccupied-room-site/1'
EXCAVATION_OBJECTIVE = 'minimum-observed-excavation/1'
EXCAVATION_POLICY = 'dfmcp.observed-excavation-room-site/1'
MAX_CAPTURE_BYTES = 575 + 20 * 1024
MAX_OUTPUT = 192 * 1024
USES = {'.': frozenset('FW'), 'F': frozenset('F'), 'W': frozenset('W'), 'X': frozenset()}
SYMBOLS = {value: key for key, value in USES.items()}


def prepare(raw: bytes, guard: layout.Guard = layout._idle) -> tuple[dict, layout._Work]:
    """Use the existing complete request validator before any optional contact."""
    value = layout._json(raw, layout.MAX_INPUT, guard)
    require(type(value) is dict, 'observed layout needs the original request')
    # Observed costs come only from the raw map, never a caller-authored mask.
    require('excavation_costs' not in value, 'observed layout cannot import tile costs')
    excavation = value.get('objective') == EXCAVATION_OBJECTIVE
    normalized_input = {**value, 'objective': layout.MINIMUM_TARGETS_OBJECTIVE} if excavation else value
    search = value.get('search', {})
    layout._object(search, set(), {'max_nodes', 'max_work'})
    work = layout._Work(search.get('max_work', layout.DEFAULT_WORK), guard)
    request, _ = layout._normalize(normalized_input, work)
    if excavation:
        request['objective'] = EXCAVATION_OBJECTIVE
        require(len(canonical(request)) <= layout.MAX_INPUT, 'normalized observed request exceeds bound')
        work()
    return request, work


def _site(capture: native.Capture, work: layout._Work) -> tuple[SiteMask, dict]:
    # Called only with a complete freshly decoded payload and a bounded region.
    counts = dict.fromkeys(('wall', 'floor', 'hidden', 'missing', 'liquid',
                           'designated', 'occupied', 'other_shape', 'map_edge'), 0)
    cells = []
    ox, oy, oz = capture.region.origin
    width = capture.region.size[0]
    for index, tile in enumerate(capture.tiles):
        work()
        symbol = 'X'
        x, y = ox + index % width, oy + index // width
        if tile.presence != 2:
            reason = 'hidden' if tile.presence == 1 else 'missing'
        else:
            _, shape, depth, magma, _, dig, building, units, *_ = tile.attributes
            reason = ('liquid' if depth or magma else 'designated' if dig else
                      'occupied' if building or units else 'other_shape' if shape not in (2, 3) else
                      'map_edge' if any(p <= 0 or p + 1 >= n for p, n in
                                        zip((x, y, oz), capture.dimensions)) else
                      'wall' if shape == 2 else 'floor')
            symbol = '.' if reason == 'wall' else 'F' if reason == 'floor' else 'X'
        cells.append(symbol)
        counts[reason] += 1
    return SiteMask(capture.region.origin, capture.region.size, ''.join(cells)), counts


@dataclass(frozen=True)
class ObservedLayout:
    _body: bytes

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-observed-room-layout/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        return {**json.loads(self._body), 'survey_digest': self.digest}

    def encode(self) -> bytes:
        return canonical(self.json())

    @classmethod
    def decode(cls, raw: bytes, guard: layout.Guard = layout._idle) -> ObservedLayout:
        value = layout._json(raw, MAX_OUTPUT, guard)
        require(type(value) is dict and {'request', 'capture_hex', 'source'} <= set(value),
                'observed layout lacks its original request and source')
        text = value['capture_hex']
        require(type(text) is str and 2 <= len(text) <= 2 * MAX_CAPTURE_BYTES
                and len(text) % 2 == 0 and all(c in '0123456789abcdef' for c in text),
                'invalid canonical map capture hex')
        manifest = native.Manifest.from_json(value['source']['manifest'])
        result = solve(canonical(value['request']), bytes.fromhex(text), manifest, guard)
        require(result.encode() == raw, 'observed layout differs from regenerated original evidence')
        return result


def solve(raw: bytes, capture_raw: bytes, manifest: native.Manifest,
          guard: layout.Guard = layout._idle) -> ObservedLayout:
    request, work = prepare(raw, guard)
    require(type(manifest) is native.Manifest, 'exact map source manifest required')
    manifest = native.Manifest.from_json(manifest.json())
    region = native.Region.from_json(request['region'])
    require(type(capture_raw) is bytes and 1 <= len(capture_raw) <= 575 + 20 * region.volume,
            'map capture exceeds the complete original domain bound')
    work()
    capture = native.decode_capture(capture_raw, manifest, region)
    work()
    require(capture.paused and (capture.folder, capture.site) == (request['world_folder'], request['site']),
            'observed layout requires the original paused fortress')
    observed, counts = _site(capture, work)
    if 'access_point' in request:
        work()
        # Check the decoded observation BEFORE caller site-mask intersection:
        # caller "F" on an observed wall must not become evidence of a floor.
        require(observed.at(tuple(request['access_point'])) == 'F',
                'access point must be observed visible dry undesignated unoccupied floor')
    effective = observed
    if 'site_mask' in request:
        original = SiteMask.from_json(request['site_mask'], work)
        symbols = []
        for a, b in zip(original.cells, observed.cells):
            work()
            symbols.append(SYMBOLS[USES[a] & USES[b]])
        effective = SiteMask(region.origin, region.size, ''.join(symbols))
    constrained = {**request, 'site_mask': effective.json()}
    costs = None
    if request.get('objective') == EXCAVATION_OBJECTIVE:
        from room_layout_costs import TileCosts
        # Use the pre-intersection observation. Caller F on a wall remains a
        # charged wall, and hidden/unusable cells remain excluded by effective.
        cost_cells = []
        for cell in observed.cells:
            work()
            cost_cells.append('0' if cell == 'F' else '1')
        constrained.update(objective=layout.COST_OBJECTIVE, excavation_costs=''.join(cost_cells))
        costs = TileCosts.parse(request['region'], constrained['excavation_costs'], work)
    found = layout.solve(canonical(constrained), work)
    body = {'schema': SCHEMA, 'policy': EXCAVATION_POLICY if costs is not None else POLICY, 'request': request,
            'status': found.json()['status'], 'layout': found.json(),
            'source': {**capture.binding(), 'game_tick': capture.tick,
                       'capture_sha256': hashlib.sha256(capture_raw).hexdigest(),
                       'map_witness': capture.witness, 'profile': 'map/1.5'},
            'capture_hex': capture_raw.hex(), 'observed_counts': counts,
            'effective_site_mask': effective.json(), 'work_units': work.used,
            'complete_payload_decoded': True, 'native_acquisition_independently_attested': False,
            'native_mining_eligibility_proven': False, 'aquifer_safety_proven': False,
            'adjacent_hazards_observed': False, 'construction_support_proven': False,
            'current_terrain_proven': False, 'mutation_authority_granted': False,
            'production_admitted': False}
    if 'access_point' in request:
        body['access_evidence'] = {
            'point': request['access_point'], 'epistemic_status': 'observed',
            'game_tick': capture.tick, 'map_witness': capture.witness,
            'point_floor_observed': True, 'same_capture_as_layout': True,
            'point_is_existing_fortress_entrance_proven': False,
            'native_pathfinding_proven': False, 'current_access_proven': False}
    if costs is not None:
        selected = found.json()['room_plan']
        optimization = found.json()['optimization']
        wall_targets = floor_targets = None
        if selected is not None:
            mask = costs.target_mask(selected, work)
            wall_targets = floor_targets = 0
            for index, tile in enumerate(capture.tiles):
                work()
                if not mask & (1 << index):
                    continue
                require(tile.presence == 2, 'costed target lacks visible raw evidence')
                _, shape, depth, magma, _, dig, building, units, *_ = tile.attributes
                require(shape in (2, 3) and not (depth or magma or dig or building or units),
                        'costed target lacks usable raw terrain evidence')
                wall_targets += int(shape == 2)
                floor_targets += int(shape == 3)
            require(wall_targets == optimization['score']['excavation_cost']
                    and wall_targets + floor_targets == selected['target_tiles'],
                    'ranked cost differs from complete original map evidence')
        body['excavation_evidence'] = {
            'objective': EXCAVATION_OBJECTIVE, 'epistemic_status': 'certified_derived',
            'map_witness': capture.witness, 'game_tick': capture.tick,
            'tile_costs_digest': costs.digest, 'observed_wall_targets': wall_targets,
            'observed_floor_targets': floor_targets,
            'selected_target_tiles': None if selected is None else selected['target_tiles'],
            'first_feasible_wall_targets': None if optimization['first_feasible_score'] is None
                else optimization['first_feasible_score']['excavation_cost'],
            'minimum_in_compiler_domain_proven': optimization['minimum_in_compiler_domain_proven'],
            'scope': optimization['scope'], 'corridors_rerouted_by_cost': False,
            'native_mining_eligibility_proven': False, 'actual_mining_work_observed': False,
            'current_terrain_proven': False}
        body['work_units'] = work.used
    result = ObservedLayout(canonical(body))
    require(len(result.encode()) <= MAX_OUTPUT, 'complete observed layout exceeds output bound')
    guard()
    return result
