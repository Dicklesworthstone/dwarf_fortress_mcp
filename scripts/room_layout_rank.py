"""Bounded whole-layout comparison; never score a partial project as a plan.

Internal helper for room_layout's explicit minimum-target-tiles objective.
The caller owns normalization, collision checks, complete RoomPlan compilation,
node limits and the ONE shared work/deadline guard. All geometry is advisory.
Bead: dfmcp-wp-pln01-spatial-blueprint-layout-pwd (WP-05/WP-10).
"""
from __future__ import annotations

import json
from typing import TYPE_CHECKING, Callable

from furniture_plan import require

if TYPE_CHECKING:
    from room_layout import _Candidate, _Profile
    from room_provisioning import RoomPlan

Placement = tuple[int, int, int, int]  # y, x, z, rotation, in lexical area order
Score = tuple[int, int, tuple[Placement, ...]]


class Ranking:
    """Retain one complete incumbent and a compact first-feasible comparison."""
    def __init__(self, profiles: tuple[_Profile, ...], focus: list[int],
                 work: Callable[[], None]):
        self.work = work
        self.focus = tuple(focus)
        self.base_tiles = sum(len(profile.floors) for profile in profiles)
        self.names = tuple(json.loads(profile.area)['name'] for profile in profiles)
        self.entries = []
        for profile in profiles:
            work()
            self.entries.append({json.loads(v.area).get('rotation', 0): v.entry
                                 for v in profile.variants or (profile,)})
        self.best: RoomPlan | None = None
        self.best_score: Score | None = None
        self.first_score: Score | None = None
        self.first_digest: str | None = None
        self.feasible = self.updates = self.pruned = 0

    def candidate_score(self, index: int, candidate: _Candidate) -> tuple[int, Placement]:
        self.work()
        entry = self.entries[index][candidate.rotation]
        distance = sum(abs(p + offset - focus) for p, offset, focus in
                       zip(candidate.origin, entry, self.focus))
        x, y, z = candidate.origin
        return distance, (y, x, z, candidate.rotation)

    def lower_bound(self, chosen: dict[int, _Candidate], available: list) -> Score:
        """Relax mutual conflicts and corridor costs, never relax the real plan.

        Every shape has a rotation-invariant tile count. Disjoint complete areas
        therefore need at least base_tiles, even before corridors/access. Each
        still-available area's independent minimum entry distance is a lower
        bound on its contribution. Its independent minimum placement tuple is
        also a lower bound when the summed distance happens to tie. Those minima
        need not describe a mutually feasible layout: they are ONLY a bound.
        """
        options = {index: candidates for _, index, candidates in available}
        total, placements = 0, []
        for index in range(len(self.names)):
            self.work()
            candidates = (chosen[index],) if index in chosen else options[index]
            minimum_distance, minimum_placement = None, None
            for candidate in candidates:
                distance, placement = self.candidate_score(index, candidate)
                if minimum_distance is None or distance < minimum_distance:
                    minimum_distance = distance
                if minimum_placement is None or placement < minimum_placement:
                    minimum_placement = placement
            require(minimum_distance is not None, 'ranking encountered an empty candidate domain')
            total += minimum_distance
            placements.append(minimum_placement)
        return self.base_tiles, total, tuple(placements)

    def cannot_improve(self, chosen: dict[int, _Candidate], available: list) -> Score | None:
        if self.best_score is None:
            return None
        bound = self.lower_bound(chosen, available)
        if bound >= self.best_score:
            self.pruned += 1
            return bound
        return None

    def consider(self, plan: RoomPlan) -> None:
        """Only called AFTER the existing complete compiler has accepted a leaf."""
        self.work()
        body = plan.json()
        areas = body['intent']['areas']
        require(tuple(area['name'] for area in areas) == self.names,
                'ranked plan omitted or substituted an original area')
        entries = {area['name']: area['entry_point'] for area in body['areas']}
        distance, placements = 0, []
        for area in areas:
            self.work()
            distance += sum(abs(p - focus) for p, focus in zip(entries[area['name']], self.focus))
            x, y, z = area['origin']
            placements.append((y, x, z, area.get('rotation', 0)))
        score = body['target_tiles'], distance, tuple(placements)
        require(score[0] >= self.base_tiles, 'complete ranked plan lost original floor tiles')
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
        return {'target_tiles': score[0], 'total_entry_distance': score[1],
                'placements_yxz_rotation': [list(p) for p in score[2]]}

    def summary(self, complete: bool) -> dict:
        self.work()
        return {'objective': 'minimum-target-tiles/1',
                'comparison_order': ['target_tiles', 'total_entry_distance', 'placements_yxz_rotation'],
                'area_order': list(self.names), 'score': self.score_json(self.best_score),
                'first_feasible_score': self.score_json(self.first_score),
                'first_feasible_plan_digest': self.first_digest,
                'selected_plan_digest': None if self.best is None else self.best.digest,
                'feasible_layouts_evaluated': self.feasible, 'incumbent_updates': self.updates,
                'branches_pruned_by_lower_bound': self.pruned,
                'unavoidable_area_tiles': self.base_tiles,
                'search_complete': complete,
                'stop_reason': 'search_complete' if complete else 'node_limit_reached',
                'minimum_in_compiler_domain_proven': complete and self.best is not None,
                'scope': 'declared_origins_rotations_and_selected_deterministic_connection_compiler',
                'actual_mining_work_observed': False, 'native_pathfinding_proven': False}
