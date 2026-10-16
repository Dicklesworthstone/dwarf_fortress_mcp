"""One padded observation -> complete room layout -> original-room dig handoff.

The existing solver, terrain reducer and handoff codec own their semantics.
This composition retains the parent payload and exact projection witnesses;
no source read, game effect, native authority or second observation is implied.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json

import excavation_observer as native
import map_capture_projection as projection
import room_layout as layout
import room_layout_observed as observed
import room_terrain as terrain
from furniture_plan import canonical, require
from room_excavation_handoff import RoomExcavationHandoff, bounded_json
from room_provisioning import RoomPlan
from room_site_mask import SiteMask

SCHEMA = 'dfmcp.room-layout-excavation/1'
POLICY = 'dfmcp.single-padded-capture-room-excavation/1'
MAX_OUTPUT = 512 * 1024


def prepare(raw: bytes, guard: layout.Guard = layout._idle
            ) -> tuple[dict, native.Region, layout._Work]:
    """Reject oversized/unrepresentable full context before native contact."""
    request, work = observed.prepare(raw, guard)
    domain = native.Region.from_json(request['region'])
    padded = native.Region(tuple(n - 1 for n in domain.origin), tuple(n + 2 for n in domain.size))
    require(padded.volume <= projection.MAX_TILES, 'full layout halo exceeds one bounded capture')
    work()
    return request, padded, work


def _halo_mask(capture: native.Capture, domain: native.Region,
               work: layout._Work) -> tuple[SiteMask, int]:
    """Unsafe-to-excavate walls can still be preserved as required room walls.

    Only remaining WALL targets require the 26-neighbor context. Already-open
    floors never become excavation targets or inherit an invented mining duty.
    This uses the terrain reducer's exact existing hidden/liquid/designation
    checks, not a new claim about pressure, aquifers or structural stability.
    """
    ox, oy, oz = capture.region.origin
    width, height, _ = capture.region.size

    def at(x: int, y: int, z: int) -> native.Tile:
        work()
        return capture.tiles[((z - oz) * height + y - oy) * width + x - ox]

    symbols, excluded = [], 0
    x0, y0, z = domain.origin
    for y in range(y0, y0 + domain.size[1]):
        for x in range(x0, x0 + domain.size[0]):
            tile = at(x, y, z)
            symbol = '.'
            if tile.presence == 2 and tile.attributes[1] == 2:
                for dz in (-1, 0, 1):
                    for dy in (-1, 0, 1):
                        for dx in (-1, 0, 1):
                            if (dx or dy or dz) and terrain._problem(at(x + dx, y + dy, z + dz)):
                                symbol = 'W'
                excluded += int(symbol == 'W')
            symbols.append(symbol)
    return SiteMask(domain.origin, domain.size, ''.join(symbols)), excluded


def _solve(raw: bytes, capture_raw: bytes, manifest: native.Manifest, address: str,
           guard: layout.Guard) -> tuple[ExcavationLayout, RoomExcavationHandoff | None]:
    request, padded, work = prepare(raw, guard)
    native.endpoint(address)
    capture = projection.decode(capture_raw, manifest, padded, work)
    require(capture.paused and (capture.folder, capture.site) == (request['world_folder'], request['site']),
            'room excavation layout requires the original paused fortress')
    domain = native.Region.from_json(request['region'])
    plan_view = projection.project(capture_raw, manifest, padded, domain, work)
    halo_mask, halo_excluded = _halo_mask(capture, domain, work)
    permitted = halo_mask
    if 'site_mask' in request:
        caller = SiteMask.from_json(request['site_mask'], work)
        cells = []
        for left, right in zip(caller.cells, halo_mask.cells):
            work()
            cells.append(observed.SYMBOLS[observed.USES[left] & observed.USES[right]])
        permitted = SiteMask(domain.origin, domain.size, ''.join(cells))
    # Retain the caller's original request separately. This narrower request is
    # explicitly derived from the SAME full source, never caller authority.
    narrowed = {**request, 'site_mask': permitted.json()}
    found = observed.solve(canonical(narrowed), plan_view.raw, manifest, work).json()
    handoff, report, room_view = None, None, None
    status = found['status']
    if status == 'layout_found':
        plan = RoomPlan.decode(canonical(found['layout']['room_plan']), work)
        selected = terrain.selection(plan, work)
        room_view = projection.project(capture_raw, manifest, padded, selected.region, work)
        report = terrain.survey(plan, room_view.raw, manifest, work)
        status = report['status']
        if status == 'excavation_proposed':
            handoff = RoomExcavationHandoff.create(plan, room_view.raw, manifest, address, work)
            require(handoff.json()['survey_digest'] == report['survey_digest'],
                    'room handoff differs from the complete original terrain survey')
        # Store the selected RoomPlan once, at layout.room_plan. The digest is
        # still the EXISTING full survey digest; rehydration is explicit below.
        report.pop('room_plan')
    layout_evidence = {key: value for key, value in found.items()
                       if key not in ('layout', 'request', 'capture_hex')}
    body = {'schema': SCHEMA, 'policy': POLICY, 'status': status,
            'request': request, 'map_endpoint': address,
            'source': {**capture.binding(), 'profile': 'map/1.5', 'game_tick': capture.tick,
                       'paused_at_capture': capture.paused, 'map_witness': capture.witness,
                       'capture_sha256': hashlib.sha256(capture_raw).hexdigest()},
            'capture_hex': capture_raw.hex(), 'layout': found['layout'],
            'layout_evidence': layout_evidence, 'terrain_survey': report,
            'halo_constraints': {'policy': 'existing_remaining_wall_halo_checks',
                                 'floor_forbidden_wall_cells': halo_excluded,
                                 'site_mask': halo_mask.json()},
            'projections': {'layout': plan_view.json(),
                            'terrain': None if room_view is None else room_view.json()},
            'handoff_digest': None if handoff is None else handoff.digest,
            'terrain_room_plan_location': 'layout.room_plan',
            'single_source_capture': True, 'additional_native_read': False,
            'native_acquisition_independently_attested': False,
            'native_excavation_eligibility_proven': False, 'room_completion_proven': False,
            'existing_effects_reconciled': False, 'replacement_effect_key_authorized': False,
            'mutation_authority_granted': False, 'game_effect_performed': False,
            'production_admitted': False, 'work_limit': work.maximum}
    result = ExcavationLayout(canonical(body))
    require(len(result.encode()) <= MAX_OUTPUT, 'complete excavation layout exceeds artifact bound')
    work()
    return result, handoff


@dataclass(frozen=True)
class ExcavationLayout:
    _body: bytes

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-layout-excavation/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        require(type(self._body) is bytes and 1 <= len(self._body) <= MAX_OUTPUT,
                'invalid excavation layout body')
        return {**json.loads(self._body), 'compilation_digest': self.digest}

    def encode(self) -> bytes:
        return canonical(self.json())

    @classmethod
    def decode(cls, raw: bytes, guard: layout.Guard = layout._idle) -> ExcavationLayout:
        return replay(raw, guard)[0]


def replay(raw: bytes, guard: layout.Guard = layout._idle
           ) -> tuple[ExcavationLayout, RoomExcavationHandoff | None]:
    """Recompute all decisions and projected bytes before returning a handoff."""
    value = bounded_json(raw, MAX_OUTPUT, guard, depth_limit=16)
    require(type(value) is dict and value.get('schema') == SCHEMA
            and value.get('policy') == POLICY
            and {'request', 'capture_hex', 'source', 'map_endpoint'} <= set(value),
            'invalid excavation layout envelope')
    text = value['capture_hex']
    require(type(text) is str and 2 <= len(text) <= 2 * projection.MAX_BYTES and len(text) % 2 == 0,
            'invalid original padded map capture bound')
    original = bytes.fromhex(text)
    require(original.hex() == text, 'noncanonical original padded map capture')
    manifest = native.Manifest.from_json(value['source']['manifest'])
    restored, handoff = _solve(canonical(value['request']), original, manifest,
                               value['map_endpoint'], guard)
    require(restored.encode() == raw, 'excavation layout differs from original request or source')
    guard()
    return restored, handoff


def solve(raw: bytes, capture_raw: bytes, manifest: native.Manifest, address: str,
          guard: layout.Guard = layout._idle
          ) -> tuple[ExcavationLayout, RoomExcavationHandoff | None]:
    """Return the complete result and optional EXISTING original-room handoff.

    The handoff is absent on blockage, empty work, no layout or an inconclusive
    node search. Callers may not substitute the full excavation proposal then.
    """
    return _solve(raw, capture_raw, manifest, address, guard)
