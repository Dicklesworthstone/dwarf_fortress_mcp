"""Retain original room intent and raw survey evidence across excavation handoff.

An exported artifact is reproducible evidence, NOT an attestation of acquisition,
mutation authority, an effect inventory, or permission to replace uncertain work.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/WP-10.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json

import excavation_observer as e
import room_terrain as terrain
from furniture_allocation import Guard, idle
from furniture_plan import canonical, require, unique
from room_provisioning import RoomPlan

SCHEMA = 'dfmcp.room-excavation-handoff/1'
POLICY = 'dfmcp.original-room-survey-to-fixed-excavation/1'
MAX_BYTES = 262144
FIELDS = frozenset(('schema', 'policy', 'room_plan', 'map_endpoint', 'map_manifest',
                    'capture_hex', 'survey_digest', 'remaining_blueprint', 'remaining_mask_digest'))


def bounded_json(raw: bytes, maximum: int = MAX_BYTES, guard: Guard = idle,
                 *, depth_limit: int = 12) -> object:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'room handoff byte bound exceeded')
    depth, quoted, escaped = 0, False, False
    for offset, byte in enumerate(raw):
        if offset % 256 == 0:
            guard()
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            require(depth <= depth_limit, 'room handoff nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    def nonfinite(_value):
        raise ValueError('nonfinite room handoff number')
    result = json.loads(raw.decode('utf-8'), object_pairs_hook=unique, parse_constant=nonfinite)
    guard()
    return result


@dataclass(frozen=True)
class RoomExcavationHandoff:
    """Canonical immutable artifact. Consumers must use decode at trust boundaries."""
    _body: bytes

    @classmethod
    def create(cls, plan: RoomPlan, raw: bytes, manifest: e.Manifest, address: str,
               guard: Guard = idle) -> RoomExcavationHandoff:
        guard()
        e.endpoint(address)
        # The reducer regenerates the WHOLE intent and decodes native bytes. It
        # refuses unknown cells/hazards/wall deficits/capacity as a whole.
        result = terrain.survey(plan, raw, manifest, guard)
        require(result['status'] == 'excavation_proposed' and result['remaining_blueprint'] is not None,
                'handoff requires a complete nonempty unblocked excavation proposal')
        value = {'schema': SCHEMA, 'policy': POLICY, 'room_plan': result['room_plan'],
                 'map_endpoint': address, 'map_manifest': manifest.json(), 'capture_hex': raw.hex(),
                 'survey_digest': result['survey_digest'],
                 'remaining_blueprint': result['remaining_blueprint'],
                 'remaining_mask_digest': result['remaining_mask_digest']}
        body = canonical(value)
        require(len(body) <= MAX_BYTES - 128, 'complete room handoff exceeds artifact bound')
        handoff = cls(body)
        require(len(handoff.encode()) <= MAX_BYTES, 'complete room handoff exceeds artifact bound')
        guard()
        return handoff

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> RoomExcavationHandoff:
        value = bounded_json(raw, guard=guard)
        require(type(value) is dict and set(value) == FIELDS | {'handoff_digest'}
                and value['schema'] == SCHEMA and value['policy'] == POLICY, 'invalid room handoff fields')
        plan = RoomPlan.decode(canonical(value['room_plan']), guard)
        manifest = e.Manifest.from_json(value['map_manifest'])
        hexadecimal = value['capture_hex']
        require(type(hexadecimal) is str and 2 <= len(hexadecimal) <= 2 * terrain.MAX_CAPTURE_BYTES
                and len(hexadecimal) % 2 == 0, 'invalid room handoff capture bound')
        evidence = bytes.fromhex(hexadecimal)
        require(evidence.hex() == hexadecimal, 'noncanonical room handoff capture')
        restored = cls.create(plan, evidence, manifest, value['map_endpoint'], guard)
        # Re-hashing forged derived output is not sufficient. Regenerate every
        # output from the retained original intention and actual evidence bytes.
        require(restored.encode() == raw, 'room handoff differs from original intent or survey evidence')
        guard()
        return restored

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-excavation-handoff/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        require(type(self._body) is bytes and 1 <= len(self._body) <= MAX_BYTES, 'invalid room handoff body')
        return {**json.loads(self._body), 'handoff_digest': self.digest}

    def encode(self) -> bytes:
        return canonical(self.json())

    def plan(self, guard: Guard = idle) -> RoomPlan:
        return RoomPlan.decode(canonical(self.json()['room_plan']), guard)

    def source(self, guard: Guard = idle) -> dict:
        value = self.json()
        selected = terrain.selection(self.plan(guard), guard)
        observed = e.decode_capture(bytes.fromhex(value['capture_hex']),
                                    e.Manifest.from_json(value['map_manifest']), selected.region)
        guard()
        return {'endpoint': value['map_endpoint'], **observed.binding(), 'game_tick': observed.tick,
                'observation_witness': observed.witness, 'profile': 'map/1.5'}

    def check_native_source(self, address: str, manifest: dict, observed: dict,
                            guard: Guard = idle) -> None:
        """Check a dig decoder's source; never equate independent generations.

        The enclosing dig client still checks raw bytes, eligibility, incarnation,
        sequence, witnesses, confirmation and original-key receipts independently.
        Matching selectors cannot detect an unobserved same-tick save restore.
        """
        source = self.source(guard)
        e.endpoint(address)
        native_manifest = e.Manifest.from_json(manifest)
        require(address == source['endpoint'], 'dig endpoint differs from retained room survey')
        require(type(observed) is dict and {'folder', 'site', 'dimensions', 'tick', 'paused'} <= set(observed),
                'decoded native source required')
        for name in ('df_version', 'dfhack_version'):
            require(getattr(native_manifest, name) == source['manifest'][name], 'dig software differs from room survey')
        require(type(observed['folder']) is str and type(observed['site']) is int
                and type(observed['dimensions']) is list and len(observed['dimensions']) == 3,
                'invalid decoded native source identity')
        for size in observed['dimensions']:
            e.integer(size, 1, 32768)
        require(all(observed[key] == source[key] for key in ('folder', 'site', 'dimensions')),
                'dig fortress or dimensions differ from original room source')
        require(e.integer(observed['tick'], 0, e.MAX_TICK) >= source['game_tick']
                and observed['paused'] is True, 'dig source precedes room survey or is unpaused')
        guard()

    def summary(self, guard: Guard = idle) -> dict:
        value = self.json()
        result = {'schema': SCHEMA, 'handoff_digest': self.digest, 'survey_digest': value['survey_digest'],
                  'room_plan': value['room_plan'], 'survey_source': self.source(guard),
                  'remaining_blueprint': value['remaining_blueprint'],
                  'remaining_mask_digest': value['remaining_mask_digest'],
                  'complete_original_intent_retained': True, 'raw_survey_retained_in_handoff': True,
                  'map_and_dig_incarnations_independent': True, 'existing_effects_reconciled': False,
                  'replacement_effect_key_authorized': False, 'room_completion_proven': False,
                  'mutation_authority_granted': False, 'production_admitted': False}
        guard()
        return result
