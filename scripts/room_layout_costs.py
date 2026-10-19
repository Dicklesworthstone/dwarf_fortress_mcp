"""Binary per-tile costs for complete room-layout comparison, never terrain facts.

A zero is an explicit planning cost, not proof of an existing floor. The observed
wrapper derives costs separately from original map bytes. Hard constraints and
corridor generation stay with RoomPlan. Bead: dfmcp-wp-pln01-spatial-blueprint-layout-pwd.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
from typing import Callable

from furniture_plan import canonical, integer, require
from room_layout_rank import Placement, Ranking

OBJECTIVE = 'minimum-excavation-cost/1'
POLICY = 'dfmcp.mrv-costed-room-layout/1'
MAX_CELLS = 1024
Score = tuple[int, int, int, tuple[Placement, ...]]
Guard = Callable[[], None]


@dataclass(frozen=True)
class TileCosts:
    origin: tuple[int, int, int]
    size: tuple[int, int, int]
    cells: str
    charged: int

    @classmethod
    def parse(cls, region: dict, cells: object, work: Guard) -> TileCosts:
        work()
        require(type(region) is dict and set(region) == {'origin', 'size'}, 'invalid cost region')
        require(type(region['origin']) is list and type(region['size']) is list
                and len(region['origin']) == len(region['size']) == 3, 'invalid cost coordinates')
        origin = tuple(integer(n, 1, 32766) for n in region['origin'])
        size = tuple(integer(n, 1, 128) for n in region['size'])
        require(size[2] == 1 and size[0] * size[1] <= MAX_CELLS
                and all(a + n <= 32767 for a, n in zip(origin, size)), 'cost domain exceeds bounds')
        require(type(cells) is str and len(cells) == size[0] * size[1],
                'costs must cover every original domain cell')
        charged = 0
        for index, cell in enumerate(cells):
            work()
            require(cell in '01', 'tile costs must be exact binary characters')
            if cell == '1':
                charged |= 1 << index
        return cls(origin, size, cells, charged)

    @property
    def digest(self) -> str:
        body = {'origin': self.origin, 'size': self.size, 'cells': self.cells}
        return hashlib.sha256(b'dfmcp-room-layout-tile-costs/1\0' + canonical(body)).hexdigest()

    def target_mask(self, body: dict, work: Guard) -> int:
        """Score the accepted compiler's EXACT union, including access/corridors."""
        parts = body['excavation_blueprint']['parts']
        require(type(parts) is list and 1 <= len(parts) <= 32, 'invalid complete costed blueprint')
        mask, count = 0, 0
        for part in parts:
            work()
            region = part['region']
            require(part['shape'] == 'floor' and type(region['origin']) is list
                    and type(region['size']) is list and len(region['origin']) == len(region['size']) == 3,
                    'costed plan must contain bounded floor cuboids')
            x, y, z = (integer(n, 1, 32766) for n in region['origin'])
            width, height, levels = (integer(n, 1, 128) for n in region['size'])
            require(z == self.origin[2] and levels == 1
                    and self.origin[0] <= x and x + width <= self.origin[0] + self.size[0]
                    and self.origin[1] <= y and y + height <= self.origin[1] + self.size[1],
                    'costed target outside original cost domain')
            count += width * height
            require(count <= 512, 'costed plan exceeds complete tile allowance')
            for yy in range(y, y + height):
                for xx in range(x, x + width):
                    work()
                    bit = 1 << ((yy - self.origin[1]) * self.size[0] + xx - self.origin[0])
                    require(not mask & bit, 'costed plan repeats a target')
                    mask |= bit
        require(type(body['target_tiles']) is int and count == body['target_tiles'],
                'costed plan lost original target count')
        return mask


