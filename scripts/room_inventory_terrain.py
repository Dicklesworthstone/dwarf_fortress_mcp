"""Re-site a room project against its original padded map and the selected census."""
from __future__ import annotations

from dataclasses import dataclass

import excavation_observer as native
import map_capture_projection as projection
import room_layout as layout
import room_layout_excavation as excavation
from furniture_plan import require
from room_excavation_handoff import RoomExcavationHandoff, bounded_json
import room_terrain as terrain


@dataclass(frozen=True)
class CapturedSite:
    original: dict
    constrained: dict
    source: dict
    map_bytes: bytes
    manifest: native.Manifest
    padded: native.Region
    address: str
    compilation_digest: str
    work: layout._Work

    @classmethod
    def replay(cls, raw: bytes, address: str, guard: layout.Guard) -> CapturedSite:
        envelope = bounded_json(raw, excavation.MAX_OUTPUT, guard, depth_limit=16)
        require(type(envelope) is dict and type(envelope.get('request')) is dict
                and type(envelope['request'].get('search')) is dict, 'missing archived layout budget')
        work = layout._Work(envelope['request']['search'].get('max_work'), guard)
        archive, _ = excavation.replay(raw, work)
        body = archive.json()
        require(body['map_endpoint'] == address, 'map and inventory endpoints differ')
        source = body['source']
        return cls(body['request'], body['layout']['request'], source, bytes.fromhex(body['capture_hex']),
                   native.Manifest.from_json(source['manifest']), native.Region.from_json(source['region']),
                   address, archive.digest, work)

    def check_census(self, operations, manifest) -> None:
        require(operations.paused and self.source['paused_at_capture']
                and (operations.tick, operations.folder, operations.site) ==
                    (self.source['game_tick'], self.source['folder'], self.source['site'])
                and manifest.software == (self.manifest.df_version, self.manifest.dfhack_version),
                'map and inventory require matching paused tick, fortress and software')

    def survey(self, plan, work):
        selection = terrain.selection(plan, work)
        view = projection.project(self.map_bytes, self.manifest, self.padded, selection.region, work)
        report = terrain.survey(plan, view.raw, self.manifest, work)
        return report, view

    def summarize(self, plan, work):
        survey, view = self.survey(plan, work)
        require(survey['status'] in ('excavation_proposed', 'terrain_shapes_satisfied_at_sample'),
                'selected room no longer fits its surveyed terrain')
        dig = None
        if survey['status'] == 'excavation_proposed':
            original = RoomExcavationHandoff.create(plan, view.raw, self.manifest, self.address, work)
            require(original.json()['survey_digest'] == survey['survey_digest'],
                    'excavation proposal differs from selected site')
            dig = original.json()
        survey.pop('room_plan')
        return survey, dig