class CostRanking:
    """One incumbent; compare cost, complete footprint, entry distance and ties."""
    def __init__(self, profiles, request: dict, work: Guard):
        self.geometry = Ranking(profiles, request['focus'], work)
        self.costs = TileCosts.parse(request['region'], request['excavation_costs'], work)
        self.work = work
        self.best = None
        self.best_score: Score | None = None
        self.first_score: Score | None = None
        self.first_digest = None
        self.feasible = self.updates = self.pruned = 0

    def lower_bound(self, chosen: dict, available: list) -> Score:
        # Independent per-area minima relax collisions and all additional
        # corridors. Disjoint original areas have additive nonnegative costs.
        # Every tuple coordinate is a lower bound; their minimizing candidates
        # need not coincide. A zero-cost corridor may add tiles, never subtract.
        footprint, distance, placements = self.geometry.lower_bound(chosen, available)
        options = {index: candidates for _, index, candidates in available}
        cost = 0
        for index in range(len(self.geometry.names)):
            candidates = (chosen[index],) if index in chosen else options[index]
            minimum = None
            for candidate in candidates:
                self.work()
                value = (candidate.floors & self.costs.charged).bit_count()
                minimum = value if minimum is None else min(minimum, value)
            require(minimum is not None, 'cost ranking encountered an empty domain')
            cost += minimum
        return cost, footprint, distance, placements

    def cannot_improve(self, chosen: dict, available: list) -> Score | None:
        if self.best_score is not None:
            bound = self.lower_bound(chosen, available)
            if bound >= self.best_score:
                self.pruned += 1
                return bound
        return None

    def consider(self, plan) -> None:
        self.work()
        body = plan.json()
        areas = body['intent']['areas']
        require(tuple(a['name'] for a in areas) == self.geometry.names,
                'costed plan changed original areas')
        targets = self.costs.target_mask(body, self.work)
        entries = {a['name']: a['entry_point'] for a in body['areas']}
        distance, placements = 0, []
        for area in areas:
            self.work()
            distance += sum(abs(p - f) for p, f in zip(entries[area['name']], self.geometry.focus))
            x, y, z = area['origin']
            placements.append((y, x, z, area.get('rotation', 0)))
        score = (targets & self.costs.charged).bit_count(), body['target_tiles'], distance, tuple(placements)
        require(score[1] >= self.geometry.base_tiles, 'costed plan lost original floor tiles')
        self.feasible += 1
        if self.first_score is None:
            self.first_score, self.first_digest = score, plan.digest
        if self.best_score is None or score < self.best_score:
            self.best, self.best_score = plan, score
            self.updates += 1
        self.work()

    @staticmethod
    def score_json(score: Score | None) -> dict | None:
        if score is None:
            return None
        return {'excavation_cost': score[0], 'target_tiles': score[1],
                'total_entry_distance': score[2], 'placements_yxz_rotation': [list(p) for p in score[3]]}

    def summary(self, complete: bool) -> dict:
        self.work()
        return {'objective': OBJECTIVE, 'comparison_order': [
                    'excavation_cost', 'target_tiles', 'total_entry_distance', 'placements_yxz_rotation'],
                'area_order': list(self.geometry.names), 'score': self.score_json(self.best_score),
                'first_feasible_score': self.score_json(self.first_score),
                'first_feasible_plan_digest': self.first_digest,
                'selected_plan_digest': None if self.best is None else self.best.digest,
                'feasible_layouts_evaluated': self.feasible, 'incumbent_updates': self.updates,
                'branches_pruned_by_lower_bound': self.pruned, 'unavoidable_area_tiles': self.geometry.base_tiles,
                'search_complete': complete, 'stop_reason': 'search_complete' if complete else 'node_limit_reached',
                'minimum_in_compiler_domain_proven': complete and self.best is not None,
                'scope': 'declared_origins_rotations_and_selected_deterministic_connection_compiler',
                'tile_costs_digest': self.costs.digest, 'cost_evidence': 'caller_declared_not_observed',
                'corridors_rerouted_by_cost': False, 'actual_mining_work_observed': False,
                'native_pathfinding_proven': False}
